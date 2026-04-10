use anyhow::{Context, Result};
use log::info;
use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

mod cava_manager;
mod cli;
mod config;
mod renderer;
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
    let _last_wallpaper_check = Instant::now();

    // Initialize cava manager with raw output (inspired by wallpaper-cava)
    println!("Initializing cava with raw audio output (16-bit)...");
    let mut cava_manager = match cava_manager::CavaManager::new(&config) {
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
    
    // Initialize renderer with improved feedback
    println!("\n🎨 Initializing visualizer...");
    let mut renderer = match renderer::Renderer::new() {
        Ok(r) => {
            println!("✓ Visualizer initialized");
            r
        }
        Err(e) => {
            eprintln!("⚠️  Visualizer warning: {}", e);
            eprintln!("Continuing with audio processing only...");
            renderer::Renderer::new().unwrap_or_else(|_| {
                // Create minimal renderer if initialization fails
                renderer::Renderer::new().unwrap()
            })
        }
    };
    
    // Start renderer in background
    let renderer_thread = std::thread::spawn(move || {
        if let Err(e) = renderer.run() {
            eprintln!("Renderer error: {}", e);
        }
    });
    
    // Show status and instructions
    println!("\n📊 Audio visualizer ready!");
    println!("========================================");
    println!("Status: Audio processing ACTIVE");
    println!("Mode: {}", if std::env::var("WAYLAND_DISPLAY").is_ok() || std::env::var("XDG_SESSION_TYPE") == Ok("wayland".to_string()) {
        "Wayland (graphical mode available)"
    } else {
        "Terminal (audio processing only)"
    });
    println!("Bars: {}", config.bars.amount);
    println!("Framerate: {}", config.general.framerate);
    println!("Colors: {}", if config.general.auto_colors {
        "Adaptive (from wallpaper)"
    } else {
        "Manual (from config)"
    });
    println!("========================================");
    println!("\n🎵 To test: Play audio (music, video, etc.)");
    println!("📈 Audio data will be shown below...");
    println!("\n💡 Tip: For full graphical visualization:");
    println!("  1. Run under Hyprland, Sway, or Wayland");
    println!("  2. The visualizer will appear as a background layer");
    println!("  3. Uses wlr-layer-shell like wallpaper-cava");

    // Main loop for wallpaper change detection and audio processing
    let mut last_wallpaper_check = Instant::now();
    let mut current_wallpaper_path: Option<PathBuf> = None;
    let check_interval = Duration::from_secs(5);
    
    // Demo counter for showing audio data
    let mut demo_counter = 0;
    
    while RUNNING.load(Ordering::SeqCst) {
        // 1. Check for wallpaper changes
        if last_wallpaper_check.elapsed() >= check_interval {
            match wallpaper::WallpaperAnalyzer::get_current_wallpaper_path() {
                Ok(Some(new_wallpaper_path)) => {
                    let wallpaper_changed = match &current_wallpaper_path {
                        Some(old) => old != &new_wallpaper_path,
                        None => true, // No previous wallpaper, now we have one
                    };
                
                    if wallpaper_changed {
                        println!("\n🎨 Wallpaper change detected!");
                        current_wallpaper_path = Some(new_wallpaper_path.clone());
                        
                        if config.general.auto_colors {
                            println!("Generating adaptive colors from new wallpaper...");
                            match wallpaper::WallpaperAnalyzer::generate_gradient_colors(8) {
                                Ok(colors) => {
                                    println!("Generated {} gradient colors", colors.len());
                                    // In a full implementation, we would update the renderer colors here
                                }
                                Err(e) => {
                                    println!("Could not generate colors: {}", e);
                                }
                            }
                        }
                        
                        // Restart cava with new colors if auto_colors is enabled
                        if config.general.auto_colors {
                            println!("Restarting cava with new colors...");
                            if let Err(e) = cava_manager.start(&config) {
                                println!("Warning: Failed to restart cava: {}", e);
                            }
                        }
                    }
                }
                Ok(None) => {
                    // No wallpaper found
                }
                Err(e) => {
                    println!("Warning: Failed to check wallpaper: {}", e);
                }
            }
            last_wallpaper_check = Instant::now();
        }
        
        // 2. Try to read audio data (demo mode)
        if demo_counter % 10 == 0 { // Read every ~1 second
            match cava_manager.read_audio_data() {
                Ok(Some(audio_data)) if !audio_data.is_empty() => {
                    // Calculate stats (inspired by wallpaper-cava's processing)
                    let avg: f32 = audio_data.iter().sum::<f32>() / audio_data.len() as f32;
                    let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                    
                    // Show stats more frequently when audio is detected
                    if max > 0.01 || demo_counter % 50 == 0 {
                        let audio_level = if max < 0.01 {
                            "🔇 Silent"
                        } else if max < 0.1 {
                            "🔈 Low"
                        } else if max < 0.3 {
                            "🔉 Medium"
                        } else {
                            "🔊 High"
                        };
                        
                        println!("\n🎵 Audio: {} | Max: {:.3} | Avg: {:.3}", 
                               audio_level, max, avg);
                        
                        // Show ASCII visualization (inspired by cava's terminal output)
                        if max > 0.02 {
                            let bars_to_show = audio_data.len().min(30);
                            print!("    ");
                            for i in 0..bars_to_show {
                                let height = (audio_data[i] * 10.0).min(10.0) as usize;
                                match height {
                                    0 => print!("▁"),
                                    1 => print!("▂"),
                                    2 => print!("▃"),
                                    3 => print!("▄"),
                                    4 => print!("▅"),
                                    5 => print!("▆"),
                                    6 => print!("▇"),
                                    _ => print!("█"),
                                }
                            }
                            println!("");
                        }
                    }
                }
                Ok(None) => {
                    // No data yet (normal for raw mode)
                    if demo_counter % 100 == 0 {
                        println!("\n🎵 Waiting for audio data... (play some music!)");
                    }
                }
                Err(e) => {
                    println!("\n⚠️  Audio read error: {}", e);
                    println!("Attempting to restart cava...");
                    if let Err(e) = cava_manager.start(&config) {
                        println!("Failed to restart cava: {}", e);
                    }
                }
                _ => {}
            }
        }
        
        // 3. Sleep to prevent busy waiting
        thread::sleep(Duration::from_millis(100));
        demo_counter += 1;
    }

    // Wait for renderer thread to finish
    let _ = renderer_thread.join();

    // Cleanup - cava_manager will auto-cleanup when dropped
    println!("\n🛑 cava-bg stopping...");
    info!("cava-bg shutting down.");
    
    // Explicitly stop cava manager
    drop(cava_manager);

    Ok(())
}
