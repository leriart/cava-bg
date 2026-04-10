// Versión integrada de cava-bg que combina:
// 1. Renderizado Wayland/OpenGL de wallpaper-cava
// 2. Detección de colores adaptativos de cava-bg
// 3. Detección de cambios de fondo de cava-bg

use anyhow::{Context, Result};
use clap::Parser;
use log::{error, info, warn};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::shell::wlr_layer::{Anchor, Layer, LayerShell, LayerSurface};
use smithay_client_toolkit::compositor::CompositorState;
use smithay_client_toolkit::output::{OutputHandler, OutputState};
use smithay_client_toolkit::registry::{ProvidesRegistryState, RegistryState};
use wayland_client::protocol::wl_output;
use wayland_client::{Connection, Proxy, QueueHandle};
use wayland_egl::WlEglSurface;
use khronos_egl as egl;

use std::collections::HashMap;
use std::ffi::CString;
use std::fs;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio, ChildStdout};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

mod cli;
mod config;
mod wallpaper;

use cli::*;
use config::*;
use wallpaper::WallpaperAnalyzer;

static RUNNING: AtomicBool = AtomicBool::new(true);

// Shaders (mismos que wallpaper-cava)
const VERTEX_SHADER_SRC: &str = include_str!("shaders/vertex_shader.glsl");
const FRAGMENT_SHADER_SRC: &str = include_str!("shaders/fragment_shader.glsl");

fn handle_signal() -> Arc<AtomicBool> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    ctrlc::set_handler(move || {
        r.store(false, Ordering::SeqCst);
        RUNNING.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl-C handler");

    running
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    width: u32,
    height: u32,
    layer_shell: LayerShell,
    layer_surface: LayerSurface,
    surface: wayland_client::protocol::wl_surface::WlSurface,
    cava_reader: Option<BufReader<ChildStdout>>,
    cava_process: Option<std::process::Child>,
    wl_egl_surface: WlEglSurface,
    egl_surface: egl::Surface,
    egl_config: egl::Config,
    egl_context: egl::Context,
    egl_display: egl::Display,
    shader_program: u32,
    vao: u32,
    vbo: u32,
    ebo: u32,
    gradient_colors_ssbo: u32,
    windows_size_location: i32,
    bar_count: u32,
    bar_gap: f32,
    background_color: [f32; 4],
    preferred_output_name: Option<String>,
    compositor: CompositorState,
    config: Config,
    current_wallpaper_path: Option<PathBuf>,
    last_wallpaper_check: Instant,
}

impl AppState {
    fn draw(&mut self, conn: &Connection, qh: &QueueHandle<Self>) {
        // Primero, verificar si hay cambios en el fondo de pantalla
        let now = Instant::now();
        if now.duration_since(self.last_wallpaper_check) >= Duration::from_secs(5) {
            self.check_wallpaper_changes();
            self.last_wallpaper_check = now;
        }
        
        // Leer datos de audio de cava
        if let Some(reader) = &mut self.cava_reader {
            let mut cava_buffer = vec![0u8; self.bar_count as usize * 2];
            if reader.read_exact(&mut cava_buffer).is_err() {
                // Error al leer, posiblemente cava se cerró
                self.restart_cava();
                return;
            }
            
            // Convertir datos raw de 16-bit a valores normalizados
            let mut unpacked_data = vec![0.0f32; self.bar_count as usize];
            for (i, chunk) in cava_buffer.chunks_exact(2).enumerate() {
                let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                unpacked_data[i] = (num as f32) / 65530.0;
            }
            
            // Renderizar barras
            self.render_bars(&unpacked_data);
        }
        
        // Swap buffers
        egl::API.swap_buffers(self.egl_display, self.egl_surface).unwrap();
        
        // Solicitar siguiente frame
        self.surface.frame(qh, self.surface.clone());
    }
    
    fn render_bars(&mut self, bar_heights: &[f32]) {
        let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width = bar_width * self.bar_gap;
        
        let mut vertices = Vec::with_capacity(self.bar_count as usize * 8);
        
        for i in 0..self.bar_count as usize {
            let height = 2.0 * bar_heights[i] - 1.0; // Convertir de 0-1 a -1 a 1
            
            // Vértices para un cuadrilátero (2 triángulos)
            // Esquina superior izquierda
            vertices.push(bar_gap_width * i as f32 + bar_width * i as f32 - 1.0);
            vertices.push(height);
            
            // Esquina superior derecha
            vertices.push(bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0);
            vertices.push(height);
            
            // Esquina inferior izquierda
            vertices.push(bar_gap_width * i as f32 + bar_width * i as f32 - 1.0);
            vertices.push(-1.0);
            
            // Esquina inferior derecha
            vertices.push(bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0);
            vertices.push(-1.0);
        }
        
        unsafe {
            gl::BindVertexArray(self.vao);
            gl::BindBuffer(gl::ARRAY_BUFFER, self.vbo);
            gl::BufferData(
                gl::ARRAY_BUFFER,
                (vertices.len() * std::mem::size_of::<f32>()) as gl::types::GLsizeiptr,
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
            gl::Uniform2f(self.windows_size_location, self.width as f32, self.height as f32);
            
            // Dibujar elementos (2 triángulos por barra = 6 índices por barra)
            gl::DrawElements(
                gl::TRIANGLES,
                (self.bar_count as usize * 6) as gl::types::GLsizei,
                gl::UNSIGNED_SHORT,
                std::ptr::null(),
            );
            
            gl::BindVertexArray(0);
        }
    }
    
    fn check_wallpaper_changes(&mut self) {
        let new_wallpaper_path = WallpaperAnalyzer::get_current_wallpaper_path().unwrap_or(None);
        
        let wallpaper_changed = match (&self.current_wallpaper_path, &new_wallpaper_path) {
            (Some(old), Some(new)) => old != new,
            (None, Some(_)) => true,
            (Some(_), None) => true,
            (None, None) => false,
        };
        
        if wallpaper_changed {
            info!("Wallpaper change detected!");
            self.current_wallpaper_path = new_wallpaper_path.clone();
            
            if let Some(path) = &self.current_wallpaper_path {
                println!("New wallpaper detected: {}", path.display());
                
                if self.config.general.auto_colors {
                    // Generar nuevos colores del fondo
                    match WallpaperAnalyzer::generate_gradient_colors(8) {
                        Ok(gradient_colors) => {
                            println!("Generated {} gradient colors from wallpaper", gradient_colors.len());
                            self.update_gradient_colors(&gradient_colors);
                        }
                        Err(e) => {
                            warn!("Failed to generate gradient colors: {}", e);
                            println!("Using manual colors from configuration");
                        }
                    }
                }
                
                // Reiniciar cava con nueva configuración
                self.restart_cava();
            }
        }
    }
    
    fn update_gradient_colors(&mut self, gradient_colors: &[[f32; 4]]) {
        let gradient_colors_size = gradient_colors.len() as i32;
        let mut buffer_data: Vec<u8> = gradient_colors_size.to_le_bytes().to_vec();
        
        // Padding para alineación (vec4 necesita alineación de 16 bytes)
        buffer_data.extend([0u8; 12]); // 3 floats de padding
        
        for color in gradient_colors {
            for &component in color {
                buffer_data.extend_from_slice(&component.to_le_bytes());
            }
        }
        
        unsafe {
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, self.gradient_colors_ssbo);
            gl::BufferData(
                gl::SHADER_STORAGE_BUFFER,
                buffer_data.len() as gl::types::GLsizeiptr,
                buffer_data.as_ptr() as *const _,
                gl::STATIC_DRAW,
            );
            gl::BindBufferBase(gl::SHADER_STORAGE_BUFFER, 0, self.gradient_colors_ssbo);
            gl::BindBuffer(gl::SHADER_STORAGE_BUFFER, 0);
        }
    }
    
    fn restart_cava(&mut self) {
        // Detener cava anterior
        if let Some(mut process) = self.cava_process.take() {
            let _ = process.kill();
            let _ = process.wait();
            self.cava_reader = None;
        }
        
        // Generar configuración de cava
        let cava_config = self.config.to_cava_config();
        
        // Configurar cava para salida raw
        let mut cava_output_config = HashMap::new();
        cava_output_config.insert("method".to_string(), "raw".to_string());
        cava_output_config.insert("raw_target".to_string(), "/dev/stdout".to_string());
        cava_output_config.insert("bit_format".to_string(), "16bit".to_string());
        
        let cava_config_str = format!("[general]\n{}\n\n[output]\n{}\n\n[smoothing]\n{}",
            cava_config,
            cava_output_config.iter()
                .map(|(k, v)| format!("{} = {}", k, v))
                .collect::<Vec<_>>()
                .join("\n"),
            if let Some(nr) = self.config.smoothing.noise_reduction {
                format!("noise_reduction = {:.2}", nr)
            } else {
                "".to_string()
            }
        );
        
        // Iniciar nuevo proceso cava
        match Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
        {
            Ok(mut process) => {
                if let Some(stdin) = process.stdin.take() {
                    use std::io::Write;
                    let mut writer = std::io::BufWriter::new(stdin);
                    let _ = writer.write_all(cava_config_str.as_bytes());
                    let _ = writer.flush();
                }
                
                if let Some(stdout) = process.stdout.take() {
                    self.cava_reader = Some(BufReader::new(stdout));
                    self.cava_process = Some(process);
                    info!("cava restarted with new colors!");
                }
            }
            Err(e) => {
                error!("Failed to start cava process: {}", e);
            }
        }
    }
}

fn main() -> Result<()> {
    env_logger::init();
    
    let args = Cli::parse();
    
    if args.version {
        println!("cava-bg v{}", env!("CARGO_PKG_VERSION"));
        println!("Repository: https://github.com/leriart/cava-bg");
        println!();
        println!("Integrated version with Wayland/OpenGL rendering");
        println!("Combines wallpaper-cava rendering with adaptive color detection");
        return Ok(());
    }
    
    if args.test_config {
        println!("Testing configuration and wallpaper analysis...");
        let config = Config::load(&args.config).context("Failed to load config")?;
        println!("Configuration loaded successfully:");
        println!("  Framerate: {}", config.general.framerate);
        println!("  Bars: {}", config.bars.amount);
        println!("  Colors: {}", config.colors.colors.len());
        println!("  Background color: {:?}", config.general.background_color);
        
        println!();
        println!("Testing wallpaper color detection and gradient generation...");
        match WallpaperAnalyzer::generate_gradient_colors(8) {
            Ok(colors) => {
                println!("Successfully generated {} gradient colors from wallpaper:", colors.len());
                for (i, color) in colors.iter().enumerate() {
                    let hex = format!(
                        "#{:02x}{:02x}{:02x}",
                        (color[0] * 255.0) as u8,
                        (color[1] * 255.0) as u8,
                        (color[2] * 255.0) as u8
                    );
                    println!("  Color {}: {} (RGB: {:.3}, {:.3}, {:.3})", i + 1, hex, color[0], color[1], color[2]);
                }
            }
            Err(e) => {
                println!("Failed to generate colors from wallpaper: {}", e);
                println!("Using default gradient colors instead.");
            }
        }
        
        return Ok(());
    }
    
    let config = Config::load(&args.config).context("Failed to load config")?;
    
    // Verificar que cava está instalado
    if Command::new("cava").arg("--version").output().is_err() {
        eprintln!("cava is not installed. Please install it:");
        eprintln!("  Arch: sudo pacman -S cava");
        eprintln!("  Debian/Ubuntu: sudo apt install cava");
        eprintln!("  Fedora: sudo dnf install cava");
        return Ok(());
    }
    
    // Configurar manejador de señales
    let _signal_handler = handle_signal();
    
    println!("cava-bg starting with Wayland/OpenGL rendering!");
    println!("Features:");
    println!("  - Adaptive gradient colors from wallpaper");
    println!("  - Automatic wallpaper change detection");
    println!("  - Native Wayland layer shell rendering");
    println!("  - Real-time audio visualization");
    println!("Press Ctrl+C to exit.");
    println!();
    
    // Inicializar Wayland
    let conn = Connection::connect_to_env()
        .context("Failed to connect to Wayland compositor. Are you running Wayland?")?;
    
    // TODO: Implementar el resto de la inicialización de Wayland/OpenGL
    // Esto requeriría integrar más código de wallpaper-cava
    
    println!("Note: Full Wayland/OpenGL integration is a work in progress.");
    println!("Current version has adaptive color detection and cava management,");
    println!("but full graphical rendering requires more development.");
    
    // Por ahora, mantener el comportamiento actual mejorado
    let mut current_wallpaper_path: Option<PathBuf> = None;
    let mut cava_process: Option<std::process::Child> = None;
    let mut last_wallpaper_check = Instant::now();
    let check_interval = Duration::from_secs(5);
    
    // Iniciar cava inicial
    let cava_config_path = dirs::cache_dir()
        .context("Failed to get cache directory")?
        .join("cava-bg-cava-config");
    
    let cava_config = config.to_cava_config();
    fs::write(&cava_config_path, &cava_config)
        .context("Failed to write cava config")?;
    
    info!("Starting cava process...");
    match Command::new("cava")
        .arg("-p")
        .arg(&cava_config_path)
        .stdout(Stdio::piped())
        .spawn()
    {
        Ok(process) => {
            cava_process = Some(process);
            println!("cava started successfully!");
            println!("Config: {}", cava_config_path.display());
        }
        Err(e) => {
            warn