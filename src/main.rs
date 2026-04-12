mod app_config;
mod cli_help;
mod wayland_renderer;
mod wallpaper;

use anyhow::{Context, Result};
use log::info;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use app_config::{Config, GeneralConfig, BarConfig, SmoothingConfig, ConfigColor};
use cli_help::print_help;
use wayland_renderer::WaylandRenderer;

fn create_default_config(path: &PathBuf) -> Result<()> {
    let default_config = Config {
        general: GeneralConfig {
            framerate: 60,
            background_color: ConfigColor::Simple("#000000".to_string()),
            autosens: Some(true),
            sensitivity: None,
            preferred_output: None,
            dynamic_colors: true,
        },
        bars: BarConfig {
            amount: 30,
            gap: 0.05,
        },
        colors: {
            let mut map = HashMap::new();
            map.insert("gradient1".to_string(), ConfigColor::Simple("#89b4fa".to_string()));
            map.insert("gradient2".to_string(), ConfigColor::Simple("#cba6f7".to_string()));
            map.insert("gradient3".to_string(), ConfigColor::Simple("#f38ba8".to_string()));
            map
        },
        smoothing: SmoothingConfig {
            monstercat: Some(0.5),
            waves: None,
            noise_reduction: None,
        },
    };
    let toml_str = toml::to_string_pretty(&default_config)
        .context("Failed to serialize default config")?;
    fs::write(path, toml_str)
        .with_context(|| format!("Failed to write default config to {:?}", path))?;
    info!("Created default config at {:?}", path);
    Ok(())
}

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

fn main() -> Result<()> {
    env_logger::init();
    let args: Vec<String> = env::args().collect();

    if args.len() == 2 && (args[1] == "kill" || args[1] == "--kill") {
        return kill_cava_bg();
    }

    if args.len() == 2 && (args[1] == "-h" || args[1] == "--help") {
        print_help();
        exit(0);
    }

    let config_path = if args.len() == 3 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else if args.len() == 1 {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default = PathBuf::from(format!("{}/.config/cava-bg/config.toml", home));
        if default.exists() {
            default
        } else {
            // Si no existe, creamos el directorio y el archivo por defecto
            if let Some(parent) = default.parent() {
                fs::create_dir_all(parent)
                    .context("Failed to create config directory")?;
            }
            create_default_config(&default)?;
            default
        }
    } else {
        print_help();
        exit(0);
    };

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Unable to read config file: {:?}", config_path))?;
    let config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Error parsing config: {:?}", config_path))?;

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