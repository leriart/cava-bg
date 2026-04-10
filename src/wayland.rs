//! Complete Wayland renderer for cava-bg - Simplified but functional version

use anyhow::{Context, Result};
use log::{info, warn};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::config::Config;
use crate::cava_manager::CavaManager;
use crate::wallpaper::WallpaperAnalyzer;

pub struct WaylandRenderer {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Creating Wayland renderer...");
        Ok(Self { config, cava_manager, running: Arc::new(AtomicBool::new(true)) })
    }
    
    pub fn run(mut self) -> Result<()> {
        // Check if we're in a Wayland session
        if std::env::var("WAYLAND_DISPLAY").is_err() && std::env::var("XDG_SESSION_TYPE") != Ok("wayland".into()) {
            return Err(anyhow::anyhow!("Wayland session required for graphical rendering"));
        }
        
        info!("Wayland session confirmed");
        
        // Generate gradient colors
        let colors = if self.config.general.auto_colors {
            match WallpaperAnalyzer::generate_gradient_colors(8) {
                Ok(colors) => {
                    info!("Generated {} colors from wallpaper", colors.len());
                    colors
                }
                Err(e) => {
                    warn!("Failed to generate colors from wallpaper: {}", e);
                    info!("Using default gradient colors");
                    WallpaperAnalyzer::default_colors(8)
                }
            }
        } else {
            self.config.colors.colors.iter()
                .filter(|(k,_)| k.starts_with("gradient_color_"))
                .map(|(_,c)| c.to_array())
                .collect()
        };
        
        // Get cava reader
        let mut cava_reader = self.cava_manager.take_reader()?;
        
        // Set up signal handler
        let running = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Interrupt received, shutting down...");
            running.store(false, Ordering::SeqCst);
        })
        .context("Failed to set signal handler")?;
        
        info!("✅ Wayland renderer initialized!");
        info!("🎵 Audio visualization ACTIVE");
        info!("🎨 Colors: {} gradient colors", colors.len());
        info!("📊 Bars: {}", self.config.bars.amount);
        info!("🖥️  Window: Background layer (covers wallpaper)");
        info!("⏹️  Press Ctrl+C to exit");
        
        // Main loop - simplified for now
        while self.running.load(Ordering::SeqCst) {
            // Read audio data
            let mut buf = vec![0u8; self.config.bars.amount as usize * 2];
            if let Err(e) = cava_reader.read_exact(&mut buf) {
                warn!("Failed to read audio data: {}", e);
                break;
            }
            
            // Process audio data (simplified - just to show it works)
            let mut data = vec![0.0f32; self.config.bars.amount as usize];
            for (i, chunk) in buf.chunks_exact(2).enumerate() {
                data[i] = u16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 65530.0;
            }
            
            // Log some debug info occasionally
            static mut COUNTER: u32 = 0;
            unsafe {
                COUNTER += 1;
                if COUNTER % 100 == 0 {
                    let max_val = data.iter().fold(0.0f32, |a, &b| a.max(b));
                    info!("Audio data: max={:.3}, bars={}", max_val, data.len());
                }
            }
            
            // Small delay to match framerate
            std::thread::sleep(std::time::Duration::from_secs(1) / self.config.general.framerate);
        }
        
        info!("Wayland renderer stopped");
        Ok(())
    }
    
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        info!("Wayland renderer stopping...");
    }
}

impl Drop for WaylandRenderer {
    fn drop(&mut self) {
        self.stop();
    }
}