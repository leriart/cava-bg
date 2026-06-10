// unused: #![allow(unknown_literals)]

mod app_config;
mod bar_geometry;
mod cli;
mod config_gui;
mod layer_finder;
mod layer_system;
mod parallax_system;
mod perf_monitor;
mod supervisor;
mod video_decoder;
mod wallpaper;
mod wallpaper_detector;
mod wayland_renderer;
mod xray_animator;

use anyhow::{Context, Result};
use cli::Cli;
use log::{error, info, warn};
use sd_notify::NotifyState;
use serde::Deserialize;

use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::io::IntoRawFd;
use std::os::unix::process::CommandExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use app_config::Config;
use config_gui::run_config_gui;
use wayland_renderer::WaylandRenderer;

static HEARTBEAT_TICK: AtomicU64 = AtomicU64::new(0);

fn heartbeat_tick() {
    HEARTBEAT_TICK.fetch_add(1, Ordering::SeqCst);
}

fn default_config_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(format!("{home}/.config/cava-bg/config.toml"))
}

fn pid_file_path(output: Option<&str>) -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    match output {
        Some(name) => PathBuf::from(format!("{home}/.config/cava-bg/daemon.{name}.pid")),
        None => PathBuf::from(format!("{home}/.config/cava-bg/daemon.pid")),
    }
}

fn daemon_log_path(output: Option<&str>) -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    match output {
        Some(name) => PathBuf::from(format!("{home}/.config/cava-bg/daemon.{name}.log")),
        None => PathBuf::from(format!("{home}/.config/cava-bg/daemon.log")),
    }
}

fn runtime_outputs_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("runtime-outputs.json"))
        .unwrap_or_else(|| PathBuf::from("/tmp/runtime-outputs.json"))
}

fn start_watchdog_thread(running: Arc<AtomicBool>) {
    if let Some(interval) = sd_notify::watchdog_enabled() {
        let mut first_watchdog_error = true;
        thread::spawn(move || {
            let half = (interval / 2).max(Duration::from_millis(100));
            let mut last_tick = crate::HEARTBEAT_TICK.load(Ordering::SeqCst);

            while running.load(Ordering::SeqCst) {
                thread::sleep(half);
                let current_tick = crate::HEARTBEAT_TICK.load(Ordering::SeqCst);

                if current_tick == last_tick {
                    warn!(
                        "[SYSTEMD] Heartbeat stalled or system suspended, skipping watchdog ping"
                    );
                    continue;
                }
                last_tick = current_tick;

                if let Err(e) = sd_notify::notify(&[NotifyState::Watchdog]) {
                    if first_watchdog_error {
                        warn!("Failed to notify systemd (WATCHDOG=1): {}", e);
                        first_watchdog_error = false;
                    }
                }
            }
        });
        info!(
            "[SYSTEMD] Watchdog enabled (interval: {}ms)",
            interval.as_millis()
        );
    }
}

#[derive(Debug, Deserialize)]
struct RuntimeOutputInfo {
    name: String,
    index: u32,
    width: u32,
    height: u32,
    position: [i32; 2],
    configured: bool,
}

struct DaemonContext {
    config_path: PathBuf,
    pid_file: PathBuf,
    debug_mode: bool,
    systemd_mode: bool,
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create directory {}", parent.to_string_lossy()))?;
    }
    Ok(())
}

const MAX_LOG_SIZE: u64 = 1024 * 1024; // 1MB

fn rotate_log_file(log_path: &Path) -> bool {
    if let Ok(metadata) = fs::metadata(log_path) {
        if metadata.len() > MAX_LOG_SIZE {
            let rotated = PathBuf::from(format!("{}.old", log_path.display()));
            return fs::rename(log_path, &rotated).is_ok();
        }
    }
    false
}

fn rotate_daemon_log(output: Option<&str>) {
    let log_path = daemon_log_path(output);
    if rotate_log_file(&log_path) {
        if let Ok(new_file) = OpenOptions::new().create(true).append(true).open(&log_path) {
            let fd = new_file.into_raw_fd();
            unsafe {
                libc::dup2(fd, libc::STDOUT_FILENO);
                libc::dup2(fd, libc::STDERR_FILENO);
                libc::close(fd);
            }
        }
    }
}

fn append_daemon_log_line(message: &str, output: Option<&str>) {
    let log_path = daemon_log_path(output);
    if ensure_parent_dir(&log_path).is_err() {
        return;
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(file, "{message}");
        let _ = file.flush();
    }
}

fn daemon_debug_log(debug_mode: bool, systemd_mode: bool, message: &str, output: Option<&str>) {
    info!("{message}");
    if !systemd_mode {
        append_daemon_log_line(message, output);
    }
    if debug_mode {
        eprintln!("{message}");
    }
}

fn legacy_pid_file_paths() -> Vec<PathBuf> {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    vec![
        PathBuf::from(format!("{home}/.config/cava-bg/cava-bg.pid")),
        PathBuf::from("/tmp/cava-bg.pid"),
    ]
}

fn read_runtime_outputs(config_path: &Path) -> Result<Vec<RuntimeOutputInfo>> {
    let path = runtime_outputs_path(config_path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let content = fs::read_to_string(&path)
        .with_context(|| format!("Could not read runtime output state {}", path.display()))?;
    let outputs = serde_json::from_str::<Vec<RuntimeOutputInfo>>(&content)
        .with_context(|| format!("Could not parse runtime output state {}", path.display()))?;
    Ok(outputs)
}

fn print_outputs(config_path: &Path) -> Result<()> {
    let outputs = read_runtime_outputs(config_path)?;
    if outputs.is_empty() {
        println!("No outputs discovered yet. Start the daemon first and try again.");
        return Ok(());
    }

    println!("Detected outputs (runtime):");
    for output in outputs {
        println!(
            "- {} (index: {}, {}x{}, pos: {},{}, configured: {})",
            output.name,
            output.index,
            output.width,
            output.height,
            output.position[0],
            output.position[1],
            output.configured
        );
    }
    Ok(())
}

fn print_status(config_path: &Path, systemd_mode: bool) -> Result<()> {
    if systemd_mode {
        println!("Use 'systemctl --user status cava-bg' to check service status.");
        return Ok(());
    }

    let supervisor_file = pid_file_path(None);
    let supervisor_running = read_pid_file(&supervisor_file)?
        .map(|pid| process_exists(pid) && is_cava_bg_process(pid) && is_supervisor_process(pid))
        .unwrap_or(false);

    if supervisor_running {
        println!("Supervisor: running");
        let supervisor_pid = read_pid_file(&supervisor_file)?.unwrap_or(0);
        println!("  PID: {}", supervisor_pid);
    } else {
        // Check main daemon pid + legacy + per-output pid files
        let pid_files = collect_pid_files();
        let main_pid = read_pid_file(&supervisor_file)?;
        let main_running = main_pid
            .map(|pid| process_exists(pid) && is_cava_bg_process(pid))
            .unwrap_or(false);
        let any_running = main_running
            || pid_files.iter().any(|pf| {
                read_pid_file(pf)
                    .ok()
                    .flatten()
                    .map(|pid| process_exists(pid) && is_cava_bg_process(pid))
                    .unwrap_or(false)
            });

        if let Some(pid) = main_pid.filter(|_| main_running) {
            println!("Daemon: running (no supervisor, PID {})", pid);
        } else if any_running {
            println!("Daemon: running (legacy/per-output only)");
        } else {
            println!("Daemon: stopped");
        }
    }

    // Show per-output children
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_dir = PathBuf::from(format!("{home}/.config/cava-bg"));
    if let Ok(entries) = fs::read_dir(&config_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if let Some(output) = name
                    .strip_prefix("daemon.")
                    .and_then(|n| n.strip_suffix(".pid"))
                {
                    if let Some(pid) = read_pid_file(&path).ok().flatten() {
                        let running = process_exists(pid) && is_cava_bg_process(pid);
                        println!(
                            "  Output '{}': {} (PID {})",
                            output,
                            if running { "running" } else { "stopped" },
                            pid
                        );
                    }
                }
            }
        }
    }

    print_outputs(config_path)?;
    Ok(())
}

fn set_output_enabled(config_path: &Path, output: &str, enabled: bool) -> Result<()> {
    ensure_config_exists(config_path)?;
    let content = fs::read_to_string(config_path)
        .with_context(|| format!("Could not read {}", config_path.display()))?;
    let mut cfg: Config = toml::from_str(&content)
        .with_context(|| format!("Could not parse {}", config_path.display()))?;
    cfg.normalize_compat_fields();

    let entry = cfg.output.entry(output.to_string()).or_default();
    entry.enabled = Some(enabled);
    entry.name = Some(output.to_string());

    let serialized = toml::to_string_pretty(&cfg).context("Could not serialize config")?;
    fs::write(config_path, serialized)
        .with_context(|| format!("Could not save {}", config_path.display()))?;

    println!(
        "Output '{}' {} in config.",
        output,
        if enabled { "enabled" } else { "disabled" }
    );
    Ok(())
}

fn process_exists(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        // Treat zombie processes as non-running so `off` can clean stale PID files.
        let stat_path = format!("/proc/{pid}/stat");
        if let Ok(stat) = fs::read_to_string(stat_path) {
            let is_zombie = stat
                .split_whitespace()
                .nth(2)
                .map(|state| state == "Z")
                .unwrap_or(false);
            if is_zombie {
                return false;
            }
        }
        return true;
    }

    let errno = std::io::Error::last_os_error().raw_os_error();
    matches!(errno, Some(libc::EPERM))
}

/// Checks if the given PID belongs to the cava-bg daemon by inspecting its command-line arguments.
pub fn is_cava_bg_process(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let cmdline_bytes = match std::fs::read(&cmdline_path) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let args: Vec<&str> = cmdline_bytes
        .split(|&b| b == 0)
        .filter_map(|b| std::str::from_utf8(b).ok())
        .filter(|s| !s.is_empty())
        .collect();

    if args.len() < 2 {
        return false;
    }

    args.iter().any(|&arg| {
        [
            "on",
            "restart",
            "__run",
            "__supervisor",
            "--debug",
            "--config",
        ]
        .contains(&arg)
    }) && !args.iter().any(|&arg| {
        [
            "off",
            "kill",
            "status",
            "outputs",
            "output-on",
            "output-off",
            "gui",
            "help",
        ]
        .contains(&arg)
    })
}

fn is_supervisor_process(pid: i32) -> bool {
    if pid <= 0 {
        return false;
    }

    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let cmdline_bytes = match std::fs::read(&cmdline_path) {
        Ok(bytes) => bytes,
        Err(_) => return false,
    };

    let args: Vec<&str> = cmdline_bytes
        .split(|&b| b == 0)
        .filter_map(|b| std::str::from_utf8(b).ok())
        .filter(|s| !s.is_empty())
        .collect();

    if args.len() < 2 {
        return false;
    }

    args.contains(&"__supervisor")
}

fn read_pid_file(pid_file: &Path) -> Result<Option<i32>> {
    if !pid_file.exists() {
        return Ok(None);
    }

    let mut file = File::open(pid_file)
        .with_context(|| format!("Could not open PID file {}", pid_file.display()))?;
    let mut pid_str = String::new();
    file.read_to_string(&mut pid_str)
        .with_context(|| format!("Could not read PID file {}", pid_file.display()))?;

    let trimmed = pid_str.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    let pid = trimmed.parse::<i32>().with_context(|| {
        format!(
            "PID file {} contains an invalid PID: '{}'",
            pid_file.display(),
            trimmed
        )
    })?;

    Ok(Some(pid))
}

fn write_pid_file_atomic(
    pid_file: &Path,
    pid: u32,
    debug_mode: bool,
    systemd_mode: bool,
    output: Option<&str>,
) -> Result<()> {
    ensure_parent_dir(pid_file)?;

    let pid_text = format!("{pid}\n");
    let tmp_file = pid_file.with_extension(format!("tmp.{}", std::process::id()));
    let max_attempts = 3;

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        &format!("Writing PID to file: {}", pid_file.display()),
        output,
    );

    for attempt in 1..=max_attempts {
        let write_result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .create(true)
                .write(true)
                .truncate(true)
                .open(&tmp_file)
                .with_context(|| {
                    format!("Could not open temporary PID file {}", tmp_file.display())
                })?;

            file.write_all(pid_text.as_bytes()).with_context(|| {
                format!("Could not write temporary PID file {}", tmp_file.display())
            })?;
            file.flush().with_context(|| {
                format!("Could not flush temporary PID file {}", tmp_file.display())
            })?;
            file.sync_all().with_context(|| {
                format!("Could not sync temporary PID file {}", tmp_file.display())
            })?;

            fs::rename(&tmp_file, pid_file).with_context(|| {
                format!(
                    "Could not atomically replace PID file {}",
                    pid_file.display()
                )
            })?;

            let written = fs::read_to_string(pid_file)
                .with_context(|| format!("Could not verify PID file {}", pid_file.display()))?;
            if written.trim() != pid.to_string() {
                anyhow::bail!(
                    "PID verification failed for {} (expected {}, found '{}')",
                    pid_file.display(),
                    pid,
                    written.trim()
                );
            }

            Ok(())
        })();

        match write_result {
            Ok(_) => {
                daemon_debug_log(
                    debug_mode,
                    systemd_mode,
                    "PID file written successfully",
                    output,
                );
                return Ok(());
            }
            Err(err) => {
                let _ = fs::remove_file(&tmp_file);
                warn!(
                    "Attempt {}/{} failed while writing PID file {}: {:#}",
                    attempt,
                    max_attempts,
                    pid_file.display(),
                    err
                );
                if !systemd_mode {
                    append_daemon_log_line(
                        &format!(
                            "Attempt {}/{} failed while writing PID file {}: {:#}",
                            attempt,
                            max_attempts,
                            pid_file.display(),
                            err
                        ),
                        output,
                    );
                }

                if attempt == max_attempts {
                    return Err(err);
                }

                thread::sleep(Duration::from_millis(150));
            }
        }
    }

    anyhow::bail!("Unexpected PID write loop exit")
}

fn check_single_instance(
    pid_file: &Path,
    debug_mode: bool,
    systemd_mode: bool,
    output: Option<&str>,
) -> Result<bool> {
    let mut pid_locations = vec![pid_file.to_path_buf()];
    pid_locations.extend(legacy_pid_file_paths());

    for candidate in pid_locations {
        if !candidate.exists() {
            continue;
        }

        match read_pid_file(&candidate)? {
            Some(old_pid) if process_exists(old_pid) && is_cava_bg_process(old_pid) => {
                eprintln!(
                    "Another instance of cava-bg is already running (PID {}).",
                    old_pid
                );
                eprintln!("Use 'cava-bg off' to stop it.");
                return Ok(false);
            }
            Some(old_pid) if process_exists(old_pid) => {
                warn!(
                    "Removing stale PID file {} (PID {} belongs to a different process)",
                    candidate.display(),
                    old_pid
                );
                let _ = fs::remove_file(&candidate);
            }
            Some(old_pid) => {
                warn!(
                    "Removing stale PID file {} (PID {} no longer exists)",
                    candidate.display(),
                    old_pid
                );
                let _ = fs::remove_file(&candidate);
            }
            None => {
                warn!("Removing empty PID file {}", candidate.display());
                let _ = fs::remove_file(&candidate);
            }
        }
    }

    write_pid_file_atomic(
        pid_file,
        std::process::id(),
        debug_mode,
        systemd_mode,
        output,
    )?;
    Ok(true)
}

fn terminate_pid(pid: i32) {
    unsafe {
        libc::kill(pid, libc::SIGTERM);
    }
}

fn kill_pid_hard(pid: i32) {
    unsafe {
        libc::kill(pid, libc::SIGKILL);
    }
}

fn find_cava_bg_processes() -> Result<Vec<i32>> {
    let output = Command::new("ps")
        .args(["aux"])
        .output()
        .context("Could not execute 'ps aux' to search for cava-bg processes")?;

    if !output.status.success() {
        anyhow::bail!("'ps aux' returned a non-zero status while searching for cava-bg");
    }

    let current_pid = std::process::id() as i32;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut pids = Vec::new();

    for line in stdout.lines() {
        if !line.contains("cava-bg") {
            continue;
        }

        if line.contains("grep cava-bg") || line.contains("cava-bg off") {
            continue;
        }

        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 2 {
            continue;
        }

        if let Ok(pid) = parts[1].parse::<i32>() {
            if pid > 0 && pid != current_pid && !pids.contains(&pid) {
                pids.push(pid);
            }
        }
    }

    Ok(pids)
}

fn stop_pid_with_escalation(pid: i32) -> bool {
    if !process_exists(pid) {
        return true;
    }

    terminate_pid(pid);

    for _ in 0..20 {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }

    warn!("PID {} did not exit after SIGTERM. Sending SIGKILL.", pid);
    kill_pid_hard(pid);

    for _ in 0..10 {
        if !process_exists(pid) {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }

    false
}

fn collect_pid_files() -> Vec<PathBuf> {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let config_dir = PathBuf::from(format!("{home}/.config/cava-bg"));
    let mut files = Vec::new();

    let basic = pid_file_path(None);
    if basic.exists() {
        files.push(basic);
    }

    for legacy in legacy_pid_file_paths() {
        if legacy.exists() && !files.contains(&legacy) {
            files.push(legacy);
        }
    }

    if let Ok(entries) = fs::read_dir(&config_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("daemon.") && name.ends_with(".pid") && !files.contains(&path) {
                    files.push(path);
                }
            }
        }
    }

    files
}

fn run_systemctl(args: &[&str]) -> Result<std::process::Output> {
    let timeout = Duration::from_secs(10);
    let mut child = Command::new("systemctl")
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn systemctl")?;

    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().context("systemctl command failed"),
            Ok(None) => {
                if start.elapsed() > timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    anyhow::bail!("systemctl command timed out after 10s");
                }
                thread::sleep(Duration::from_millis(50));
            }
            Err(e) => anyhow::bail!("systemctl wait failed: {}", e),
        }
    }
}

fn systemd_unit_is_active() -> bool {
    match run_systemctl(&["--user", "is-active", "cava-bg.service"]) {
        Ok(output) => {
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            status == "active"
        }
        Err(e) => {
            warn!("systemctl check failed (assuming inactive): {}", e);
            false
        }
    }
}

fn kill_existing_instance(systemd_mode: bool) -> Result<()> {
    if systemd_unit_is_active() {
        let output = run_systemctl(&["--user", "stop", "cava-bg.service"])?;
        if output.status.success() {
            println!("cava-bg.service stopped via systemctl.");
            return Ok(());
        }
        anyhow::bail!("systemctl --user stop cava-bg.service failed.");
    }

    if systemd_mode {
        anyhow::bail!("Use 'systemctl --user stop cava-bg' instead.");
    }

    let pid_files = collect_pid_files();

    let mut process_found = false;
    let mut killed_any = false;
    let mut failed = Vec::new();

    for pid_file in &pid_files {
        let Some(pid) = read_pid_file(pid_file)? else {
            let _ = fs::remove_file(pid_file);
            continue;
        };

        let alive = process_exists(pid) && is_cava_bg_process(pid);
        if !alive {
            let _ = fs::remove_file(pid_file);
            continue;
        }

        process_found = true;

        if stop_pid_with_escalation(pid) {
            killed_any = true;
            println!(
                "cava-bg process stopped (PID {}, pidfile: {}).",
                pid,
                pid_file.display()
            );
            let _ = fs::remove_file(pid_file);
        } else {
            failed.push(pid);
        }
    }

    if !process_found {
        let discovered = find_cava_bg_processes()?;
        if !discovered.is_empty() {
            println!(
                "No PID files found. Found running cava-bg processes via fallback scan: {:?}",
                discovered
            );
            println!("Attempting to stop discovered processes...");

            for candidate in discovered {
                if stop_pid_with_escalation(candidate) {
                    killed_any = true;
                    println!("Stopped cava-bg process PID {}.", candidate);
                } else {
                    failed.push(candidate);
                }
            }
        }
    }

    if killed_any {
        return Ok(());
    }

    if !failed.is_empty() {
        anyhow::bail!(
            "Some cava-bg processes could not be stopped: {:?}. Try 'kill -9 <pid>' manually. For diagnosis run 'cava-bg on --debug' and inspect {}",
            failed,
            daemon_log_path(None).display()
        );
    }

    anyhow::bail!(
        "No running daemon was found. Suggestion: run 'cava-bg on --debug' to diagnose startup issues and review {}",
        daemon_log_path(None).display()
    )
}

pub fn create_default_config(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    let cfg = crate::app_config::Config::default();
    let toml_str = toml::to_string_pretty(&cfg).context("Failed to serialize default config")?;
    fs::write(path, &toml_str)?;
    info!("Created default config at {:?}", path);
    Ok(())
}

fn ensure_config_exists(config_path: &Path) -> Result<()> {
    if !config_path.exists() {
        create_default_config(config_path)
            .with_context(|| format!("Failed to create default config at {:?}", config_path))?;
    }
    Ok(())
}

fn start_daemon(config_path: &Path, output_filter: Option<&str>, supervisor: bool) -> Result<()> {
    let exe = env::current_exe().context("Could not resolve the current executable")?;
    let devnull = File::options()
        .read(true)
        .write(true)
        .open("/dev/null")
        .context("Could not open /dev/null")?;

    let log_path = daemon_log_path(output_filter);
    ensure_parent_dir(&log_path)?;
    rotate_log_file(&log_path);
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("Could not open daemon log file {}", log_path.display()))?;

    let mut cmd = Command::new(exe);
    if let Some(output) = output_filter {
        cmd.arg("__run")
            .arg("--config")
            .arg(config_path)
            .arg("--output")
            .arg(output);
    } else if supervisor {
        cmd.arg("__supervisor").arg("--config").arg(config_path);
    } else {
        cmd.arg("__run").arg("--config").arg(config_path);
    }
    cmd.stdin(Stdio::from(devnull.try_clone()?))
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file));

    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().context("Could not start the daemon")?;
    let pid_file = pid_file_path(output_filter);

    for _ in 0..30 {
        if let Some(status) = child.try_wait().context("Could not poll daemon process")? {
            anyhow::bail!(
                "Daemon exited too early with status {}. Check logs at {}",
                status,
                log_path.display()
            );
        }

        if let Some(pid) = read_pid_file(&pid_file)? {
            println!("cava-bg daemon started in background (daemon PID {}).", pid);
            println!("PID file: {}", pid_file.display());
            println!("Log file: {}", log_path.display());
            return Ok(());
        }

        thread::sleep(Duration::from_millis(100));
    }

    println!(
        "Daemon launcher PID {} started, but PID file is still pending at {}.",
        child.id(),
        pid_file.display()
    );
    println!("Check logs at {}", log_path.display());
    Ok(())
}

fn run_foreground(
    daemon_context: DaemonContext,
    output_filter: Option<String>,
    supervised: bool,
) -> Result<()> {
    if supervised {
        unsafe {
            libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
        }
    }

    let config_path = daemon_context.config_path;
    let pid_file = daemon_context.pid_file;
    let debug_mode = daemon_context.debug_mode;
    let systemd_mode = daemon_context.systemd_mode;

    ensure_config_exists(&config_path)?;

    let child_pid = std::process::id();
    daemon_debug_log(
        debug_mode,
        systemd_mode,
        &format!("[DAEMON] Process started, PID: {}", child_pid),
        output_filter.as_deref(),
    );

    if systemd_mode {
        info!("[SYSTEMD] Running in systemd mode (PID {child_pid})");
    }
    if !check_single_instance(
        &pid_file,
        debug_mode,
        systemd_mode,
        output_filter.as_deref(),
    )? {
        std::process::exit(1);
    }

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Unable to read config file: {:?}", config_path))?;
    let mut config: Config = match toml::from_str(&config_str) {
        Ok(c) => c,
        Err(e) => {
            eprintln!(
                "Warning: Could not parse config ({}). Creating a fresh config...",
                e
            );
            let backup = config_path.with_extension("toml.legacy");
            if let Err(copy_err) = fs::copy(&config_path, &backup) {
                eprintln!(
                    "Warning: Could not back up old config to {:?}: {}",
                    backup, copy_err
                );
            } else {
                eprintln!("Backed up old config to {:?}", backup);
            }
            create_default_config(&config_path)?;
            let fresh_str = fs::read_to_string(&config_path)
                .with_context(|| format!("Unable to read fresh config: {:?}", config_path))?;
            toml::from_str(&fresh_str)
                .with_context(|| "Fresh config failed to parse (this shouldn\'t happen)")?
        }
    };
    config.normalize_compat_fields();
    if let Some(output_name) = &output_filter {
        config.general.preferred_outputs = vec![output_name.clone()];
        daemon_debug_log(
            debug_mode,
            systemd_mode,
            &format!("[DAEMON] Output filter enabled for '{output_name}'"),
            output_filter.as_deref(),
        );
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let pid_cleanup = pid_file.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        let _ = fs::remove_file(&pid_cleanup);
    })
    .expect("Error setting signal handler for SIGINT/SIGTERM");

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        "Signal handlers installed for SIGINT/SIGTERM",
        output_filter.as_deref(),
    );

    if systemd_mode && !supervised {
        if let Err(e) = sd_notify::notify(&[NotifyState::Ready]) {
            warn!("Failed to notify systemd (READY=1): {}", e);
        } else {
            info!("[SYSTEMD] Notified systemd: READY=1");
        }

        start_watchdog_thread(running.clone());
    }

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        "[DAEMON] Entering main loop...",
        output_filter.as_deref(),
    );

    if !systemd_mode {
        rotate_daemon_log(output_filter.as_deref());
    }

    if !debug_mode && !systemd_mode {
        let log_running = running.clone();
        let output_filter_clone = output_filter.clone();
        thread::spawn(move || {
            while log_running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(60));
                rotate_daemon_log(output_filter_clone.as_deref());
            }
        });
    }

    let mut restart_attempt: u64 = 0;
    while running.load(Ordering::SeqCst) {
        restart_attempt += 1;
        daemon_debug_log(
            debug_mode,
            systemd_mode,
            "[DAEMON] Initializing Wayland connection...",
            output_filter.as_deref(),
        );
        daemon_debug_log(
            debug_mode,
            systemd_mode,
            "[DAEMON] Starting renderer...",
            output_filter.as_deref(),
        );

        let render_result = panic::catch_unwind(AssertUnwindSafe(|| {
            let renderer = WaylandRenderer::new(
                config.clone(),
                running.clone(),
                Some(config_path.clone()),
                supervised,
            );
            renderer.run()
        }));

        match render_result {
            Ok(Ok(())) => {
                if running.load(Ordering::SeqCst) {
                    daemon_debug_log(
                        debug_mode,
                        systemd_mode,
                        "[DAEMON] Renderer returned without error. Restarting keep-alive loop...",
                        output_filter.as_deref(),
                    );
                }
            }
            Ok(Err(err)) => {
                error!("Daemon renderer error: {:#}", err);
                daemon_debug_log(
                    debug_mode,
                    systemd_mode,
                    &format!("[DAEMON ERROR] Failed to start: {:#}", err),
                    output_filter.as_deref(),
                );
            }
            Err(payload) => {
                let panic_message = if let Some(msg) = payload.downcast_ref::<&str>() {
                    (*msg).to_string()
                } else if let Some(msg) = payload.downcast_ref::<String>() {
                    msg.clone()
                } else {
                    "Unknown panic payload".to_string()
                };

                error!("Daemon panicked: {}", panic_message);
                daemon_debug_log(
                    debug_mode,
                    systemd_mode,
                    &format!(
                        "[DAEMON ERROR] Failed to start: Daemon panicked: {}",
                        panic_message
                    ),
                    output_filter.as_deref(),
                );
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }

        daemon_debug_log(
            debug_mode,
            systemd_mode,
            &format!(
                "[DAEMON] Keep-alive retry in 2s (attempt {})",
                restart_attempt
            ),
            output_filter.as_deref(),
        );
        for _ in 0..20 {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    if systemd_mode && !supervised {
        if let Err(e) = sd_notify::notify(&[NotifyState::Stopping]) {
            warn!("Failed to notify systemd (STOPPING=1): {}", e);
        }
    }
    let _ = fs::remove_file(&pid_file);
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    let systemd_mode = cli.systemd;
    if systemd_mode && env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    env_logger::init();

    let config_path = cli
        .config
        .as_ref()
        .map(PathBuf::from)
        .unwrap_or_else(default_config_path);
    let pid_file = pid_file_path(cli.output.as_deref());

    let daemon_context = DaemonContext {
        config_path: config_path.clone(),
        pid_file,
        debug_mode: cli.debug,
        systemd_mode,
    };

    match cli.command {
        None if cli.config.is_some() => {
            run_foreground(daemon_context, cli.output, false)?;
        }
        None | Some(cli::Command::On) => {
            ensure_config_exists(&config_path)?;
            if cli.debug || systemd_mode {
                if cli.debug {
                    println!("Running cava-bg in debug foreground mode (no daemon detach).");
                    if !systemd_mode {
                        println!("Daemon log file: {}", daemon_log_path(None).display());
                    }
                }
                if cli.supervisor {
                    supervisor::run_supervisor(daemon_context)?;
                } else {
                    if systemd_mode {
                        info!("Starting in systemd mode (foreground, journald logging)");
                    }
                    run_foreground(daemon_context, cli.output, false)?;
                }
            } else {
                start_daemon(&config_path, cli.output.as_deref(), cli.supervisor)?;
            }
        }
        Some(cli::Command::Off) | Some(cli::Command::Kill) => {
            kill_existing_instance(systemd_mode)?;
        }
        Some(cli::Command::Restart) => {
            if systemd_unit_is_active() {
                let output = run_systemctl(&["--user", "restart", "cava-bg.service"])?;
                if output.status.success() {
                    println!("cava-bg.service restarted via systemctl.");
                    return Ok(());
                }
                anyhow::bail!("systemctl --user restart cava-bg.service failed.");
            }

            if systemd_mode {
                anyhow::bail!("Use 'systemctl --user restart cava-bg' instead.");
            }

            kill_existing_instance(systemd_mode)?;
            ensure_config_exists(&config_path)?;
            if cli.debug {
                println!("Running cava-bg in debug foreground mode (no daemon detach).");
                println!("Daemon log file: {}", daemon_log_path(None).display());
                run_foreground(daemon_context, cli.output, false)?;
            } else {
                start_daemon(&config_path, cli.output.as_deref(), cli.supervisor)?;
            }
        }
        Some(cli::Command::Outputs) => {
            print_outputs(&config_path)?;
        }
        Some(cli::Command::Status) => {
            print_status(&config_path, systemd_mode)?;
        }
        Some(cli::Command::OutputOn { output }) => {
            set_output_enabled(&config_path, &output, true)?;
        }
        Some(cli::Command::OutputOff { output }) => {
            set_output_enabled(&config_path, &output, false)?;
        }
        Some(cli::Command::Gui) => {
            ensure_config_exists(&config_path)?;
            run_config_gui(&config_path)?;
        }
        Some(cli::Command::__Run) => {
            if cli.supervised && cli.output.is_none() {
                anyhow::bail!("--supervised requires --output <name>");
            }

            run_foreground(daemon_context, cli.output, cli.supervised)?;
        }
        Some(cli::Command::__Supervisor) => {
            supervisor::run_supervisor(daemon_context)?;
        }
    }

    Ok(())
}
