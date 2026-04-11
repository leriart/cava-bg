use anyhow::Result;
use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;

pub struct Renderer {
    running: Arc<AtomicBool>,
    config: Config,
    cava_manager: Option<CavaManager>,
}

impl Renderer {
    pub fn new(config: Config, mut cava_manager: CavaManager) -> Result<Self> {
        let _ = cava_manager.take_reader();
        Ok(Self {
            running: Arc::new(AtomicBool::new(true)),
            config,
            cava_manager: Some(cava_manager),
        })
    }

    pub fn run(&mut self) -> Result<()> {
        info!("Running in terminal mode (audio visualization only)");
        let mut frame_count = 0;
        let start_time = std::time::Instant::now();
        let mut last_audio_update = 0;

        while self.running.load(Ordering::SeqCst) {
            frame_count += 1;
            if frame_count - last_audio_update > 120 {
                info!("Audio visualization active ({} frames)", frame_count);
                last_audio_update = frame_count;
            }
            thread::sleep(Duration::from_millis(16));
        }
        let elapsed = start_time.elapsed();
        info!("Terminal renderer stopped after {:?}, {} frames", elapsed, frame_count);
        Ok(())
    }
}