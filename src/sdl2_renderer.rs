use anyhow::Result;
use log::info;
use sdl2::event::Event;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub struct Sdl2Renderer {
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
    sdl_context: sdl2::Sdl,   // Guardamos el contexto SDL para poder crear event_pump
    bar_count: usize,
    bar_gap: f32,
    colors: Vec<[f32; 4]>,
    audio_rx: Receiver<Vec<f32>>,
    running: Arc<AtomicBool>,
}

impl Sdl2Renderer {
    pub fn new(
        bar_count: usize,
        bar_gap: f32,
        colors: Vec<[f32; 4]>,
        audio_rx: Receiver<Vec<f32>>,
        running: Arc<AtomicBool>,
    ) -> Result<Self> {
        let sdl_context = sdl2::init().map_err(|e| anyhow::anyhow!("SDL2 init failed: {}", e))?;
        let video_subsystem = sdl_context.video().map_err(|e| anyhow::anyhow!("SDL2 video: {}", e))?;

        let display_mode = video_subsystem.current_display_mode(0)
            .map_err(|e| anyhow::anyhow!("Failed to get display mode: {}", e))?;
        let width = display_mode.w as u32;
        let height = display_mode.h as u32;

        let window = video_subsystem
            .window("cava-bg", width, height)
            .position_centered()
            .borderless()
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create window: {}", e))?;

        let canvas = window.into_canvas().build()
            .map_err(|e| anyhow::anyhow!("Failed to create canvas: {}", e))?;

        info!("SDL2 renderer initialized: {}x{}", width, height);
        Ok(Self {
            canvas,
            sdl_context,
            bar_count,
            bar_gap,
            colors,
            audio_rx,
            running,
        })
    }

    pub fn run(&mut self) -> Result<()> {
        let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width = bar_width * self.bar_gap;
        let window_height = self.canvas.window().size().1;
        let mut event_pump = self.sdl_context.event_pump()
            .map_err(|e| anyhow::anyhow!("Failed to get event pump: {}", e))?;

        self.canvas.set_draw_color(Color::RGBA(0, 0, 0, 0));
        self.canvas.clear();
        self.canvas.present();

        info!("SDL2 renderer entering main loop");

        while self.running.load(Ordering::SeqCst) {
            for event in event_pump.poll_iter() {
                if let Event::Quit { .. } = event {
                    return Ok(());
                }
            }

            if let Ok(audio_data) = self.audio_rx.try_recv() {
                self.canvas.set_draw_color(Color::RGBA(0, 0, 0, 0));
                self.canvas.clear();

                for (i, &height) in audio_data.iter().enumerate().take(self.bar_count) {
                    let x = (bar_gap_width * i as f32 + bar_width * i as f32) as i32;
                    let w = (bar_width * window_height as f32) as u32;
                    let h = (height * window_height as f32) as u32;
                    let color = self.colors[i % self.colors.len()];
                    self.canvas.set_draw_color(Color::RGBA(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                        (color[3] * 255.0) as u8,
                    ));
                    let _ = self.canvas.fill_rect(Rect::new(
                        x,
                        (window_height - h) as i32,
                        w,
                        h,
                    ));
                }
                self.canvas.present();
            }

            std::thread::sleep(Duration::from_millis(16));
        }

        info!("SDL2 renderer stopped");
        Ok(())
    }
}