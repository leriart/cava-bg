mod app_config;
mod wallpaper;
mod wayland_renderer;

use anyhow::{Context, Result};
use log::{error, info};
use std::collections::HashMap;
use std::fs;
use std::io::{BufReader, Write};
use std::process::{ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, SystemTime};

use app_config::*;
use wallpaper::WallpaperAnalyzer;
use wayland_renderer::WaylandRenderer;

const MAX_RETRIES: u32 = 5;

fn main() -> Result<()> {
    env_logger::init();

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

    let auto_colors_enabled = config.general.auto_colors;

    // Auto-colors inicial
    if auto_colors_enabled {
        match WallpaperAnalyzer::generate_gradient_colors(8) {
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
                        ConfigColor::Complex(HexColorConfig {
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

    // Función para lanzar cava (se usará en cada reintento)
    let spawn_cava = || -> Result<BufReader<ChildStdout>> {
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
        Ok(BufReader::new(cava_stdout))
    };

    // Control de ejecución
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    // Canal para actualizaciones de color, compartido con Arc<Mutex<Receiver>>
    let (color_tx, color_rx) = mpsc::channel();
    let shared_color_rx = Arc::new(Mutex::new(color_rx));

    // Hilo de vigilancia de wallpaper (único, no se reinicia)
    if auto_colors_enabled {
        let tx = color_tx.clone();
        thread::spawn(move || {
            let mut last_path: Option<std::path::PathBuf> = None;
            let mut last_modified: Option<SystemTime> = None;
            let mut last_sent = SystemTime::now();
            loop {
                thread::sleep(Duration::from_millis(1500));
                match WallpaperAnalyzer::find_wallpaper() {
                    Some(current_path) => {
                        let modified = std::fs::metadata(&current_path)
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
                            if now.duration_since(last_sent).unwrap_or(Duration::ZERO) < Duration::from_millis(500) {
                                last_path = Some(current_path);
                                last_modified = modified;
                                continue;
                            }
                            
                            match WallpaperAnalyzer::generate_gradient_colors(8) {
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
                    None => {
                        thread::sleep(Duration::from_secs(3));
                    }
                }
            }
        });
    }

    // Reinicio automático del renderer
    let mut retries = 0;
    loop {
        // Iniciar cava (nuevo proceso)
        let cava_reader = match spawn_cava() {
            Ok(r) => r,
            Err(e) => {
                error!("Failed to spawn cava: {}", e);
                if retries >= MAX_RETRIES {
                    break;
                }
                retries += 1;
                thread::sleep(Duration::from_secs(2));
                continue;
            }
        };

        let renderer = WaylandRenderer::new(
            config.clone(),
            cava_reader,
            shared_color_rx.clone(),
            running.clone(),
        );
        match renderer.run() {
            Ok(()) => break, // Salida limpia
            Err(e) => {
                error!("Renderer failed: {}", e);
                if retries >= MAX_RETRIES {
                    error!("Max retries reached, giving up.");
                    break;
                }
                retries += 1;
                info!("Restarting renderer in 2 seconds (attempt {}/{})...", retries, MAX_RETRIES);
                thread::sleep(Duration::from_secs(2));
            }
        }
    }

    Ok(())
}