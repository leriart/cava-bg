//! Complete Wayland renderer for cava-bg
//! Based on wallpaper-cava implementation

use anyhow::{anyhow, Context, Result};
use log::{info, warn};
use std::ffi::CString;
use std::io::Read;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;
use crate::wallpaper::WallpaperAnalyzer;

// Wayland and EGL
use khronos_egl as egl;
use gl::types::{GLsizei, GLsizeiptr};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::ProvidesRegistryState;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::RegistryState,
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_registry, registry_handlers,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;
use wayland_client::{
    globals::registry_queue_init,
    protocol::wl_output,
    Connection, QueueHandle,
};
use wayland_egl::WlEglSurface;

// ============================================================================
// Shaders (simplified version)
// ============================================================================

const VERTEX_SHADER_SRC: &str = r#"
#version 330 core
layout (location = 0) in vec2 position;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER_SRC: &str = r#"
#version 330 core
uniform vec4 uColor;
out vec4 fragColor;
void main() {
    fragColor = uColor;
}
"#;

// ============================================================================
// App State
// ============================================================================

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    width: u32,
    height: u32,
    layer_shell: LayerShell,
    layer_surface: LayerSurface,
    surface: WlSurface,
    cava_reader: Box<dyn Read + Send>,
    wl_egl_surface: WlEglSurface,
    egl_surface: egl::Surface,
    egl_config: egl::Config,
    egl_context: egl::Context,
    egl_display: egl::Display,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    bar_count: u32,
    bar_gap: f32,
    colors: Vec<[f32; 4]>,
    running: Arc<AtomicBool>,
    compositor: CompositorState,
}

impl AppState {
    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>) -> Result<()> {
        // Read audio data
        let mut cava_buffer: Vec<u8> = vec![0; self.bar_count as usize * 2];
        match self.cava_reader.read_exact(&mut cava_buffer) {
            Ok(_) => {
                let mut unpacked_data: Vec<f32> = vec![0.0; self.bar_count as usize];
                for (unpacked_data_index, i) in (0..cava_buffer.len()).step_by(2).enumerate() {
                    let num = u16::from_le_bytes([cava_buffer[i], cava_buffer[i + 1]]);
                    unpacked_data[unpacked_data_index] = (num as f32) / 65530.0;
                }

                // Calculate bar dimensions
                let bar_width: f32 =
                    2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
                let bar_gap_width: f32 = bar_width * self.bar_gap;

                // Generate vertices
                let mut vertices: Vec<f32> = vec![0.0; self.bar_count as usize * 8];
                let _fwidth: f32 = self.width as f32;
                let _fheight: f32 = self.height as f32;

                for i in 0..self.bar_count as usize {
                    let bar_height: f32 = 2.0 * unpacked_data[i] - 1.0;
                    let color_idx = i % self.colors.len();
                    let _color = self.colors[color_idx];
                    
                    vertices[i * 8] = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                    vertices[i * 8 + 1] = bar_height;
                    vertices[i * 8 + 2] = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                    vertices[i * 8 + 3] = bar_height;
                    vertices[i * 8 + 4] = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                    vertices[i * 8 + 5] = -1.0;
                    vertices[i * 8 + 6] = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                    vertices[i * 8 + 7] = -1.0;
                }

                unsafe {
                    gl::BindVertexArray(self.vao);
                    gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
                    gl::BufferData(
                        gl::ARRAY_BUFFER,
                        (vertices.len() * std::mem::size_of::<f32>()) as GLsizeiptr,
                        vertices.as_ptr() as *const _,
                        gl::DYNAMIC_DRAW,
                    );
                    gl::Enable(gl::BLEND);
                    gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);
                    gl::ClearColor(0.0, 0.0, 0.0, 0.0);
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                    gl::UseProgram(self.shader_program);
                    
                    // Draw each bar with its color
                    for i in 0..self.bar_count as usize {
                        let color_idx = i % self.colors.len();
                        let color = self.colors[color_idx];
                        let color_loc = unsafe { gl::GetUniformLocation(self.shader_program, CString::new("uColor").unwrap().as_ptr()) };
                        unsafe {
                            gl::Uniform4f(color_loc, color[0], color[1], color[2], color[3]);
                        }
                        gl::DrawArrays(gl::TRIANGLE_STRIP, (i * 4) as i32, 4);
                    }
                    
                    gl::BindVertexArray(0);
                }

                // Swap buffers
                let egl_api = &egl::API;
                egl_api.swap_buffers(self.egl_display, self.egl_surface)
                    .context("Failed to swap buffers")?;
                self.surface.frame(qh, self.surface.clone());
                Ok(())
            }
            Err(e) => {
                warn!("Failed to read audio data: {}", e);
                Err(anyhow!("Audio read error: {}", e))
            }
        }
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            let old_surface = self.surface.clone();
            self.surface = self.compositor.create_surface(qh);
            self.layer_surface = self.layer_shell.create_layer_surface(
                qh,
                self.surface.clone(),
                Layer::Bottom,
                Some("cava-bg"),
                Some(&output),
            );
            if let Some(logical_size) = info.logical_size {
                self.width = logical_size.0 as u32;
                self.height = logical_size.1 as u32;
            }
            self.layer_surface.set_size(self.width, self.height);
            self.layer_surface.set_anchor(Anchor::TOP);
            self.surface.commit();
            old_surface.destroy();
        }
    }

    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.new_output(conn, qh, output);
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_registry!(AppState);
delegate_layer!(AppState);

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![];
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _time: u32,
    ) {
        if self.running.load(Ordering::SeqCst) {
            if let Err(e) = self.draw(conn, qh) {
                warn!("Draw error: {}", e);
            }
        }
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {
        self.running.store(false, Ordering::SeqCst);
    }

    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        _configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
    }
}

// ============================================================================
// Public Renderer
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
        // 1. Check Wayland environment
        if std::env::var("WAYLAND_DISPLAY").is_err() {
            return Err(anyhow!("WAYLAND_DISPLAY not set"));
        }

        // 2. Generate colors from wallpaper or config
        let colors = if self.config.general.auto_colors {
            WallpaperAnalyzer::generate_gradient_colors(8)
                .unwrap_or_else(|_| WallpaperAnalyzer::default_colors(8))
        } else {
            self.config.colors.colors.iter()
                .filter(|(k,_)| k.starts_with("gradient_color_"))
                .map(|(_,c)| c.to_array())
                .collect()
        };
        let colors = if colors.is_empty() { WallpaperAnalyzer::default_colors(8) } else { colors };

        let bar_count = self.config.bars.amount as u32;
        let cava_reader = self.cava_manager.take_reader()?;

        // 3. Connect to Wayland
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();

        // 4. Create event loop
        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to initialize event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow!("Failed to insert wayland source: {:?}", e))?;

        // 5. Initialize compositor and layer shell
        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let surface = compositor.create_surface(&qh);
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;
        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            surface.clone(),
            Layer::Bottom,
            Some("cava-bg"),
            None,
        );
        layer_surface.set_size(256, 256);
        layer_surface.set_anchor(Anchor::TOP);
        surface.commit();

        // 6. Initialize EGL
        let egl_api = &egl::API;
        egl_api.bind_api(egl::OPENGL_API).context("Failed to bind OpenGL API")?;
        let egl_display = unsafe {
            egl_api.get_display(conn.display().id().as_ptr() as *mut std::ffi::c_void)
                .context("Failed to get EGL display")?
        };
        egl_api.initialize(egl_display).context("Failed to initialize EGL")?;

        const ATTRIBUTES: [i32; 9] = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::NONE,
        ];

        let egl_config = egl_api.choose_first_config(egl_display, &ATTRIBUTES)
            .context("Failed to choose EGL config")?
            .context("No suitable EGL config found")?;

        const CONTEXT_ATTRIBUTES: [i32; 7] = [
            egl::CONTEXT_MAJOR_VERSION, 3,
            egl::CONTEXT_MINOR_VERSION, 3,
            egl::CONTEXT_OPENGL_PROFILE_MASK,
            egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];

        let egl_context = egl_api.create_context(
            egl_display,
            egl_config,
            None,
            &CONTEXT_ATTRIBUTES,
        ).context("Failed to create EGL context")?;

        let wl_egl_surface = WlEglSurface::new(surface.id(), 256, 256)
            .context("Failed to create Wayland EGL surface")?;
        let egl_surface = unsafe {
            egl_api.create_window_surface(
                egl_display,
                egl_config,
                wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            ).context("Failed to create EGL window surface")?
        };

        egl_api.make_current(
            egl_display,
            Some(egl_surface),
            Some(egl_surface),
            Some(egl_context),
        ).context("Failed to make EGL context current")?;

        gl::load_with(|name| egl_api.get_proc_address(name).unwrap() as *const std::ffi::c_void);

        // 7. Compile shaders
        let vert_shader_source = CString::new(VERTEX_SHADER_SRC).unwrap();
        let vert_shader = unsafe { gl::CreateShader(gl::VERTEX_SHADER) };
        unsafe {
            gl::ShaderSource(
                vert_shader,
                1,
                &vert_shader_source.as_ptr(),
                std::ptr::null(),
            );
            gl::CompileShader(vert_shader);
        }

        let frag_shader_source = CString::new(FRAGMENT_SHADER_SRC).unwrap();
        let frag_shader = unsafe { gl::CreateShader(gl::FRAGMENT_SHADER) };
        unsafe {
            gl::ShaderSource(
                frag_shader,
                1,
                &frag_shader_source.as_ptr(),
                std::ptr::null(),
            );
            gl::CompileShader(frag_shader);
        }

        let shader_program = unsafe { gl::CreateProgram() };
        unsafe {
            gl::AttachShader(shader_program, vert_shader);
            gl::AttachShader(shader_program, frag_shader);
            gl::LinkProgram(shader_program);
            let mut status = gl::FALSE as gl::types::GLint;
            gl::GetProgramiv(shader_program, gl::LINK_STATUS, &mut status);
            if status != 1 {
                let mut error_log_size: gl::types::GLint = 0;
                gl::GetProgramiv(shader_program, gl::INFO_LOG_LENGTH, &mut error_log_size);
                let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
                gl::GetProgramInfoLog(
                    shader_program,
                    error_log_size,
                    &mut error_log_size,
                    error_log.as_mut_ptr() as *mut _,
                );
                error_log.set_len(error_log_size as usize);
                let log = String::from_utf8(error_log).unwrap();
                return Err(anyhow!("Shader link error: {}", log));
            }
        }

        // 8. Create buffers
        let mut vbo = 0;
        let mut vao = 0;

        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::BindVertexArray(vao);
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                (2 * std::mem::size_of::<f32>()) as GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::BindVertexArray(0);
        }

        // 9. Create app state
        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            width: 256,
            height: 256,
            layer_shell,
            layer_surface,
            surface,
            cava_reader: Box::new(cava_reader),
            wl_egl_surface,
            egl_surface,
            egl_config,
            egl_context,
            egl_display,
            shader_program,
            vao,
            vbo,
            bar_count,
            bar_gap: self.config.bars.gap,
            colors: colors.clone(),
            running: self.running.clone(),
            compositor,
        };

        info!("✅ Wayland renderer started");
        info!("🎨 Colors: {} | 📊 Bars: {}", colors.len(), bar_count);
        info!("🖥️  Window: {}x{}", app_state.width, app_state.height);
        info!("⏹️  Press Ctrl+C to exit");

        // 10. Signal handler
        let running_clone = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Shutting down...");
            running_clone.store(false, Ordering::SeqCst);
        }).context("Failed to set Ctrl+C handler")?;

        // 11. Run event loop
        let frame_duration = Duration::from_secs(1) / self.config.general.framerate;
        event_loop
            .run(frame_duration, &mut app_state, |_| {})
            .context("Event loop failed")?;

        // 12. Cleanup
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