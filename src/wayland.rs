//! Complete Wayland renderer for cava-bg
//! Single-file implementation with real layer-shell and OpenGL rendering

use anyhow::{anyhow, Context, Result};
use log::{debug, error, info, warn};
use std::ffi::{CStr, CString};
use std::io::Read;
use std::os::unix::io::AsRawFd;
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;
use crate::wallpaper::WallpaperAnalyzer;

// Wayland client imports
use wayland_client::{
    protocol::{
        wl_compositor::WlCompositor,
        wl_display::WlDisplay,
        wl_egl_window::WlEglWindow,
        wl_output::WlOutput,
        wl_registry::WlRegistry,
        wl_seat::WlSeat,
        wl_shm::WlShm,
        wl_surface::WlSurface,
    },
    Connection, Dispatch, QueueHandle, delegate_noop,
};
use wayland_protocols::wp::layer_shell::v1::client::{
    zwlr_layer_shell_v1::ZwlrLayerShellV1,
    zwlr_layer_surface_v1::{self, ZwlrLayerSurfaceV1},
};

// EGL and OpenGL imports
use khronos_egl as egl;
use gl::types::{GLint, GLuint, GLvoid};

// ============================================================================
// Wayland State
// ============================================================================

struct WaylandState {
    connection: Connection,
    display: WlDisplay,
    queue_handle: QueueHandle<Self>,
    compositor: Option<WlCompositor>,
    layer_shell: Option<ZwlrLayerShellV1>,
    surface: Option<WlSurface>,
    layer_surface: Option<ZwlrLayerSurfaceV1>,
    egl_window: Option<WlEglWindow>,
    output: Option<WlOutput>,
    
    // EGL/OpenGL
    egl_display: egl::EGLDisplay,
    egl_context: egl::EGLContext,
    egl_surface: egl::EGLSurface,
    egl_config: egl::EGLConfig,
    
    // Shader program
    shader_program: GLuint,
    vao: GLuint,
    vbo: GLuint,
    
    // Audio data
    audio_data: Vec<f32>,
    bar_count: usize,
    
    // Colors
    colors: Vec<[f32; 4]>,
    
    // Dimensions
    width: i32,
    height: i32,
    
    // Configuration
    framerate: u32,
    
    // Running flag
    running: Arc<AtomicBool>,
}

impl WaylandState {
    fn new(
        connection: Connection,
        display: WlDisplay,
        queue_handle: QueueHandle<Self>,
        bar_count: usize,
        framerate: u32,
        colors: Vec<[f32; 4]>,
        running: Arc<AtomicBool>,
    ) -> Self {
        Self {
            connection,
            display,
            queue_handle,
            compositor: None,
            layer_shell: None,
            surface: None,
            layer_surface: None,
            egl_window: None,
            output: None,
            egl_display: ptr::null(),
            egl_context: ptr::null(),
            egl_surface: ptr::null(),
            egl_config: ptr::null(),
            shader_program: 0,
            vao: 0,
            vbo: 0,
            audio_data: vec![0.0; bar_count],
            bar_count,
            colors,
            width: 1920,
            height: 1080,
            framerate,
            running,
        }
    }
}

// Implement Dispatch for WaylandState to handle events
impl Dispatch<WlRegistry, ()> for WaylandState {
    fn event(
        state: &mut Self,
        _: &WlRegistry,
        _: <WlRegistry as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        // Registry events handled during setup
    }
}

impl Dispatch<WlCompositor, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlCompositor,
        _: <WlCompositor as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerShellV1, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &ZwlrLayerShellV1,
        _: <ZwlrLayerShellV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSurface, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlSurface,
        _: <WlSurface as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwlrLayerSurfaceV1, ()> for WaylandState {
    fn event(
        state: &mut Self,
        layer_surface: &ZwlrLayerSurfaceV1,
        event: <ZwlrLayerSurfaceV1 as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use zwlr_layer_surface_v1::Event;
        match event {
            Event::Configure { serial, width, height } => {
                debug!("Layer surface configured: {}x{}", width, height);
                state.width = width as i32;
                state.height = height as i32;
                
                // Acknowledge configure
                layer_surface.ack_configure(serial);
                
                // Resize EGL window if already created
                if !state.egl_window.is_none() {
                    unsafe {
                        // EGL window resize would go here if needed
                        // For now we just update viewport on next draw
                    }
                }
            }
            Event::Closed => {
                info!("Layer surface closed");
                state.running.store(false, Ordering::SeqCst);
            }
            _ => {}
        }
    }
}

impl Dispatch<WlOutput, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlOutput,
        _: <WlOutput as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlSeat,
        _: <WlSeat as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlShm, ()> for WaylandState {
    fn event(
        _: &mut Self,
        _: &WlShm,
        _: <WlShm as wayland_client::Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

// ============================================================================
// OpenGL Shaders
// ============================================================================

const VERTEX_SHADER: &str = r#"
#version 330 core
layout (location = 0) in vec2 aPos;

uniform float uBarHeights[128];
uniform int uBarCount;
uniform float uGap;
uniform vec4 uColors[128];
uniform vec2 uResolution;

out vec4 fragColor;

void main() {
    int barIndex = gl_InstanceID;
    float barHeight = uBarHeights[barIndex];
    
    // Calculate position for this bar instance
    float totalBars = float(uBarCount);
    float barWidth = (2.0 / totalBars) * (1.0 - uGap);
    float spacing = (2.0 / totalBars) * uGap;
    float xOffset = -1.0 + (float(barIndex) * (barWidth + spacing)) + spacing/2.0;
    
    // Scale aPos (which is a unit square from 0 to 1)
    vec2 pos = aPos;
    pos.x = xOffset + pos.x * barWidth;
    pos.y = -1.0 + pos.y * barHeight * 2.0;
    
    gl_Position = vec4(pos, 0.0, 1.0);
    
    // Pass color based on bar index
    fragColor = uColors[barIndex % 128];
}
"#;

const FRAGMENT_SHADER: &str = r#"
#version 330 core
in vec4 fragColor;
out vec4 FragColor;

void main() {
    FragColor = fragColor;
}
"#;

// ============================================================================
// Wayland Renderer Public Interface
// ============================================================================

pub struct WaylandRenderer {
    config: Config,
    cava_manager: CavaManager,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        info!("Creating Wayland renderer...");
        Ok(Self {
            config,
            cava_manager,
            running: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn run(mut self) -> Result<()> {
        // Check Wayland session
        if std::env::var("WAYLAND_DISPLAY").is_err()
            && std::env::var("XDG_SESSION_TYPE") != Ok("wayland".into())
        {
            return Err(anyhow!("Wayland session required for graphical rendering"));
        }
        info!("Wayland session confirmed");

        // Generate gradient colors
        let colors = if self.config.general.auto_colors {
            match WallpaperAnalyzer::generate_gradient_colors(8) {
                Ok(colors) => {
                    info!("Generated {} colors from wallpaper", colors.len());
                    colors
                }
                Err(e) => {
                    warn!("Failed to generate colors from wallpaper: {}", e);
                    info!("Using default gradient colors");
                    WallpaperAnalyzer::default_colors(8)
                }
            }
        } else {
            self.config
                .colors
                .colors
                .iter()
                .filter(|(k, _)| k.starts_with("gradient_color_"))
                .map(|(_, c)| c.to_array())
                .collect()
        };

        // Ensure we have at least one color
        let colors = if colors.is_empty() {
            WallpaperAnalyzer::default_colors(8)
        } else {
            colors
        };

        // Get cava reader
        let mut cava_reader = self.cava_manager.take_reader()?;
        let bar_count = self.config.bars.amount as usize;

        // Connect to Wayland
        let connection = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let display = connection.display();
        let mut event_queue = connection.new_event_queue();
        let qh = event_queue.handle();

        // Create state
        let mut state = WaylandState::new(
            connection.clone(),
            display.clone(),
            qh.clone(),
            bar_count,
            self.config.general.framerate,
            colors,
            self.running.clone(),
        );

        // Get registry and bind globals
        let registry = display.get_registry(&qh, ());
        event_queue.roundtrip(&mut state).context("Roundtrip failed")?;

        // Bind compositor
        state.compositor = Some(
            registry
                .bind::<WlCompositor, _, _>(&qh, 4..=5, ())
                .context("Compositor not available")?,
        );

        // Bind layer shell
        state.layer_shell = Some(
            registry
                .bind::<ZwlrLayerShellV1, _, _>(&qh, 1..=1, ())
                .context("Layer shell protocol not available. Is compositor wlroots-based?")?,
        );

        // Bind output (for size)
        state.output = registry.bind::<WlOutput, _, _>(&qh, 1..=4, ()).ok();

        event_queue.roundtrip(&mut state).context("Roundtrip after binding")?;

        // Create surface
        let surface = state
            .compositor
            .as_ref()
            .unwrap()
            .create_surface(&qh, ());
        state.surface = Some(surface.clone());

        // Create layer surface
        let layer_surface = state
            .layer_shell
            .as_ref()
            .unwrap()
            .get_layer_surface(&surface, state.output.as_ref(), &qh, ());
        layer_surface.set_layer(zwlr_layer_surface_v1::Layer::Background);
        layer_surface.set_namespace("cava-bg".to_string());
        layer_surface.set_exclusive_zone(-1); // Don't reserve space
        layer_surface.set_keyboard_interactivity(0);
        layer_surface.set_anchor(zwlr_layer_surface_v1::Anchor::all());
        state.layer_surface = Some(layer_surface);

        // Commit surface to trigger configure
        surface.commit();
        event_queue.roundtrip(&mut state).context("Roundtrip after layer surface creation")?;

        // Initialize EGL
        Self::init_egl(&mut state)?;

        // Initialize OpenGL
        Self::init_gl(&mut state)?;

        info!("✅ Wayland renderer initialized!");
        info!("🎵 Audio visualization ACTIVE");
        info!("🎨 Colors: {} gradient colors", state.colors.len());
        info!("📊 Bars: {}", bar_count);
        info!("🖥️  Window: Background layer");
        info!("⏹️  Press Ctrl+C to exit");

        // Set up signal handler
        let running_clone = self.running.clone();
        ctrlc::set_handler(move || {
            info!("Interrupt received, shutting down...");
            running_clone.store(false, Ordering::SeqCst);
        })
        .context("Failed to set signal handler")?;

        // Main loop
        let frame_duration = Duration::from_secs_f64(1.0 / state.framerate as f64);
        let mut frame_counter = 0u64;

        while state.running.load(Ordering::SeqCst) {
            // Process Wayland events
            if let Err(e) = event_queue.dispatch_pending(&mut state) {
                error!("Wayland dispatch error: {}", e);
                break;
            }

            // Read audio data
            let mut buf = vec![0u8; bar_count * 2];
            match cava_reader.read_exact(&mut buf) {
                Ok(_) => {
                    // Convert raw 16-bit to float
                    for (i, chunk) in buf.chunks_exact(2).enumerate() {
                        let value = u16::from_le_bytes([chunk[0], chunk[1]]);
                        state.audio_data[i] = value as f32 / 65530.0;
                    }
                }
                Err(e) => {
                    warn!("Failed to read audio data: {}", e);
                    // Continue with previous data or zeros
                }
            }

            // Draw frame
            Self::draw_frame(&mut state)?;

            // Swap buffers
            unsafe {
                if egl::SwapBuffers(state.egl_display, state.egl_surface) == egl::FALSE {
                    let err = egl::GetError();
                    error!("eglSwapBuffers failed: {:?}", err);
                }
            }

            // Commit surface
            if let Some(surface) = &state.surface {
                surface.commit();
            }

            // Flush Wayland connection
            if let Err(e) = event_queue.flush() {
                error!("Failed to flush Wayland events: {}", e);
            }

            // Log occasionally
            frame_counter += 1;
            if frame_counter % (state.framerate as u64 * 5) == 0 {
                let max_val = state.audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                info!("Frame {}, max audio: {:.3}", frame_counter, max_val);
            }

            // Frame rate limiting
            std::thread::sleep(frame_duration);
        }

        // Cleanup
        Self::cleanup_gl(&mut state);
        Self::cleanup_egl(&mut state);
        info!("Wayland renderer stopped");
        Ok(())
    }

    pub fn stop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        info!("Wayland renderer stopping...");
    }

    // ========================================================================
    // EGL Initialization
    // ========================================================================

    fn init_egl(state: &mut WaylandState) -> Result<()> {
        unsafe {
            // Get EGL display
            let egl_display = egl::GetPlatformDisplay(
                egl::PLATFORM_WAYLAND_KHR,
                state.display.id().as_ptr() as *mut _,
                ptr::null(),
            );
            if egl_display == egl::NO_DISPLAY {
                return Err(anyhow!("Failed to get EGL display"));
            }
            state.egl_display = egl_display;

            // Initialize EGL
            let mut major = 0;
            let mut minor = 0;
            if egl::Initialize(egl_display, &mut major, &mut minor) == egl::FALSE {
                let err = egl::GetError();
                return Err(anyhow!("Failed to initialize EGL: {:?}", err));
            }
            info!("EGL initialized: {}.{}", major, minor);

            // Choose config
            let config_attribs = [
                egl::SURFACE_TYPE,
                egl::WINDOW_BIT,
                egl::RED_SIZE,
                8,
                egl::GREEN_SIZE,
                8,
                egl::BLUE_SIZE,
                8,
                egl::ALPHA_SIZE,
                8,
                egl::RENDERABLE_TYPE,
                egl::OPENGL_BIT,
                egl::NONE,
            ];

            let mut configs = [ptr::null(); 1];
            let mut num_configs = 0;
            if egl::ChooseConfig(
                egl_display,
                config_attribs.as_ptr(),
                configs.as_mut_ptr(),
                1,
                &mut num_configs,
            ) == egl::FALSE
                || num_configs == 0
            {
                return Err(anyhow!("Failed to choose EGL config"));
            }
            state.egl_config = configs[0];

            // Create EGL window from Wayland surface
            let surface = state.surface.as_ref().unwrap();
            let egl_window = wayland_egl::WlEglWindow::new(surface, state.width, state.height)?;
            state.egl_window = Some(egl_window.clone());

            // Create EGL surface
            let egl_surface = egl::CreatePlatformWindowSurface(
                egl_display,
                state.egl_config,
                egl_window.ptr() as *mut _,
                ptr::null(),
            );
            if egl_surface == egl::NO_SURFACE {
                let err = egl::GetError();
                return Err(anyhow!("Failed to create EGL surface: {:?}", err));
            }
            state.egl_surface = egl_surface;

            // Create EGL context
            let context_attribs = [
                egl::CONTEXT_MAJOR_VERSION,
                3,
                egl::CONTEXT_MINOR_VERSION,
                3,
                egl::CONTEXT_OPENGL_PROFILE_MASK,
                egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
                egl::NONE,
            ];

            let egl_context = egl::CreateContext(
                egl_display,
                state.egl_config,
                egl::NO_CONTEXT,
                context_attribs.as_ptr(),
            );
            if egl_context == egl::NO_CONTEXT {
                let err = egl::GetError();
                return Err(anyhow!("Failed to create EGL context: {:?}", err));
            }
            state.egl_context = egl_context;

            // Make current
            if egl::MakeCurrent(
                egl_display,
                egl_surface,
                egl_surface,
                egl_context,
            ) == egl::FALSE
            {
                let err = egl::GetError();
                return Err(anyhow!("Failed to make EGL context current: {:?}", err));
            }

            // Load OpenGL functions
            gl::load_with(|name| {
                let cname = CString::new(name).unwrap();
                egl::GetProcAddress(cname.as_ptr()) as *const _
            });

            Ok(())
        }
    }

    // ========================================================================
    // OpenGL Initialization
    // ========================================================================

    fn init_gl(state: &mut WaylandState) -> Result<()> {
        unsafe {
            // Compile shaders
            let vertex_shader = Self::compile_shader(VERTEX_SHADER, gl::VERTEX_SHADER)?;
            let fragment_shader = Self::compile_shader(FRAGMENT_SHADER, gl::FRAGMENT_SHADER)?;

            // Link program
            let program = gl::CreateProgram();
            gl::AttachShader(program, vertex_shader);
            gl::AttachShader(program, fragment_shader);
            gl::LinkProgram(program);

            let mut success = 0;
            gl::GetProgramiv(program, gl::LINK_STATUS, &mut success);
            if success == 0 {
                let mut len = 0;
                gl::GetProgramiv(program, gl::INFO_LOG_LENGTH, &mut len);
                let mut buf = vec![0u8; len as usize];
                gl::GetProgramInfoLog(
                    program,
                    len,
                    ptr::null_mut(),
                    buf.as_mut_ptr() as *mut _,
                );
                let log = String::from_utf8_lossy(&buf);
                return Err(anyhow!("Shader link failed: {}", log));
            }

            gl::DeleteShader(vertex_shader);
            gl::DeleteShader(fragment_shader);
            state.shader_program = program;

            // Create VAO
            let mut vao = 0;
            gl::GenVertexArrays(1, &mut vao);
            gl::BindVertexArray(vao);
            state.vao = vao;

            // Create VBO for unit square
            let vertices: [f32; 8] = [
                0.0, 0.0, // bottom-left
                1.0, 0.0, // bottom-right
                1.0, 1.0, // top-right
                0.0, 1.0, // top-left
            ];

            let indices: [u32; 6] = [0, 1, 2, 0, 2, 3];

            let mut vbo = 0;
            gl::GenBuffers(1, &mut vbo);
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as isize,
                vertices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );

            let mut ebo = 0;
            gl::GenBuffers(1, &mut ebo);
            gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
            gl::BufferData(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * std::mem::size_of::<u32>()) as isize,
                indices.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );

            gl::VertexAttribPointer(
                0,
                2,
                gl::FLOAT,
                gl::FALSE,
                (2 * std::mem::size_of::<f32>()) as i32,
                ptr::null(),
            );
            gl::EnableVertexAttribArray(0);

            state.vbo = vbo;

            // Set clear color to transparent
            gl::ClearColor(0.0, 0.0, 0.0, 0.0);
            gl::Enable(gl::BLEND);
            gl::BlendFunc(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA);

            Ok(())
        }
    }

    unsafe fn compile_shader(src: &str, shader_type: GLuint) -> Result<GLuint> {
        let shader = gl::CreateShader(shader_type);
        let c_str = CString::new(src).unwrap();
        gl::ShaderSource(shader, 1, &c_str.as_ptr(), ptr::null());
        gl::CompileShader(shader);

        let mut success = 0;
        gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut success);
        if success == 0 {
            let mut len = 0;
            gl::GetShaderiv(shader, gl::INFO_LOG_LENGTH, &mut len);
            let mut buf = vec![0u8; len as usize];
            gl::GetShaderInfoLog(shader, len, ptr::null_mut(), buf.as_mut_ptr() as *mut _);
            let log = String::from_utf8_lossy(&buf);
            return Err(anyhow!("Shader compilation failed: {}", log));
        }

        Ok(shader)
    }

    // ========================================================================
    // Drawing
    // ========================================================================

    fn draw_frame(state: &mut WaylandState) -> Result<()> {
        unsafe {
            // Set viewport
            gl::Viewport(0, 0, state.width, state.height);
            gl::Clear(gl::COLOR_BUFFER_BIT);

            // Use shader
            gl::UseProgram(state.shader_program);

            // Set uniforms
            let bar_count_loc = gl::GetUniformLocation(
                state.shader_program,
                CString::new("uBarCount").unwrap().as_ptr(),
            );
            gl::Uniform1i(bar_count_loc, state.bar_count as i32);

            let gap_loc = gl::GetUniformLocation(
                state.shader_program,
                CString::new("uGap").unwrap().as_ptr(),
            );
            gl::Uniform1f(gap_loc, 0.1); // Use config later

            // Prepare bar heights array
            let heights_loc = gl::GetUniformLocation(
                state.shader_program,
                CString::new("uBarHeights").unwrap().as_ptr(),
            );
            gl::Uniform1fv(heights_loc, state.bar_count as i32, state.audio_data.as_ptr());

            // Prepare colors array
            let mut color_array = vec![0.0f32; state.colors.len() * 4];
            for (i, color) in state.colors.iter().enumerate() {
                color_array[i * 4] = color[0];
                color_array[i * 4 + 1] = color[1];
                color_array[i * 4 + 2] = color[2];
                color_array[i * 4 + 3] = color[3];
            }
            let colors_loc = gl::GetUniformLocation(
                state.shader_program,
                CString::new("uColors").unwrap().as_ptr(),
            );
            gl::Uniform4fv(colors_loc, state.colors.len() as i32, color_array.as_ptr());

            // Set resolution uniform
            let res_loc = gl::GetUniformLocation(
                state.shader_program,
                CString::new("uResolution").unwrap().as_ptr(),
            );
            gl::Uniform2f(res_loc, state.width as f32, state.height as f32);

            // Draw instanced
            gl::BindVertexArray(state.vao);
            gl::DrawElementsInstanced(
                gl::TRIANGLES,
                6,
                gl::UNSIGNED_INT,
                ptr::null(),
                state.bar_count as i32,
            );
            gl::BindVertexArray(0);
        }

        Ok(())
    }

    // ========================================================================
    // Cleanup
    // ========================================================================

    fn cleanup_gl(state: &mut WaylandState) {
        unsafe {
            if state.shader_program != 0 {
                gl::DeleteProgram(state.shader_program);
            }
            if state.vao != 0 {
                gl::DeleteVertexArrays(1, &state.vao);
            }
            if state.vbo != 0 {
                gl::DeleteBuffers(1, &state.vbo);
            }
        }
    }

    fn cleanup_egl(state: &mut WaylandState) {
        unsafe {
            if state.egl_display != ptr::null() {
                egl::MakeCurrent(
                    state.egl_display,
                    egl::NO_SURFACE,
                    egl::NO_SURFACE,
                    egl::NO_CONTEXT,
                );
                if state.egl_context != ptr::null() {
                    egl::DestroyContext(state.egl_display, state.egl_context);
                }
                if state.egl_surface != ptr::null() {
                    egl::DestroySurface(state.egl_display, state.egl_surface);
                }
                egl::Terminate(state.egl_display);
            }
            // EGL window will be dropped automatically
        }
    }
}

impl Drop for WaylandRenderer {
    fn drop(&mut self) {
        self.stop();
    }
}