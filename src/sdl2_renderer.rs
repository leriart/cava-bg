use anyhow::{Context, Result};
use log::{info, error};
use sdl2::event::Event;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc::Receiver;
use std::time::Duration;

pub struct Sdl2Renderer {
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
    bar_count: usize,
    bar_gap: f32,
    colors: Vec<[f32; 4]>,
    audio_rx: Receiver<Vec<f32>>,
    running: Arc<AtomicBool>,
}

impl Sdl2Renderer {
    pub fn new(
        width: u32,
        height: u32,
        bar_count: usize,
        bar_gap: f32,
        colors: Vec<[f32; 4]>,
        audio_rx: Receiver<Vec<f32>>,
        running: Arc<AtomicBool>,
    ) -> Result<Self> {
        let sdl_context = sdl2::init().context("Failed to initialize SDL2")?;
        let video_subsystem = sdl_context.video().context("Failed to get SDL2 video subsystem")?;
        let window = video_subsystem
            .window("cava-bg", width, height)
            .position_centered()
            .borderless()
            .build()
            .context("Failed to create SDL2 window")?;
        let canvas = window.into_canvas().build().context("Failed to create SDL2 canvas")?;

        info!("SDL2 renderer initialized: {}x{}", width, height);
        Ok(Self {
            canvas,
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

        // Configurar ventana como siempre encima (opcional) y transparente
        self.canvas.set_draw_color(Color::RGBA(0, 0, 0, 0));
        self.canvas.clear();
        self.canvas.present();

        info!("SDL2 renderer entering main loop");

        while self.running.load(Ordering::SeqCst) {
            // Procesar eventos (por si el usuario cierra la ventana)
            for event in self.canvas.window().subsystem().event_pump()?.poll_iter() {
                if let Event::Quit { .. } = event {
                    return Ok(());
                }
            }

            // Recibir nuevos datos de audio (no bloqueante)
            if let Ok(audio_data) = self.audio_rx.try_recv() {
                self.canvas.set_draw_color(Color::RGBA(0, 0, 0, 0));
                self.canvas.clear();

                for (i, &height) in audio_data.iter().enumerate().take(self.bar_count) {
                    let x = (bar_gap_width * i as f32 + bar_width * i as f32) as i32;
                    let w = (bar_width) as u32;
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

            // Control de framerate (~60 fps)
            std::thread::sleep(Duration::from_millis(16));
        }

        info!("SDL2 renderer stopped");
        Ok(())
    }
}