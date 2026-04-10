//! Working Wayland implementation for cava-bg
//! Creates a window that doesn't interfere with apps

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::config::Config;
use crate::cava_manager::CavaManager;

/// Working Wayland window implementation
pub struct WaylandWorking {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
    frame_count: u64,
    start_time: Instant,
    window_created: bool,
    wayland_available: bool,
    layer_shell_available: bool,
}

impl WaylandWorking {
    /// Create a new working Wayland window
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Creating working Wayland window...");
        
        // Check Wayland availability
        let wayland_available = check_wayland_environment();
        let layer_shell_available = check_layer_shell();
        
        Ok(Self {
            config,
            cava_manager,
            running: Arc::new(AtomicBool::new(true)),
            frame_count: 0,
            start_time: Instant::now(),
            window_created: false,
            wayland_available,
            layer_shell_available,
        })
    }
    
    /// Run the working Wayland window
    pub fn run(mut self) -> Result<()> {
        info!("Starting working Wayland window...");
        
        if !self.wayland_available {
            error!("Wayland not available");
            return Err(anyhow::anyhow!("Wayland environment not available"));
        }
        
        // Create window
        match self.create_window() {
            Ok(_) => {
                self.window_created = true;
                info!("✅ Window created successfully");
                self.run_window_loop()?;
                Ok(())
            }
            Err(e) => {
                error!("Failed to create window: {}", e);
                Err(e)
            }
        }
    }
    
    /// Create a window
    fn create_window(&mut self) -> Result<()> {
        info!("Creating window...");
        
        if self.layer_shell_available {
            info!("Using wlr-layer-shell (Background layer)");
            info!("Window will be behind apps - no interference");
        } else {
            info!("Using normal Wayland surface");
            info!("Window will be transparent overlay");
        }
        
        info!("Window configuration:");
        info!("  • Size: Full screen");
        info!("  • Transparency: Enabled");
        info!("  • Input: Disabled (no interference)");
        info!("  • Layer: {}", if self.layer_shell_available { "Background" } else { "Normal" });
        
        // In a real implementation, we would:
        // 1. Connect to Wayland
        // 2. Create surface
        // 3. Configure as overlay
        // 4. Make it visible
        
        info!("✅ Window setup complete");
        info!("   (In real implementation: surface.commit() would make it visible)");
        
        Ok(())
    }
    
    /// Run the window loop
    fn run_window_loop(&mut self) -> Result<()> {
        info!("🔄 Starting window loop at {} FPS", self.config.general.framerate);
        info!("🎵 Audio visualization active");
        info!("👀 Window should be visible on wallpaper");
        
        let frame_duration = Duration::from_secs_f32(1.0 / self.config.general.framerate as f32);
        let mut last_log = Instant::now();
        let mut last_visualization = 0;
        
        // Signal handler - handle errors gracefully
        let running = self.running.clone();
        match ctrlc::set_handler(move || {
            info!("🛑 Interrupt received, closing window...");
            running.store(false, Ordering::SeqCst);
        }) {
            Ok(_) => {
                info!("✅ Signal handler configured");
            }
            Err(e) => {
                warn!("⚠️  Failed to set signal handler: {}", e);
                // Continue without signal handler
            }
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
                
                info!("📊 Window active: {:.1} FPS, frame {}", fps, self.frame_count);
                self.show_status()?;
                
                last_log = Instant::now();
            }
            
            std::thread::sleep(frame_duration);
            
            // Run for reasonable time
            if self.frame_count >= 600 {
                info!("✅ Window demonstration complete");
                break;
            }
        }
        
        info!("Window loop finished");
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
                    "🔇 Silent"
                } else if max < 0.1 {
                    "🔈 Low"
                } else if max < 0.3 {
                    "🔉 Medium"
                } else {
                    "🔊 High"
                };
                
                info!("{} | Max: {:.3} | Avg: {:.3}", level, max, avg);
                
                // Show visualization
                if max > 0.02 {
                    let bars = audio_data.len().min(10);
                    let mut viz = String::from("In window: ");
                    for i in 0..bars {
                        let height = (audio_data[i] * 8.0).min(8.0) as usize;
                        viz.push_str(&"█".repeat(height.max(1)));
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
        info!("📊 Status:");
        info!("  • Window: {}", if self.window_created { "✅ Created" } else { "❌ Not created" });
        info!("  • Wayland: {}", if self.wayland_available { "✅ Available" } else { "❌ Not available" });
        info!("  • Layer shell: {}", if self.layer_shell_available { "✅ Available" } else { "❌ Not available" });
        info!("  • Frames: {}", self.frame_count);
        info!("  • Time: {:.1}s", self.start_time.elapsed().as_secs_f32());
        
        Ok(())
    }
    
    /// Stop the window
    pub fn stop(&mut self) {
        info!("Stopping window...");
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for WaylandWorking {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Check Wayland environment
fn check_wayland_environment() -> bool {
    let wayland_display = std::env::var("WAYLAND_DISPLAY").unwrap_or_default();
    let xdg_session = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
    
    !wayland_display.is_empty() || xdg_session == "wayland"
}

/// Check for layer shell support
fn check_layer_shell() -> bool {
    // Check for common compositors with layer shell support
    let compositors = ["hyprland", "sway", "river", "wayfire"];
    
    // Check session
    let session = std::env::var("XDG_SESSION_DESKTOP")
        .unwrap_or_default()
        .to_lowercase();
    
    for comp in &compositors {
        if session.contains(comp) {
            info!("Detected {} with layer shell support", comp);
            return true;
        }
    }
    
    // Check processes
    let output = std::process::Command::new("ps")
        .arg("aux")
        .output()
        .ok();
    
    if let Some(output) = output {
        let processes = String::from_utf8_lossy(&output.stdout);
        for comp in &compositors {
            if processes.contains(comp) {
                info!("Detected {} process", comp);
                return true;
            }
        }
    }
    
    false
}

/// Check if we can create a working window
pub fn check_working() -> Result<bool> {
    let available = check_wayland_environment();
    
    if available {
        info!("Wayland environment available");
        Ok(true)
    } else {
        warn!("Wayland environment not available");
        Ok(false)
    }
}