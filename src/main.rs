mod app_config;
mod cli_help;
mod wayland_renderer;
mod wallpaper;

use anyhow::{Context, Result};
use log::info;
use std::collections::HashMap; // <-- importación agregada
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use app_config::{Config, GeneralConfig, BarConfig, SmoothingConfig, ConfigColor};
use cli_help::print_help;
use wayland_renderer::WaylandRenderer;

fn kill_cava_bg() -> Result<()> {
    use std::process::Command;
    let output = Command::new("pkill")
        .arg("cava-bg")
        .output()
        .context("Failed to execute pkill")?;
    if output.status.success() {
        println!("cava-bg process terminated");
        Ok(())
    } else {
        anyhow::bail!("No cava-bg process found or failed to kill");
    }
}

fn create_default_config(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let default_config = Config {
        general: GeneralConfig {
            framerate: 60,
            background_color: ConfigColor::Simple("#000000".to_string()),
            autosens: Some(true),
            sensitivity: None,
            preferred_output: None,
            dynamic_colors: Some(true), // por defecto activado
        },
        bars: BarConfig {
            amount: 32,
            gap: 0.05,
        },
        colors: {
            let mut map = HashMap::new();
            map.insert("gradient1".to_string(), ConfigColor::Simple("#ff0000".to_string()));
            map.insert("gradient2".to_string(), ConfigColor::Simple("#00ff00".to_string()));
            map.insert("gradient3".to_string(), ConfigColor::Simple("#0000ff".to_string()));
            map
        },
        smoothing: SmoothingConfig {
            monstercat: Some(0.5),
            waves: None,
            noise_reduction: None,
        },
    };
    let toml_string = toml::to_string_pretty(&default_config)?;
    fs::write(path, toml_string)?;
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();

    // Comando kill
    if args.len() == 2 && args[1] == "kill" {
        return kill_cava_bg();
    }

    let config_path = if args.len() == 3 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else if args.len() == 1 {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default = PathBuf::from(format!("{}/.config/cava-bg/config.toml", home));
        if !default.exists() {
            info!("Creating default config at {:?}", default);
            create_default_config(&default)
                .with_context(|| format!("Failed to create default config at {:?}", default))?;
        }
        default
    } else {
        print_help();
        exit(0);
    };

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Unable to read config file: {:?}", config_path))?;
    let mut config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Error parsing config: {:?}", config_path))?;

    // Asegurar que dynamic_colors tenga un valor por defecto
    if config.general.dynamic_colors.is_none() {
        config.general.dynamic_colors = Some(true);
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting cava-bg with wgpu backend");
    let renderer = WaylandRenderer::new(config, running);
    renderer.run()?;

    Ok(())
}