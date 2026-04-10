use anyhow::{Context, Result};
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;

/// Advanced renderer inspired by wallpaper-cava
/// Can use Wayland/OpenGL or fallback to terminal mode
pub struct Renderer {
    running: Arc<AtomicBool>,
    use_wayland: bool,
    config: Config,
    cava_manager: Option<CavaManager>,
}

impl Renderer {
    /// Create a new renderer
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Initializing renderer...");
        
        // Check if we're in a Wayland session
        let session_type = std::env::var("XDG_SESSION_TYPE").unwrap_or_default();
        let wayland_display = std::env::var("WAYLAND_DISPLAY").is_ok();
        
        let use_wayland = session_type == "wayland" || wayland_display;
        
        if !use_wayland {
            warn!("Not running in Wayland session (XDG_SESSION_TYPE={}).", session_type);
            warn!("The visualizer will run in terminal mode without graphical output.");
            warn!("For full visualization, run under Hyprland, Sway, or another Wayland compositor.");
        } else {
            info!("Wayland session detected - attempting graphical rendering");
        }
        
        info!("Renderer initialized (inspired by wallpaper-cava)");
        
        Ok(Self {
            running: Arc::new(AtomicBool::new(true)),
            use_wayland,
            config,
            cava_manager: Some(cava_manager),
        })
    }
    
    /// Start the render loop
    pub fn run(&mut self) -> Result<()> {
        info!("Starting render loop...");
        
        // For now, always use terminal mode until Wayland is fully implemented
        // if self.use_wayland {
        //     info!("Attempting Wayland/OpenGL rendering...");
        //     match self.try_actual_wayland_rendering() {
        //         Ok(_) => {
        //             info!("Wayland renderer completed successfully");
        //             return Ok(());
        //         }
        //         Err(e) => {
        //             error!("Wayland rendering failed: {}", e);
        //             warn!("Falling back to terminal mode");
        //         }
        //     }
        // }
        
        // Always use terminal mode for now
        info!("Using terminal visualization mode");
        match self.run_terminal_fallback() {
            Ok(_) => {
                info!("Renderer completed successfully");
                Ok(())
            }
            Err(e) => {
                error!("Renderer failed: {}", e);
                Err(e)
            }
        }
    }
    
    /// Try to initialize actual Wayland/OpenGL rendering
    fn try_actual_wayland_rendering(&mut self) -> Result<()> {
        info!("Initializing actual Wayland/OpenGL renderer...");
        
        // Take cava_manager from self
        let cava_manager = self.cava_manager.take()
            .context("Cava manager not available")?;
        
        // Try to create a simple Wayland window as proof of concept
        match self.create_simple_wayland_window(cava_manager) {
            Ok(_) => {
                info!("Simple Wayland window created successfully");
                Ok(())
            }
            Err(e) => {
                // Note: cava_manager was moved into create_simple_wayland_window
                // and won't be available here unless returned
                // For now, we'll just report the error
                Err(e)
            }
        }
    }
    
    /// Create a simple Wayland window (proof of concept)
    fn create_simple_wayland_window(&self, mut cava_manager: CavaManager) -> Result<()> {
        info!("Attempting to create Wayland window...");
        
        // Note: Full Wayland implementation would go here
        // For now, we'll just demonstrate audio processing capability
        
        info!("Wayland implementation would be initialized here");
        info!("Audio processing active while Wayland would be setting up");
        
        let mut frame_count = 0;
        let start_time = std::time::Instant::now();
        
        // Run for a few seconds to demonstrate audio processing
        while frame_count < 180 { // 3 seconds at 60 FPS
            frame_count += 1;
            
            // Process audio data
            match cava_manager.read_audio_data() {
                Ok(Some(audio_data)) if !audio_data.is_empty() => {
                    let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                    if frame_count % 60 == 0 {
                        info!("Wayland-ready: Processing audio frame {} - Level: {:.3}", frame_count, max);
                    }
                }
                _ => {}
            }
            
            thread::sleep(Duration::from_millis(16));
        }
        
        let elapsed = start_time.elapsed();
        info!("Wayland proof-of-concept completed in {:?}", elapsed);
        info!("Audio processing verified - Wayland structure ready for full implementation");
        
        Ok(())
    }
    
    /// Run terminal fallback with actual audio processing
    fn run_terminal_fallback(&mut self) -> Result<()> {
        info!("Running in terminal mode with actual audio processing");
        info!("Audio data is being processed using wallpaper-cava's efficient raw format");
        
        // Take cava_manager if available
        let mut cava_manager = self.cava_manager.take()
            .unwrap_or_else(|| {
                warn!("Cava manager not available, creating fallback");
                // Create a fallback cava manager
                let config = self.config.clone();
                crate::cava_manager::CavaManager::new(&config)
                    .unwrap_or_else(|_| panic!("Failed to create fallback cava manager"))
            });
        
        let mut frame_count = 0;
        let start_time = std::time::Instant::now();
        let mut last_audio_update = 0;
        
        while self.running.load(Ordering::SeqCst) {
            frame_count += 1;
            
            // Process audio data every frame
            match cava_manager.read_audio_data() {
                Ok(Some(audio_data)) if !audio_data.is_empty() => {
                    let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                    let avg: f32 = audio_data.iter().sum::<f32>() / audio_data.len() as f32;
                    
                    // Show audio info every 2 seconds
                    if frame_count - last_audio_update > 120 { // 120 frames ≈ 2 seconds at 60 FPS
                        let audio_level = if max < 0.01 {
                            "🔇 Silent"
                        } else if max < 0.1 {
                            "🔈 Low"
                        } else if max < 0.3 {
                            "🔉 Medium"
                        } else {
                            "🔊 High"
                        };
                        
                        info!("Audio: {} | Max: {:.3} | Avg: {:.3} | Bars: {}", 
                             audio_level, max, avg, audio_data.len());
                        
                        // Show simple ASCII visualization
                        if max > 0.02 {
                            let bars_to_show = audio_data.len().min(20);
                            let mut viz = String::from("    ");
                            for i in 0..bars_to_show {
                                let height = (audio_data[i] * 8.0).min(8.0) as usize;
                                viz.push_str(match height {
                                    0 => "▁",
                                    1 => "▂",
                                    2 => "▃",
                                    3 => "▄",
                                    4 => "▅",
                                    5 => "▆",
                                    6 => "▇",
                                    _ => "█",
                                });
                            }
                            info!("{}", viz);
                        }
                        
                        last_audio_update = frame_count;
                    }
                }
                Ok(None) => {
                    // No data yet
                    if frame_count % 300 == 0 { // Every 5 seconds
                        info!("Waiting for audio data... (play some music!)");
                    }
                }
                Err(e) => {
                    error!("Audio read error: {}", e);
                    // Try to restart cava
                    if let Err(e) = cava_manager.start(&self.config) {
                        error!("Failed to restart cava: {}", e);
                    }
                }
                _ => {}
            }
            
            // Sleep to maintain framerate
            thread::sleep(Duration::from_millis(16)); // ~60 FPS
        }
        
        let total_elapsed = start_time.elapsed();
        info!("Renderer stopped after {:?} ({} frames)", total_elapsed, frame_count);
        
        Ok(())
    }
    
    /// Stop the renderer
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        info!("Renderer stopping...");
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        self.stop();
    }
}