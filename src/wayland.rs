//! Complete Wayland renderer for cava-bg
//! Based on wallpaper-cava implementation with proper EGL API usage

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

// Wayland and EGL - follow wallpaper-cava pattern
use khronos_egl as egl;
use egl::API as egl_api; // Important: alias egl::API as egl_api
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
// Shaders (from wallpaper-cava)
// ============================================================================

const VERTEX_SHADER_SRC: &str = r#"
#version 430 core
in vec2 position;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER_SRC: &str = r#"
#version 430 core
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
    gradient_colors_ssbo: u32,
    windows_size_location: i32,
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    preferred_output_name: Option<String>,
    compositor: CompositorState,
    running: Arc<AtomicBool>,
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

                // Calculate bar dimensions (from wallpaper-cava)
                let bar_width: f32 =
                    2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
                let bar_gap_width: f32 = bar_width * self.bar_gap;

                // Generate vertices (from wallpaper-cava)
                let mut vertices: Vec<f32> = vec![0.0; self.bar_count as usize * 8];
                let fwidth: f32 = self.width as f32;
                let fheight: f32 = self.height as f32;

                for i in 0..self.bar_count as usize {
                    let bar_height: f32 = 2.0 * unpacked_data[i] - 1.0;
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
                    gl::ClearColor(
                        self.background_color[0],
                        self.background_color[1],
                        self.background_color[2],
                        self.background_color[3],
                    );
                    gl::Clear(gl::COLOR_BUFFER_BIT);
                    gl::UseProgram(self.shader_program);
                    gl::Uniform2f(self.windows_size_location, fwidth, fheight);
                    gl::DrawElements(
                        gl::TRIANGLES,
                        (self.bar_count as usize * 3 * std::mem::size_of::<u16>()) as GLsizei,
                        gl::UNSIGNED_SHORT,
                        ptr::null(),
                    );
                    gl::BindVertexArray(0);
                }

                // Swap buffers using egl_api (from wallpaper-cava)
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
            let mut need_configuration = false;
            if let Some(output_name) = info.name {
                if let Some(preferred_output_name) = self.preferred_output_name.clone() {
                    if output_name == preferred_output_name {
                        need_configuration = true;
                    }
                }
            }
            if self.preferred_output_name.is_none() {
                need_configuration = true;
            }
            if need_configuration {
                let old_surface = self.surface.clone();
                self.surface = self.compositor.create_surface(qh);
                self.layer_surface = self.layer_shell.create_layer_surface(
                    qh,
                    self.surface.clone(),
                    Layer::Top,  // Changed from Bottom to Top
                    Some("cava-bg"),
                    Some(&output),
                );
                if let Some(logical_size) = info.logical_size {
                    self.width = logical_size.0 as u32;
                    self.height = logical_size.1 as u32;
                }
                self.layer_surface.set_size(self.width, self.height);
                // Combine all anchors to cover entire screen
                self.layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
                self.layer_surface.set_exclusive_zone(-1);   // -1 means no exclusive zone
                self.surface.commit();
                old_surface.destroy();
            }
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

        // 3. Connect to Wayland (from wallpaper-cava)
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();

        // 4. Create event loop (from wallpaper-cava)
        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to initialize event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow!("Failed to insert wayland source: {:?}", e))?;

        // 5. Initialize compositor and layer shell (from wallpaper-cava)
        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let surface = compositor.create_surface(&qh);
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;
        let layer_surface = layer_shell.create_layer_surface(
            &qh,
            surface.clone(),
            Layer::Top,  // Changed from Bottom to Top - appears above wallpaper but below windows
            Some("cava-bg"),
            None,
        );
        // Initial size - will be updated when we get output info
        layer_surface.set_size(1920, 1080);
        // Combine all anchors to cover entire screen
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);   // -1 means no exclusive zone, doesn't push windows
        surface.commit();

        // 6. Initialize EGL (from wallpaper-cava)
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
            egl::CONTEXT_MAJOR_VERSION, 4,
            egl::CONTEXT_MINOR_VERSION, 6,
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

        // 7. Compile shaders (from wallpaper-cava)
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

        // 8. Create buffers (from wallpaper-cava)
        let mut vbo = 0;
        let mut vao = 0;
        let mut ebo = 0;
        let mut gradient_colors_ssbo = 0;

        // Prepare gradient colors buffer (from wallpaper-cava)
        let gradient_colors_size = colors.len() as i32;
        let mut buffer_data: Vec<u8> = (gradient_colors_size).to_le_bytes().to_vec();
        buffer_data.extend([0, 0, 0, 0].repeat(3)); // Fix for vec4 alignment
        for color in colors.iter() {
            for color_value in color {
                buffer_data.extend_from_slice(&color_value.to_le_bytes());
            }
        }

        // Create indices (from wallpaper-cava)
        let mut indices: Vec<u16> = vec![0; bar_count as usize * 6];
        for i in 0..bar_count as usize {
            indices[i * 6] = i as u16 * 4;
            indices[i * 6 + 1] = i as u16 * 4 + 1;
            indices[i * 6 + 2] = i as u16 * 4 + 2;
            indices[i * 6 + 3] = i as u16 * 4 + 1;
            indices[i * 6 + 4] = i as u16 * 4 + 2;
            indices[i * 6 + 5] = i as u16 * 4 + 3;
        }

        let window_size_string = CString::new("WindowSize").unwrap();
        let windows_size_location = unsafe {
            gl::GetUniformLocation(shader_program, window_size_string.as_ptr())
        };

        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::BindVertexArray(vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
            gl::GenBuffers(1, &mut gradient_colors_ssbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<u16>()) as GLsizeiptr,
                indices.as_ptr() as *const std::ffi::c_void,
                gl::STATIC_DRAW,
            );
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, gradient_colors_ssbo);
            gl::BufferData(
                gl::SHADER_STORAGE_BUFFER,
                buffer_data.len() as GLsizeiptr,
                buffer_data.as_ptr() as *const std::ffi::c_void,
                gl::STATIC_DRAW,
            );
            gl::BindBufferBase(gl::SHADER_STORAGE_BUFFER, 0, gradient_colors_ssbo);
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
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
            width: 1920,  // Initial size
            height: 1080,
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
            gradient_colors_ssbo,
            windows_size_location,
            bar_count,
            bar_gap: self.config.bars.gap,
            background_color: [0.0, 0.0, 0.0, 0.0], // Transparent background
            preferred_output_name: None,
            compositor,
            running: self.running.clone(),
        };

        info!("Wayland renderer started");
        info!("Colors: {} | Bars: {}", colors.len(), bar_count);
        info!("Layer: Top (above wallpaper, below windows)");
        info!("Anchor: TOP|BOTTOM|LEFT|RIGHT (full screen coverage)");
        info!("Press Ctrl+C to exit");

        // 10. Signal handler - use the global signal handler from main.rs
        // Note: ctrlc::set_handler() can only be called once per process
        // The handler is already set in main.rs, so we just use the running flag

        // 11. Run event loop (from wallpaper-cava)
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