// src/main.rs
// Punto de entrada principal con soporte para comando "kill", configuración en ~/.config/cava-bg/
// y renderizador Wayland nativo (compatible con hardware antiguo).

mod app_config;
mod wallpaper;
mod wayland_renderer;
mod cava_backend;

use anyhow::{Context, Result};
use log::{error, info};
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, exit};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use app_config::Config;
use cava_backend::CavaBackend;
use wayland_renderer::WaylandRenderer;

const CONFIG_DIR: &str = "cava-bg";
const CONFIG_FILE: &str = "config.toml";

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();

    // Comando "kill": terminar cualquier instancia existente de cava-bg
    if args.len() >= 2 && args[1] == "kill" {
        return kill_existing_instance();
    }

    // Determinar la ruta del archivo de configuración
    let config_path = get_config_path(&args);

    // Crear directorio de configuración si no existe
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Cargar o crear configuración por defecto
    let config = load_or_create_config(&config_path)?;

    // Detección automática de colores del wallpaper
    let mut config = config;
    if config.general.auto_colors {
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
            Err(e) => {
                error!("Failed to generate auto colors: {}", e);
            }
        }
    }

    // Bandera para controlar la ejecución (Ctrl+C)
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");

    // Iniciar el backend de Cava
    let bar_count = config.bars.amount as usize;
    let (_cava_backend, audio_rx) = CavaBackend::new(bar_count, &config)
        .context("Failed to start cava backend")?;

    // Lanzar el renderizador Wayland nativo
    info!("Starting Wayland renderer (OpenGL 3.0)");
    let renderer = WaylandRenderer::new(config, audio_rx, running);
    renderer.run()?;

    Ok(())
}

/// Obtiene la ruta del archivo de configuración según los argumentos de línea de comandos
/// o la ruta por defecto en ~/.config/cava-bg/config.toml
fn get_config_path(args: &[String]) -> PathBuf {
    if args.len() == 3 && args[1] == "--config" {
        return PathBuf::from(&args[2]);
    }
    let home = dirs::home_dir().expect("Could not determine home directory");
    home.join(".config").join(CONFIG_DIR).join(CONFIG_FILE)
}

/// Carga la configuración desde un archivo TOML; si no existe, crea una por defecto.
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

/// Mata cualquier proceso existente de cava-bg usando `pkill`.
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