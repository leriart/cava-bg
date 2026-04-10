//! Actual Wayland/OpenGL renderer implementation inspired by wallpaper-cava

use anyhow::{Context, Result};
use gl::types::{GLsizei, GLsizeiptr};
use log::{error, info};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerSurface,
};
use smithay_client_toolkit::{
    compositor::CompositorState,
    output::OutputState,
    registry::RegistryState,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{
    globals::registry_queue_init,
    Connection, QueueHandle,
};
use wayland_egl::WlEglSurface;

use core::ffi;
use egl::API as egl;
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::config::Config;
use crate::cava_manager::CavaManager;

// Shader sources (from wallpaper-cava)
const VERTEX_SHADER_SRC: &str = r#"#version 430 core
in vec2 position;
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

/// Actual Wayland/OpenGL renderer
pub struct ActualRenderer {
    running: Arc<AtomicBool>,
    config: Config,
    cava_manager: Option<CavaManager>,
}

impl ActualRenderer {
    /// Create a new actual renderer
    pub fn new(config: Config, cava_manager: CavaManager) -> Result<Self> {
        Ok(Self {
            running: Arc::new(AtomicBool::new(true)),
            config,
            cava_manager: Some(cava_manager),
        })
    }
    
    /// Run the actual renderer
    pub fn run(&mut self) -> Result<()> {
        info!("Starting actual Wayland/OpenGL renderer...");
        
        match self.initialize_and_run() {
            Ok(_) => {
                info!("Renderer completed successfully");
                Ok(())
            }
            Err(e) => {
                error!("Renderer failed: {}", e);
                Err(e)
            }
        }
    }
    
    /// Initialize and run the render loop
    fn initialize_and_run(&mut self) -> Result<()> {
        // Connect to Wayland
        let conn = Connection::connect_to_env()
            .context("Failed to connect to Wayland display")?;
        
        let (globals, event_queue) = registry_queue_init(&conn)
            .context("Failed to initialize Wayland registry")?;
        
        let qh = event_queue.handle();
        let mut event_loop: EventLoop<()> = EventLoop::try_new()
            .context("Failed to initialize event loop")?;
        
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .context("Failed to insert Wayland source")?;
        
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
            Layer::Background,
            Some("cava-bg"),
            None,
        );
        
        // Set layer surface properties (cover entire screen)
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        layer_surface.set_size(256, 256); // Initial size
        surface.commit();
        
        info!("Wayland layer surface created");
        
        // Initialize EGL
        egl.bind_api(egl::OPENGL_API)
            .context("Failed to bind OpenGL API")?;
        
        let egl_display = unsafe {
            egl.get_display(conn.display().id().as_ptr() as *mut std::ffi::c_void)
                .context("Failed to get EGL display")?
        };
        
        egl.initialize(egl_display)
            .context("Failed to initialize EGL")?;
        
        // EGL configuration
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
        
        // Create EGL context
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
        
        // Create EGL surface
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
        
        // Make context current
        egl.make_current(
            egl_display,
            Some(egl_surface),
            Some(egl_surface),
            Some(egl_context),
        )
        .context("Failed to make EGL context current")?;
        
        // Load OpenGL functions
        gl::load_with(|name| egl.get_proc_address(name).unwrap() as *const std::ffi::c_void);
        
        // Get OpenGL version
        let version = unsafe {
            let data = gl::GetString(gl::VERSION) as *const i8;
            CString::from_raw(data as *mut _).into_string().unwrap()
        };
        
        info!("OpenGL version: {}", version);
        info!("EGL version: {}", egl.version());
        
        // Compile shaders
        let shader_program = self.compile_shaders()
            .context("Failed to compile shaders")?;
        
        // Set up OpenGL state
        let (vao, vbo, ebo, gradient_ssbo) = self.setup_opengl_buffers(shader_program)
            .context("Failed to setup OpenGL buffers")?;
        
        info!("OpenGL setup complete");
        
        // Get cava manager
        let cava_manager = self.cava_manager.take()
            .context("Cava manager not available")?;
        
        // Simple render loop (simplified version)
        info!("Starting render loop...");
        self.simple_render_loop(
            shader_program,
            vao,
            vbo,
            ebo,
            gradient_ssbo,
            cava_manager,
        )?;
        
        // Cleanup
        unsafe {
            gl::DeleteProgram(shader_program);
            gl::DeleteVertexArrays(1, &vao);
            gl::DeleteBuffers(1, &vbo);
            gl::DeleteBuffers(1, &ebo);
            gl::DeleteBuffers(1, &gradient_ssbo);
        }
        
        egl.destroy_surface(egl_display, egl_surface)
            .context("Failed to destroy EGL surface")?;
        egl.destroy_context(egl_display, egl_context)
            .context("Failed to destroy EGL context")?;
        egl.terminate(egl_display)
            .context("Failed to terminate EGL")?;
        
        info!("Renderer cleanup complete");
        Ok(())
    }
    
    /// Compile vertex and fragment shaders
    fn compile_shaders(&self) -> Result<u32> {
        // Compile vertex shader
        let vert_shader_source = CString::new(VERTEX_SHADER_SRC)
            .context("Failed to create CString for vertex shader")?;
        
        let vert_shader = unsafe { gl::CreateShader(gl::VERTEX_SHADER) };
        unsafe {
            gl::ShaderSource(
                vert_shader,
                1,
                &vert_shader_source.as_ptr(),
                std::ptr::null(),
            );
            gl::CompileShader(vert_shader);
            
            // Check compilation status
            let mut status = gl::FALSE as gl::types::GLint;
            gl::GetShaderiv(vert_shader, gl::COMPILE_STATUS, &mut status);
            if status != 1 {
                let mut error_log_size: gl::types::GLint = 0;
                gl::GetShaderiv(vert_shader, gl::INFO_LOG_LENGTH, &mut error_log_size);
                let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
                gl::GetShaderInfoLog(
                    vert_shader,
                    error_log_size,
                    &mut error_log_size,
                    error_log.as_mut_ptr() as *mut _,
                );
                error_log.set_len(error_log_size as usize);
                let log = String::from_utf8(error_log)
                    .context("Failed to parse shader error log")?;
                return Err(anyhow::anyhow!("Vertex shader compilation failed: {}", log));
            }
        }
        
        // Compile fragment shader
        let frag_shader_source = CString::new(FRAGMENT_SHADER_SRC)
            .context("Failed to create CString for fragment shader")?;
        
        let frag_shader = unsafe { gl::CreateShader(gl::FRAGMENT_SHADER) };
        unsafe {
            gl::ShaderSource(
                frag_shader,
                1,
                &frag_shader_source.as_ptr(),
                std::ptr::null(),
            );
            gl::CompileShader(frag_shader);
            
            // Check compilation status
            let mut status = gl::FALSE as gl::types::GLint;
            gl::GetShaderiv(frag_shader, gl::COMPILE_STATUS, &mut status);
            if status != 1 {
                let mut error_log_size: gl::types::GLint = 0;
                gl::GetShaderiv(frag_shader, gl::INFO_LOG_LENGTH, &mut error_log_size);
                let mut error_log: Vec<u8> = Vec::with_capacity(error_log_size as usize);
                gl::GetShaderInfoLog(
                    frag_shader,
                    error_log_size,
                    &mut error_log_size,
                    error_log.as_mut_ptr() as *mut _,
                );
                error_log.set_len(error_log_size as usize);
                let log = String::from_utf8(error_log)
                    .context("Failed to parse shader error log")?;
                return Err(anyhow::anyhow!("Fragment shader compilation failed: {}", log));
            }
        }
        
        // Link shader program
        let shader_program = unsafe { gl::CreateProgram() };
        unsafe {
            gl::AttachShader(shader_program, vert_shader);
            gl::AttachShader(shader_program, frag_shader);
            gl::LinkProgram(shader_program);
            
            // Check linking status
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
                let log = String::from_utf8(error_log)
                    .context("Failed to parse program error log")?;
                return Err(anyhow::anyhow!("Shader program linking failed: {}", log));
            }
            
            // Clean up shaders
            gl::DeleteShader(vert_shader);
            gl::DeleteShader(frag_shader);
        }
        
        Ok(shader_program)
    }
    
    /// Set up OpenGL buffers (VAO, VBO, EBO, SSBO)
    fn setup_opengl_buffers(&self, shader_program: u32) -> Result<(u32, u32, u32, u32)> {
        let mut vao = 0;
        let mut vbo = 0;
        let mut ebo = 0;
        let mut gradient_ssbo = 0;
        
        unsafe {
            // Generate buffers
            gl::GenVertexArrays(1, &mut vao);
            gl::GenBuffers(1, &mut vbo);
            gl::GenBuffers(1, &mut ebo);
            gl::GenBuffers(1, &mut gradient_ssbo);
            
            // Bind VAO
            gl::BindVertexArray(vao);
            
            // Bind VBO
            gl::BindBuffer(gl::ARRAY_BUFFER, vbo);
            
            // Set up vertex attribute
            gl::VertexAttribPointer(
                0, // location
                2, // size (x, y)
                gl::FLOAT,
                gl::FALSE,
                (2 * std::mem::size_of::<f32>()) as gl::types::GLsizei,
                std::ptr::null(),
            );
            gl::EnableVertexAttribArray(0);
            
            // Unbind
            gl::BindVertexArray(0);
            gl::BindBuffer(gl::ARRAY_BUFFER, 0);
        }
        
        Ok((vao, vbo, ebo, gradient_ssbo))
    }
    
    /// Simple render loop (simplified for now)
    fn simple_render_loop(
        &self,
        shader_program: u32,
        vao: u32,
        _vbo: u32,
        _ebo: u32,
        _gradient_ssbo: u32,
        mut cava_manager: CavaManager,
    ) -> Result<()> {
        info!("Renderer ready - waiting for audio...");
        
        let mut frame_count = 0;
        
        while self.running.load(Ordering::SeqCst) {
            frame_count += 1;
            
            // Try to read audio data
            match cava_manager.read_audio_data() {
                Ok(Some(audio_data)) if !audio_data.is_empty() => {
                    // Calculate some stats
                    let max = audio_data.iter().fold(0.0f32, |a, &b| a.max(b));
                    
                    if frame_count % 60 == 0 { // Log every second at 60 FPS
                        info!("Rendering frame {} - Audio level: {:.3}", frame_count, max);
                    }
                    
                    // Here we would update vertex data based on audio
                    // and actually render with OpenGL
                }
                Ok(None) => {
                    // No data yet
                }
                Err(e) => {
                    error!("Error reading audio: {}", e);
                }
                _ => {}
            }
            
            // Simple OpenGL rendering
            unsafe {
                gl::ClearColor(0.0, 0.0, 0.0, 0.0); // Transparent black
                gl::Clear(gl::COLOR_BUFFER_BIT);
                
                gl::UseProgram(shader_program);
                gl::BindVertexArray(vao);
                
                // Draw something simple
                gl::DrawArrays(gl::TRIANGLES, 0, 3);
                
                gl::BindVertexArray(0);
                gl::UseProgram(0);
            }
            
            // Swap buffers (would need EGL surface swap)
            // egl.swap_buffers(...)
            
            // Sleep to prevent busy loop
            std::thread::sleep(Duration::from_millis(16)); // ~60 FPS
        }
        
        info!("Render loop stopped after {}