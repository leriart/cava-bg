use anyhow::{Context, Result};
use log::{error, info, warn};
use sd_notify::NotifyState;
use smithay_client_toolkit::delegate_output;
use smithay_client_toolkit::delegate_registry;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::reexports::calloop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use smithay_client_toolkit::registry_handlers;
use wayland_client::backend::ObjectId;
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_output;
use wayland_client::Connection;
use wayland_client::Proxy;
use wayland_client::QueueHandle;

use std::collections::HashMap;
use std::env;
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::SystemTime;
use std::time::{Duration, Instant};

use crate::app_config::Config;
use crate::app_config::OutputDescriptor;
use crate::check_single_instance;
use crate::daemon_debug_log;
use crate::daemon_log_path;
use crate::ensure_config_exists;
use crate::heartbeat_tick;
use crate::pid_file_path;
use crate::rotate_daemon_log;
use crate::runtime_outputs_path;
use crate::start_watchdog_thread;
use crate::wayland_renderer::RuntimeOutputStatus;
use crate::DaemonContext;

#[derive(Clone)]
struct OutputGeometry {
    logical_position: (i32, i32),
    logical_size: (i32, i32),
}

struct SupervisorState {
    registry_state: RegistryState,
    output_state: OutputState,
    children: HashMap<String, Child>,
    active_outputs: HashMap<String, OutputGeometry>,
    last_spawn_attempt: HashMap<String, Instant>,
    output_id_to_name: HashMap<ObjectId, String>,
    exe: PathBuf,
    config_path: PathBuf,
    debug_mode: bool,
    systemd_mode: bool,
    pid_file: PathBuf,
    running: Arc<AtomicBool>,
    config_last_modified: Option<SystemTime>,
    dirty: bool,
}

impl SupervisorState {
    fn spawn_output_child(&self, output_name: &str, log_file: Option<&File>) -> Result<Child> {
        let mut cmd = Command::new(&self.exe);
        cmd.arg("__run")
            .arg("--supervised")
            .arg("--config")
            .arg(&self.config_path)
            .arg("--output")
            .arg(output_name);

        if self.systemd_mode {
            cmd.arg("--systemd");
        }

        cmd.stdin(Stdio::null());

        if let Some(file) = log_file {
            cmd.stdout(Stdio::from(file.try_clone()?))
                .stderr(Stdio::from(file.try_clone()?));
        } else {
            cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        }

        let mut child = cmd
            .spawn()
            .with_context(|| format!("Could not spawn child for output '{}'", output_name))?;

        if log_file.is_none() {
            let name = output_name.to_string();
            if let Some(stdout) = child.stdout.take() {
                thread::spawn(move || {
                    for line in BufReader::new(stdout).lines() {
                        match line {
                            Ok(line) => {
                                let formatted_line =
                                    if let Some((prefix, message)) = line.split_once("] ") {
                                        format!("{}] [{}] {}", prefix, name, message)
                                    } else {
                                        format!("[{}] {}", name, line)
                                    };
                                eprintln!("{}", formatted_line)
                            }
                            Err(e) => {
                                eprintln!(
                                    "[SUPERVISOR] Error reading stdout from '{}': {}",
                                    name, e
                                );
                                break;
                            }
                        }
                    }
                });
            }
            let name = output_name.to_string();
            if let Some(stderr) = child.stderr.take() {
                thread::spawn(move || {
                    for line in BufReader::new(stderr).lines() {
                        match line {
                            Ok(line) => {
                                let formatted_line =
                                    if let Some((prefix, message)) = line.split_once("] ") {
                                        format!("{}] [{}] {}", prefix, name, message)
                                    } else {
                                        format!("[{}] {}", name, line)
                                    };
                                eprintln!("{}", formatted_line)
                            }
                            Err(e) => {
                                eprintln!(
                                    "[SUPERVISOR] Error reading stderr from '{}': {}",
                                    name, e
                                );
                                break;
                            }
                        }
                    }
                });
            }
        }

        Ok(child)
    }

    fn spawn_child_for_output(&mut self, name: &str) {
        if let Some(last) = self.last_spawn_attempt.get(name) {
            if last.elapsed() < Duration::from_secs(5) {
                return;
            }
        }

        self.last_spawn_attempt
            .insert(name.to_string(), Instant::now());

        let log_file = if self.systemd_mode {
            None
        } else {
            let log_path = daemon_log_path(Some(name));
            match OpenOptions::new().create(true).append(true).open(&log_path) {
                Ok(f) => Some(f),
                Err(e) => {
                    error!("[SUPERVISOR] Failed to open log for '{}': {}", name, e);
                    return;
                }
            }
        };
        match self.spawn_output_child(name, log_file.as_ref()) {
            Ok(child) => {
                daemon_debug_log(
                    self.debug_mode,
                    self.systemd_mode,
                    &format!(
                        "[SUPERVISOR] Spawned child for '{}' (PID {})",
                        name,
                        child.id()
                    ),
                    None,
                );
                self.children.insert(name.to_string(), child);
                self.dirty = true;
            }
            Err(e) => {
                error!("[SUPERVISOR] Failed to spawn child for '{}': {:#}", name, e);
            }
        }
    }

    fn kill_child_for_output(&mut self, name: &str) {
        self.last_spawn_attempt.remove(name);

        if let Some(mut child) = self.children.remove(name) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = fs::remove_file(pid_file_path(Some(name)));
            daemon_debug_log(
                self.debug_mode,
                self.systemd_mode,
                &format!("[SUPERVISOR] Killed child for '{}'", name),
                None,
            );
            self.dirty = true;
        }
    }

    fn reap_children(&mut self) {
        let mut exited = Vec::new();
        for (name, child) in self.children.iter_mut() {
            match child.try_wait() {
                Ok(Some(status)) => {
                    exited.push((name.clone(), status));
                }
                Ok(None) => {}
                Err(e) => {
                    error!("[SUPERVISOR] Error polling child '{}': {}", name, e);
                }
            }
        }

        for (name, status) in exited {
            self.children.remove(&name);
            self.dirty = true;
            let _ = fs::remove_file(pid_file_path(Some(&name)));
            if self.active_outputs.contains_key(&name) {
                warn!(
                    "[SUPERVISOR] Child '{}' exited with {}, restarting...",
                    name, status
                );
                self.spawn_child_for_output(&name);
            } else {
                daemon_debug_log(
                    self.debug_mode,
                    self.systemd_mode,
                    &format!(
                        "[SUPERVISOR] Child '{}' exited, output disconnected, not restarting",
                        name
                    ),
                    None,
                );
            }
        }
    }

    fn persist_runtime_outputs(&mut self) {
        if !self.dirty {
            return;
        }

        let path = runtime_outputs_path(&self.config_path);

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let mut names: Vec<&String> = self
            .active_outputs
            .keys()
            .filter(|name| self.children.contains_key(*name))
            .collect();
        names.sort();

        let payload: Vec<RuntimeOutputStatus> = names
            .into_iter()
            .enumerate()
            .map(|(i, name)| {
                let geo = &self.active_outputs[name];
                RuntimeOutputStatus {
                    name: name.clone(),
                    index: i as u32,
                    width: geo.logical_size.0.max(0) as u32,
                    height: geo.logical_size.1.max(0) as u32,
                    position: [geo.logical_position.0, geo.logical_position.1],
                    configured: true,
                }
            })
            .collect();
        match serde_json::to_string_pretty(&payload) {
            Ok(json) => {
                if let Err(err) = std::fs::write(&path, json) {
                    error!(
                        "[SUPERVISOR] Failed to write runtime output state {}: {err}",
                        path.display()
                    );
                }
            }
            Err(err) => error!("[SUPERVISOR] Failed to serialize runtime output state: {err}"),
        }
        self.dirty = false;
    }

    fn check_config_changes(&mut self) {
        let metadata = match std::fs::metadata(&self.config_path) {
            Ok(m) => m,
            Err(_) => return,
        };

        let modified = match metadata.modified() {
            Ok(m) => m,
            Err(_) => return,
        };

        if self.config_last_modified == Some(modified) {
            return;
        }

        self.config_last_modified = Some(modified);

        let content = match std::fs::read_to_string(&self.config_path) {
            Ok(c) => c,
            Err(e) => {
                error!("[SUPERVISOR] Failed to read config: {}", e);
                return;
            }
        };

        let config: Config = match toml::from_str(&content) {
            Ok(c) => c,
            Err(e) => {
                error!("[SUPERVISOR] Failed to parse config: {}", e);
                return;
            }
        };

        let connected_outputs: Vec<(String, OutputGeometry)> = self
            .active_outputs
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        for (output_name, _geo) in connected_outputs {
            let descriptor = OutputDescriptor {
                name: output_name.clone(),
                connector: Some(output_name.clone()),
                index: None,
            };

            let is_enabled = config.resolve_for_output(&descriptor).is_some();
            let is_running = self.children.contains_key(&output_name);

            if is_enabled && !is_running {
                daemon_debug_log(
                    self.debug_mode,
                    self.systemd_mode,
                    &format!(
                        "[SUPERVISOR] Output '{}' enabled in config. Spawning child.",
                        output_name
                    ),
                    None,
                );
                self.spawn_child_for_output(&output_name);
            } else if !is_enabled && is_running {
                daemon_debug_log(
                    self.debug_mode,
                    self.systemd_mode,
                    &format!(
                        "[SUPERVISOR] Output '{}' disabled in config. Killing child.",
                        output_name
                    ),
                    None,
                );
                self.kill_child_for_output(&output_name);
            }
        }
    }
}

impl OutputHandler for SupervisorState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            let name = info
                .name
                .clone()
                .unwrap_or_else(|| format!("unknown-{}", output.id().protocol_id()));
            self.output_id_to_name.insert(output.id(), name.clone());

            daemon_debug_log(
                self.debug_mode,
                self.systemd_mode,
                &format!("[SUPERVISOR] New output detected: '{}'", name),
                None,
            );
            self.active_outputs.insert(
                name.clone(),
                OutputGeometry {
                    logical_position: info.logical_position.unwrap_or((0, 0)),
                    logical_size: info.logical_size.unwrap_or((1920, 1080)),
                },
            );
            self.dirty = true;

            if !self.children.contains_key(&name) {
                self.spawn_child_for_output(&name);
            }
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            let obj_id = output.id();
            let new_name = info
                .name
                .clone()
                .unwrap_or_else(|| format!("unknown-{}", obj_id.protocol_id()));

            if let Some(old_name) = self.output_id_to_name.get(&obj_id).cloned() {
                if old_name != new_name {
                    daemon_debug_log(
                        self.debug_mode,
                        self.systemd_mode,
                        &format!(
                            "[SUPERVISOR] Output renamed: '{}' -> '{}'",
                            old_name, new_name
                        ),
                        None,
                    );

                    self.output_id_to_name.insert(obj_id, new_name.clone());
                    self.active_outputs.remove(&old_name);
                    self.active_outputs.insert(
                        new_name.clone(),
                        OutputGeometry {
                            logical_position: info.logical_position.unwrap_or((0, 0)),
                            logical_size: info.logical_size.unwrap_or((1920, 1080)),
                        },
                    );
                    self.dirty = true;

                    self.kill_child_for_output(&old_name);
                    self.spawn_child_for_output(&new_name);
                } else {
                    if let Some(geo) = self.active_outputs.get_mut(&new_name) {
                        geo.logical_position = info.logical_position.unwrap_or((0, 0));
                        geo.logical_size = info.logical_size.unwrap_or((1920, 1080));
                        self.dirty = true;
                    }
                }
            }
        }
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(name) = self.output_id_to_name.remove(&output.id()) {
            daemon_debug_log(
                self.debug_mode,
                self.systemd_mode,
                &format!("[SUPERVISOR] Output destroyed: '{}'", name),
                None,
            );
            self.active_outputs.remove(&name);
            self.last_spawn_attempt.remove(&name);
            self.kill_child_for_output(&name);
            self.dirty = true;
        }
    }
}

impl ProvidesRegistryState for SupervisorState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState];
}

delegate_output!(SupervisorState);
delegate_registry!(SupervisorState);

pub fn run_supervisor(daemon_context: DaemonContext) -> Result<()> {
    let config_path = daemon_context.config_path;
    let pid_file = daemon_context.pid_file;
    let debug_mode = daemon_context.debug_mode;
    let systemd_mode = daemon_context.systemd_mode;

    ensure_config_exists(&config_path)?;

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        &format!("[SUPERVISOR] Started, PID: {}", std::process::id()),
        None,
    );

    if systemd_mode {
        info!("[SUPERVISOR] Running in systemd mode");
    }
    if !check_single_instance(&pid_file, debug_mode, systemd_mode, None)? {
        std::process::exit(1);
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let pid_cleanup = pid_file.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        let _ = fs::remove_file(&pid_cleanup);
    })
    .expect("Error setting signal handler for SIGINT/SIGTERM");

    let exe = env::current_exe().context("Could not resolve the current executable")?;

    let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
    let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
    let qh = event_queue.handle();
    let mut event_loop = calloop::EventLoop::try_new().context("Failed to create event loop")?;
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle.clone())
        .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

    let mut state = SupervisorState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        children: HashMap::new(),
        active_outputs: HashMap::new(),
        last_spawn_attempt: HashMap::new(),
        output_id_to_name: HashMap::new(),
        exe,
        config_last_modified: None,
        config_path,
        debug_mode,
        systemd_mode,
        pid_file,
        running,
        dirty: false,
    };

    if !systemd_mode {
        let log_running = state.running.clone();
        thread::spawn(move || {
            while log_running.load(Ordering::SeqCst) {
                thread::sleep(Duration::from_secs(60));
                rotate_daemon_log(None);
            }
        });
    }

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        "[SUPERVISOR] Signal handlers installed",
        None,
    );

    if systemd_mode {
        if let Err(e) = sd_notify::notify(&[NotifyState::Ready]) {
            warn!("Failed to notify systemd (READY=1): {}", e);
        } else {
            info!("[SUPERVISOR] Notified systemd: READY=1");
        }

        start_watchdog_thread(state.running.clone());
    }

    daemon_debug_log(
        debug_mode,
        systemd_mode,
        "[SUPERVISOR] Entering Wayland event loop (hotplug enabled)",
        None,
    );
    while state.running.load(Ordering::SeqCst) {
        heartbeat_tick();
        event_loop
            .dispatch(Some(Duration::from_millis(500)), &mut state)
            .context("Event loop dispatch error")?;
        state.reap_children();
        state.check_config_changes();
        state.persist_runtime_outputs();
    }

    daemon_debug_log(
        state.debug_mode,
        systemd_mode,
        "[SUPERVISOR] Shutting down children...",
        None,
    );
    for (name, mut child) in state.children.drain() {
        let _ = child.kill();
        let _ = child.wait();
        let _ = fs::remove_file(pid_file_path(Some(&name)));
    }

    if systemd_mode {
        if let Err(e) = sd_notify::notify(&[NotifyState::Stopping]) {
            warn!("Failed to notify systemd (STOPPING=1): {}", e);
        }
    }
    let _ = fs::remove_file(&state.pid_file);
    Ok(())
}
