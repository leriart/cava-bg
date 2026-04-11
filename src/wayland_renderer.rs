use anyhow::{anyhow, Context, Result};
use gl::types::{GLsizei, GLsizeiptr, GLint};
use khronos_egl as egl;
use log::{debug, error, info, warn};
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

use std::collections::HashMap;
use std::ffi::{c_void, CString};
use std::io::{BufReader, Read};
use std::process::ChildStdout;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{mem, ptr};

use crate::app_config::{array_from_config_color, Config};

// Shaders compatibles con OpenGL 3.0 (GLSL 1.30)
const VERTEX_SHADER_SRC: &str = r#"
#version 130
in vec2 position;
void main() {
    gl_Position = vec4(position, 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER_SRC: &str = r#"
#version 130
uniform vec2 WindowSize;
uniform vec4 gradient_colors[32];
uniform int gradient_colors_size;
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

struct PerOutputState {
    surface: WlSurface,
    layer_surface: LayerSurface,
    wl_egl_surface: WlEglSurface,
    egl_surface: egl::Surface,
    width: u32,
    height: u32,
    configured: bool,
}

pub struct WaylandRenderer {
    config: Config,
    audio_rx: Receiver<Vec<f32>>,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(config: Config, audio_rx: Receiver<Vec<f32>>, running: Arc<AtomicBool>) -> Self {
        Self { config, audio_rx, running }
    }

    pub fn run(self) -> Result<()> {
        info!("Iniciando renderizador Wayland con OpenGL 3.0 legacy");
        std::env::set_var("EGL_PLATFORM", "wayland");

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();
        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let frame_duration = Duration::from_secs(1) / self.config.general.framerate;
        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;

        egl::API
            .bind_api(egl::OPENGL_API)
            .context("Failed to bind EGL API")?;
        let egl_display = unsafe {
            egl::API
                .get_display(conn.display().id().as_ptr() as *mut c_void)
                .context("Failed to get EGL display")?
        };
        egl::API
            .initialize(egl_display)
            .context("Failed to initialize EGL")?;

        const ATTRIBUTES_WITH_ALPHA: [i32; 9] = [
            egl::RED_SIZE, 8,
            egl::GREEN_SIZE, 8,
            egl::BLUE_SIZE, 8,
            egl::ALPHA_SIZE, 8,
            egl::NONE,
        ];
        let egl_config = egl::API
            .choose_first_config(egl_display, &ATTRIBUTES_WITH_ALPHA)
            .context("Failed to choose EGL config")?
            .context("No EGL config found")?;

        let context_attribs = [
            egl::CONTEXT_MAJOR_VERSION, 3,
            egl::CONTEXT_MINOR_VERSION, 0,
            egl::CONTEXT_OPENGL_PROFILE_MASK, egl::CONTEXT_OPENGL_COMPATIBILITY_PROFILE_BIT,
            egl::NONE,
        ];
        let egl_context = egl::API
            .create_context(egl_display, egl_config, None, &context_attribs)
            .context("Failed to create EGL context")?;

        let dummy_surface = egl::API
            .create_pbuffer_surface(egl_display, egl_config, &[egl::WIDTH, 1, egl::HEIGHT, 1, egl::NONE])
            .context("Failed to create pbuffer surface")?;
        egl::API
            .make_current(egl_display, Some(dummy_surface), Some(dummy_surface), Some(egl_context))
            .context("Failed to make context current")?;

        gl::load_with(|name| {
            let name_c = CString::new(name).unwrap();
            match egl::API.get_proc_address(name_c.to_str().unwrap()) {
                Some(proc) => proc as *const c_void,
                None => ptr::null(),
            }
        });

        let vert_shader = compile_shader(gl::VERTEX_SHADER, VERTEX_SHADER_SRC)?;
        let frag_shader = compile_shader(gl::FRAGMENT_SHADER, FRAGMENT_SHADER_SRC)?;
        let shader_program = link_program(vert_shader, frag_shader)?;
        unsafe {
            gl::DeleteShader(vert_shader);
            gl::DeleteShader(frag_shader);
        }
        egl::API.destroy_surface(egl_display, dummy_surface).ok();
        info!("OpenGL context inicializado (legacy 3.0)");

        let gradient_colors_rgba: Vec<[f32; 4]> = self
            .config
            .colors
            .iter()
            .map(|(_, color)| array_from_config_color(color.clone()))
            .collect();

        let bar_count = self.config.bars.amount as usize;
        let mut indices: Vec<u16> = vec![0; bar_count * 6];
        for i in 0..bar_count {
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

        let window_size_loc = unsafe { gl::GetUniformLocation(shader_program, b"WindowSize\0".as_ptr() as *const _) };
        let colors_count_loc = unsafe { gl::GetUniformLocation(shader_program, b"gradient_colors_size\0".as_ptr() as *const _) };
        let mut color_locs = [-1; 32];
        for i in 0..32 {
            let name = format!("gradient_colors[{}]", i);
            let cname = CString::new(name).unwrap();
            color_locs[i] = unsafe { gl::GetUniformLocation(shader_program, cname.as_ptr()) };
        }

        let updating_colors = Arc::new(AtomicBool::new(false));

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            layer_shell,
            compositor,
            per_output: HashMap::new(),
            egl_display,
            egl_config,
            egl_context,
            shader_program,
            vao,
            vbo,
            window_size_loc,
            colors_count_loc,
            color_locs,
            bar_count: self.config.bars.amount,
            bar_gap: self.config.bars.gap,
            background_color: [0.0, 0.0, 0.0, 0.0],
            preferred_output_name: self.config.general.preferred_output,
            cava_reader: self.cava_reader,
            color_rx: self.color_rx,
            gradient_colors: gradient_colors_rgba,
            running: self.running,
            updating_colors,
            test_phase: 0.0,
            frame_count: 0,
        };

        event_loop
            .run(frame_duration, &mut app_state, |_| {})
            .context("Event loop failed")?;

        Ok(())
    }
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    layer_shell: LayerShell,
    compositor: CompositorState,
    per_output: HashMap<String, PerOutputState>,
    egl_display: egl::Display,
    egl_config: egl::Config,
    egl_context: egl::Context,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    window_size_loc: GLint,
    colors_count_loc: GLint,
    color_locs: [GLint; 32],
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    preferred_output_name: Option<String>,
    cava_reader: BufReader<ChildStdout>,
    color_rx: Arc<Mutex<Receiver<Vec<[f32; 4]>>>>,
    gradient_colors: Vec<[f32; 4]>,
    running: Arc<AtomicBool>,
    updating_colors: Arc<AtomicBool>,
    test_phase: f32,
    frame_count: u64,
}

impl AppState {
    fn update_colors(&mut self, new_colors: &[[f32; 4]]) {
        self.updating_colors.store(true, Ordering::SeqCst);
        self.gradient_colors = new_colors.to_vec();
        self.updating_colors.store(false, Ordering::SeqCst);
        info!("Colores degradado actualizados: {} colores", self.gradient_colors.len());
    }

    fn ensure_output(&mut self, qh: &QueueHandle<Self>, output: &wl_output::WlOutput) -> Result<()> {
        let info = self.output_state.info(output).context("Failed to get output info")?;
        let name = info.name.clone().unwrap_or_else(|| "unknown".to_string());
        if self.per_output.contains_key(&name) {
            return Ok(());
        }
        if let Some(ref pref) = self.preferred_output_name {
            if &name != pref {
                debug!("Omitiendo output {} (preferido es {})", name, pref);
                return Ok(());
            }
        }
        info!("Creando superficie para output {}", name);
        let surface = self.compositor.create_surface(qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            qh,
            surface.clone(),
            Layer::Bottom,
            Some("cava-bg"),
            Some(output),
        );
        let logical_size = info.logical_size.unwrap_or((1920, 1080));
        let width = logical_size.0 as u32;
        let height = logical_size.1 as u32;
        layer_surface.set_size(width, height);
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        surface.commit();
        let wl_egl_surface = WlEglSurface::new(surface.id(), width as i32, height as i32)
            .context("Failed to create WlEglSurface")?;
        let egl_surface = unsafe {
            egl::API
                .create_window_surface(
                    self.egl_display,
                    self.egl_config,
                    wl_egl_surface.ptr() as egl::NativeWindowType,
                    None,
                )
                .context("Failed to create EGL window surface")?
        };
        self.per_output.insert(
            name.clone(),
            PerOutputState {
                surface,
                layer_surface,
                wl_egl_surface,
                egl_surface,
                width,
                height,
                configured: false,
            },
        );
        info!("Superficie creada para {}: {}x{}", name, width, height);
        Ok(())
    }

    fn draw_output(&mut self, name: &str, qh: &QueueHandle<Self>) {
        let state = match self.per_output.get_mut(name) {
            Some(s) if s.configured => s,
            _ => return,
        };

        unsafe {
            if egl::API
                .make_current(
                    self.egl_display,
                    Some(state.egl_surface),
                    Some(state.egl_surface),
                    Some(self.egl_context),
                )
                .is_err()
            {
                error!("make_current fallo para {}", name);
                return;
            }
            gl::Viewport(0, 0, state.width as GLsizei, state.height as GLsizei);
        }

        let mut unpacked_data: Vec<f32> = vec![0.0; self.bar_count as usize];
        let mut cava_buffer: Vec<u8> = vec![0; self.bar_count as usize * 2];
        let used_test = match self.cava_reader.read_exact(&mut cava_buffer) {
            Ok(_) => {
                for (i, chunk) in cava_buffer.chunks_exact(2).enumerate() {
                    let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                    unpacked_data[i] = (num as f32) / 65530.0;
                }
                false
            }
            Err(e) => {
                self.test_phase += 0.1;
                for i in 0..unpacked_data.len() {
                    unpacked_data[i] = ((self.test_phase + i as f32 * 0.5).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
                }
                if self.frame_count % 60 == 0 {
                    warn!("Usando datos de prueba (cava read error: {}), alturas: {:?}", e, &unpacked_data[0..3]);
                }
                true
            }
        };

        if self.frame_count % 120 == 0 {
            debug!("Barra 0 altura: {:.3} (test={})", unpacked_data[0], used_test);
        }

        let bar_width: f32 =
            2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width: f32 = bar_width * self.bar_gap;
        let mut vertices: Vec<f32> = vec![0.0; self.bar_count as usize * 8];

        for i in 0..self.bar_count as usize {
            let bar_height: f32 = 2.0 * unpacked_data[i] - 1.0;
            let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
            let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
            vertices[i * 8] = x0;
            vertices[i * 8 + 1] = bar_height;
            vertices[i * 8 + 2] = x1;
            vertices[i * 8 + 3] = bar_height;
            vertices[i * 8 + 4] = x0;
            vertices[i * 8 + 5] = -1.0;
            vertices[i * 8 + 6] = x1;
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
            gl::Uniform2f(self.window_size_loc, state.width as f32, state.height as f32);
            gl::Uniform1i(self.colors_count_loc, self.gradient_colors.len() as i32);
            for (i, color) in self.gradient_colors.iter().enumerate() {
                if i < 32 && self.color_locs[i] != -1 {
                    gl::Uniform4f(self.color_locs[i], color[0], color[1], color[2], color[3]);
                }
            }
            let index_count = (self.bar_count as usize * 6) as GLsizei;
            gl::DrawElements(gl::TRIANGLES, index_count, gl::UNSIGNED_SHORT, ptr::null());
            gl::BindVertexArray(0);
        }

        if let Err(e) = egl::API.swap_buffers(self.egl_display, state.egl_surface) {
            error!("Falló swap buffers para {}: {}", name, e);
        }
        state.surface.frame(qh, state.surface.clone());
        self.frame_count += 1;
    }

    pub fn draw(&mut self, _conn: &Connection, qh: &QueueHandle<Self>) {
        if !self.running.load(Ordering::SeqCst) {
            info!("Apagando graceful...");
            for (_, state) in self.per_output.iter() {
                unsafe { egl::API.destroy_surface(self.egl_display, state.egl_surface).ok(); }
            }
            unsafe {
                egl::API.make_current(self.egl_display, None, None, None).ok();
                egl::API.destroy_context(self.egl_display, self.egl_context).ok();
                egl::API.terminate(self.egl_display).ok();
            }
            std::process::exit(0);
        }

        if self.updating_colors.load(Ordering::SeqCst) {
            for (_, state) in self.per_output.iter() {
                state.surface.frame(qh, state.surface.clone());
            }
            return;
        }

        let new_colors = {
            match self.color_rx.lock() {
                Ok(guard) => guard.try_recv().ok(),
                Err(_) => None,
            }
        };
        if let Some(colors) = new_colors {
            self.update_colors(&colors);
        }

        let names: Vec<String> = self.per_output.keys().cloned().collect();
        for name in names {
            self.draw_output(&name, qh);
        }
    }
}

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Err(e) = self.ensure_output(qh, &output) {
            error!("Fallo al crear output: {}", e);
        }
    }
    fn update_output(&mut self, _conn: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.new_output(_conn, qh, output);
    }
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = match self.output_state.info(&output) {
            Some(i) => i,
            None => return,
        };
        let name = info.name.unwrap_or_else(|| "unknown".to_string());
        if let Some(state) = self.per_output.remove(&name) {
            unsafe { egl::API.destroy_surface(self.egl_display, state.egl_surface).ok(); }
            info!("Output {} eliminado", name);
        }
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
    fn frame(&mut self, conn: &Connection, qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {
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
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let mut target_name = None;
        for (name, state) in self.per_output.iter_mut() {
            if &state.layer_surface == layer {
                let width = configure.new_size.0;
                let height = configure.new_size.1;
                if width == state.width && height == state.height && state.configured {
                    return;
                }
                state.width = width;
                state.height = height;
                unsafe {
                    egl::API.make_current(self.egl_display, None, None, None).ok();
                    egl::API.destroy_surface(self.egl_display, state.egl_surface).ok();
                }
                state.wl_egl_surface = match WlEglSurface::new(
                    state.surface.id(),
                    state.width as i32,
                    state.height as i32,
                ) {
                    Ok(s) => s,
                    Err(e) => {
                        error!("Fallo crear WlEglSurface tras configure: {}", e);
                        return;
                    }
                };
                state.surface.commit();
                state.egl_surface = unsafe {
                    match egl::API.create_window_surface(
                        self.egl_display,
                        self.egl_config,
                        state.wl_egl_surface.ptr() as egl::NativeWindowType,
                        None,
                    ) {
                        Ok(s) => s,
                        Err(e) => {
                            error!("Fallo crear EGL surface tras configure: {}", e);
                            return;
                        }
                    }
                };
                state.configured = true;
                target_name = Some(name.clone());
                info!("Output {} configurado: {}x{}", name, state.width, state.height);
                break;
            }
        }
        if let Some(name) = target_name {
            self.draw_output(&name, qh);
        }
    }
}

fn compile_shader(shader_type: u32, src: &str) -> Result<u32> {
    unsafe {
        let shader = gl::CreateShader(shader_type);
        let c_str = CString::new(src).unwrap();
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), std::ptr::null());
        gl::CompileShader(shader);
        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut log_len = 0;
            gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut log_len);
            let mut log = vec![0u8; log_len as usize];
            gl::GetShaderInfoLog(
                shader,
                log_len,
                std::ptr::null_mut(),
                log.as_mut_ptr() as *mut _,
            );
            let msg = String::from_utf8_lossy(&log);
            return Err(anyhow!("Error compilación shader: {}", msg));
        }
        Ok(shader)
    }
}

fn link_program(vs: u32, fs: u32) -> Result<u32> {
    unsafe {
        let prog = gl::CreateProgram();
        gl::AttachShader(prog, vs);
        gl::AttachShader(prog, fs);
        gl::LinkProgram(prog);
        let mut success = 0;
        gl::GetProgramiv(prog, gl::LINK_STATUS, &mut success);
        if success == 0 {
            let mut log_len = 0;
            gl::GetProgramiv(prog, gl::INFO_LOG_LENGTH, &mut log_len);
            let mut log = vec![0u8; log_len as usize];
            gl::GetProgramInfoLog(
                prog,
                log_len,
                std::ptr::null_mut(),
                log.as_mut_ptr() as *mut _,
            );
            let msg = String::from_utf8_lossy(&log);
            return Err(anyhow!("Error enlace programa: {}", msg));
        }
        Ok(prog)
    }
}