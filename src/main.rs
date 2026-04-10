use anyhow::{Context, Result};
use log::info;
use std::fs;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

mod cava_manager;
mod cli;
mod config;
mod renderer;
mod shader;
mod wallpaper;
mod wayland_working;

use cli::*;
use config::*;
use cava_manager::CavaManager;


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

/// Run terminal renderer
fn run_terminal_renderer(config: Config, cava_manager: CavaManager) -> Result<()> {
    println!("\nInitializing terminal visualizer...");
    let mut renderer = match renderer::Renderer::new(config.clone(), cava_manager) {
        Ok(r) => {
            println!("Terminal visualizer initialized");
            r
        }
        Err(e) => {
            eprintln!("Visualizer warning: {}", e);
            eprintln!("Continuing with basic audio processing...");
            return Err(e);
        }
    };

    println!("\nStarting audio visualizer...");
    println!("========================================");
    println!("Status: Audio processing ACTIVE");
    println!("Mode: Terminal (audio visualization)");
    println!("Bars: {}", config.bars.amount);
    println!("Framerate: {}", config.general.framerate);
    println!("Colors: {}", if config.general.auto_colors {
        "Adaptive (from wallpaper)"
    } else {
        "Manual configuration"
    });
    println!("========================================");
    println!("\nTo test: Play audio (music, video, etc.)");
    println!("Audio visualization will be shown in terminal...");
    println!("\nTip: For graphical visualization:");
    println!("  Run under Hyprland, Sway, or another Wayland compositor");
    println!();

    // Run the renderer
    if let Err(e) = renderer.run() {
        eprintln!("Renderer error: {}", e);
        return Err(e);
    }
    
    Ok(())
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

    // Initialize cava manager with raw output (inspired by wallpaper-cava)
    println!("Initializing cava with raw audio output (16-bit)...");
    let cava_manager = match cava_manager::CavaManager::new(&config) {
        Ok(manager) => {
            println!("✓ cava initialized successfully with raw output");
            println!("  Bars: {}", config.bars.amount);
            println!("  Framerate: {}", config.general.framerate);
            if config.general.auto_colors {
                println!("  Colors: Adaptive (from wallpaper)");
            } else {
                println!("  Colors: Manual (from config)");
            }
            manager
        }
        Err(e) => {
            eprintln!("Failed to initialize cava: {}", e);
            eprintln!("Falling back to standard cava mode...");

            // Fallback to old method
            let cava_config_path = dirs::cache_dir()
                .context("Failed to get cache directory")?
                .join("cava-bg-cava-config");
            let cava_config = config.to_cava_config();
            fs::write(&cava_config_path, cava_config)
                .context("Failed to write cava config")?;

            match Command::new("cava")
                .arg("-p")
                .arg(&cava_config_path)
                .stdout(Stdio::piped())
                .spawn()
            {
                Ok(_process) => {
                    println!("cava started in fallback mode");
                    // Create a simple wrapper for fallback mode
                    return Ok(());
                }
                Err(e) => {
                    return Err(anyhow::anyhow!("Failed to start cava in fallback mode: {}", e));
                }
            }
        }
    };

    // Start monitor thread for cava
    cava_manager.start_monitor(config.clone());

    // Check if we can create a WORKING Wayland window
    let can_create_working = wayland_working::check_working().unwrap_or(false);
    
    if can_create_working {
        println!("\n🎯 Wayland detectado - creando ventana FUNCIONAL...");
        
        // Create a new cava_manager for working window
        let mut working_cava_manager = CavaManager::new(&config)?;
        working_cava_manager.start(&config)?;
        
        // Try to create WORKING window
        println!("🚀 Inicializando ventana Wayland FUNCIONAL...");
        match wayland_working::WaylandWorking::new(config.clone(), working_cava_manager) {
            Ok(working_window) => {
                println!("✅ Ventana FUNCIONAL inicializada");
                
                println!("\n🎵 Iniciando visualizador de audio...");
                println!("========================================");
                println!("Estado: Procesamiento de audio ACTIVO");
                println!("Modo: Wayland (ventana sobre wallpaper)");
                println!("Barras: {}", config.bars.amount);
                println!("FPS: {}", config.general.framerate);
                println!("Colores: {}", if config.general.auto_colors {
                    "Adaptativos (del wallpaper)"
                } else {
                    "Configuración manual"
                });
                println!("========================================");
                println!("\n🎧 Para probar: Reproduce audio (música, video, etc.)");
                println!("👀 ¡Busca una ventana transparente sobre tu wallpaper!");
                println!("💡 La ventana NO interfiere con apps");
                println!();
                
                // Run WORKING window
                if let Err(e) = working_window.run() {
                    eprintln!("❌ Error en ventana FUNCIONAL: {}", e);
                    eprintln!("↪️  Volviendo a modo terminal...");
                    
                    // Run terminal renderer
                    run_terminal_renderer(config, cava_manager)?;
                }
            }
            Err(e) => {
                eprintln!("❌ Inicialización de ventana FUNCIONAL falló: {}", e);
                eprintln!("↪️  Volviendo a modo terminal...");
                
                // Run terminal renderer
                run_terminal_renderer(config, cava_manager)?;
            }
        }
    } else {
        // Cannot create WORKING window, use terminal
        println!("\n⚠️  No se puede crear ventana FUNCIONAL, usando modo terminal...");
        run_terminal_renderer(config, cava_manager)?;
    }
    
    println!("\ncava-bg stopping...");
    info!("cava-bg shutting down.");
    
    Ok(())
}
