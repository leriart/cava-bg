use anyhow::{Context, Result};
use gl::types::{GLsizei, GLsizeiptr};
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
use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::{BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_surface},
    Connection, QueueHandle,
};
use wayland_egl::WlEglSurface;

extern crate khronos_egl as egl;

mod config;
use config::*;

mod shader;
use shader::*;

const VERTEX_SHADER_SRC: &str = include_str!("shaders/vertex_shader.glsl");
const FRAGMENT_SHADER_SRC: &str = include_str!("shaders/fragment_shader.glsl");

fn main() -> Result<()> {
    env_logger::init();
    
    info!("Starting Cavabg - Hyprland native CAVA visualizer");
    
    // Load configuration
    let config = Config::load()?;
    info!("Configuration loaded: {} bars, {} fps", config.bars.amount, config.general.framerate);
    
    // Start cava process
    let cava_process = start_cava(&config)?;
    let cava_reader = BufReader::new(cava_process.stdout.unwrap());
    
    // Setup Wayland connection
    let conn = Connection::connect_to_env()
        .context("Failed to connect to Wayland display")?;
    
    let (globals, event_queue) = registry_queue_init(&conn)
        .context("Failed to initialize Wayland registry")?;
    
    let qh = event_queue.handle();
    let mut event_loop: EventLoop<AppState> = EventLoop::try_new()
        .context("Failed to initialize event loop")?;
    
    let loop_handle = event_loop.handle();
    WaylandSource::new(conn.clone(), event_queue)
        .insert(loop_handle)
        .context("Failed to insert Wayland source into event loop")?;
    
    let frame_duration = Duration::from_secs(1) / config.general.framerate;
    
    // Create compositor and surface
    let compositor = CompositorState::bind(&globals, &qh)
        .context("wl_compositor not available")?;
    
    let surface = compositor.create_surface(&qh);
    
    // Create layer shell surface
    let layer_shell = LayerShell::bind(&globals, &qh)
        .context("layer shell not available")?;
    
    let layer_surface = layer_shell.create_layer_surface(
        &qh,
        surface.clone(),
        Layer::Bottom,
        Some("cavabg"),
        None,
    );
    
    // Set initial size and anchor
    layer_surface.set_size(256, 256);
    layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
    surface.commit();
    
    // Initialize EGL and OpenGL
    let (egl_display, egl_config, egl_context, egl_surface, wl_egl_surface) = 
        init_egl(&conn, &surface)?;
    
    // Load OpenGL functions
    gl::load_with(|name| egl.get_proc_address(name).unwrap() as *const std::ffi::c_void);
    
    let version = unsafe {
        let data = gl::GetString(gl::VERSION) as *const i8;
        CString::from_raw(data as *mut _).into_string().unwrap()
    };
    
    info!("OpenGL version: {}", version);
    info!("EGL version: {}", egl.version());
    
    // Create shader program
    let shader_program = create_shader_program(VERTEX_SHADER_SRC, FRAGMENT_SHADER_SRC)?;
    
    // Get uniform location
    let window_size_string = CString::new("WindowSize").unwrap();
    let windows_size_location = unsafe {
        gl::GetUniformLocation(shader_program, window_size_string.as_ptr())
    };
    
    // Setup vertex buffers and gradient colors
    let (vao, vbo, ebo, gradient_colors_ssbo) = setup_buffers(&config)?;
    
    // Create application state
    let mut app_state = AppState {
        registry_state: RegistryState::new(&globals),
        output_state: OutputState::new(&globals, &qh),
        width: 256,
        height: 256,
        layer_shell,
        layer_surface,
        surface,
        cava_reader,
        wl_egl_surface,
        egl_surface,
        egl_config,
        egl_context,
        egl_display,
        shader_program,
        vao,
        vbo,
        windows_size_location,
        bar_count: config.bars.amount,
        bar_gap: config.bars.gap,
        background_color: config.general.background_color.to_array(),
        preferred_output_name: config.general.preferred_output.clone(),
        compositor,
        config,
    };
    
    // Run event loop
    info!("Entering main event loop");
    event_loop
        .run(frame_duration, &mut app_state, |_| {})
        .context("Event loop failed")?;
    
    Ok(())
}

fn start_cava(config: &Config) -> Result<std::process::Child> {
    let cava_output_config: HashMap<String, String> = HashMap::from([
        ("method".into(), "raw".into()),
        ("raw_target".into(), "/dev/stdout".into()),
        ("bit_format".into(), "16bit".into()),
    ]);
    
    let cava_config = CavaConfig {
        general: CavaGeneralConfig {
            framerate: config.general.framerate,
            bars: config.bars.amount,
            autosens: config.general.autosens,
            sensitivity: config.general.sensitivity,
        },
        smoothing: CavaSmoothingConfig {
            monstercat: config.smoothing.monstercat,
            waves: config.smoothing.waves,
            noise_reduction: config.smoothing.noise_reduction,
        },
        output: cava_output_config,
    };
    
    let string_cava_config = toml::to_string(&cava_config)
        .context("Failed to serialize CAVA config")?;
    
    let mut cmd = Command::new("cava");
    cmd.arg("-p").arg("/dev/stdin");
    
    let mut cava_process = cmd
        .stdout(Stdio::piped())
        .stdin(Stdio::piped())
        .spawn()
        .context("Failed to spawn cava process. Make sure cava is installed.")?;
    
    let mut cava_stdin = cava_process.stdin.take()
        .context("Failed to get cava stdin")?;
    
    cava_stdin.write_all(string_cava_config.as_bytes())
        .context("Failed to write config to cava")?;
    
    drop(cava_stdin);
    
    Ok(cava_process)
}

fn init_egl(
    conn: &Connection,
    surface: &WlSurface,
) -> Result<(
    egl::Display,
    egl::Config,
    egl::Context,
    egl::Surface,
    WlEglSurface,
)> {
    egl.bind_api(egl::OPENGL_API)
        .context("Failed to bind OpenGL API")?;
    
    let egl_display = unsafe {
        egl.get_display(conn.display().id().as_ptr() as *mut std::ffi::c_void)
            .context("Failed to get EGL display")?
    };
    
    egl.initialize(egl_display)
        .context("Failed to initialize EGL")?;
    
    const ATTRIBUTES: [i32; 9] = [
        egl::RED_SIZE, 8,
        egl::GREEN_SIZE, 8,
        egl::BLUE_SIZE, 8,
        egl::ALPHA_SIZE, 8,
        egl::NONE,
    ];
    
    let egl_config = egl
        .choose_first_config(egl_display, &ATTRIBUTES)
        .context("Failed to choose EGL config")?
        .context("No suitable EGL config found")?;
    
    const CONTEXT_ATTRIBUTES: [i32; 7] = [
        egl::CONTEXT_MAJOR_VERSION, 4,
        egl::CONTEXT_MINOR_VERSION, 6,
        egl::CONTEXT_OPENGL_PROFILE_MASK,
        egl::CONTEXT_OPENGL_CORE_PROFILE_BIT,
        egl::NONE,
    ];
    
    let egl_context = egl
        .create_context(egl_display, egl_config, None, &CONTEXT_ATTRIBUTES)
        .context("Failed to create EGL context")?;
    
    let wl_egl_surface = WlEglSurface::new(surface.id(), 256, 256)
        .context("Failed to create Wayland EGL surface")?;
    
    let egl_surface = unsafe {
        egl.create_window_surface(
            egl_display,
            egl_config,
            wl_egl_surface.ptr() as egl::NativeWindowType,
            None,
        )
        .context("Failed to create EGL window surface")?
    };
    
    egl.make_current(
        egl_display,
        Some(egl_surface),
        Some(egl_surface),
        Some(egl_context),
    )
    .context("Failed to make EGL context current")?;
    
    Ok((egl_display, egl_config, egl_context, egl_surface, wl_egl_surface))
}

fn setup_buffers(config: &Config) -> Result<(u32, u32, u32, u32)> {
    let mut vbo = 0;
    let mut vao = 0;
    let mut ebo = 0;
    let mut gradient_colors_ssbo = 0;
    
    // Convert gradient colors to RGBA arrays
    let gradient_colors_rgba: Vec<[f32; 4]> = config
        .colors
        .colors
        .values()
        .map(|color| color.to_array())
        .collect();
    
    let gradient_colors_size = gradient_colors_rgba.len() as i32;
    
    // Prepare buffer data with proper alignment
    let mut buffer_data: Vec<u8> = gradient_colors_size.to_le_bytes().to_vec();
    buffer_data.extend([0, 0, 0, 0].repeat(3)); // Padding for vec4 alignment
    
    for color in gradient_colors_rgba.iter() {
        for color_value in color {
            buffer_data.extend_from_slice(&color_value.to_le_bytes());
        }
    }
    
    // Create indices for bars (6 indices per bar = 2 triangles)
    let mut indices: Vec<u16> = vec![0; config.bars.amount as usize * 6];
    for i in 0..config.bars.amount as usize {
        indices[i * 6] = i as u16 * 4;
        indices[i * 6 + 1] = i as u16 * 4 + 1;
        indices[i * 6 + 2] = i as u16 * 4 + 2;
        indices[i * 6 + 3] = i as u16 * 4 + 1;
        indices[i * 6 + 4] = i as u16 * 4 + 2;
        indices[i * 6 + 5] = i as u16 * 4 + 3;
    }
    
    unsafe {
        gl::GenVertexArrays(1, &mut vao);
        gl::BindVertexArray(vao);
        
        gl::GenBuffers(1, &mut vbo);
        gl::GenBuffers(1, &mut ebo);
        gl::GenBuffers(1, &mut gradient_colors_ssbo);
        
        // Setup element buffer
        gl::BindBuffer(gl::ELEMENT_ARRAY_BUFFER, ebo);
        gl::BufferData(
            gl::ELEMENT_ARRAY_BUFFER,
            (indices.len() * std::mem::size_of::<u16>()) as GLsizeiptr,
            indices.as_ptr() as *const std::ffi::c_void,
            gl::STATIC_DRAW,
        );
        
        // Setup gradient colors SSBO
        gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, gradient_colors_ssbo);
        gl::BufferData(
            gl::SHADER_STORAGE_BUFFER,
            buffer_data.len() as GLsizeiptr,
            buffer_data.as_ptr() as *const std::ffi::c_void,
            gl::STATIC_DRAW,
        );
        gl::BindBufferBase(gl::SHADER_STORAGE_BUFFER, 0, gradient_colors_ssbo);
        gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
        
        // Setup vertex attribute
        gl::VertexAttribPointer(
            0,
            2,
            gl::FLOAT,
            gl::FALSE,
            (2 * std::mem::size_of::<f32>()) as gl::types::GLsizei,
            std::ptr::null(),
        );
        gl::EnableVertexAttribArray(0);
        
        gl::BindVertexArray(0);
    }
    
    Ok((vao, vbo, ebo, gradient_colors_ssbo))
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    width: u32,
    height: u32,
    layer_shell: LayerShell,
    layer_surface: LayerSurface,
    surface: WlSurface,
    cava_reader: BufReader<std::process::ChildStdout>,
    wl_egl_surface: WlEglSurface,
    egl_surface: egl::Surface,
    egl_config: egl::Config,
    egl_context: egl::Context,
    egl_display: egl::Display,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    windows_size_location: i32,
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    preferred_output_name: Option<String>,
    compositor: CompositorState,
    config: Config,
}

impl AppState {
    fn draw(&mut self, conn: &Connection, qh: &QueueHandle<Self>) {
        // Read CAVA data
        let mut cava_buffer = vec![0; self.bar_count as usize * 2];
        if let Err(e) = self.cava_reader.read_exact(&mut cava_buffer) {
            error!("Failed to read from cava: {}", e);
            return;
        }
        
        // Unpack 16-bit values to normalized floats
        let mut unpacked_data = vec![0.0; self.bar_count as usize];
        for (unpacked_data_index, i) in (0..cava_buffer.len()).step_by(2).enumerate() {
            let num = u16::from_le_bytes([cava_buffer[i], cava_buffer[i + 1]]);
            unpacked_data[unpacked_data_index] = (num as f32) / 65530.0;
        }
        
        // Calculate bar dimensions
        let bar_width: f32 =
            2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width: f32 = bar_width * self.bar_gap;
        
        // Generate vertices for all bars
        let mut vertices: Vec<f32> = vec![0.0; self.bar_count as usize * 8];
        
        for i in 0..self.bar_count as usize {
            let bar_height: f32 = 2.0 * unpacked_data[i] - 1.0;
            
            // Top-left
            vertices[i * 8] = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
            vertices[i * 8 + 1] = bar_height;
            
            // Top-right
            vertices[i * 8 + 2] = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
            vertices[i * 8 + 3] = bar_height;
            
            // Bottom-left
            vertices[i * 8 + 4] = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
            vertices[i * 8 + 5] = -1.0;
            
            // Bottom-right
            vertices[i * 8 + 6] = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
            vertices[i * 8 + 7] = -1