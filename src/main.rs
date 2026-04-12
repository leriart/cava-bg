mod app_config;
mod wallpaper;
mod wayland_renderer;

use anyhow::{Context, Result};
use log::info;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};

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

    let config = load_or_create_config(&config_path)?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Canal para colores (no usado activamente si no hay auto_colors, pero mantenemos por compatibilidad)
    let (_color_tx, color_rx) = mpsc::channel();
    let shared_color_rx = Arc::new(Mutex::new(color_rx));

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
        let default_config = Config {
            general: app_config::GeneralConfig {
                framerate: 60,
                background_color: app_config::ConfigColor::Simple("#000000".to_string()),
                autosens: Some(true),
                sensitivity: Some(1.0),
                preferred_output: None,
            },
            bars: app_config::BarConfig {
                amount: 8,
                gap: 0.1,
            },
            colors: {
                let mut colors = std::collections::HashMap::new();
                let default_colors = vec![
                    "#94e2d5", "#89dceb", "#74c7ec", "#89b4fa",
                    "#cba6f7", "#f5c2e7", "#eba0ac", "#f38ba8"
                ];
                for (i, hex) in default_colors.iter().enumerate() {
                    colors.insert(
                        format!("gradient_color_{}", i + 1),
                        app_config::ConfigColor::Simple(hex.to_string()),
                    );
                }
                colors
            },
            smoothing: app_config::SmoothingConfig {
                monstercat: Some(1.0),
                waves: Some(0),
                noise_reduction: Some(0.8),
            },
        };
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