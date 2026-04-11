// src/wayland_renderer.rs
// Versión corregida para khronos-egl 6.0 con feature "static"

use anyhow::{Context, Result};
use gl::types::{GLsizei, GLsizeiptr};
use khronos_egl as egl;
use log::{error, info, warn};
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
    protocol::{wl_output, wl_surface},
    Connection, QueueHandle,
};
use wayland_egl::WlEglSurface;

use std::ffi::{c_void, CString};
use std::io::{BufReader, Read};
use std::process::ChildStdout;
use std::sync::mpsc::Receiver;
use std::time::Duration;
use std::{mem, ptr};

use crate::config::Config;

const VERTEX_SHADER_SRC: &str = include_str!("shaders/vertex_shader.glsl");
const FRAGMENT_SHADER_SRC: &str = include_str!("shaders/fragment_shader.glsl");

pub struct WaylandRenderer {
    config: Config,
    cava_reader: BufReader<ChildStdout>,
    color_rx: Receiver<Vec<[f32; 4]>>,
}

impl WaylandRenderer {
    pub fn new(
        config: Config,
        cava_reader: BufReader<ChildStdout>,
        color_rx: Receiver<Vec<[f32; 4]>>,
    ) -> Self {
        Self {
            config,
            cava_reader,
            color_rx,
        }
    }

    pub fn run(self) -> Result<()> {
        info!("Starting Wayland renderer (wallpaper-cava core)");

        let framerate = self.config.general.framerate;
        let bar_count = self.config.bars.amount;
        let bar_gap = self.config.bars.gap;
        let background_color = self.config.general.background_color.to_array();
        let preferred_output = self.config.general.preferred_output.clone();

        let gradient_colors: Vec<[f32; 4]> = self
            .config
            .colors
            .colors
            .values()
            .map(|c: &crate::config::Color| c.to_array())
            .collect();

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();
        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let frame_duration = Duration::from_secs(1) / framerate;
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

        // EGL initialization (usando feature "static")
        egl::bind_api(egl::OPENGL_API).context("Failed to bind EGL API")?;
        let egl_display = unsafe {
            egl::get_display(conn.display().id().as_ptr() as *mut c_void)
                .context("Failed to get EGL display")?
        };
        egl::initialize(egl_display).context("Failed to initialize EGL")?;

        const ATTRIBUTES: [i32; 9] = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::NONE,
        ];
        let egl_config = egl::choose_first_config(egl_display, &ATTRIBUTES)
            .context("Failed to choose EGL config")?
            .context("No EGL config found")?;

        const CONTEXT_ATTRIBUTES: [i32; 7] = [
            egl::CONTEXT_MAJOR_VERSION, 4,
            egl::CONTEXT_MINOR_VERSION, 6,
            egl::CONTEXT_OPENGL_PROFILE_MASK, egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
            egl::NONE,
        ];
        let egl_context = egl::create_context(egl_display, egl_config, None, &CONTEXT_ATTRIBUTES)
            .context("Failed to create EGL context")?;

        let wl_egl_surface = WlEglSurface::new(surface.id(), 256, 256)
            .context("Failed to create WlEglSurface")?;
        let egl_surface = unsafe {
            egl::create_window_surface(
                egl_display,
                egl_config,
                wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            )
            .context("Failed to create EGL window surface")?
        };
        egl::make_current(
            egl_display,
            Some(egl_surface),
            Some(egl_surface),
            Some(egl_context),
        )
        .context("Failed to make EGL context current")?;

        gl::load_with(|name| egl::get_proc_address(name).unwrap() as *const c_void);

        let vert_shader = compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER_SRC)?;
        let frag_shader = compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
        let shader_program = link_program(vert_shader, frag_shader)?;
        unsafe {
            gl::DeleteShader(vert_shader);
            gl::DeleteShader(frag_shader);
        }

        let (gradient_colors_ssbo, _) = create_ssbo(&gradient_colors);

        let mut indices: Vec<u16> = vec![0; bar_count as usize * 6];
        for i in 0..bar_count as usize {
            let base = (i * 4) as u16;
            indices[i * 6] = base;
            indices[i * 6 + 1] = base + 1;
            indices[i * 6 + 2] = base + 2;
            indices[i * 6 + 3] = base + 1;
            indices[i * 6 + 4] = base + 2;
            indices[i * 6 + 5] = base + 3;
        }

        let mut vao = 0;
        let mut vbo = 0;
        let mut ebo = 0;
        unsafe {
            gl::GenVertexArrays(1, &mut vao);
            gl::BindVertexArray(vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * mem::size_of::<u16>()) as GLsizeiptr,
                indices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                (2 * mem::size_of::<f32>()) as GLsizei,
                ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            gl::BindVertexArray(0);
        }

        let window_size_string = CString::new("WindowSize").unwrap();
        let window_size_location =
            unsafe { gl::GetUniformLocation(shader_program, window_size_string.as_ptr()) };

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            width: 256,
            height: 256,
            layer_shell,
            layer_surface,
            surface,
            cava_reader: self.cava_reader,
            wl_egl_surface,
            egl_surface,
            egl_config,
            egl_context,
            egl_display,
            shader_program,
            vao,
            vbo,
            window_size_location,
            bar_count,
            bar_gap,
            background_color,
            preferred_output_name: preferred_output,
            compositor,
            color_rx: self.color_rx,
            gradient_colors_ssbo,
            gradient_colors,
        };

        info!("Entering event loop with frame duration {:?}", frame_duration);
        event_loop
            .run(frame_duration, &mut app_state, |_| {})
            .context("Event loop failed")?;

        Ok(())
    }
}

fn create_ssbo(colors: &[[f32; 4]]) -> (u32, Vec<[f32; 4]>) {
    let gradient_colors_len = colors.len() as i32;
    let mut buffer_data: Vec<u8> = gradient_colors_len.to_le_bytes().to_vec();
    buffer_data.extend([0, 0, 0, 0].repeat(3));
    for color in colors {
        for &value in color {
            buffer_data.extend_from_slice(&value.to_le_bytes());
        }
    }

    let mut ssbo = 0;
    unsafe {
        gl::GenBuffers(1, &mut ssbo);
        gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, ssbo);
        gl::BufferData(
            gl::SHADER_STORAGE_BUFFER,
            buffer_data.len() as GLsizeiptr,
            buffer_data.as_ptr() as *const _,
            gl::STATIC_DRAW,
        );
        gl::BindBufferBase(gl::SHADER_STORAGE_BUFFER, 0, ssbo);
        gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
    }
    (ssbo, colors.to_vec())
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    width: u32,
    height: u32,
    layer_shell: LayerShell,
    layer_surface: LayerSurface,
    surface: WlSurface,
    cava_reader: BufReader<ChildStdout>,
    wl_egl_surface: WlEglSurface,
    egl_surface: egl::Surface,
    egl_config: egl::Config,
    egl_context: egl::Context,
    egl_display: egl::Display,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    window_size_location: i32,
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    preferred_output_name: Option<String>,
    compositor: CompositorState,
    color_rx: Receiver<Vec<[f32; 4]>>,
    gradient_colors_ssbo: u32,
    gradient_colors: Vec<[f32; 4]>,
}

impl AppState {
    fn update_colors(&mut self, new_colors: &[[f32; 4]]) {
        self.gradient_colors = new_colors.to_vec();
        let gradient_colors_len = self.gradient_colors.len() as i32;
        let mut buffer_data = gradient_colors_len.to_le_bytes().to_vec();
        buffer_data.extend([0, 0, 0, 0].repeat(3));
        for color in &self.gradient_colors {
            for &value in color {
                buffer_data.extend_from_slice(&value.to_le_bytes());
            }
        }
        unsafe {
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, self.gradient_colors_ssbo);
            gl::BufferData(
                gl::SHADER_STORAGE_BUFFER,
                buffer_data.len() as GLsizeiptr,
                buffer_data.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
        }
        info!("Updated gradient colors from wallpaper change");
    }

    fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>) {
        if let Ok(new_colors) = self.color_rx.try_recv() {
            self.update_colors(&new_colors);
        }

        let mut cava_buffer: Vec<u8> = vec![0; self.bar_count as usize * 2];
        if let Err(e) = self.cava_reader.read_exact(&mut cava_buffer) {
            warn!("Failed to read from cava: {}", e);
            self.surface.frame(qh, self.surface.clone());
            return;
        }

        let mut unpacked_data: Vec<f32> = vec![0.0; self.bar_count as usize];
        for (unpacked_data_index, i) in (0..cava_buffer.len()).step_by(2).enumerate() {
            let num = u16::from_le_bytes([cava_buffer[i], cava_buffer[i + 1]]);
            unpacked_data[unpacked_data_index] = (num as f32) / 65530.0;
        }

        let bar_width: f32 =
            2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width: f32 = bar_width * self.bar_gap;
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
                (vertices.len() * mem::size_of::<f32>()) as GLsizeiptr,
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
            gl::Uniform2f(self.window_size_location, fwidth, fheight);
            gl::DrawElements(
                gl::TRIANGLES,
                (self.bar_count as usize * 3 * mem::size_of::<u16>()) as GLsizei,
                gl::UNSIGNED_SHORT,
                ptr::null(),
            );
            gl::BindVertexArray(0);
        }

        if let Err(e) = egl::swap_buffers(self.egl_display, self.egl_surface) {
            error!("Failed to swap buffers: {}", e);
        }
        self.surface.frame(qh, self.surface.clone());
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
        let info = match self.output_state.info(&output) {
            Some(i) => i,
            None => return,
        };
        let mut need_configuration = false;
        if let Some(ref pref) = self.preferred_output_name {
            if let Some(ref name) = info.name {
                if name == pref {
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
                Layer::Bottom,
                Some("cava-bg"),
                Some(&output),
            );
            let logical_size = info.logical_size.unwrap_or((256, 256));
            self.width = logical_size.0 as u32;
            self.height = logical_size.1 as u32;
            self.layer_surface.set_size(self.width, self.height);
            self.layer_surface.set_anchor(Anchor::TOP);
            self.surface.commit();
            old_surface.destroy();

            // Recreate EGL surface
            egl::make_current(self.egl_display, None, None, None).ok();
            egl::destroy_surface(self.egl_display, self.egl_surface).ok();
            self.wl_egl_surface = WlEglSurface::new(self.surface.id(), self.width as i32, self.height as i32)
                .expect("Failed to create new WlEglSurface");
            self.egl_surface = unsafe {
                egl::create_window_surface(
                    self.egl_display,
                    self.egl_config,
                    self.wl_egl_surface.ptr() as egl::NativeWindowType,
                    None,
                )
                .expect("Failed to create new EGL surface")
            };
            egl::make_current(
                self.egl_display,
                Some(self.egl_surface),
                Some(self.egl_surface),
                Some(self.egl_context),
            )
            .expect("Failed to make EGL context current");
            unsafe {
                gl::Viewport(0, 0, self.width as GLsizei, self.height as GLsizei);
            }
            info!("Output changed: {}x{}", self.width, self.height);
        }
    }

    fn update_output(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        self.new_output(_conn, qh, output);
    }

    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _output: wl_output::WlOutput,
    ) {
    }
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_factor: i32,
    ) {
    }

    fn transform_changed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _new_transform: wl_output::Transform,
    ) {
    }

    fn frame(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
        self.draw(conn, qh);
    }

    fn surface_enter(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }

    fn surface_leave(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _output: &wl_output::WlOutput,
    ) {
    }
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}

    fn configure(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        _layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let width = configure.new_size.0;
        let height = configure.new_size.1;
        if width == self.width && height == self.height {
            return;
        }

        self.width = width;
        self.height = height;

        egl::make_current(self.egl_display, None, None, None).ok();
        egl::destroy_surface(self.egl_display, self.egl_surface).ok();
        self.wl_egl_surface = WlEglSurface::new(self.surface.id(), self.width as i32, self.height as i32)
            .expect("Failed to create new WlEglSurface");
        self.surface.commit();
        self.egl_surface = unsafe {
            egl::create_window_surface(
                self.egl_display,
                self.egl_config,
                self.wl_egl_surface.ptr() as egl::NativeWindowType,
                None,
            )
            .expect("Failed to create new EGL surface")
        };
        egl::make_current(
            self.egl_display,
            Some(self.egl_surface),
            Some(self.egl_surface),
            Some(self.egl_context),
        )
        .expect("Failed to make EGL context current");
        unsafe {
            gl::Viewport(0, 0, self.width as GLsizei, self.height as GLsizei);
        }
        self.draw(_conn, qh);
        info!("Layer configured: {}x{}", self.width, self.height);
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

fn compile_shader(shader_type: u32, src: &str) -> Result<u32> {
    unsafe {
        let shader = gl::CreateShader(shader_type);
        let c_str = CString::new(src).unwrap();
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), ptr::null());
        gl::CompileShader(shader);

        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 512];
            gl::GetShaderInfoLog(shader, 512, ptr::null_mut(), log.as_mut_ptr() as *mut _);
            let msg = String::from_utf8_lossy(&log);
            error!("Shader compilation failed: {}", msg);
            return Err(anyhow::anyhow!("Shader compilation failed: {}", msg));
        }
        Ok(shader)
    }
}

fn link_program(vs: u32, fs: u32) -> Result<u32> {
    unsafe {
        let program = gl::CreateProgram();
        gl::AttachShader(program, vs);
        gl::AttachShader(program, fs);
        gl::LinkProgram(program);

        let mut success = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
        if success == 0 {
            let mut log = vec![0u8; 512];
            gl::GetProgramInfoLog(program, 512, ptr::null_mut(), log.as_mut_ptr() as *mut _);
            let msg = String::from_utf8_lossy(&log);
            error!("Program linking failed: {}", msg);
            return Err(anyhow::anyhow!("Program linking failed: {}", msg));
        }
        Ok(program)
    }
}