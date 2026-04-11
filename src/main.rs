// src/main.rs
mod app_config;
mod wallpaper;

use anyhow::{Context, Result};
use log::{error, info};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Write};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use app_config::*;
use wallpaper::WallpaperAnalyzer;

mod wayland_renderer;
use wayland_renderer::WaylandRenderer;

fn main() -> Result<()> {
    env_logger::init();

    // Manejo simple de argumentos (como el original)
    let args: Vec<String> = std::env::args().collect();
    let config_filename = if args.len() == 3 && args[1] == "--config" {
        args[2].clone()
    } else {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let path = format!("{}/.config/wallpaper-cava/config.toml", home);
        if fs::metadata(&path).is_ok() {
            path
        } else {
            "config.toml".to_string()
        }
    };

    let config_str = fs::read_to_string(&config_filename)
        .context("Unable to read config file")?;
    let mut config: Config = toml::from_str(&config_str)
        .map_err(|e| anyhow::anyhow!("Error parsing config: {}", e))?;

    // --- Auto-colors: si está activado en la configuración nueva (lo añadimos manualmente) ---
    // Podemos añadir un campo `auto_colors` en GeneralConfig, pero para no modificar app_config,
    // usamos una variable de entorno o un archivo adicional. Para simplificar, siempre intentamos
    // extraer colores del wallpaper y reemplazar los definidos en config.colors.
    if let Ok(generated) = WallpaperAnalyzer::generate_gradient_colors(8) {
        info!("Auto-colors: replacing config colors with wallpaper palette");
        config.colors.clear();
        for (i, &color) in generated.iter().enumerate() {
            let hex = format!("#{:02x}{:02x}{:02x}", 
                (color[0]*255.0) as u8, (color[1]*255.0) as u8, (color[2]*255.0) as u8);
            config.colors.insert(
                format!("gradient_color_{}", i+1),
                ConfigColor::Complex(HexColorConfig { hex, alpha: Some(color[3]) })
            );
        }
    } else {
        info!("Using colors from config file");
    }

    // Configurar cava (exactamente como en el original)
    let cava_output_config: HashMap<String, String> = HashMap::from([
        ("method".into(), "raw".into()),
        ("raw_target".into(), "/dev/stdout".into()),
        ("bit_format".into(), "16bit".into()),
    ]);
    let cava_config = CavaConfig {
        general: CavaGeneralConfig {
            framerate: config.general.framerate,
            bars: config.bars.amount,
            autosens: config.general.autosens,
            sensitivity: config.general.sensitivity,
        },
        smoothing: CavaSmoothingConfig {
            monstercat: config.smoothing.monstercat,
            waves: config.smoothing.waves,
            noise_reduction: config.smoothing.noise_reduction,
        },
        output: cava_output_config,
    };
    let string_cava_config = toml::to_string(&cava_config).unwrap();

    let mut cmd = Command::new("cava");
    cmd.arg("-p").arg("/dev/stdin");
    let mut cava_process = cmd
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn cava process")?;

    if let Some(mut stdin) = cava_process.stdin.take() {
        stdin.write_all(string_cava_config.as_bytes())?;
        stdin.flush()?;
    }
    let cava_stdout = cava_process.stdout.take().context("failed to get cava stdout")?;
    let cava_reader = BufReader::new(cava_stdout);

    // Configurar Ctrl+C para salir limpiamente
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    }).expect("Error setting Ctrl-C handler");

    // Iniciar el renderer Wayland (idéntico al original)
    let renderer = WaylandRenderer::new(config, cava_reader);
    renderer.run()?;

    Ok(())
}