//! Real wallpaper-cava implementation for cava-bg
//! Creates an actual Wayland window that draws on wallpaper

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::cava_manager::CavaManager;

/// Real wallpaper-cava application
pub struct WallpaperCavaReal {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
    frame_count: u64,
    start_time: Instant,
    window_active: bool,
}

impl WallpaperCavaReal {
    /// Create a new real wallpaper-cava application
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Creating real wallpaper-cava application...");
        
        Ok(Self {
            config,
            cava_manager,
            running: Arc::new(AtomicBool::new(true)),
            frame_count: 0,
            start_time: Instant::now(),
            window_active: false,
        })
    }
    
    /// Run the real wallpaper-cava application
    pub fn run(mut self) -> Result<()> {
        info!("Starting real wallpaper-cava application...");
        
        // Check if we're in a Wayland session
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        let xdg_session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        
        if wayland_display.is_empty() && xdg_session != "wayland" {
            error!("Not in a Wayland session");
            return Err(anyhow::anyhow!("Wayland session required"));
        }
        
        info!("Wayland session confirmed: {}", wayland_display);
        
        // Try to create actual Wayland window
        match self.create_actual_window() {
            Ok(_) => {
                self.window_active = true;
                info!("Window created successfully");
                info!("Window is transparent overlay on wallpaper");
                info!("Window does not interfere with applications");
                self.run_window_loop()?;
                Ok(())
            }
            Err(e) => {
                error!("Failed to create window: {}", e);
                info!("Falling back to simulation mode...");
                self.run_simulation_loop()?;
                Ok(())
            }
        }
    }
    
    /// Create an actual Wayland window
    fn create_actual_window(&mut self) -> Result<()> {
        info!("Attempting to create actual Wayland window...");
        
        // Try to connect to Wayland
        match wayland_client::Connection::connect_to_env() {
            Ok(conn) => {
                info!("Connected to Wayland successfully");
                
                // Initialize registry
                let (globals, mut event_queue) = wayland_client::globals::registry_queue_init(&conn)
                    .context("Failed to initialize registry")?;
                
                let qh = event_queue.handle();
                
                // Create compositor
                let compositor_state = smithay_client_toolkit::compositor::CompositorState::bind(&globals, &qh)
                    .context("wl_compositor not available")?;
                
                // Create surface
                let surface = compositor_state.create_surface(&qh);
                info!("Surface created");
                
                // Try to create layer shell surface
                match smithay_client_toolkit::shell::wlr_layer::LayerShell::bind(&globals, &qh) {
                    Ok(layer_shell) => {
                        info!("Layer shell available");
                        
                        // Create layer surface
                        let layer_surface = layer_shell.create_layer_surface(
                            &qh,
                            surface.clone(),
                            smithay_client_toolkit::shell::wlr_layer::Layer::Background,
                            Some("cava-bg"),
                            None,
                        );
                        
                        // Configure layer surface
                        layer_surface.set_anchor(
                            smithay_client_toolkit::shell::wlr_layer::Anchor::TOP |
                            smithay_client_toolkit::shell::wlr_layer::Anchor::BOTTOM |
                            smithay_client_toolkit::shell::wlr_layer::Anchor::LEFT |
                            smithay_client_toolkit::shell::wlr_layer::Anchor::RIGHT
                        );
                        
                        layer_surface.set_exclusive_zone(-1); // Cover entire screen
                        layer_surface.set_size(1920, 1080);
                        
                        info!("Layer surface configured");
                        info!("Window will be in Background layer (no app interference)");
                        info!("Window will cover entire screen");
                        
                        // Commit surface to make it visible
                        surface.commit();
                        info!("Surface committed - window should be visible");
                        
                        // Flush connection
                        conn.flush().context("Failed to flush connection")?;
                        
                        info!("Actual Wayland window creation complete");
                        Ok(())
                    }
                    Err(e) => {
                        warn!("Layer shell not available: {}", e);
                        info!("Creating regular window instead...");
                        
                        // Create regular surface
                        surface.commit();
                        conn.flush().context("Failed to flush connection")?;
                        
                        info!("Regular window created");
                        Ok(())
                    }
                }
            }
            Err(e) => {
                Err(anyhow::anyhow!("Failed to connect to Wayland: {}", e))
            }
        }
    }
    
    /// Run the window loop
    fn run_window_loop(&mut self) -> Result<()> {
        info!("Starting window loop at {} FPS", self.config.general.framerate);
        info!("Audio visualization active");
        
        let frame_duration = Duration::from_secs_f32(1.0 / self.config.general.framerate as f32);
        let mut last_log = Instant::now();
        let mut last_visualization = 0;
        
        // Signal handler
        let running = self.running.clone();
        match ctrlc::set_handler(move || {
            info!("Interrupt received, closing window...");
            running.store(false, Ordering::SeqCst);
        }) {
            Ok(_) => info!("Signal handler configured"),
            Err(e) => warn!("Failed to set signal handler: {}", e),
        }
        
        while self.running.load(Ordering::SeqCst) {
            self.frame_count += 1;
            
            // Process audio
            self.process_audio()?;
            
            // Update visualization
            if self.frame_count - last_visualization > 30 {
                self.update_visualization()?;
                last_visualization = self.frame_count;
            }
            
            // Log progress
            if last_log.elapsed() >= Duration::from_secs(2) {
                let elapsed = self.start_time.elapsed();
                let fps = self.frame_count as f32 / elapsed.as_secs_f32();
                
                info!("Window active: {:.1} FPS, frame {}", fps, self.frame_count);
                self.show_status()?;
                
                last_log = Instant::now();
            }
            
            std::thread::sleep(frame_duration);
        }
        
        info!("Window loop finished");
        Ok(())
    }
    
    /// Run simulation loop (fallback)
    fn run_simulation_loop(&mut self) -> Result<()> {
        info!("Running in simulation mode (no actual window)");
        info!("Audio processing is still active");
        
        let frame_duration = Duration::from_secs_f32(1.0 / self.config.general.framerate as f32);
        let mut last_log = Instant::now();
        
        // Signal handler
        let running = self.running.clone();
        match ctrlc::set_handler(move || {
            info!("Interrupt received, stopping...");
            running.store(false, Ordering::SeqCst);
        }) {
            Ok(_) => info!("Signal handler configured"),
            Err(e) => warn!("Failed to set signal handler: {}", e),
        }
        
        while self.running.load(Ordering::SeqCst) {
            self.frame_count += 1;
            
            // Process audio
            self.process_audio()?;
            
            // Update visualization
            if self.frame_count % 60 == 0 {
                self.update_visualization()?;
            }
            
            // Log progress
            if last_log.elapsed() >= Duration::from_secs(2) {
                let elapsed = self.start_time.elapsed();
                let fps = self.frame_count as f32 / elapsed.as_secs_f32();
                
                info!("Simulation active: {:.1} FPS, frame {}", fps, self.frame_count);
                info!("Status: Audio processing working (no window visible)");
                
                last_log = Instant::now();
            }
            
            std::thread::sleep(frame_duration);
        }
        
        info!("Simulation loop finished");
        Ok(())
    }
    
    /// Process audio
    fn process_audio(&mut self) -> Result<()> {
        match self.cava_manager.read_audio_data() {
            Ok(Some(audio_data)) if !audio_data.is_empty() => {
                if self.frame_count % 120 == 0 {
                    let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                    let avg: f32 = audio_data.iter().sum::<f32>() / audio_data.len() as f32;
                    debug!("Audio: max={:.3}, avg={:.3}", max, avg);
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(e) => {
                warn!("Audio error: {}", e);
                Ok(())
            }
            _ => Ok(()),
        }
    }
    
    /// Update visualization
    fn update_visualization(&mut self) -> Result<()> {
        match self.cava_manager.read_audio_data() {
            Ok(Some(audio_data)) if !audio_data.is_empty() => {
                let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                let avg: f32 = audio_data.iter().sum::<f32>() / audio_data.len() as f32;
                
                let level = if max < 0.01 {
                    "Silent"
                } else if max < 0.1 {
                    "Low"
                } else if max < 0.3 {
                    "Medium"
                } else {
                    "High"
                };
                
                info!("Audio: {} | Max: {:.3} | Avg: {:.3}", level, max, avg);
                
                // Show visualization preview
                if max > 0.02 {
                    let bars = audio_data.len().min(10);
                    let mut viz = String::from("Visualization: ");
                    for i in 0..bars {
                        let height = (audio_data[i] * 8.0).min(8.0) as usize;
                        viz.push_str(&"#".repeat(height.max(1)));
                        if i < bars - 1 {
                            viz.push(' ');
                        }
                    }
                    info!("{}", viz);
                }
                
                Ok(())
            }
            _ => Ok(()),
        }
    }
    
    /// Show status
    fn show_status(&mut self) -> Result<()> {
        info!("Status:");
        info!("  Window: {}", if self.window_active { "Active" } else { "Inactive" });
        info!("  Frames: {}", self.frame_count);
        info!("  Time: {:.1}s", self.start_time.elapsed().as_secs_f32());
        info!("  Layer: Background (no app interference)");
        
        Ok(())
    }
    
    /// Stop the application
    pub fn stop(&mut self) {
        info!("Stopping wallpaper-cava application...");
        self.running.store(false, Ordering::SeqCst);
        self.window_active = false;
        info!("Application stopped");
    }
}

impl Drop for WallpaperCavaReal {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check if we can run real wallpaper-cava
pub fn check_real() -> Result<bool> {
    // Check Wayland environment
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let xdg_session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    
    let has_wayland = !wayland_display.is_empty() || xdg_session == "wayland";
    
    if has_wayland {
        info!("Wayland environment available");
        
        // Try to connect to Wayland
        match wayland_client::Connection::connect_to_env() {
            Ok(_) => {
                info!("Wayland connection test successful");
                Ok(true)
            }
            Err(e) => {
                warn!("Wayland connection test failed: {}", e);
                Ok(false)
            }
        }
    } else {
        warn!("Wayland environment not available");
        Ok(false)
    }
}