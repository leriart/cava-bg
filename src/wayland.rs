//! Renderer usando winit + glutin (pantalla completa transparente)

use anyhow::{anyhow, Context, Result};
use crossbeam_channel::{bounded, Receiver, TryRecvError};
use gl::types::*;
use log::{error, info};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, Version};
use glutin::display::Display;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, WindowSurface};
use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::event::{Event, WindowEvent};
use winit::event_loop::{ControlFlow, EventLoop};
use winit::window::{Window, WindowBuilder};

use crate::config::Config;
use crate::cava_manager::CavaManager;
use crate::wallpaper::WallpaperAnalyzer;

// -----------------------------------------------------------------------------
// Shaders
// -----------------------------------------------------------------------------

const VERTEX_SHADER_SRC: &str = r#"#version 430 core
layout(location = 0) in vec2 position;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER_SRC: &str = r#"#version 430 core
uniform vec4 BarColor;
out vec4 fragColor;
void main() {
    fragColor = BarColor;
}
"#;

// -----------------------------------------------------------------------------
// Audio reader thread
// -----------------------------------------------------------------------------

fn start_audio_reader(
    mut reader: Box<dyn std::io::Read + Send>,
    bar_count: u32,
) -> Receiver<Vec<f32>> {
    let (sender, receiver) = bounded(2);
    std::thread::spawn(move || {
        let mut buffer = vec![0u8; (bar_count as usize) * 2];
        loop {
            match reader.read_exact(&mut buffer) {
                Ok(_) => {
                    let mut audio = vec![0.0f32; bar_count as usize];
                    for (i, chunk) in buffer.chunks_exact(2).enumerate() {
                        let val = u16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 65535.0;
                        audio[i] = val;
                    }
                    if sender.send(audio).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    error!("Audio read error: {}", e);
                    break;
                }
            }
        }
        info!("Audio reader thread stopped");
    });
    receiver
}

// -----------------------------------------------------------------------------
// Funciones auxiliares de shader
// -----------------------------------------------------------------------------

fn compile_shader(shader_type: GLenum, src: &str) -> Result<GLuint> {
    unsafe {
        let shader = gl::CreateShader(shader_type);
        let c_str = CString::new(src).unwrap();
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 512];
            gl::GetShaderInfoLog(shader, 512, std::ptr::null_mut(), log.as_mut_ptr() as *mut _);
            let msg = String::from_utf8_lossy(&log);
            return Err(anyhow!("Shader compilation failed: {}", msg));
        }
        Ok(shader)
    }
}

fn link_program(vs: GLuint, fs: GLuint) -> Result<GLuint> {
    unsafe {
        let prog = gl::CreateProgram();
        gl::AttachShader(prog, vs);
        gl::AttachShader(prog, fs);
        gl::LinkProgram(prog);
        let mut success = 0;
        gl::GetProgramiv(prog, gl::LINK_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 512];
            gl::GetProgramInfoLog(prog, 512, std::ptr::null_mut(), log.as_mut_ptr() as *mut _);
            let msg = String::from_utf8_lossy(&log);
            return Err(anyhow!("Program linking failed: {}", msg));
        }
        Ok(prog)
    }
}

// -----------------------------------------------------------------------------
// Renderer con winit/glutin (pantalla completa transparente)
// -----------------------------------------------------------------------------

pub struct WaylandRenderer {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        Ok(Self {
            config,
            cava_manager,
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn run(mut self) -> Result<()> {
        // Colores del gradiente
        let colors = if self.config.general.auto_colors {
            WallpaperAnalyzer::generate_gradient_colors(8)
                .unwrap_or_else(|_| WallpaperAnalyzer::default_colors(8))
        } else {
            let mut cols = Vec::new();
            for (k, v) in &self.config.colors.colors {
                if k.starts_with("gradient_color_") {
                    cols.push(v.to_array());
                }
            }
            if cols.is_empty() {
                WallpaperAnalyzer::default_colors(8)
            } else {
                cols
            }
        };

        let bar_count = self.config.bars.amount;
        let bar_gap = self.config.bars.gap.clamp(0.0, 1.0);
        let background = [0.0, 0.0, 0.0, 0.0];
        let framerate = self.config.general.framerate;

        // Iniciar lector de audio en segundo plano
        let reader = self.cava_manager.take_reader()?;
        let audio_rx = start_audio_reader(Box::new(reader), bar_count);

        // Crear event loop y ventana
        let event_loop = EventLoop::new()?;
        let window = WindowBuilder::new()
            .with_title("cava-bg")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top(true)
            .with_fullscreen(Some(winit::window::Fullscreen::Borderless(None)))
            .build(&event_loop)?;

        let size = window.inner_size();
        info!("Window size: {}x{}", size.width, size.height);

        // Configurar OpenGL con glutin
        let display_handle = window.display_handle()?.as_raw();
        let window_handle = window.window_handle()?.as_raw();
        let template = ConfigTemplateBuilder::new()
            .with_alpha_size(8)
            .with_transparency(true)
            .build();
        let gl_display = unsafe { Display::new(display_handle, template).context("Failed to create display")? };
        let config = gl_display
            .find_configs(template)
            .map_err(|e| anyhow!("No configs: {}", e))?
            .next()
            .ok_or_else(|| anyhow!("No suitable config"))?;
        let context_attributes = ContextAttributesBuilder::new()
            .with_context_api(glutin::context::ContextApi::OpenGl(Some(Version::new(4, 3))))
            .build(Some(window_handle));
        let not_current_context = unsafe {
            gl_display
                .create_context(&config, &context_attributes)
                .context("Failed to create context")?
        };
        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(window_handle, size.width, size.height);
        let surface = unsafe {
            not_current_context
                .create_surface(&gl_display, attrs)
                .context("Failed to create surface")?
        };
        let context = not_current_context.make_current(&surface).context("Failed to make current")?;

        // Cargar funciones OpenGL
        gl::load_with(|s| {
            let cstr = CString::new(s).unwrap();
            gl_display.get_proc_address(&cstr).cast()
        });

        // Inicializar OpenGL
        let vs = compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER_SRC)?;
        let fs = compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
        let program = link_program(vs, fs)?;
        unsafe {
            gl::DeleteShader(vs);
            gl::DeleteShader(fs);
        }
        let bar_color_loc = unsafe { gl::GetUniformLocation(program, CString::new("BarColor").unwrap().as_ptr()) };

        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, 2 * std::mem::size_of::<f32>() as GLsizei, std::ptr::null());
            gl::EnableVertexAttribArray(0);
            gl::BindVertexArray(0);
        }

        let mut last_audio = vec![0.0; bar_count as usize];
        let frame_duration = Duration::from_secs_f64(1.0 / framerate as f64);
        let mut last_frame = Instant::now();
        let running = self.running.clone();

        // Ctrl+C handler
        let running_clone = running.clone();
        ctrlc::set_handler(move || {
            info!("Ctrl+C received, shutting down...");
            running_clone.store(false, Ordering::SeqCst);
        })?;

        info!("Winit/Glutin renderer running. Press Ctrl+C to exit.");

        // Bucle de eventos (usando run_app para evitar deprecación)
        event_loop.run_app(&mut App {
            running,
            audio_rx,
            last_audio,
            frame_duration,
            last_frame,
            program,
            vao,
            vbo,
            bar_color_loc,
            colors,
            bar_count,
            bar_gap,
            background,
            size,
            surface,
            context,
            gl_display,
            window,
        })?;

        Ok(())
    }
}

struct App {
    running: Arc<AtomicBool>,
    audio_rx: Receiver<Vec<f32>>,
    last_audio: Vec<f32>,
    frame_duration: Duration,
    last_frame: Instant,
    program: GLuint,
    vao: GLuint,
    vbo: GLuint,
    bar_color_loc: GLint,
    colors: Vec<[f32; 4]>,
    bar_count: u32,
    bar_gap: f32,
    background: [f32; 4],
    size: winit::dpi::PhysicalSize<u32>,
    surface: Surface<WindowSurface>,
    context: glutin::context::PossiblyCurrentContext,
    gl_display: Display,
    window: Window,
}

impl winit::application::ApplicationHandler for App {
    fn resumed(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {}
    fn window_event(
        &mut self,
        _event_loop: &winit::event_loop::ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.running.store(false, Ordering::SeqCst);
            }
            WindowEvent::RedrawRequested => {
                // Leer audio
                while let Ok(audio) = self.audio_rx.try_recv() {
                    self.last_audio = audio;
                }

                // Calcular geometría de barras
                let total_gap = (self.bar_count as f32 - 1.0) * self.bar_gap;
                let bar_width = (2.0 - total_gap) / self.bar_count as f32;
                let gap_width = self.bar_gap;

                let mut vertices = Vec::with_capacity(self.bar_count as usize * 4 * 2);
                for i in 0..self.bar_count as usize {
                    let x1 = -1.0 + i as f32 * (bar_width + gap_width);
                    let x2 = x1 + bar_width;
                    let height = self.last_audio[i];
                    let y_top = -1.0 + 2.0 * height;
                    let y_bottom = -1.0;
                    vertices.extend_from_slice(&[x1, y_bottom, x1, y_top, x2, y_bottom, x2, y_top]);
                }

                unsafe {
                    gl::Enable(gl::BLEND);
                    gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
                    gl::ClearColor(self.background[0], self.background[1], self.background[2], self.background[3]);
                    gl::Clear(gl::COLOR_BUFFER_BIT);

                    gl::UseProgram(self.program);
                    gl::BindVertexArray(self.vao);
                    gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
                    gl::BufferData(
                        gl::ARRAY_BUFFER,
                        (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                        vertices.as_ptr() as *const _,
                        gl::DYNAMIC_DRAW,
                    );

                    for i in 0..self.bar_count as usize {
                        let color = self.colors[i % self.colors.len()];
                        gl::Uniform4f(self.bar_color_loc, color[0], color[1], color[2], color[3]);
                        gl::DrawArrays(gl::TRIANGLE_STRIP, (i * 4) as GLint, 4);
                    }

                    gl::BindVertexArray(0);
                    gl::Disable(gl::BLEND);
                }

                self.surface.swap_buffers(&self.gl_display).unwrap_or_else(|e| error!("Swap buffers: {}", e));
                self.last_frame = Instant::now();
                self.window.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        if !self.running.load(Ordering::SeqCst) {
            _event_loop.exit();
            return;
        }
        let now = Instant::now();
        let elapsed = now - self.last_frame;
        if elapsed < self.frame_duration {
            let sleep = self.frame_duration - elapsed;
            std::thread::sleep(sleep);
        }
        self.window.request_redraw();
    }

    fn exiting(&mut self, _event_loop: &winit::event_loop::ActiveEventLoop) {
        info!("Exiting...");
    }
}