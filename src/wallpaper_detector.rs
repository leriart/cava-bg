use log::{debug, info, warn};
use once_cell::sync::Lazy;
use parking_lot::Mutex;
use serde_json::Value;
use std::collections::BTreeSet;
use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use walkdir::WalkDir;

const DETECTION_CACHE_TTL: Duration = Duration::from_secs(5);

const MEDIA_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "bmp", "webp", "gif", "mp4", "webm", "mkv", "mov", "avi", "avif", "heif",
    "heic",
];

const KNOWN_WALLPAPER_PROCESSES: &[&str] = &[
    "mpvpaper",
    "awww",
    "awww-daemon",
    "hyprpaper",
    "swaybg",
    "wpaperd",
    "waypaper",
    "waytrogen",
    "hpaper",
    "walt",
    "wlsbg",
    "wallrizz",
    "ambxst",
    "quickshell",
    "qmlscene",
    "qml",
];

const QT_MULTIMEDIA_PATTERNS: &[&str] = &[
    "ambxst",
    "qml",
    "qmlscene",
    "quickshell",
    "qtmultimedia",
    "qt5-multimedia",
    "qt6-multimedia",
];

#[derive(Debug, Clone, Default)]
pub struct WaylandEnvironment {
    pub xdg_current_desktop: Option<String>,
    pub wayland_display: Option<String>,
    pub xdg_session_type: Option<String>,
    pub sway_sock: Option<String>,
    pub hyprland_instance_signature: Option<String>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct DetectionCache {
    wallpaper: Option<PathBuf>,
    compositor: Option<String>,
    layer_processes: Vec<BackgroundProcessInfo>,
    timestamp: Instant,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct BackgroundProcessInfo {
    pub pid: i32,
    pub process_name: String,
    pub args: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub media_files: Vec<PathBuf>,
    pub open_files: Vec<PathBuf>,
    pub uses_qt_multimedia: bool,
}

#[derive(Debug, Clone)]
struct ProcSnapshot {
    pid: i32,
    comm: String,
    args: Vec<String>,
}

static DETECTION_CACHE: Lazy<Mutex<Option<DetectionCache>>> = Lazy::new(|| Mutex::new(None));

pub fn detect_wayland_environment() -> WaylandEnvironment {
    WaylandEnvironment {
        xdg_current_desktop: env::var("XDG_CURRENT_DESKTOP").ok(),
        wayland_display: env::var("WAYLAND_DISPLAY").ok(),
        xdg_session_type: env::var("XDG_SESSION_TYPE").ok(),
        sway_sock: env::var("SWAYSOCK").ok(),
        hyprland_instance_signature: env::var("HYPRLAND_INSTANCE_SIGNATURE").ok(),
    }
}

#[allow(dead_code)]
pub fn detect_compositor() -> Option<String> {
    let env_info = detect_wayland_environment();
    detect_compositor_with_env(&env_info)
}

pub fn get_current_wallpaper() -> Option<PathBuf> {
    if let Some(cache) = DETECTION_CACHE.lock().clone() {
        if cache.timestamp.elapsed() < DETECTION_CACHE_TTL {
            vlog(
                format!(
                    "Using cached wallpaper detection (age={} ms)",
                    cache.timestamp.elapsed().as_millis()
                )
                .as_str(),
            );
            return cache.wallpaper;
        }
    }

    let env_info = detect_wayland_environment();
    debug!(
        "Wallpaper detection env: XDG_CURRENT_DESKTOP={:?}, WAYLAND_DISPLAY={:?}, XDG_SESSION_TYPE={:?}, SWAYSOCK={:?}, HYPRLAND_INSTANCE_SIGNATURE={:?}",
        env_info.xdg_current_desktop,
        env_info.wayland_display,
        env_info.xdg_session_type,
        env_info.sway_sock,
        env_info.hyprland_instance_signature
    );

    let compositor = detect_compositor_with_env(&env_info);
    if let Some(comp) = compositor.as_deref() {
        info!("Detected compositor: {}", comp);
    }

    let mut detected = None;

    // Priority 1: IPC/direct commands
    let priority_1: &[(&str, fn() -> Option<PathBuf>)] = &[
        ("awww query", from_awww_query),
        ("hyprpaper IPC (hyprctl)", from_hyprpaper_ipc),
        ("wpaperd IPC (wpaperctl)", from_wpaperd_ipc),
    ];

    // Priority 2: D-Bus
    let priority_2: &[(&str, fn() -> Option<PathBuf>)] = &[
        ("GNOME D-Bus/gsettings", from_gnome),
        ("KDE D-Bus/plasma", from_plasma),
    ];

    // Priority 3: Ambxst (Qt/QML wallpaper daemon)
    let priority_3: &[(&str, fn() -> Option<PathBuf>)] = &[("Ambxst detection", from_ambxst)];

    // Priority 4: configuration files
    let priority_4: &[(&str, fn() -> Option<PathBuf>)] = &[
        ("hyprpaper config", from_hyprpaper_config),
        ("wpaperd config", from_wpaperd_config),
        ("waypaper config", from_waypaper_config),
        ("Waytrogen config", from_waytrogen_config),
        ("hpaper config", from_hpaper_config),
        ("Walt config", from_walt_config),
        ("wlsbg config", from_wlsbg_config),
        ("wallrizz config", from_wallrizz_config),
        ("sway config", from_sway_config),
    ];

    // Priority 5: process detection
    let priority_5: &[(&str, fn() -> Option<PathBuf>)] = &[
        ("mpvpaper process", from_mpvpaper_process),
        ("awww process", from_awww_process),
        ("hyprpaper process", from_hyprpaper_process),
        ("swaybg process", from_swaybg_process),
        ("wpaperd process", from_wpaperd_process),
        ("waypaper process", from_waypaper_process),
        ("Waytrogen process", from_waytrogen_process),
        ("hpaper process", from_hpaper_process),
        ("Walt process", from_walt_process),
        ("wlsbg process", from_wlsbg_process),
        ("wallrizz process", from_wallrizz_process),
    ];

    // Priority 6: layer background scan
    let priority_6: &[(&str, fn() -> Option<PathBuf>)] =
        &[("layer background scan", from_layer_background_scan)];

    for (priority_name, methods) in [
        ("Priority 1 (IPC/direct)", priority_1),
        ("Priority 2 (D-Bus)", priority_2),
        ("Priority 3 (Ambxst)", priority_3),
        ("Priority 4 (config files)", priority_4),
        ("Priority 5 (process detection)", priority_5),
        ("Priority 6 (layer background scan)", priority_6),
    ] {
        vlog(&format!("Trying {}", priority_name));
        for (name, detector) in methods {
            vlog(&format!("Trying {}...", name));
            if let Some(path) = detector() {
                if path.exists() {
                    debug!("Wallpaper detected via {}: {}", name, path.display());
                    detected = Some(path);
                    break;
                }
            }
        }
        if detected.is_some() {
            break;
        }
    }

    let layer_processes = scan_layer_background_processes();

    *DETECTION_CACHE.lock() = Some(DetectionCache {
        wallpaper: detected.clone(),
        compositor,
        layer_processes,
        timestamp: Instant::now(),
    });

    if detected.is_none() {
        warn!("Could not detect the active wallpaper using known methods");
    }

    detected
}

pub fn scan_layer_background_processes() -> Vec<BackgroundProcessInfo> {
    let mut found = Vec::new();
    let snapshots = collect_proc_snapshots();

    for snap in snapshots {
        let process_lower = snap.comm.to_lowercase();
        let args_lower = snap.args.join(" ").to_lowercase();

        let is_known_wallpaper_process = KNOWN_WALLPAPER_PROCESSES
            .iter()
            .any(|needle| process_lower.contains(needle) || args_lower.contains(needle));

        let qt_pattern_match = QT_MULTIMEDIA_PATTERNS
            .iter()
            .any(|needle| process_lower.contains(needle) || args_lower.contains(needle));

        let uses_qt_multimedia = process_uses_qt_multimedia(snap.pid);

        if !is_known_wallpaper_process && !qt_pattern_match && !uses_qt_multimedia {
            continue;
        }

        let cwd = fs::read_link(format!("/proc/{}/cwd", snap.pid)).ok();
        let open_files = read_open_files(snap.pid);
        let args_media = extract_media_candidates_from_args(&snap.args);
        let open_media = extract_media_candidates_from_paths(&open_files);

        let mut media_set = BTreeSet::new();
        for p in args_media.into_iter().chain(open_media) {
            media_set.insert(p);
        }

        if media_set.is_empty() {
            if let Some(source) = extract_qt_multimedia_source(&snap.args.join(" ")) {
                media_set.insert(source);
            }
        }

        if uses_qt_multimedia {
            vlog(&format!(
                "Detected Qt application with QtMultimedia: {} (pid={})",
                snap.comm, snap.pid
            ));
        }

        let info = BackgroundProcessInfo {
            pid: snap.pid,
            process_name: snap.comm,
            args: snap.args,
            cwd,
            media_files: media_set.into_iter().collect(),
            open_files,
            uses_qt_multimedia,
        };

        if let Some(first_media) = info.media_files.first() {
            info!(
                "Found wallpaper process: {} playing {}",
                info.process_name,
                first_media.display()
            );
        } else {
            debug!(
                "Found wallpaper/Qt process: {} (pid={})",
                info.process_name, info.pid
            );
        }

        found.push(info);
    }

    found
}

#[allow(dead_code)]
pub fn get_cached_detection_debug() -> Option<(Option<String>, Vec<BackgroundProcessInfo>)> {
    let cache = DETECTION_CACHE.lock();
    cache
        .as_ref()
        .map(|c| (c.compositor.clone(), c.layer_processes.clone()))
}

fn detect_compositor_with_env(env_info: &WaylandEnvironment) -> Option<String> {
    let desktop = env_info
        .xdg_current_desktop
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();
    let session_type = env_info
        .xdg_session_type
        .as_deref()
        .unwrap_or_default()
        .to_lowercase();

    if env_info.hyprland_instance_signature.is_some() || desktop.contains("hypr") {
        return Some("Hyprland".to_string());
    }
    if env_info.sway_sock.is_some() || desktop.contains("sway") {
        return Some("Sway".to_string());
    }
    if desktop.contains("cosmic") || process_exists("cosmic-comp") {
        return Some("COSMIC".to_string());
    }
    if desktop.contains("plasma") || desktop.contains("kde") || process_exists("kwin_wayland") {
        return Some("KDE Plasma Wayland".to_string());
    }
    if (desktop.contains("gnome") && session_type == "wayland")
        || process_exists("gnome-shell-wayland")
        || (process_exists("gnome-shell") && session_type == "wayland")
    {
        return Some("GNOME Wayland".to_string());
    }
    if desktop.contains("labwc") || process_exists("labwc") {
        return Some("Labwc".to_string());
    }
    if desktop.contains("river") || process_exists("river") {
        return Some("River".to_string());
    }
    if desktop.contains("wayfire") || process_exists("wayfire") {
        return Some("Wayfire".to_string());
    }
    if desktop.contains("niri") || process_exists("niri") {
        return Some("niri".to_string());
    }
    if process_exists("miriway")
        || process_exists("miral-shell")
        || process_exists("ubuntu-frame")
        || process_exists("mir-kiosk")
    {
        return Some("Mir-based compositor".to_string());
    }

    if env_info.wayland_display.is_some() {
        let known = [
            ("hyprland", "Hyprland"),
            ("sway", "Sway"),
            ("cosmic-comp", "COSMIC"),
            ("kwin_wayland", "KDE Plasma Wayland"),
            ("gnome-shell", "GNOME Wayland"),
            ("labwc", "Labwc"),
            ("river", "River"),
            ("wayfire", "Wayfire"),
            ("niri", "niri"),
        ];

        for (needle, label) in known {
            if process_exists(needle) {
                return Some(label.to_string());
            }
        }
    }

    None
}

fn from_awww_query() -> Option<PathBuf> {
    vlog("Trying awww daemon via IPC socket / command...");

    if let Ok(output) = Command::new("awww").arg("query").output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(found) = find_existing_path_in_text(&stdout) {
                return Some(found);
            }
        }
    }

    let cache_dir = dirs::home_dir()?.join(".cache/awww");
    if cache_dir.exists() {
        for entry in fs::read_dir(cache_dir).ok()?.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Ok(content) = fs::read_to_string(&path) {
                    if let Some(found) = find_existing_path_in_text(&content) {
                        return Some(found);
                    }
                }
            }
        }
    }

    None
}

fn from_awww_process() -> Option<PathBuf> {
    find_from_processes(&["awww", "awww-daemon"])
}

fn from_hyprpaper_ipc() -> Option<PathBuf> {
    vlog("Trying hyprpaper via hyprctl / IPC socket...");

    for args in [
        vec!["-j", "hyprpaper", "listloaded"],
        vec!["hyprpaper", "listloaded"],
        vec!["hyprpaper", "listactive"],
    ] {
        if let Ok(output) = Command::new("hyprctl").args(&args).output() {
            if !output.status.success() {
                continue;
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            if args.first() == Some(&"-j") {
                if let Some(found) = parse_path_from_json_or_text(&stdout) {
                    return Some(found);
                }
            } else if let Some(found) = find_existing_path_in_text(&stdout) {
                return Some(found);
            }
        }
    }

    for socket in hyprpaper_socket_candidates() {
        if socket.exists() {
            vlog(&format!(
                "Detected hyprpaper IPC socket: {}",
                socket.display()
            ));
        }
    }

    None
}

fn from_hyprpaper_config() -> Option<PathBuf> {
    let conf_paths = [
        dirs::home_dir()?.join(".config/hypr/hyprpaper.conf"),
        dirs::home_dir()?.join(".config/hypr/hyprpaper.toml"),
    ];

    for conf in conf_paths {
        if !conf.exists() {
            continue;
        }
        if let Ok(content) = fs::read_to_string(&conf) {
            for raw in content.lines() {
                let line = strip_comment(raw).trim();
                if line.starts_with("wallpaper")
                    || line.contains("preload")
                    || line.contains("path")
                {
                    if let Some(found) = find_existing_path_in_text(line) {
                        return Some(found);
                    }
                }
            }
        }
    }

    None
}

fn from_hyprpaper_process() -> Option<PathBuf> {
    find_from_processes(&["hyprpaper"])
}

fn from_swaybg_process() -> Option<PathBuf> {
    for snap in collect_proc_snapshots() {
        if !snap.comm.contains("swaybg") && !snap.args.join(" ").contains("swaybg") {
            continue;
        }

        let mut i = 0usize;
        while i < snap.args.len() {
            if snap.args[i] == "-i" {
                if let Some(next) = snap.args.get(i + 1) {
                    let p = normalize_path(next);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
            i += 1;
        }

        if let Some(found) = extract_media_candidates_from_args(&snap.args)
            .into_iter()
            .next()
        {
            return Some(found);
        }
    }

    None
}

fn from_sway_config() -> Option<PathBuf> {
    let paths = [
        dirs::home_dir()?.join(".config/sway/config"),
        dirs::home_dir()?.join(".sway/config"),
    ];

    for conf in paths {
        let Ok(content) = fs::read_to_string(&conf) else {
            continue;
        };

        for raw in content.lines() {
            let line = strip_comment(raw).trim();
            if !line.starts_with("output") || !line.contains(" bg ") {
                continue;
            }

            let parts: Vec<&str> = line.split_whitespace().collect();
            if let Some(bg_index) = parts.iter().position(|p| *p == "bg") {
                if let Some(path_token) = parts.get(bg_index + 1) {
                    let p = normalize_path(path_token);
                    if p.exists() {
                        return Some(p);
                    }
                }
            }
        }
    }

    None
}

fn from_wpaperd_ipc() -> Option<PathBuf> {
    vlog("Trying wpaperd via wpaperctl...");

    for args in [vec!["wallpaper", "get"], vec!["get"], vec!["status"]] {
        if let Ok(output) = Command::new("wpaperctl").args(&args).output() {
            if output.status.success() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                if let Some(found) = find_existing_path_in_text(&stdout) {
                    return Some(found);
                }
            }
        }
    }

    None
}

fn from_wpaperd_config() -> Option<PathBuf> {
    let conf_dir = dirs::home_dir()?.join(".config/wpaperd");
    if !conf_dir.exists() {
        return None;
    }

    for entry in fs::read_dir(conf_dir).ok()?.flatten() {
        let p = entry.path();
        let ext = p.extension().and_then(OsStr::to_str).unwrap_or_default();
        if ext != "toml" && ext != "conf" {
            continue;
        }

        if let Ok(content) = fs::read_to_string(&p) {
            for line in content.lines() {
                let l = strip_comment(line).trim();
                if l.contains("path") || l.contains("wallpaper") || l.contains("file") {
                    if let Some(found) = find_existing_path_in_text(l) {
                        return Some(found);
                    }
                }
            }
        }
    }

    None
}

fn from_wpaperd_process() -> Option<PathBuf> {
    find_from_processes(&["wpaperd"])
}

fn from_mpvpaper_process() -> Option<PathBuf> {
    for snap in collect_proc_snapshots() {
        if !snap.comm.contains("mpvpaper") && !snap.args.join(" ").contains("mpvpaper") {
            continue;
        }

        if let Some(path) = extract_media_path_from_args(&snap.args, "mpvpaper") {
            if path.exists() {
                return Some(path);
            }
        }

        if let Some(found) = extract_media_candidates_from_args(&snap.args)
            .into_iter()
            .next()
        {
            return Some(found);
        }
    }

    None
}

fn from_waypaper_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/waypaper/config.ini"),
        dirs::home_dir()?.join(".config/waypaper/config.toml"),
    ])
}

fn from_waypaper_process() -> Option<PathBuf> {
    find_from_processes(&["waypaper"])
}

fn from_waytrogen_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/waytrogen/config.toml"),
        dirs::home_dir()?.join(".config/Waytrogen/config.toml"),
        dirs::home_dir()?.join(".config/waytrogen/config.ini"),
    ])
}

fn from_waytrogen_process() -> Option<PathBuf> {
    find_from_processes(&["waytrogen"])
}

fn from_hpaper_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/hpaper/config.toml"),
        dirs::home_dir()?.join(".config/hpaper/config.ini"),
    ])
}

fn from_hpaper_process() -> Option<PathBuf> {
    find_from_processes(&["hpaper"])
}

fn from_walt_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/walt/config.toml"),
        dirs::home_dir()?.join(".config/walt/config.ini"),
        dirs::home_dir()?.join(".config/hypr/walt.conf"),
    ])
}

fn from_walt_process() -> Option<PathBuf> {
    find_from_processes(&["walt"])
}

fn from_wlsbg_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/wlsbg/config.toml"),
        dirs::home_dir()?.join(".config/wlsbg/config.ini"),
    ])
}

fn from_wlsbg_process() -> Option<PathBuf> {
    find_from_processes(&["wlsbg"])
}

fn from_wallrizz_config() -> Option<PathBuf> {
    parse_first_existing_path_from_files(&[
        dirs::home_dir()?.join(".config/wallrizz/config.toml"),
        dirs::home_dir()?.join(".config/wallrizz/config.ini"),
    ])
}

fn from_wallrizz_process() -> Option<PathBuf> {
    find_from_processes(&["wallrizz"])
}

fn from_ambxst() -> Option<PathBuf> {
    if let Some(path) = from_ambxst_process() {
        info!("Detected Ambxst wallpaper from process: {}", path.display());
        return Some(path);
    }

    if let Some(path) = from_ambxst_config() {
        info!("Detected Ambxst wallpaper from config: {}", path.display());
        return Some(path);
    }

    if let Some(path) = from_ambxst_qml() {
        info!("Detected Ambxst wallpaper from QML: {}", path.display());
        return Some(path);
    }

    None
}

fn from_ambxst_process() -> Option<PathBuf> {
    let snapshots = collect_proc_snapshots();

    for snap in snapshots {
        let proc_lc = snap.comm.to_lowercase();
        let args_lc = snap.args.join(" ").to_lowercase();
        if !proc_lc.contains("ambxst")
            && !args_lc.contains("ambxst")
            && !args_lc.contains("ambxst:wallpaper")
        {
            continue;
        }

        if let Some(found) = extract_ambxst_source_from_args(&snap.args) {
            return Some(found);
        }

        let open_files = read_open_files(snap.pid);
        if let Some(found) = extract_media_candidates_from_paths(&open_files)
            .into_iter()
            .next()
        {
            return Some(found);
        }
    }

    None
}

fn from_ambxst_config() -> Option<PathBuf> {
    let config_paths = [
        "~/.cache/ambxst/wallpapers.json",
        "~/.config/ambxst/config",
        "~/.config/Ambxst/ambxst.conf",
        "~/.config/ambxst/ambxst.json",
        "~/.ambxst/config",
    ];

    for path in config_paths {
        let Some(expanded) = expand_tilde(path) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(&expanded) else {
            continue;
        };

        if let Some(source) = parse_ambxst_config(&content) {
            vlog(&format!(
                "Ambxst config candidate from {} -> {}",
                expanded.display(),
                source.display()
            ));
            return Some(source);
        }
    }

    None
}

fn from_ambxst_qml() -> Option<PathBuf> {
    let qml_paths = [
        "~/.config/ambxst/Wallpaper.qml",
        "~/.local/share/ambxst/Wallpaper.qml",
        "~/.cache/ambxst/Wallpaper.qml",
    ];

    for path in qml_paths {
        let Some(expanded) = expand_tilde(path) else {
            continue;
        };
        let Ok(content) = fs::read_to_string(&expanded) else {
            continue;
        };

        if let Some(source) = parse_qml_source(&content) {
            vlog(&format!(
                "Ambxst QML candidate from {} -> {}",
                expanded.display(),
                source.display()
            ));
            return Some(source);
        }
    }

    None
}

fn extract_ambxst_source_from_args(args: &[String]) -> Option<PathBuf> {
    let mut idx = 0usize;
    while idx < args.len() {
        let arg = args[idx].as_str();

        if arg == "--source" || arg == "-s" {
            if let Some(next) = args.get(idx + 1) {
                if let Some(path) = normalize_maybe_media_path(next) {
                    return Some(path);
                }
            }
        } else if let Some(value) = arg.strip_prefix("--source=") {
            if let Some(path) = normalize_maybe_media_path(value) {
                return Some(path);
            }
        }

        if let Some(path) = normalize_maybe_media_path(arg) {
            return Some(path);
        }

        idx += 1;
    }

    None
}

fn parse_ambxst_config(content: &str) -> Option<PathBuf> {
    if let Ok(json) = serde_json::from_str::<Value>(content) {
        if let Some(current) = json
            .get("currentWall")
            .and_then(Value::as_str)
            .and_then(normalize_maybe_media_path)
        {
            return Some(current);
        }

        if let Some(per_screen) = json.get("perScreenWallpapers") {
            if let Some(found) = find_path_in_json_value(per_screen) {
                return Some(found);
            }
        }

        if let Some(found) = find_path_in_json_value(&json) {
            return Some(found);
        }
    }

    parse_path_from_json_or_text(content)
}

fn parse_qml_source(qml_content: &str) -> Option<PathBuf> {
    for raw in qml_content.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }

        let has_source_key = [
            "source:",
            "currentSource:",
            "wallpaperSource:",
            "currentWallpaper:",
            "effectiveWallpaper:",
            "currentWall:",
            "perScreenWallpapers",
        ]
        .iter()
        .any(|k| line.contains(k));

        if !has_source_key {
            continue;
        }

        if let Some(path) = find_existing_path_in_text(line) {
            return Some(path);
        }
    }

    None
}

fn from_layer_background_scan() -> Option<PathBuf> {
    let processes = scan_layer_background_processes();
    for proc_info in processes {
        if let Some(media) = proc_info.media_files.first() {
            return Some(media.clone());
        }
    }
    None
}

fn from_gnome() -> Option<PathBuf> {
    if let Some(p) = from_gnome_dbus() {
        return Some(p);
    }

    for key in ["picture-uri-dark", "picture-uri"] {
        let output = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.background", key])
            .output()
            .ok()?;

        if !output.status.success() {
            continue;
        }

        let value = String::from_utf8(output.stdout).ok()?;
        if let Some(found) = find_existing_path_in_text(value.trim()) {
            return Some(found);
        }
    }

    None
}

#[cfg(feature = "dbus-detection")]
fn from_gnome_dbus() -> Option<PathBuf> {
    use dbus::blocking::Connection;
    use std::time::Duration as StdDuration;

    let conn = Connection::new_session().ok()?;
    let proxy = conn.with_proxy(
        "org.gnome.Shell",
        "/org/gnome/Shell",
        StdDuration::from_millis(1200),
    );

    // org.gnome.Shell.Eval(js)
    let script =
        "imports.gi.Gio.Settings.new('org.gnome.desktop.background').get_string('picture-uri')";
    let response: Result<(bool, String), _> =
        proxy.method_call("org.gnome.Shell", "Eval", (script,));

    let (_, result) = response.ok()?;
    find_existing_path_in_text(&result)
}

#[cfg(not(feature = "dbus-detection"))]
fn from_gnome_dbus() -> Option<PathBuf> {
    None
}

fn from_plasma() -> Option<PathBuf> {
    if let Some(p) = from_plasma_dbus() {
        return Some(p);
    }

    let conf = dirs::home_dir()?.join(".config/plasma-org.kde.plasma.desktop-appletsrc");
    let content = fs::read_to_string(conf).ok()?;

    for line in content.lines() {
        if line.trim_start().starts_with("Image=") {
            if let Some(found) = find_existing_path_in_text(line) {
                return Some(found);
            }
        }
    }

    None
}

#[cfg(feature = "dbus-detection")]
fn from_plasma_dbus() -> Option<PathBuf> {
    use dbus::blocking::Connection;
    use std::time::Duration as StdDuration;

    let conn = Connection::new_session().ok()?;
    let proxy = conn.with_proxy(
        "org.kde.plasmashell",
        "/PlasmaShell",
        StdDuration::from_millis(1200),
    );

    let script = r#"
        var allDesktops = desktops();
        for (i=0;i<allDesktops.length;i++) {
          d = allDesktops[i];
          d.currentConfigGroup = ['Wallpaper', 'org.kde.image', 'General'];
          print(d.readConfig('Image'));
        }
    "#;

    let response: Result<(String,), _> =
        proxy.method_call("org.kde.PlasmaShell", "evaluateScript", (script,));

    let (result,) = response.ok()?;
    find_existing_path_in_text(&result)
}

#[cfg(not(feature = "dbus-detection"))]
fn from_plasma_dbus() -> Option<PathBuf> {
    None
}

fn hyprpaper_socket_candidates() -> Vec<PathBuf> {
    let mut sockets = Vec::new();
    if let Ok(sig) = env::var("HYPRLAND_INSTANCE_SIGNATURE") {
        if let Ok(runtime) = env::var("XDG_RUNTIME_DIR") {
            sockets.push(
                PathBuf::from(&runtime)
                    .join("hypr")
                    .join(sig)
                    .join(".hyprpaper.sock"),
            );
        }
    }
    sockets.push(
        dirs::home_dir()
            .unwrap_or_default()
            .join(".cache/hyprpaper.sock"),
    );
    sockets
}

fn parse_first_existing_path_from_files(paths: &[PathBuf]) -> Option<PathBuf> {
    for p in paths {
        let Ok(content) = fs::read_to_string(p) else {
            continue;
        };

        for line in content.lines() {
            let line = strip_comment(line).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(found) = find_existing_path_in_text(line) {
                return Some(found);
            }
        }
    }
    None
}

fn find_from_processes(process_names: &[&str]) -> Option<PathBuf> {
    let snapshots = collect_proc_snapshots();

    for snap in snapshots {
        let proc_lc = snap.comm.to_lowercase();
        let args_lc = snap.args.join(" ").to_lowercase();

        let matches = process_names
            .iter()
            .any(|name| proc_lc.contains(name) || args_lc.contains(name));
        if !matches {
            continue;
        }

        if let Some(found) = extract_media_candidates_from_args(&snap.args)
            .into_iter()
            .next()
        {
            return Some(found);
        }

        let open_files = read_open_files(snap.pid);
        if let Some(found) = extract_media_candidates_from_paths(&open_files)
            .into_iter()
            .next()
        {
            return Some(found);
        }
    }

    None
}

fn collect_proc_snapshots() -> Vec<ProcSnapshot> {
    let mut result = Vec::new();
    let Ok(entries) = fs::read_dir("/proc") else {
        return result;
    };

    for entry in entries.flatten() {
        let file_name = entry.file_name();
        let Some(pid_text) = file_name.to_str() else {
            continue;
        };
        if !pid_text.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        let Ok(pid) = pid_text.parse::<i32>() else {
            continue;
        };

        let comm = fs::read_to_string(entry.path().join("comm"))
            .unwrap_or_default()
            .trim()
            .to_string();
        if comm.is_empty() {
            continue;
        }

        let args = read_proc_cmdline(pid).unwrap_or_default();
        result.push(ProcSnapshot { pid, comm, args });
    }

    result
}

fn process_exists(needle: &str) -> bool {
    let needle = needle.to_lowercase();
    collect_proc_snapshots().into_iter().any(|p| {
        p.comm.to_lowercase().contains(&needle) || p.args.join(" ").to_lowercase().contains(&needle)
    })
}

fn read_proc_cmdline(pid: i32) -> Option<Vec<String>> {
    let cmdline_path = format!("/proc/{}/cmdline", pid);
    let bytes = fs::read(cmdline_path).ok()?;
    if bytes.is_empty() {
        return None;
    }

    let parts = bytes
        .split(|b| *b == 0)
        .filter(|part| !part.is_empty())
        .filter_map(|part| String::from_utf8(part.to_vec()).ok())
        .collect::<Vec<_>>();

    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

fn read_open_files(pid: i32) -> Vec<PathBuf> {
    let fd_dir = format!("/proc/{}/fd", pid);
    if !Path::new(&fd_dir).exists() {
        return Vec::new();
    }

    let mut files = Vec::new();
    for entry in WalkDir::new(fd_dir)
        .min_depth(1)
        .max_depth(1)
        .into_iter()
        .flatten()
    {
        if let Ok(target) = fs::read_link(entry.path()) {
            files.push(target);
        }
    }

    files
}

fn process_uses_qt_multimedia(pid: i32) -> bool {
    let maps_path = format!("/proc/{}/maps", pid);
    let Ok(content) = fs::read_to_string(maps_path) else {
        return false;
    };

    content.lines().any(|line| {
        let l = line.to_lowercase();
        l.contains("libqt") && l.contains("multimedia")
    })
}

fn extract_media_path_from_args(args: &[String], daemon_name: &str) -> Option<PathBuf> {
    let mut last_non_flag: Option<&str> = None;
    for arg in args {
        if arg.contains(daemon_name) {
            continue;
        }
        if arg.starts_with('-') {
            continue;
        }
        last_non_flag = Some(arg);
    }
    let raw = last_non_flag?;
    Some(normalize_path(raw))
}

fn extract_media_candidates_from_args(args: &[String]) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for arg in args {
        if let Some(candidate) = normalize_maybe_media_path(arg) {
            out.push(candidate);
        }
    }
    out
}

fn extract_qt_multimedia_source(proc_line: &str) -> Option<PathBuf> {
    for ext in MEDIA_EXTENSIONS {
        let dot_ext = format!(".{}", ext);
        if let Some(path) = extract_path_with_extension(proc_line, &dot_ext) {
            return Some(path);
        }
    }
    None
}

fn extract_path_with_extension(input: &str, extension: &str) -> Option<PathBuf> {
    for token in tokenize_path_candidates(input) {
        if !token.to_lowercase().contains(extension) {
            continue;
        }
        if let Some(path) = normalize_maybe_media_path(&token) {
            return Some(path);
        }
    }

    None
}

fn extract_media_candidates_from_paths(paths: &[PathBuf]) -> Vec<PathBuf> {
    paths
        .iter()
        .filter(|p| is_media_file_path(p))
        .filter(|p| p.exists())
        .cloned()
        .collect()
}

fn parse_path_from_json_or_text(input: &str) -> Option<PathBuf> {
    if let Ok(json) = serde_json::from_str::<Value>(input) {
        if let Some(path) = find_path_in_json_value(&json) {
            return Some(path);
        }
    }
    find_existing_path_in_text(input)
}

fn find_path_in_json_value(value: &Value) -> Option<PathBuf> {
    match value {
        Value::String(s) => normalize_maybe_media_path(s),
        Value::Array(items) => {
            for v in items {
                if let Some(p) = find_path_in_json_value(v) {
                    return Some(p);
                }
            }
            None
        }
        Value::Object(map) => {
            for (_k, v) in map {
                if let Some(p) = find_path_in_json_value(v) {
                    return Some(p);
                }
            }
            None
        }
        _ => None,
    }
}

fn find_existing_path_in_text(text: &str) -> Option<PathBuf> {
    for token in tokenize_path_candidates(text) {
        if let Some(path) = normalize_maybe_media_path(&token) {
            return Some(path);
        }
    }
    None
}

fn tokenize_path_candidates(text: &str) -> Vec<String> {
    let mut candidates = Vec::new();

    for part in text.split_whitespace() {
        candidates.push(part.to_string());
    }

    for part in text.split(['=', ':', ',', ';']) {
        candidates.push(part.to_string());
    }

    for part in text.split('"') {
        candidates.push(part.to_string());
    }

    candidates
}

fn normalize_maybe_media_path(raw: &str) -> Option<PathBuf> {
    let mut value = raw.trim().trim_matches('"').trim_matches('\'').to_string();
    if value.is_empty() {
        return None;
    }

    if value.starts_with("file://") {
        value = value.trim_start_matches("file://").to_string();
        value = urlencoding::decode(&value).ok()?.to_string();
    }

    if value.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            value = home
                .join(value.trim_start_matches("~/"))
                .display()
                .to_string();
        }
    }

    let path = PathBuf::from(value);
    if path.exists() && is_media_file_path(&path) {
        return Some(path);
    }

    None
}

fn is_media_file_path(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(OsStr::to_str) else {
        return false;
    };
    MEDIA_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

fn normalize_path(raw: &str) -> PathBuf {
    let value = raw.trim().trim_matches('"').trim_matches('\'');
    if value.starts_with("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(value.trim_start_matches("~/"));
        }
    }
    PathBuf::from(value)
}

fn expand_tilde(path: &str) -> Option<PathBuf> {
    if path.starts_with("~/") {
        Some(dirs::home_dir()?.join(path.trim_start_matches("~/")))
    } else {
        Some(PathBuf::from(path))
    }
}

fn strip_comment(line: &str) -> &str {
    line.split('#').next().unwrap_or(line)
}

fn verbose_enabled() -> bool {
    env::var("CAVA_BG_WALLPAPER_VERBOSE")
        .map(|v| {
            let v = v.to_lowercase();
            v == "1" || v == "true" || v == "yes" || v == "on"
        })
        .unwrap_or(false)
}

fn vlog(message: &str) {
    if verbose_enabled() {
        info!("{}", message);
    } else {
        debug!("{}", message);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_temp_media_file(ext: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("cava_bg_wallpaper_detector_tests");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join(format!("ambxst_test_wallpaper.{}", ext));
        let _ = fs::write(&file, b"test");
        file
    }

    #[test]
    fn parse_ambxst_config_reads_current_wall() {
        let media = make_temp_media_file("mp4");
        let json = format!("{{\"currentWall\":\"{}\"}}", media.display());
        let detected = parse_ambxst_config(&json).expect("expected currentWall to be parsed");
        assert_eq!(detected, media);
    }

    #[test]
    fn parse_qml_source_reads_source_property() {
        let media = make_temp_media_file("webm");
        let qml = format!("WallpaperImage {{ source: \"{}\" }}", media.display());
        let detected = parse_qml_source(&qml).expect("expected QML source to be parsed");
        assert_eq!(detected, media);
    }

    #[test]
    fn from_ambxst_qml_detects_simulated_file() {
        let home = dirs::home_dir().expect("home directory should exist");
        let qml_path = home.join(".config/ambxst/Wallpaper.qml");
        let media = make_temp_media_file("mp4");

        let _ = fs::create_dir_all(
            qml_path
                .parent()
                .expect("Wallpaper.qml parent should exist"),
        );
        let backup = fs::read_to_string(&qml_path).ok();

        let qml_content = format!(
            "PanelWindow {{ WallpaperImage {{ source: \"{}\" }} }}",
            media.display()
        );
        fs::write(&qml_path, qml_content).expect("failed to write simulated Ambxst QML");

        let detected = from_ambxst_qml().expect("expected Ambxst QML detection");
        assert_eq!(detected, media);

        if let Some(previous) = backup {
            let _ = fs::write(&qml_path, previous);
        } else {
            let _ = fs::remove_file(&qml_path);
        }
    }
}
