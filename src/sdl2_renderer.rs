use anyhow::Result;
use sdl2::event::Event;
use sdl2::pixels::Color;
use sdl2::rect::Rect;
use std::sync::mpsc::Receiver;

pub struct Sdl2Renderer {
    canvas: sdl2::render::Canvas<sdl2::video::Window>,
    bar_count: usize,
    bar_gap: f32,
    colors: Vec<[f32; 4]>,
    audio_rx: Receiver<Vec<f32>>,
}

impl Sdl2Renderer {
    pub fn new(width: u32, height: u32, bar_count: usize, bar_gap: f32, 
               colors: Vec<[f32; 4]>, audio_rx: Receiver<Vec<f32>>) -> Result<Self> {
        let sdl_context = sdl2::init()?;
        let video_subsystem = sdl_context.video()?;
        let window = video_subsystem.window("cava-bg", width, height)
            .position_centered()
            .opengl() // Intentar con OpenGL, pero SDL maneja los fallos
            .build()?;
        let canvas = window.into_canvas().build()?;
        
        Ok(Self { canvas, bar_count, bar_gap, colors, audio_rx })
    }

    pub fn run(&mut self) -> Result<()> {
        let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width = bar_width * self.bar_gap;

        loop {
            // Recibir nuevos datos de audio
            if let Ok(audio_data) = self.audio_rx.try_recv() {
                self.canvas.set_draw_color(Color::RGBA(0, 0, 0, 0));
                self.canvas.clear();
                
                for (i, &height) in audio_data.iter().enumerate() {
                    let x = (bar_gap_width * i as f32 + bar_width * i as f32) as i32;
                    let w = (bar_width) as u32;
                    let h = (height * self.canvas.window().size().1 as f32) as u32;
                    let color = self.colors[i % self.colors.len()];
                    self.canvas.set_draw_color(Color::RGBA(
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8,
                        (color[3] * 255.0) as u8,
                    ));
                    self.canvas.fill_rect(Rect::new(x, self.canvas.window().size().1 as i32 - h as i32, w, h))?;
                }
                self.canvas.present();
            }
            
            for event in self.sdl_context.event_pump()?.poll_iter() {
                if let Event::Quit { .. } = event {
                    return Ok(());
                }
            }
            ::std::thread::sleep(::std::time::Duration::from_millis(16));
        }
    }
}