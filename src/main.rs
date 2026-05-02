// unused: #![allow(unknown_literals)]

mod app_config;
mod bar_geometry;
mod cli_help;
mod config_gui;
mod layer_finder;
mod layer_system;
mod parallax_system;
mod perf_monitor;
mod video_decoder;
mod wallpaper;
mod wallpaper_detector;
mod wayland_renderer;
mod xray_animator;

use anyhow::{Context, Result};
use log::{error, info, warn};
use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{exit, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use app_config::Config;
use cli_help::print_help;
use config_gui::run_config_gui;
use serde::Deserialize;
use wayland_renderer::WaylandRenderer;

fn default_config_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(format!("{home}/.config/cava-bg/config.toml"))
}

fn pid_file_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(format!("{home}/.config/cava-bg/daemon.pid"))
}

fn daemon_log_path() -> PathBuf {
    let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
    PathBuf::from(format!("{home}/.config/cava-bg/daemon.log"))
}

fn runtime_outputs_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .map(|dir| dir.join("runtime-outputs.json"))
        .unwrap_or_else(|| PathBuf::from("/tmp/runtime-outputs.json"))
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

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create directory {}", parent.to_string_lossy()))?;
    }
    Ok(())
}

fn append_daemon_log_line(message: &str) {
    let log_path = daemon_log_path();
    if ensure_parent_dir(&log_path).is_err() {
        return;
    }

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(&log_path) {
        let _ = writeln!(file, "{message}");
        let _ = file.flush();
    }
}

fn daemon_debug_log(debug_mode: bool, message: &str) {
    info!("{message}");
    append_daemon_log_line(message);
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

fn parse_config_path(args: &[String]) -> PathBuf {
    if let Some(idx) = args.iter().position(|a| a == "--config") {
        if let Some(path) = args.get(idx + 1) {
            return PathBuf::from(path);
        }
    }
    default_config_path()
}

fn parse_output_arg(args: &[String]) -> Option<String> {
    let idx = args.iter().position(|a| a == "--output")?;
    args.get(idx + 1).cloned()
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

fn print_status(pid_file: &Path, config_path: &Path) -> Result<()> {
    let daemon_running = read_pid_file(pid_file)?
        .map(process_exists)
        .unwrap_or(false);
    println!(
        "Daemon: {}",
        if daemon_running { "running" } else { "stopped" }
    );
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

fn write_pid_file_atomic(pid_file: &Path, pid: u32, debug_mode: bool) -> Result<()> {
    ensure_parent_dir(pid_file)?;

    let pid_text = format!("{pid}\n");
    let tmp_file = pid_file.with_extension(format!("tmp.{}", std::process::id()));
    let max_attempts = 3;

    daemon_debug_log(
        debug_mode,
        &format!("Writing PID to file: {}", pid_file.display()),
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
                daemon_debug_log(debug_mode, "PID file written successfully");
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
                append_daemon_log_line(&format!(
                    "Attempt {}/{} failed while writing PID file {}: {:#}",
                    attempt,
                    max_attempts,
                    pid_file.display(),
                    err
                ));

                if attempt == max_attempts {
                    return Err(err);
                }

                thread::sleep(Duration::from_millis(150));
            }
        }
    }

    anyhow::bail!("Unexpected PID write loop exit")
}

fn check_single_instance(pid_file: &Path, debug_mode: bool) -> Result<bool> {
    let mut pid_locations = vec![pid_file.to_path_buf()];
    pid_locations.extend(legacy_pid_file_paths());

    for candidate in pid_locations {
        if !candidate.exists() {
            continue;
        }

        match read_pid_file(&candidate)? {
            Some(old_pid) if process_exists(old_pid) => {
                eprintln!(
                    "Another instance of cava-bg is already running (PID {}).",
                    old_pid
                );
                eprintln!("Use 'cava-bg off' to stop it.");
                return Ok(false);
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

    write_pid_file_atomic(pid_file, std::process::id(), debug_mode)?;
    Ok(true)
}

fn resolve_pid_file_for_off(primary: &Path) -> PathBuf {
    if primary.exists() {
        return primary.to_path_buf();
    }

    for legacy in legacy_pid_file_paths() {
        if legacy.exists() {
            warn!(
                "Using legacy PID file location: {}. It will be cleaned up automatically.",
                legacy.display()
            );
            return legacy;
        }
    }

    primary.to_path_buf()
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

fn kill_existing_instance(pid_file: &Path) -> Result<()> {
    let pid_path = resolve_pid_file_for_off(pid_file);
    let pid = match read_pid_file(&pid_path)? {
        Some(pid) => pid,
        None => {
            let _ = fs::remove_file(&pid_path);

            let discovered = find_cava_bg_processes()?;
            if !discovered.is_empty() {
                println!(
                    "PID file missing at {}. Found running cava-bg processes via fallback scan: {:?}",
                    pid_path.display(),
                    discovered
                );
                println!("Attempting to stop discovered processes...");

                let mut failed = Vec::new();
                for candidate in discovered {
                    if stop_pid_with_escalation(candidate) {
                        println!("Stopped cava-bg process PID {}.", candidate);
                    } else {
                        failed.push(candidate);
                    }
                }

                if failed.is_empty() {
                    return Ok(());
                }

                anyhow::bail!(
                    "Some cava-bg processes could not be stopped: {:?}. Try 'kill -9 <pid>' manually. For diagnosis run 'cava-bg on --debug' and inspect {}",
                    failed,
                    daemon_log_path().display()
                );
            }

            anyhow::bail!(
                "No running daemon was found (PID file is missing or empty at {}). Suggestion: run 'cava-bg on --debug' to diagnose startup issues and review {}",
                pid_path.display(),
                daemon_log_path().display()
            );
        }
    };

    if !process_exists(pid) {
        let _ = fs::remove_file(&pid_path);
        println!(
            "Found stale PID file at {} (PID {} is not running). Cleaned up.",
            pid_path.display(),
            pid
        );
        return Ok(());
    }

    if stop_pid_with_escalation(pid) {
        let _ = fs::remove_file(&pid_path);
        println!("cava-bg daemon stopped (PID {}).", pid);
        return Ok(());
    }

    anyhow::bail!(
        "Failed to stop PID {}. You may need to stop it manually with 'kill -9 {}'.",
        pid,
        pid
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

fn start_daemon(config_path: &Path, output_filter: Option<&str>) -> Result<()> {
    let exe = env::current_exe().context("Could not resolve the current executable")?;
    let devnull = File::options()
        .read(true)
        .write(true)
        .open("/dev/null")
        .context("Could not open /dev/null")?;

    let log_path = daemon_log_path();
    ensure_parent_dir(&log_path)?;
    let log_file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .with_context(|| format!("Could not open daemon log file {}", log_path.display()))?;

    let mut cmd = Command::new(exe);
    cmd.arg("__run")
        .arg("--config")
        .arg(config_path)
        .stdin(Stdio::from(devnull.try_clone()?))
        .stdout(Stdio::from(log_file.try_clone()?))
        .stderr(Stdio::from(log_file));

    if let Some(output) = output_filter {
        cmd.arg("--output").arg(output);
    }

    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let mut child = cmd.spawn().context("Could not start the daemon")?;
    let pid_file = pid_file_path();

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
    config_path: PathBuf,
    pid_file: PathBuf,
    debug_mode: bool,
    output_filter: Option<String>,
) -> Result<()> {
    ensure_config_exists(&config_path)?;

    let child_pid = std::process::id();
    daemon_debug_log(
        debug_mode,
        &format!("[DAEMON] Process started, PID: {}", child_pid),
    );

    if !check_single_instance(&pid_file, debug_mode)? {
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
    if let Some(output_name) = output_filter {
        config.general.preferred_outputs = vec![output_name.clone()];
        daemon_debug_log(
            debug_mode,
            &format!("[DAEMON] Output filter enabled for '{output_name}'"),
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

    daemon_debug_log(debug_mode, "Signal handlers installed for SIGINT/SIGTERM");
    daemon_debug_log(debug_mode, "[DAEMON] Entering main loop...");

    let mut restart_attempt: u64 = 0;
    while running.load(Ordering::SeqCst) {
        restart_attempt += 1;
        daemon_debug_log(debug_mode, "[DAEMON] Initializing Wayland connection...");
        daemon_debug_log(debug_mode, "[DAEMON] Starting renderer...");

        let render_result = panic::catch_unwind(AssertUnwindSafe(|| {
            let renderer =
                WaylandRenderer::new(config.clone(), running.clone(), Some(config_path.clone()));
            renderer.run()
        }));

        match render_result {
            Ok(Ok(())) => {
                if running.load(Ordering::SeqCst) {
                    daemon_debug_log(
                        debug_mode,
                        "[DAEMON] Renderer returned without error. Restarting keep-alive loop...",
                    );
                }
            }
            Ok(Err(err)) => {
                error!("Daemon renderer error: {:#}", err);
                daemon_debug_log(
                    debug_mode,
                    &format!("[DAEMON ERROR] Failed to start: {:#}", err),
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
                    &format!(
                        "[DAEMON ERROR] Failed to start: Daemon panicked: {}",
                        panic_message
                    ),
                );
            }
        }

        if !running.load(Ordering::SeqCst) {
            break;
        }

        daemon_debug_log(
            debug_mode,
            &format!(
                "[DAEMON] Keep-alive retry in 2s (attempt {})",
                restart_attempt
            ),
        );
        for _ in 0..20 {
            if !running.load(Ordering::SeqCst) {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    let _ = fs::remove_file(&pid_file);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let config_path = parse_config_path(&args);
    let pid_file = pid_file_path();

    let command = args.get(1).map(|s| s.as_str()).unwrap_or("on");
    let debug_mode = args.iter().any(|arg| arg == "--debug");
    let output_filter = parse_output_arg(&args);

    match command {
        "on" => {
            ensure_config_exists(&config_path)?;
            if debug_mode {
                println!("Running cava-bg in debug foreground mode (no daemon detach).");
                println!("Daemon log file: {}", daemon_log_path().display());
                run_foreground(config_path, pid_file, true, output_filter)?;
            } else {
                start_daemon(&config_path, output_filter.as_deref())?;
            }
        }
        "off" | "kill" => {
            kill_existing_instance(&pid_file)?;
        }
        "outputs" => {
            print_outputs(&config_path)?;
        }
        "status" => {
            print_status(&pid_file, &config_path)?;
        }
        "output-on" => {
            let Some(output) = output_filter.as_deref() else {
                anyhow::bail!("Use --output <name> with output-on");
            };
            set_output_enabled(&config_path, output, true)?;
        }
        "output-off" => {
            let Some(output) = output_filter.as_deref() else {
                anyhow::bail!("Use --output <name> with output-off");
            };
            set_output_enabled(&config_path, output, false)?;
        }
        "gui" => {
            ensure_config_exists(&config_path)?;
            run_config_gui(&config_path)?;
        }
        "__run" => {
            run_foreground(config_path, pid_file, false, output_filter)?;
        }
        "--config" => {
            run_foreground(config_path, pid_file, debug_mode, output_filter)?;
        }
        _ => {
            print_help();
            exit(0);
        }
    }

    Ok(())
}
