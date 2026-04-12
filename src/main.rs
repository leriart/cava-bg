mod app_config;
mod cli_help;
mod wayland_renderer;
mod wallpaper;  

use anyhow::{Context, Result};
use log::info;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::exit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use app_config::{Config, GeneralConfig, BarConfig, SmoothingConfig, ConfigColor};
use cli_help::print_help;
use wayland_renderer::WaylandRenderer;

/// Crea un archivo de configuración por defecto en la ruta especificada.
fn create_default_config(path: &PathBuf) -> Result<()> {
    // Crear el directorio padre si no existe
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
            dynamic_colors: true,  
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
    info!("Created default config at {:?}", path);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    let config_path = if args.len() == 3 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else if args.len() == 1 {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_path = PathBuf::from(format!("{}/.config/wallpaper-cava/config.toml", home));
        if !default_path.exists() {
            // Crear configuración por defecto si no existe
            create_default_config(&default_path)
                .with_context(|| format!("Failed to create default config at {:?}", default_path))?;
        }
        default_path
    } else {
        print_help();
        exit(0);
    };

    let config_str = fs::read_to_string(&config_path)
        .with_context(|| format!("Unable to read config file: {:?}", config_path))?;
    let mut config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Error parsing config: {:?}", config_path))?;

    
    if config.general.dynamic_colors.is_nil() {
        
    }

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting wallpaper-cava with wgpu backend");
    let renderer = WaylandRenderer::new(config, running);
    renderer.run()?;

    Ok(())
}