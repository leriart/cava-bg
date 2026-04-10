use anyhow::Result;
use log::{error, info, warn};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;

/// Renderer for cava-bg
/// Currently runs in terminal mode with audio visualization
pub struct Renderer {
    running: Arc<AtomicBool>,
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
        
        if use_wayland {
            info!("Wayland session detected");
            info!("Note: Graphical rendering not yet implemented");
        } else {
            info!("Running in terminal mode");
        }
        
        Ok(Self {
            running: Arc::new(AtomicBool::new(true)),
            config,
            cava_manager: Some(cava_manager),
        })
    }
    
    /// Start the render loop
    pub fn run(&mut self) -> Result<()> {
        info!("Starting render loop...");
        
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
                            "Silent"
                        } else if max < 0.1 {
                            "Low"
                        } else if max < 0.3 {
                            "Medium"
                        } else {
                            "High"
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
                                    0 => "_",
                                    1 => ".",
                                    2 => ":",
                                    3 => "=",
                                    4 => "+",
                                    5 => "*",
                                    6 => "#",
                                    _ => "@",
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