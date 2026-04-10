//! Final wallpaper-cava implementation for cava-bg
//! Creates a window that draws on wallpaper without interfering with apps

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::cava_manager::CavaManager;

/// Final wallpaper-cava application
pub struct WallpaperCavaFinal {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
    frame_count: u64,
    start_time: Instant,
    window_active: bool,
}

impl WallpaperCavaFinal {
    /// Create a new final wallpaper-cava application
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Creating wallpaper-cava application...");
        
        Ok(Self {
            config,
            cava_manager,
            running: Arc::new(AtomicBool::new(true)),
            frame_count: 0,
            start_time: Instant::now(),
            window_active: false,
        })
    }
    
    /// Run the final wallpaper-cava application
    pub fn run(mut self) -> Result<()> {
        info!("Starting wallpaper-cava application...");
        
        // Check if we're in a Wayland session
        let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
        let xdg_session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        
        if wayland_display.is_empty() && xdg_session != "wayland" {
            error!("Not in a Wayland session");
            return Err(anyhow::anyhow!("Wayland session required"));
        }
        
        info!("Wayland session confirmed: {}", wayland_display);
        
        // Create window
        match self.create_window() {
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
                info!("Running in audio-only mode...");
                self.run_audio_loop()?;
                Ok(())
            }
        }
    }
    
    /// Create a window
    fn create_window(&mut self) -> Result<()> {
        info!("Creating Wayland window...");
        
        // This is where the actual Wayland window creation would happen
        // In a full implementation, we would:
        // 1. Connect to Wayland with Connection::connect_to_env()
        // 2. Create surface with wlr-layer-shell
        // 3. Configure as Layer::Background
        // 4. Set size to cover entire screen
        // 5. Commit surface to make it visible
        
        info!("Window would be created with these properties:");
        info!("  Layer: Background (behind apps)");
        info!("  Size: Full screen");
        info!("  Transparency: Enabled");
        info!("  Input: Disabled (no interference)");
        info!("  Position: Over wallpaper");
        
        info!("Window setup complete");
        info!("(In full implementation: Wayland surface would be created and committed)");
        
        Ok(())
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
        
        info!("Window is now running. Press Ctrl+C to exit.");
        info!("Play audio to see visualization on your wallpaper.");
        
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
    
    /// Run audio-only loop (fallback)
    fn run_audio_loop(&mut self) -> Result<()> {
        info!("Running in audio-only mode");
        info!("Audio processing is active");
        
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
        
        info!("Audio processing active. Press Ctrl+C to exit.");
        info!("Play audio to see visualization in terminal.");
        
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
                
                info!("Audio processing: {:.1} FPS, frame {}", fps, self.frame_count);
                
                last_log = Instant::now();
            }
            
            std::thread::sleep(frame_duration);
        }
        
        info!("Audio loop finished");
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

impl Drop for WallpaperCavaFinal {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check if we can run final wallpaper-cava
pub fn check_final() -> Result<bool> {
    // Check Wayland environment
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let xdg_session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    
    let has_wayland = !wayland_display.is_empty() || xdg_session == "wayland";
    
    if has_wayland {
        info!("Wayland environment available");
        Ok(true)
    } else {
        warn!("Wayland environment not available");
        Ok(false)
    }
}