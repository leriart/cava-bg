//! Wayland renderer using wayland-client + wayland-protocols (wlr-layer-shell)
//! Basado en el ejemplo de swaybg.

use anyhow::{anyhow, Context, Result};
use log::{error, info, warn};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, TryRecvError};
use gl::types::*;
use khronos_egl as egl;
use wayland_client::protocol::{wl_compositor, wl_output, wl_registry, wl_surface};
use wayland_client::{
    globals::registry_queue_init,
    Connection, Dispatch, EventQueue, Proxy, QueueHandle,
};
use wayland_egl::WlEglSurface;
use wayland_protocols::wlr::layer_shell::v1::client::{
    zwlr_layer_shell_v1, zwlr_layer_surface_v1,
};

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
// AppState (implementa Dispatch para los objetos Wayland)
// -----------------------------------------------------------------------------

struct AppState {
    // Objetos Wayland
    compositor: wl_compositor::WlCompositor,
    layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    surface: wl_surface::WlSurface,
    layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    output: Option<wl_output::WlOutput>,
    output_width: i32,
    output_height: i32,
    configured: bool,

    // EGL / OpenGL
    egl_instance: Option<egl::Instance<egl::Static>>,
    egl_display: Option<egl::Display>,
    egl_context: Option<egl::Context>,
    egl_surface: Option<egl::Surface>,
    wl_egl_surface: Option<WlEglSurface>,
    shader_program: GLuint,
    vao: GLuint,
    vbo: GLuint,
    bar_color_loc: GLint,
    graphics_initialized: bool,

    // Parámetros visuales
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    gradient_colors: Vec<[f32; 4]>,

    // Audio
    audio_receiver: Receiver<Vec<f32>>,
    last_audio: Vec<f32>,

    // Control
    running: Arc<AtomicBool>,
    frame_duration: Duration,
    last_frame: Instant,
}

impl AppState {
    fn new(
        bar_count: u32,
        bar_gap: f32,
        background_color: [f32; 4],
        gradient_colors: Vec<[f32; 4]>,
        audio_receiver: Receiver<Vec<f32>>,
        running: Arc<AtomicBool>,
        framerate: u32,
    ) -> Self {
        Self {
            compositor: wl_compositor::WlCompositor::dummy(),
            layer_shell: zwlr_layer_shell_v1::ZwlrLayerShellV1::dummy(),
            surface: wl_surface::WlSurface::dummy(),
            layer_surface: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1::dummy(),
            output: None,
            output_width: 0,
            output_height: 0,
            configured: false,
            egl_instance: None,
            egl_display: None,
            egl_context: None,
            egl_surface: None,
            wl_egl_surface: None,
            shader_program: 0,
            vao: 0,
            vbo: 0,
            bar_color_loc: 0,
            graphics_initialized: false,
            bar_count,
            bar_gap,
            background_color,
            gradient_colors,
            audio_receiver,
            last_audio: vec![0.0; bar_count as usize],
            running,
            frame_duration: Duration::from_secs_f64(1.0 / framerate as f64),
            last_frame: Instant::now(),
        }
    }

    fn init_graphics(&mut self, conn: &Connection) -> Result<()> {
        let width = self.output_width;
        let height = self.output_height;
        info!("Initializing EGL/OpenGL at {}x{}", width, height);

        let egl = egl::Instance::new(egl::Static);
        let display = unsafe {
            egl.get_display(conn.display().id().as_ptr() as *mut std::ffi::c_void)
                .ok_or_else(|| anyhow!("Failed to get EGL display"))?
        };
        egl.initialize(display)
            .map_err(|e| anyhow!("EGL init: {:?}", e))?;
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| anyhow!("EGL bind API: {:?}", e))?;

        let config_attribs = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::DEPTH_SIZE, 0,
            egl::STENCIL_SIZE, 0,
            egl::RENDERABLE_TYPE, egl::OPENGL_BIT,
            egl::SURFACE_TYPE, egl::WINDOW_BIT,
            egl::NONE,
        ];
        let config = egl.choose_first_config(display, &config_attribs)
            .map_err(|e| anyhow!("EGL config: {:?}", e))?
            .ok_or_else(|| anyhow!("No EGL config"))?;

        let ctx_attribs = [
            egl::CONTEXT_MAJOR_VERSION, 4,
            egl::CONTEXT_MINOR_VERSION, 3,
            egl::CONTEXT_OPENGL_PROFILE_MASK, egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let context = egl.create_context(display, config, None, &ctx_attribs)
            .map_err(|e| anyhow!("EGL context: {:?}", e))?;

        let wl_egl_surface = WlEglSurface::new(self.surface.id(), width, height)
            .context("WlEglSurface creation")?;
        let egl_surface = unsafe {
            egl.create_window_surface(
                display,
                config,
                wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            )
            .map_err(|e| anyhow!("EGL surface: {:?}", e))?
        };

        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .map_err(|e| anyhow!("EGL make current: {:?}", e))?;

        gl::load_with(|s| {
            egl.get_proc_address(s)
                .map(|f| f as *const std::ffi::c_void)
                .unwrap_or(std::ptr::null())
        });

        unsafe {
            gl::Viewport(0, 0, width, height);
        }

        // Compilar shaders
        let vs = compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER_SRC)?;
        let fs = compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
        let program = link_program(vs, fs)?;
        unsafe {
            gl::DeleteShader(vs);
            gl::DeleteShader(fs);
        }

        let bar_color_loc = unsafe {
            gl::GetUniformLocation(program, CString::new("BarColor").unwrap().as_ptr())
        };

        let mut vao = 0;
        let mut vbo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                2 * std::mem::size_of::<f32>() as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::BindVertexArray(0);
        }

        self.egl_instance = Some(egl);
        self.egl_display = Some(display);
        self.egl_context = Some(context);
        self.egl_surface = Some(egl_surface);
        self.wl_egl_surface = Some(wl_egl_surface);
        self.shader_program = program;
        self.vao = vao;
        self.vbo = vbo;
        self.bar_color_loc = bar_color_loc;

        self.graphics_initialized = true;
        info!("Graphics initialized successfully");
        Ok(())
    }

    fn update_audio(&mut self) {
        loop {
            match self.audio_receiver.try_recv() {
                Ok(audio) => self.last_audio = audio,
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    warn!("Audio reader disconnected");
                    self.running.store(false, Ordering::SeqCst);
                    break;
                }
            }
        }
    }

    fn draw(&mut self) -> Result<()> {
        if !self.graphics_initialized || self.output_width == 0 || self.output_height == 0 {
            return Ok(());
        }

        self.update_audio();

        let total_gap = (self.bar_count as f32 - 1.0) * self.bar_gap;
        let bar_width = (2.0 - total_gap) / self.bar_count as f32;
        let gap_width = self.bar_gap;

        let mut vertices = Vec::with_capacity((self.bar_count as usize) * 4 * 2);

        for i in 0..self.bar_count as usize {
            let x1 = -1.0 + i as f32 * (bar_width + gap_width);
            let x2 = x1 + bar_width;
            let height = self.last_audio[i];
            let y_top = -1.0 + 2.0 * height;
            let y_bottom = -1.0;

            vertices.extend_from_slice(&[
                x1, y_bottom,
                x1, y_top,
                x2, y_bottom,
                x2, y_top,
            ]);
        }

        unsafe {
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
            gl::ClearColor(
                self.background_color[0],
                self.background_color[1],
                self.background_color[2],
                self.background_color[3],
            );
            gl::Clear(gl::COLOR_BUFFER_BIT);

            gl::UseProgram(self.shader_program);
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                vertices.as_ptr() as *const _,
                gl::DYNAMIC_DRAW,
            );

            for i in 0..self.bar_count as usize {
                let color = self.gradient_colors[i % self.gradient_colors.len()];
                gl::Uniform4f(self.bar_color_loc, color[0], color[1], color[2], color[3]);
                let first_vertex = (i * 4) as GLint;
                gl::DrawArrays(gl::TRIANGLE_STRIP, first_vertex, 4);
            }

            gl::BindVertexArray(0);
            gl::Disable(gl::BLEND);
        }

        if let (Some(egl), Some(display), Some(surface)) =
            (&self.egl_instance, self.egl_display, self.egl_surface)
        {
            egl.swap_buffers(display, surface)
                .map_err(|e| anyhow!("Swap buffers: {:?}", e))?;
        }

        Ok(())
    }

    fn cleanup(&mut self) {
        info!("Cleaning up graphics");
        unsafe {
            if self.shader_program != 0 {
                gl::DeleteProgram(self.shader_program);
            }
            if self.vao != 0 {
                gl::DeleteVertexArrays(1, &self.vao);
            }
            if self.vbo != 0 {
                gl::DeleteBuffers(1, &self.vbo);
            }
        }
        if let (Some(egl), Some(display), Some(context), Some(surface)) = (
            &self.egl_instance,
            self.egl_display,
            self.egl_context,
            self.egl_surface,
        ) {
            let _ = egl.make_current(display, None, None, None);
            let _ = egl.destroy_surface(display, surface);
            let _ = egl.destroy_context(display, context);
            let _ = egl.terminate(display);
        }
    }
}

// -----------------------------------------------------------------------------
// Implementación de Dispatch para los objetos Wayland
// -----------------------------------------------------------------------------

impl Dispatch<wl_registry::WlRegistry, ()> for AppState {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _data: &(),
        _conn: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global { name, interface, version } => {
                info!("Global: {} v{}", interface, version);
                if interface == "wl_compositor" {
                    let compositor = registry.bind::<wl_compositor::WlCompositor, _, _>(name, version, qh, ());
                    state.compositor = compositor;
                } else if interface == "zwlr_layer_shell_v1" {
                    let layer_shell = registry.bind::<zwlr_layer_shell_v1::ZwlrLayerShellV1, _, _>(name, version, qh, ());
                    state.layer_shell = layer_shell;
                } else if interface == "wl_output" {
                    let output = registry.bind::<wl_output::WlOutput, _, _>(name, version, qh, ());
                    state.output = Some(output);
                }
            }
            wl_registry::Event::GlobalRemove { .. } => {}
        }
    }
}

impl Dispatch<wl_compositor::WlCompositor, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_compositor::WlCompositor,
        _event: wl_compositor::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, ()> for AppState {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_output::Event::Mode { width, height, .. } => {
                if state.output.as_ref() == Some(output) {
                    state.output_width = width as i32;
                    state.output_height = height as i32;
                    info!("Output size: {}x{}", width, height);
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<wl_surface::WlSurface, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &wl_surface::WlSurface,
        _event: wl_surface::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_shell_v1::ZwlrLayerShellV1, ()> for AppState {
    fn event(
        _state: &mut Self,
        _proxy: &zwlr_layer_shell_v1::ZwlrLayerShellV1,
        _event: <zwlr_layer_shell_v1::ZwlrLayerShellV1 as Proxy>::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, ()> for AppState {
    fn event(
        state: &mut Self,
        _proxy: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        _data: &(),
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure { serial, width, height } => {
                info!("Layer configure: {}x{} (serial {})", width, height, serial);
                state.output_width = width as i32;
                state.output_height = height as i32;
                state.configured = true;
                // Acusar recibo
                state.layer_surface.ack_configure(serial);
                state.surface.commit();
            }
            zwlr_layer_surface_v1::Event::Closed => {
                info!("Layer surface closed");
                state.running.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
    }
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
// Renderer público
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
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return Err(anyhow!("WAYLAND_DISPLAY not set"));
        }

        // Colores
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

        // Lector de audio
        let reader = self.cava_manager.take_reader()?;
        let audio_rx = start_audio_reader(Box::new(reader), bar_count);

        // Conectar a Wayland
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, mut event_queue) = registry_queue_init::<AppState>(&conn)
            .context("Failed to init registry")?;
        let qh = event_queue.handle();

        // Estado inicial (con objetos dummy)
        let mut app_state = AppState::new(
            bar_count,
            bar_gap,
            background,
            colors,
            audio_rx,
            self.running.clone(),
            framerate,
        );

        // Registrar los objetos globales (el dispatch los llenará)
        // Primero, registrar el registry en el event_queue para que los eventos lleguen
        // Ya lo hicimos con registry_queue_init, que devuelve un event_queue asociado a AppState

        // Ahora, después de algunos dispatch, los objetos estarán bindeados
        // Hacemos un roundtrip para recibir los globales
        event_queue.roundtrip(&mut app_state)?;

        // Verificar que tenemos compositor y layer_shell
        if app_state.compositor.is_dummy() {
            return Err(anyhow!("wl_compositor not available"));
        }
        if app_state.layer_shell.is_dummy() {
            return Err(anyhow!("zwlr_layer_shell_v1 not available"));
        }

        // Crear superficie
        let surface = app_state.compositor.create_surface(&qh, ());
        app_state.surface = surface;

        // Crear layer surface
        let layer_surface = app_state.layer_shell.get_layer_surface(
            &qh,
            &app_state.surface,
            None, // output = None (todos)
            zwlr_layer_shell_v1::Layer::Overlay,
            Some("cava-bg".to_string()),
            (),
        );
        // Anclar a todos los bordes
        layer_surface.set_anchor(
            zwlr_layer_surface_v1::Anchor::Top
                | zwlr_layer_surface_v1::Anchor::Bottom
                | zwlr_layer_surface_v1::Anchor::Left
                | zwlr_layer_surface_v1::Anchor::Right,
        );
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_size(0, 0);
        app_state.layer_surface = layer_surface;
        app_state.surface.commit();

        // Esperar a que configure nos dé el tamaño real
        let start = Instant::now();
        while !app_state.configured && start.elapsed() < Duration::from_secs(2) {
            event_queue.dispatch_pending(&mut app_state)?;
            event_queue.flush()?;
            thread::sleep(Duration::from_millis(10));
        }
        if !app_state.configured {
            error!("Timeout waiting for layer configure");
            return Err(anyhow!("Layer not configured"));
        }

        // Inicializar gráficos
        app_state.init_graphics(&conn)?;

        // Ctrl+C handler
        let running_clone = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Ctrl+C received, shutting down...");
            running_clone.store(false, Ordering::SeqCst);
        })?;

        info!("Wayland renderer running. Press Ctrl+C to exit.");

        // Bucle principal
        while app_state.running.load(Ordering::SeqCst) {
            let _ = event_queue.dispatch_pending(&mut app_state);
            let _ = event_queue.flush();

            if app_state.last_frame.elapsed() >= app_state.frame_duration {
                if let Err(e) = app_state.draw() {
                    error!("Draw error: {}", e);
                }
                app_state.last_frame = Instant::now();
            } else {
                thread::sleep(Duration::from_millis(1));
            }
        }

        app_state.cleanup();
        info!("Wayland renderer stopped");
        Ok(())
    }
}