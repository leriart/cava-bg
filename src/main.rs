mod app_config;
mod wallpaper;
mod wayland_renderer;

use anyhow::{Context, Result};
use log::{error, info};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use app_config::Config;
use wayland_renderer::WaylandRenderer;

const CONFIG_DIR: &str = "cava-bg";
const CONFIG_FILE: &str = "config.toml";

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() >= 2 && args[1] == "kill" {
        return kill_existing_instance();
    }

    let config_path = get_config_path(&args);
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut config = load_or_create_config(&config_path)?;
    let auto_colors_enabled = config.general.auto_colors;

    if auto_colors_enabled {
        info!("Initial wallpaper detection...");
        match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
            Ok(generated) => {
                info!("Auto-colors: replacing config colors with wallpaper palette");
                config.colors.clear();
                for (i, &color) in generated.iter().enumerate() {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8
                    );
                    config.colors.insert(
                        format!("gradient_color_{}", i + 1),
                        app_config::ConfigColor::Complex(app_config::HexColorConfig {
                            hex,
                            alpha: Some(color[3]),
                        }),
                    );
                }
            }
            Err(e) => error!("Failed to generate auto colors: {}", e),
        }
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    let (color_tx, color_rx) = mpsc::channel();
    let shared_color_rx = Arc::new(Mutex::new(color_rx));

    if auto_colors_enabled {
        let tx = color_tx.clone();
        thread::spawn(move || {
            let mut last_path: Option<PathBuf> = None;
            let mut last_modified: Option<SystemTime> = None;
            let mut last_sent = SystemTime::now();

            loop {
                thread::sleep(Duration::from_millis(1500));
                match wallpaper::WallpaperAnalyzer::find_wallpaper() {
                    Some(current_path) => {
                        let modified = fs::metadata(&current_path)
                            .and_then(|m| m.modified())
                            .ok();
                        let path_changed = last_path.as_ref() != Some(&current_path);
                        let time_changed = match (&last_modified, &modified) {
                            (Some(l), Some(m)) => l != m,
                            _ => true,
                        };

                        if path_changed || time_changed {
                            info!("Wallpaper changed: {:?}", current_path);
                            let now = SystemTime::now();
                            if now
                                .duration_since(last_sent)
                                .unwrap_or(Duration::ZERO)
                                < Duration::from_millis(500)
                            {
                                last_path = Some(current_path);
                                last_modified = modified;
                                continue;
                            }
                            match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                                Ok(colors) => {
                                    if tx.send(colors).is_err() {
                                        error!("Failed to send new colors, stopping watcher.");
                                        break;
                                    }
                                    last_sent = SystemTime::now();
                                }
                                Err(e) => error!("Failed to generate colors: {}", e),
                            }
                            last_path = Some(current_path);
                            last_modified = modified;
                        }
                    }
                    None => thread::sleep(Duration::from_secs(3)),
                }
            }
        });
    }

    info!("Starting Wayland WGPU renderer");
    let renderer = WaylandRenderer::new(config, shared_color_rx, running);
    renderer.run()?;

    Ok(())
}

fn get_config_path(args: &[String]) -> PathBuf {
    if args.len() == 3 && args[1] == "--config" {
        return PathBuf::from(&args[2]);
    }
    let home = dirs::home_dir().expect("Could not determine home directory");
    home.join(".config").join(CONFIG_DIR).join(CONFIG_FILE)
}

fn load_or_create_config(path: &PathBuf) -> Result<Config> {
    if path.exists() {
        let content = fs::read_to_string(path)
            .with_context(|| format!("Failed to read config file: {}", path.display()))?;
        let config: Config = toml::from_str(&content)
            .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
        Ok(config)
    } else {
        info!("Config file not found, creating default at {:?}", path);
        let default_config = Config::default();
        let toml_string = toml::to_string_pretty(&default_config)?;
        fs::write(path, toml_string)?;
        Ok(default_config)
    }
}

fn kill_existing_instance() -> Result<()> {
    let output = Command::new("pgrep")
        .arg("-f")
        .arg("cava-bg")
        .output()
        .context("Failed to execute pgrep")?;

    if output.status.success() {
        let pids = String::from_utf8_lossy(&output.stdout);
        for pid in pids.lines() {
            info!("Killing process {}", pid);
            Command::new("kill")
                .arg(pid)
                .status()
                .context(format!("Failed to kill process {}", pid))?;
        }
        println!("cava-bg processes terminated.");
    } else {
        println!("No running cava-bg process found.");
    }
    exit(0);
}