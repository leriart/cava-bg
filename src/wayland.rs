//! COMPLETE Wayland renderer for cava-bg
//! Final complete version with correct EGL API usage

use anyhow::{anyhow, Context, Result};
use log::{info, warn, debug, error};
use std::ffi::CString;
use std::io::Read;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;
use crate::wallpaper::WallpaperAnalyzer;

// Wayland and OpenGL imports
use gl::types::{GLsizei, GLsizeiptr, GLuint, GLint, GLenum};
use khronos_egl as egl;
use smithay_client_toolkit::reexports::calloop::EventLoop as CalloopEventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::{ProvidesRegistryState, RegistryState},
    delegate_compositor, delegate_layer, delegate_output, delegate_registry,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    globals::registry_queue_init,
    protocol::wl_output,
    Connection, QueueHandle, Proxy,
};
use wayland_egl::WlEglSurface;

// ============================================================================
// Shaders
// ============================================================================

const VERTEX_SHADER_SRC: &str = r#"#version 430 core
layout(location = 0) in vec2 position;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER_SRC: &str = r#"#version 430 core
layout(std430, binding = 0) buffer GradientColors {
    int gradient_colors_size;
    vec4 gradient_colors[];
};
uniform vec2 WindowSize;
out vec4 fragColor;
void main() {
    if (gradient_colors_size == 1) {
        fragColor = gradient_colors[0];
    } else {
        float findex = (gl_FragCoord.y * float(gradient_colors_size - 1)) / WindowSize.y;
        int index = int(findex);
        float step = findex - float(index);
        if (index == gradient_colors_size - 1) {
            index--;
        }
        fragColor = mix(gradient_colors[index], gradient_colors[index + 1], step);
    }
}
"#;

// ============================================================================
// Application State
// ============================================================================

struct AppState {
    registry_state: Option<RegistryState>,
    compositor_state: Option<CompositorState>,
    output_state: Option<OutputState>,
    layer_shell: Option<LayerShell>,
    layer_surface: Option<LayerSurface>,
    egl_display: Option<egl::Display>,
    egl_context: Option<egl::Context>,
    egl_surface: Option<egl::Surface>,
    egl_config: Option<egl::Config>,
    shader_program: GLuint,
    vao: GLuint,
    vbo: GLuint,
    gradient_colors_ssbo: GLuint,
    windows_size_location: GLint,
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    cava_reader: Box<dyn Read + Send>,
    width: u32,
    height: u32,
    running: Arc<AtomicBool>,
    configured: bool,
    colors: Vec<[f32; 4]>,
    wl_egl_surface: Option<WlEglSurface>,
    egl_instance: Option<egl::Instance<egl::Static>>,
}

impl AppState {
    fn new(
        cava_reader: Box<dyn Read + Send>,
        bar_count: u32,
        bar_gap: f32,
        background_color: [f32; 4],
        colors: Vec<[f32; 4]>,
        running: Arc<AtomicBool>,
    ) -> Result<Self> {
        Ok(Self {
            registry_state: None,
            compositor_state: None,
            output_state: None,
            layer_shell: None,
            layer_surface: None,
            egl_display: None,
            egl_context: None,
            egl_surface: None,
            egl_config: None,
            shader_program: 0,
            vao: 0,
            vbo: 0,
            gradient_colors_ssbo: 0,
            windows_size_location: 0,
            bar_count,
            bar_gap,
            background_color,
            cava_reader,
            width: 0,
            height: 0,
            running,
            configured: false,
            colors,
            wl_egl_surface: None,
            egl_instance: None,
        })
    }

    fn init_graphics(&mut self, surface: &WlSurface, conn: &Connection) -> Result<()> {
        info!("Initializing EGL/OpenGL at {}x{}", self.width, self.height);
        
        // Create EGL instance (static linking)
        let egl = egl::Instance::new(egl::Static);
        
        // Get EGL display using Wayland display pointer (like wallpaper-cava)
        let display = unsafe {
            egl.get_display(conn.display().id().as_ptr() as *mut std::ffi::c_void)
                .ok_or_else(|| anyhow!("Failed to get EGL display"))?
        };
        
        // Initialize EGL
        egl.initialize(display)
            .map_err(|e| anyhow!("Failed to initialize EGL: {:?}", e))?;
        
        // Bind OpenGL API
        egl.bind_api(egl::OPENGL_API)
            .map_err(|e| anyhow!("Failed to bind OpenGL API: {:?}", e))?;
        
        // Choose config
        let config_attribs = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::DEPTH_SIZE, 0,
            egl::STENCIL_SIZE, 0,
            egl::SAMPLE_BUFFERS, 0,
            egl::RENDERABLE_TYPE, egl::OPENGL_BIT,
            egl::SURFACE_TYPE, egl::WINDOW_BIT,
            egl::NONE,
        ];
        
        let config = egl.choose_first_config(display, &config_attribs)
            .map_err(|e| anyhow!("Failed to choose EGL config: {:?}", e))?
            .ok_or_else(|| anyhow!("No suitable EGL config found"))?;
        
        // Create context
        let context_attribs = [
            egl::CONTEXT_MAJOR_VERSION, 4,
            egl::CONTEXT_MINOR_VERSION, 3,
            egl::CONTEXT_OPENGL_PROFILE_MASK, egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        
        let context = egl.create_context(display, config, None, &context_attribs)
            .map_err(|e| anyhow!("Failed to create EGL context: {:?}", e))?;
        
        // Create EGL window
        let wl_egl_surface = WlEglSurface::new(surface.id(), self.width as i32, self.height as i32)
            .context("Failed to create EGL window")?;
        
        // Create surface
        let egl_surface = unsafe {
            egl.create_window_surface(
                display,
                config,
                wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            )
            .map_err(|e| anyhow!("Failed to create EGL surface: {:?}", e))?
        };
        
        // Make current
        egl.make_current(display, Some(egl_surface), Some(egl_surface), Some(context))
            .map_err(|e| anyhow!("Failed to make EGL context current: {:?}", e))?;
        
        // Load OpenGL functions
        gl::load_with(|s| {
            egl.get_proc_address(s)
                .map(|f| f as *const std::ffi::c_void)
                .unwrap_or(std::ptr::null())
        });
        
        // Initialize OpenGL
        self.init_gl()?;
        
        self.egl_display = Some(display);
        self.egl_context = Some(context);
        self.egl_surface = Some(egl_surface);
        self.egl_config = Some(config);
        self.wl_egl_surface = Some(wl_egl_surface);
        self.egl_instance = Some(egl);
        
        info!("Graphics initialized successfully");
        Ok(())
    }
    
    fn init_gl(&mut self) -> Result<()> {
        unsafe {
            // Compile shaders
            let vertex_shader = self.compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER_SRC)?;
            let fragment_shader = self.compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
            
            // Create program
            let shader_program = gl::CreateProgram();
            gl::AttachShader(shader_program, vertex_shader);
            gl::AttachShader(shader_program, fragment_shader);
            gl::LinkProgram(shader_program);
            
            // Check linking
            let mut success: GLint = 0;
            gl::GetProgramiv(shader_program, gl::LINK_STATUS, &mut success);
            if success == 0 {
                let mut info_log = vec![0u8; 1024];
                gl::GetProgramInfoLog(
                    shader_program,
                    1024,
                    std::ptr::null_mut(),
                    info_log.as_mut_ptr() as *mut _,
                );
                return Err(anyhow!("Shader linking failed: {}", 
                    String::from_utf8_lossy(&info_log)));
            }
            
            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);
            
            // Create VAO and VBO
            let mut vao: GLuint = 0;
            let mut vbo: GLuint = 0;
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            
            gl::BindVertexArray(vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            
            gl::VertexAttribPointer(0, 2, gl::FLOAT, gl::FALSE, 2 * std::mem::size_of::<f32>() as GLsizei, std::ptr::null());
            gl::EnableVertexAttribArray(0);
            
            // Create SSBO for gradient colors
            let mut gradient_colors_ssbo: GLuint = 0;
            gl::GenBuffers(1, &mut gradient_colors_ssbo);
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, gradient_colors_ssbo);
            
            let mut ssbo_data = vec![0.0f32; 1 + self.colors.len() * 4];
            ssbo_data[0] = self.colors.len() as f32;
            for (i, color) in self.colors.iter().enumerate() {
                let base = 1 + i * 4;
                ssbo_data[base] = color[0];
                ssbo_data[base + 1] = color[1];
                ssbo_data[base + 2] = color[2];
                ssbo_data[base + 3] = color[3];
            }
            
            gl::BufferData(
                gl::SHADER_STORAGE_BUFFER,
                (ssbo_data.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                ssbo_data.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::BindBufferBase(gl::SHADER_STORAGE_BUFFER, 0, gradient_colors_ssbo);
            
            // Get uniform location
            let windows_size_location = gl::GetUniformLocation(shader_program, CString::new("WindowSize").unwrap().as_ptr());
            
            self.shader_program = shader_program;
            self.vao = vao;
            self.vbo = vbo;
            self.gradient_colors_ssbo = gradient_colors_ssbo;
            self.windows_size_location = windows_size_location;
            
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
            
            info!("OpenGL initialized: shader={}, vao={}, vbo={}, ssbo={}",
                shader_program, vao, vbo, gradient_colors_ssbo);
        }
        
        Ok(())
    }
    
    fn compile_shader(&self, shader_type: GLenum, source: &str) -> Result<GLuint> {
        unsafe {
            let shader = gl::CreateShader(shader_type);
            let c_str = CString::new(source).unwrap();
            gl::ShaderSource(shader, 1, &c_str.as_ptr(), std::ptr::null());
            gl::CompileShader(shader);
            
            let mut success: GLint = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
            if success == 0 {
                let mut info_log = vec![0u8; 1024];
                gl::GetShaderInfoLog(
                    shader,
                    1024,
                    std::ptr::null_mut(),
                    info_log.as_mut_ptr() as *mut _,
                );
                return Err(anyhow!("Shader compilation failed: {}", 
                    String::from_utf8_lossy(&info_log)));
            }
            
            Ok(shader)
        }
    }
    
    fn draw(&mut self) -> Result<()> {
        // Read audio data
        let mut cava_buffer: Vec<u8> = vec![0; self.bar_count as usize * 2];
        match self.cava_reader.read_exact(&mut cava_buffer) {
            Ok(_) => {
                // Process audio data
                let mut audio_data: Vec<f32> = vec![0.0; self.bar_count as usize];
                let mut max_val = 0.0f32;
                for (i, chunk) in cava_buffer.chunks_exact(2).enumerate() {
                    let value = u16::from_le_bytes([chunk[0], chunk[1]]) as f32 / 65535.0;
                    audio_data[i] = value;
                    if value > max_val {
                        max_val = value;
                    }
                }
                
                // Log every 60 frames
                use std::sync::atomic::{AtomicU32, Ordering};
                static FRAME_COUNT: AtomicU32 = AtomicU32::new(0);
                let frame_num = FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
                if frame_num % 60 == 0 {
                    debug!("Draw frame {}: bars={}, max_audio={:.3}, size={}x{}", 
                        frame_num, self.bar_count, max_val, self.width, self.height);
                }
                
                // Calculate bar geometry
                let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
                let bar_gap_width = bar_width * self.bar_gap;
                
                // Generate vertices
                let mut vertices: Vec<f32> = Vec::with_capacity(self.bar_count as usize * 8);
                for i in 0..self.bar_count as usize {
                    let bar_height = 2.0 * audio_data[i] - 1.0;
                    let x1 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                    let x2 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                    
                    vertices.extend_from_slice(&[x1, bar_height]);
                    vertices.extend_from_slice(&[x2, bar_height]);
                    vertices.extend_from_slice(&[x1, -1.0]);
                    vertices.extend_from_slice(&[x2, -1.0]);
                }
                
                // Render
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
                    gl::Uniform2f(self.windows_size_location, self.width as f32, self.height as f32);
                    
                    gl::BindVertexArray(self.vao);
                    gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
                    gl::BufferData(
                        gl::ARRAY_BUFFER,
                        (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                        vertices.as_ptr() as *const _,
                        gl::DYNAMIC_DRAW,
                    );
                    
                    // Draw each bar
                    for i in 0..self.bar_count as usize {
                        let offset = (i * 6) as GLsizei;
                        gl::DrawArrays(gl::TRIANGLE_STRIP, offset, 4);
                    }
                    
                    gl::BindVertexArray(0);
                    gl::Disable(gl::BLEND);
                }
                
                // Swap buffers
                if let (Some(egl_instance), Some(display), Some(surface)) = (&self.egl_instance, self.egl_display, self.egl_surface) {
                    egl_instance.swap_buffers(display, surface)
                        .map_err(|e| anyhow!("Failed to swap buffers: {:?}", e))?;
                }
                
                Ok(())
            }
            Err(e) => {
                warn!("Failed to read audio data: {}", e);
                Err(anyhow!("Audio read error: {}", e))
            }
        }
    }
    
    fn cleanup(&mut self) {
        info!("Cleaning up graphics resources...");
        
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
            if self.gradient_colors_ssbo != 0 {
                gl::DeleteBuffers(1, &self.gradient_colors_ssbo);
            }
        }
        
        info!("Graphics cleanup complete");
    }
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
        info!("Scale factor changed");
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
        info!("Transform changed");
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        info!("Surface entered output");
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
        info!("Surface left output");
    }

    fn frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        self.output_state.as_mut().unwrap()
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        info!("New output detected");
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
        info!("Output destroyed");
    }
}

impl LayerShellHandler for AppState {
    fn closed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer_surface: &LayerSurface,
    ) {
        info!("Layer surface closed");
        self.running.store(false, Ordering::SeqCst);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer_surface: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        self.width = configure.new_size.0;
        self.height = configure.new_size.1;
        self.configured = true;
        info!("Layer surface configured: {}x{}", self.width, self.height);
    }
}

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        self.registry_state.as_mut().unwrap()
    }

    fn runtime_add_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _id: u32,
        _interface: &str,
        _version: u32,
    ) {
    }

    fn runtime_remove_global(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _id: u32,
        _interface: &str,
    ) {
    }
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_layer!(AppState);
delegate_registry!(AppState);

// ============================================================================
// WaylandRenderer - Public Interface
// ============================================================================

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
        // Check Wayland
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return Err(anyhow!("WAYLAND_DISPLAY not set"));
        }

        // Get colors
        let colors = if self.config.general.auto_colors {
            WallpaperAnalyzer::generate_gradient_colors(8)
                .unwrap_or_else(|_| WallpaperAnalyzer::default_colors(8))
        } else {
            self.config
                .colors
                .colors
                .iter()
                .filter(|(k, _)| k.starts_with("gradient_color_"))
                .map(|(_, c)| c.to_array())
                .collect()
        };
        let colors = if colors.is_empty() {
            WallpaperAnalyzer::default_colors(8)
        } else {
            colors
        };

        let bar_count = self.config.bars.amount as u32;
        let bar_gap = self.config.bars.gap as f32 / 100.0;
        let cava_reader = self.cava_manager.take_reader()?;
        let background_color = [0.0, 0.0, 0.0, 0.0];

        info!("========================================");
        info!("Wayland renderer starting");
        info!("Bars: {}, FPS: {}", bar_count, self.config.general.framerate);
        info!("Colors: {}", colors.len());
        info!("Layer: Background (like wallpaper)");
        info!("Size: 0,0 (auto-size to output)");
        info!("Anchors: ALL (full coverage)");
        info!("========================================");
        info!("Press Ctrl+C to exit");

        // Setup Ctrl+C handler
        let running_clone = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Ctrl+C received, shutting down...");
            running_clone.store(false, Ordering::SeqCst);
        })
        .context("Failed to set Ctrl+C handler")?;

        // Connect to Wayland
        let conn = Connection::connect_to_env()
            .context("Failed to connect to Wayland")?;
        
        let (globals, event_queue) = registry_queue_init(&conn)
            .context("Failed to initialize registry")?;
        let qh = event_queue.handle();

        // Create app state
        let colors_clone = colors.clone();
        let mut app_state = AppState::new(
            Box::new(cava_reader),
            bar_count,
            bar_gap,
            background_color,
            colors_clone,
            self.running.clone(),
        )?;

        // Initialize registry and output
        app_state.registry_state = Some(RegistryState::new(&globals));
        app_state.output_state = Some(OutputState::new(&globals, &qh));
        
        // Create compositor and surface
        let compositor_state = CompositorState::bind(&globals, &qh)
            .context("wl_compositor not available")?;
        let surface = compositor_state.create_surface(&qh);
        app_state.compositor_state = Some(compositor_state);
        
        // Create layer shell surface
        app_state.layer_shell = Some(LayerShell::bind(&globals, &qh)
            .context("layer shell not available")?);
        let layer_shell = app_state.layer_shell.as_ref().unwrap();
        
        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            surface.clone(),
            Layer::Background,
            Some("cava-bg"),
            None,
        );

        // Configure layer surface
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_size(0, 0);
        surface.commit();

        app_state.layer_surface = Some(layer_surface);

        // Create event loop
        let mut event_loop: CalloopEventLoop<AppState> = CalloopEventLoop::try_new()
            .context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();

        // Add Wayland source
        match WaylandSource::new(conn.clone(), event_queue).insert(loop_handle) {
            Ok(_) => {},
            Err(e) => return Err(anyhow!("Failed to insert Wayland source: {:?}", e)),
        }

        // Main loop
        let frame_duration = Duration::from_secs_f64(1.0 / self.config.general.framerate as f64);
        let mut last_frame = std::time::Instant::now();
        let mut graphics_initialized = false;

        while app_state.running.load(Ordering::SeqCst) {
            let timeout = frame_duration
                .checked_sub(last_frame.elapsed())
                .unwrap_or(Duration::ZERO);
            
            event_loop.dispatch(Some(timeout), &mut app_state)
                .context("Event loop dispatch failed")?;

            if !app_state.configured {
                continue;
            }

            // Initialize graphics once configured
            if !graphics_initialized && app_state.width > 0 && app_state.height > 0 {
                match app_state.init_graphics(&surface, &conn) {
                    Ok(_) => {
                        graphics_initialized = true;
                        info!("Graphics initialized successfully at {}x{}", 
                            app_state.width, app_state.height);
                    }
                    Err(e) => {
                        warn!("Failed to initialize graphics: {}", e);
                        app_state.running.store(false, Ordering::SeqCst);
                        break;
                    }
                }
            }

            // Draw frames
            if graphics_initialized && last_frame.elapsed() >= frame_duration {
                match app_state.draw() {
                    Ok(_) => {},
                    Err(e) => {
                        warn!("Draw error: {}", e);
                    }
                }
                last_frame = std::time::Instant::now();
            }
        }

        // Cleanup
        app_state.cleanup();
        info!("Wayland renderer stopped");
        Ok(())
    }
}