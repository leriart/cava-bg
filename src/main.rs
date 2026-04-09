use anyhow::{Context, Result};
use clap::Parser;
use log::{info, warn};
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

mod cli;
mod config;
mod shader;
mod wallpaper;

use cli::*;
use config::*;

static RUNNING: AtomicBool = AtomicBool::new(true);

fn handle_signal() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        RUNNING.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    running
}

fn main() -> Result<()> {
    env_logger::init();

    let args = Cli::parse();

    if args.version {
        println!("cava-bg v{}", env!("CARGO_PKG_VERSION"));
        println!("Repository: https://github.com/leriart/cava-bg");
        println!();
        println!("A native Hyprland implementation of wallpaper-cava");
        println!("Displays CAVA audio visualizations as a layer over the wallpaper");
        println!("with adaptive color detection and automatic wallpaper change detection.");
        return Ok(());
    }

    if args.test_config {
        println!("Testing configuration and wallpaper analysis...");
        let config = Config::load(&args.config).context("Failed to load config")?;
        println!("Configuration loaded successfully:");
        println!("  Framerate: {}", config.general.framerate);
        println!("  Bars: {}", config.bars.amount);
        println!("  Colors: {}", config.colors.colors.len());
        println!("  Background color: {:?}", config.general.background_color);

        println!();
        println!("Testing wallpaper color detection and gradient generation...");
        match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
            Ok(colors) => {
                println!(
                    "Successfully generated {} gradient colors from wallpaper:",
                    colors.len()
                );
                for (i, color) in colors.iter().enumerate() {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8
                    );
                    println!(
                        "  Color {}: {} (RGB: {:.3}, {:.3}, {:.3})",
                        i + 1,
                        hex,
                        color[0],
                        color[1],
                        color[2]
                    );
                }
            }
            Err(e) => {
                println!("Failed to generate colors from wallpaper: {}", e);
                println!("Using default gradient colors instead.");
                let default_colors = wallpaper::WallpaperAnalyzer::default_colors(8);
                for (i, color) in default_colors.iter().enumerate() {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8
                    );
                    println!("  Default color {}: {}", i + 1, hex);
                }
            }
        }

        return Ok(());
    }

    let config = Config::load(&args.config).context("Failed to load config")?;

    info!("cava-bg starting with config: {:?}", args.config);
    info!("Auto colors: {}", config.general.auto_colors);

    // Check if cava is installed
    if Command::new("cava").arg("--version").output().is_err() {
        eprintln!("cava is not installed. Please install it:");
        eprintln!("  Arch: sudo pacman -S cava");
        eprintln!("  Debian/Ubuntu: sudo apt install cava");
        eprintln!("  Fedora: sudo dnf install cava");
        return Ok(());
    }

    // Set up signal handler
    let _signal_handler = handle_signal();

    println!("cava-bg starting with adaptive gradient colors and wallpaper change detection!");
    println!("Press Ctrl+C to exit.");
    println!();

    let mut current_wallpaper_path: Option<PathBuf> = None;
    let mut cava_process: Option<std::process::Child> = None;
    let mut last_wallpaper_check = Instant::now();
    let check_interval = Duration::from_secs(5); // Check for wallpaper changes every 5 seconds

    // Main loop
    while RUNNING.load(Ordering::SeqCst) {
        // Check for wallpaper changes
        if last_wallpaper_check.elapsed() >= check_interval {
            let new_wallpaper_path =
                wallpaper::WallpaperAnalyzer::get_current_wallpaper_path().unwrap_or(None);

            let wallpaper_changed = match (&current_wallpaper_path, &new_wallpaper_path) {
                (Some(old), Some(new)) => old != new,
                (None, Some(_)) => true, // No previous wallpaper, now we have one
                (Some(_), None) => true, // Had wallpaper, now it's gone
                (None, None) => false,   // No wallpaper before or now
            };

            if wallpaper_changed {
                info!("Wallpaper change detected!");

                // Kill existing cava process if running
                if let Some(mut process) = cava_process.take() {
                    info!("Stopping previous cava process...");
                    process.kill().ok();
                    process.wait().ok();
                }

                // Update current wallpaper path
                current_wallpaper_path = new_wallpaper_path.clone();

                if let Some(path) = &current_wallpaper_path {
                    println!("New wallpaper detected: {}", path.display());

                    let cava_config_path = dirs::cache_dir()
                        .context("Failed to get cache directory")?
                        .join("cava-bg-cava-config");

                    if config.general.auto_colors {
                        // Generate adaptive gradient colors from new wallpaper
                        info!("Generating gradient colors from new wallpaper (auto_colors enabled)...");
                        match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                            Ok(gradient_colors) => {
                                println!(
                                    "Generated {} gradient colors from wallpaper:",
                                    gradient_colors.len()
                                );

                                // Update config with new gradient colors
                                let mut adaptive_config = config.clone();
                                adaptive_config.colors.colors.clear();

                                for (i, color) in gradient_colors.iter().enumerate() {
                                    let hex = format!(
                                        "#{:02x}{:02x}{:02x}",
                                        (color[0] * 255.0) as u8,
                                        (color[1] * 255.0) as u8,
                                        (color[2] * 255.0) as u8
                                    );
                                    println!("  Color {}: {}", i + 1, hex);
                                    adaptive_config.colors.colors.insert(
                                        format!("gradient_color_{}", i + 1),
                                        Color::Hex(hex),
                                    );
                                }

                                // Generate cava config with new colors
                                let cava_config = adaptive_config.to_cava_config();
                                fs::write(&cava_config_path, cava_config)
                                    .context("Failed to write cava config")?;
                            }
                            Err(e) => {
                                warn!("Failed to generate gradient colors: {}", e);
                                println!("Using manual colors from configuration.");
                                // Use manual colors from config
                                let cava_config = config.to_cava_config();
                                fs::write(&cava_config_path, cava_config)
                                    .context("Failed to write cava config")?;
                            }
                        }
                    } else {
                        // Use manual colors from config
                        info!("Using manual colors (auto_colors disabled)");
                        println!("Using manual colors from configuration.");
                        let cava_config = config.to_cava_config();
                        fs::write(&cava_config_path, cava_config)
                            .context("Failed to write cava config")?;
                    }

                    info!("Starting cava process...");

                    // Start new cava process
                    match Command::new("cava")
                        .arg("-p")
                        .arg(&cava_config_path)
                        .stdout(Stdio::piped())
                        .spawn()
                    {
                        Ok(process) => {
                            cava_process = Some(process);
                            println!("cava restarted with new colors!");
                            println!("Config: {}", cava_config_path.display());
                        }
                        Err(e) => {
                            warn!("Failed to start cava process: {}", e);
                        }
                    }
                } else {
                    println!("Wallpaper removed or not found.");
                }
            }

            last_wallpaper_check = Instant::now();
        }

        // Sleep to prevent busy waiting
        thread::sleep(Duration::from_millis(100));
    }

    // Cleanup
    info!("Shutting down...");

    if let Some(mut process) = cava_process.take() {
        info!("Stopping cava process...");
        process.kill().ok();
        process.wait().ok();
    }

    println!("cava-bg stopped.");
    info!("cava-bg shutting down.");

    Ok(())
}
