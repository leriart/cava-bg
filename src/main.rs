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
use std::fs::File;
use std::io::{Read, Write};

use app_config::Config;
use cli_help::print_help;
use wayland_renderer::WaylandRenderer;

const PID_FILE: &str = "/tmp/cava-bg.pid";

/// Verifica si ya hay una instancia en ejecución.
fn check_single_instance() -> Result<bool> {
    if let Ok(mut file) = File::open(PID_FILE) {
        let mut pid_str = String::new();
        file.read_to_string(&mut pid_str).ok();
        if let Ok(old_pid) = pid_str.trim().parse::<i32>() {
            let exists = unsafe { libc::kill(old_pid, 0) == 0 };
            if exists {
                eprintln!("Another instance of cava-bg is already running (PID {}).", old_pid);
                eprintln!("Use 'cava-bg kill' to stop it.");
                return Ok(false);
            }
        }
    }
    let mut file = File::create(PID_FILE)?;
    write!(file, "{}", std::process::id())?;
    Ok(true)
}

/// Mata la instancia existente usando el archivo PID.
fn kill_existing_instance() -> Result<()> {
    if let Ok(mut file) = File::open(PID_FILE) {
        let mut pid_str = String::new();
        file.read_to_string(&mut pid_str)?;
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe { libc::kill(pid, libc::SIGTERM); }
            std::thread::sleep(std::time::Duration::from_millis(200));
            let _ = fs::remove_file(PID_FILE);
            println!("cava-bg process (PID {}) terminated.", pid);
            return Ok(());
        }
    }
    Err(anyhow::anyhow!("No PID file found or process not running."))
}

/// Crea un archivo de configuración por defecto en la ruta especificada.
/// El archivo incluye comentarios explicativos en inglés.
fn create_default_config(path: &PathBuf) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Usamos un delimitador de cadena raw con tres almohadillas para evitar conflictos
    // con las secuencias "# que aparecen en el contenido.
    let default_config_str = r###"# =============================================================================
# cava-bg configuration file
# =============================================================================
#
# This file controls the appearance and behavior of the cava-bg audio visualizer.
# After editing, restart cava-bg for changes to take effect.
#
# For more information, visit: https://github.com/leriart/cava-bg
#

# -----------------------------------------------------------------------------
# General settings
# -----------------------------------------------------------------------------
[general]
# Framerate of the visualization (frames per second). Higher = smoother but more CPU usage.
# Default: 60
framerate = 60

# Automatically extract colors from your current wallpaper.
# Set to false to use manually defined colors in the [colors] section.
dynamic_colors = true

# Corner radius of the entire layer window (in pixels). 0 = square corners.
# Works only if the background is not fully transparent. Useful for rounded overlays.
corner_radius = 0.0

# Background color of the layer. Use alpha = 0.0 for complete transparency.
[general.background_color]
hex = "#000000"
alpha = 0.0

# Uncomment to let cava auto-adjust sensitivity (recommended for most users)
# autosens = true
# Uncomment to set a fixed sensitivity (0-200). Disables autosens.
# sensitivity = 100
# Uncomment to show only on a specific monitor (get name via `hyprctl monitors` or `wlr-randr`)
# preferred_output = "DP-1"

# -----------------------------------------------------------------------------
# Bars settings
# -----------------------------------------------------------------------------
[bars]
# Number of bars to display. More bars = finer resolution but more GPU load.
amount = 76

# Gap between bars as a fraction of bar width. 0.1 = gap is 10% of bar width.
gap = 0.1

# Opacity of the bars (0.0 = fully transparent, 1.0 = fully opaque).
bar_alpha = 0.7

# -----------------------------------------------------------------------------
# Colors (only used when dynamic_colors = false)
# -----------------------------------------------------------------------------
[colors]
# Each entry defines a color in the gradient (from bottom to top).
# You can add or remove entries; the number of colors determines gradient steps.
gradient_color_1 = "#94e2d5"   # Teal
gradient_color_2 = "#89dceb"   # Sky
gradient_color_3 = "#74c7ec"   # Sapphire
gradient_color_4 = "#89b4fa"   # Blue
gradient_color_5 = "#cba6f7"   # Mauve
gradient_color_6 = "#f5c2e7"   # Pink
gradient_color_7 = "#eba0ac"   # Red
gradient_color_8 = "#f38ba8"   # Maroon

# -----------------------------------------------------------------------------
# Smoothing (passed directly to cava)
# -----------------------------------------------------------------------------
[smoothing]
# Uncomment to enable Monstercat smoothing (0 = off, 0.5 = medium, 1 = very smooth)
# monstercat = 0.5
# Uncomment to enable waves (requires cava compiled from GitHub)
# waves = 0
# Uncomment to set noise reduction (0 = fast/noisy, 1 = slow/smooth)
# noise_reduction = 0.77
"###;

    fs::write(path, default_config_str)?;
    info!("Created default config at {:?}", path);
    Ok(())
}

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();

    if args.len() == 2 && args[1] == "kill" {
        return kill_existing_instance();
    }

    if !check_single_instance()? {
        std::process::exit(1);
    }

    let config_path = if args.len() == 3 && args[1] == "--config" {
        PathBuf::from(&args[2])
    } else if args.len() == 1 {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        let default_path = PathBuf::from(format!("{}/.config/cava-bg/config.toml", home));
        if !default_path.exists() {
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
    let config: Config = toml::from_str(&config_str)
        .with_context(|| format!("Error parsing config: {:?}", config_path))?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        let _ = fs::remove_file(PID_FILE);
    })
    .expect("Error setting Ctrl-C handler");

    info!("Starting cava-bg with wgpu backend");
    let renderer = WaylandRenderer::new(config, running);
    renderer.run()?;

    let _ = fs::remove_file(PID_FILE);
    Ok(())
}