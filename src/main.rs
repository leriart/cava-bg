mod cli;
mod config;
mod cava_manager;
mod wallpaper;
mod wayland_renderer;

use anyhow::{Context, Result};
use log::{error, info};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread;

use notify::{RecursiveMode, Watcher};

fn main() -> Result<()> {
    let cli = cli::Cli::parse();

    if cli.version {
        cli::Cli::show_version();
        return Ok(());
    }

    cli.init_logging();

    let mut config = config::Config::load(&cli.config).context("Failed to load config")?;

    if cli.test_config {
        println!("Testing configuration and wallpaper analysis...");
        println!(
            "Config loaded: framerate={}, bars={}",
            config.general.framerate, config.bars.amount
        );
        if config.general.auto_colors {
            match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                Ok(colors) => {
                    println!("Generated {} colors from wallpaper:", colors.len());
                    for (i, c) in colors.iter().enumerate() {
                        let hex = format!(
                            "#{:02x}{:02x}{:02x}",
                            (c[0] * 255.0) as u8,
                            (c[1] * 255.0) as u8,
                            (c[2] * 255.0) as u8
                        );
                        println!("  {}: {} {:?}", i + 1, hex, c);
                    }
                }
                Err(e) => println!("Failed to generate colors: {}", e),
            }
        }
        return Ok(());
    }

    if Command::new("cava").arg("--version").output().is_err() {
        eprintln!("cava is not installed. Please install it first.");
        eprintln!("  Arch: sudo pacman -S cava");
        eprintln!("  Debian/Ubuntu: sudo apt install cava");
        eprintln!("  Fedora: sudo dnf install cava");
        return Ok(());
    }

    info!("Starting cava-bg v{}", env!("CARGO_PKG_VERSION"));

    if config.general.auto_colors {
        info!("Auto-colors enabled, analyzing wallpaper...");
        match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
            Ok(gradient) => {
                config.colors.colors.clear();
                for (i, &[r, g, b, a]) in gradient.iter().enumerate() {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (r * 255.0) as u8,
                        (g * 255.0) as u8,
                        (b * 255.0) as u8
                    );
                    config.colors.colors.insert(
                        format!("gradient_color_{}", i + 1),
                        config::Color::HexWithAlpha { hex, alpha: a },
                    );
                }
                info!("Generated {} gradient colors from wallpaper", gradient.len());
            }
            Err(e) => {
                error!("Failed to extract colors from wallpaper: {}", e);
                info!("Using default colors");
            }
        }
    }

    let mut cava_manager =
        cava_manager::CavaManager::new(&config).context("Failed to start cava manager")?;
    let cava_reader = cava_manager
        .take_reader()
        .context("Failed to get cava reader")?;

    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
    })
    .expect("Failed to set Ctrl+C handler");

    let (color_tx, color_rx) = mpsc::channel();

    if config.general.auto_colors {
        let tx = color_tx.clone();
        thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Ok(Some(wallpaper_path)) = wallpaper::WallpaperAnalyzer::find_wallpaper() {
                let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
                    if let Ok(_event) = res {
                        info!("Wallpaper changed, regenerating colors...");
                        if let Ok(colors) = wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                            tx.send(colors).ok();
                        }
                    }
                }).expect("Failed to create watcher");
                
                if watcher
                    .watch(&wallpaper_path, RecursiveMode::NonRecursive)
                    .is_ok()
                {
                    info!("Watching wallpaper for changes: {:?}", wallpaper_path);
                    loop {
                        std::thread::park();
                    }
                } else {
                    error!("Failed to watch wallpaper file");
                }
            }
        });
    }

    let wayland_renderer =
        wayland_renderer::WaylandRenderer::new(config.clone(), cava_reader, color_rx, running);
    if let Err(e) = wayland_renderer.run() {
        error!("Wayland renderer failed: {}", e);
        return Err(e);
    }

    Ok(())
}