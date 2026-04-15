use anyhow::{Context, Result};
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
use wgpu::util::DeviceExt;
use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::process::{Command, Stdio};
use std::ptr::NonNull;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use std::path::PathBuf;

use crate::app_config::{
    array_from_config_color, Config, CavaConfig, CavaGeneralConfig, CavaSmoothingConfig,
    HiddenImageConfig, HiddenImageEffect, PaletteType
};
use crate::wallpaper::WallpaperAnalyzer;

// Shader con crop PreserveAspectCrop
const SHADER_WGSL: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) world_pos: vec2<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.world_pos = (in.position + vec2<f32>(1.0, 1.0)) * 0.5;
    return out;
}

struct Uniforms {
    gradient_colors: array<vec4<f32>, 32>,
    params: vec4<f32>,
    window_size: vec2<f32>,
    texture_size: vec2<f32>,
    _pad: vec4<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(1) @binding(0) var hidden_texture: texture_2d<f32>;
@group(1) @binding(1) var hidden_sampler: sampler;

fn apply_effect(color: vec3<f32>, effect: i32) -> vec3<f32> {
    switch effect {
        case 0: { return color; }
        case 1: {
            let gray = dot(color, vec3<f32>(0.299, 0.587, 0.114));
            return vec3<f32>(gray, gray, gray);
        }
        case 2: { return vec3<f32>(1.0) - color; }
        case 3: {
            let r = color.r * 0.393 + color.g * 0.769 + color.b * 0.189;
            let g = color.r * 0.349 + color.g * 0.686 + color.b * 0.168;
            let b = color.r * 0.272 + color.g * 0.534 + color.b * 0.131;
            return vec3<f32>(r, g, b);
        }
        case 4: {
            let lum = dot(color, vec3<f32>(0.299, 0.587, 0.114));
            let idx_float = lum * 7.99;
            if (idx_float < 1.0) { return vec3<f32>(0.878, 0.859, 0.953); }
            else if (idx_float < 2.0) { return vec3<f32>(0.961, 0.761, 0.906); }
            else if (idx_float < 3.0) { return vec3<f32>(0.953, 0.545, 0.659); }
            else if (idx_float < 4.0) { return vec3<f32>(0.922, 0.627, 0.675); }
            else if (idx_float < 5.0) { return vec3<f32>(0.796, 0.651, 0.969); }
            else if (idx_float < 6.0) { return vec3<f32>(0.537, 0.706, 0.980); }
            else if (idx_float < 7.0) { return vec3<f32>(0.455, 0.780, 0.925); }
            else { return vec3<f32>(0.580, 0.886, 0.835); }
        }
        default: { return color; }
    }
}

// Calcula UVs con PreserveAspectCrop
fn compute_crop_uv(world_pos: vec2<f32>) -> vec2<f32> {
    let screen_ratio = uniforms.window_size.x / uniforms.window_size.y;
    let tex_ratio = uniforms.texture_size.x / uniforms.texture_size.y;
    
    var scale_factor: f32;
    var uv: vec2<f32>;
    
    if (tex_ratio > screen_ratio) {
        // Textura más ancha: se ajusta a altura, se recorta en ancho
        scale_factor = uniforms.window_size.y / uniforms.texture_size.y;
        let scaled_width = uniforms.texture_size.x * scale_factor;
        let offset_x = (uniforms.window_size.x - scaled_width) * 0.5;
        uv.x = (world_pos.x * uniforms.window_size.x - offset_x) / scaled_width;
        uv.y = world_pos.y;
    } else {
        // Textura más alta: se ajusta a ancho, se recorta en alto
        scale_factor = uniforms.window_size.x / uniforms.texture_size.x;
        let scaled_height = uniforms.texture_size.y * scale_factor;
        let offset_y = (uniforms.window_size.y - scaled_height) * 0.5;
        uv.x = world_pos.x;
        uv.y = (world_pos.y * uniforms.window_size.y - offset_y) / scaled_height;
    }
    
    uv.y = 1.0 - uv.y; // Invertir Y
    return uv;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    let y = uniforms.window_size.y - in.position.y;
    let height = uniforms.window_size.y;
    let colors_count = i32(uniforms.params.x);
    let bar_alpha = uniforms.params.y;
    let use_hidden_image = uniforms.params.z > 0.5;
    let effect_type = i32(uniforms.params.w);
    
    var base_color: vec4<f32>;
    if (colors_count == 1) {
        base_color = uniforms.gradient_colors[0];
    } else {
        let findex = (y * f32(colors_count - 1)) / height;
        let index = i32(findex);
        let step = findex - f32(index);
        var idx = index;
        if (idx == colors_count - 1) { idx = idx - 1; }
        base_color = mix(uniforms.gradient_colors[idx], uniforms.gradient_colors[idx + 1], step);
    }
    
    var final_color = base_color;
    if (use_hidden_image) {
        let uv = compute_crop_uv(in.world_pos);
        let tex_color = textureSample(hidden_texture, hidden_sampler, uv);
        let processed_rgb = apply_effect(tex_color.rgb, effect_type);
        final_color = vec4<f32>(processed_rgb, tex_color.a);
    }
    
    let alpha = final_color.a * bar_alpha;
    return vec4<f32>(final_color.rgb, alpha);
}
"#;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    gradient_colors: [[f32; 4]; 32],
    params: [f32; 4],
    window_size: [f32; 2],
    texture_size: [f32; 2],
    _pad: [f32; 4],
}

impl Uniforms {
    fn new(
        colors: &[[f32; 4]],
        width: f32,
        height: f32,
        bar_alpha: f32,
        use_hidden_image: bool,
        effect: HiddenImageEffect,
        tex_width: f32,
        tex_height: f32,
    ) -> Self {
        let mut grad = [[0.0; 4]; 32];
        for (i, c) in colors.iter().enumerate().take(32) {
            grad[i] = *c;
        }
        let effect_code = match effect {
            HiddenImageEffect::None => 0,
            HiddenImageEffect::Grayscale => 1,
            HiddenImageEffect::Invert => 2,
            HiddenImageEffect::Sepia => 3,
            HiddenImageEffect::Palette(PaletteType::Catppuccin) => 4,
            _ => 0,
        };
        Self {
            gradient_colors: grad,
            params: [
                colors.len() as f32,
                bar_alpha,
                if use_hidden_image { 1.0 } else { 0.0 },
                effect_code as f32,
            ],
            window_size: [width, height],
            texture_size: [tex_width, tex_height],
            _pad: [0.0; 4],
        }
    }
}

struct PerOutputState {
    surface: WlSurface,
    layer_surface: LayerSurface,
    wgpu_surface: wgpu::Surface<'static>,
    wgpu_device: wgpu::Device,
    wgpu_queue: wgpu::Queue,
    wgpu_config: wgpu::SurfaceConfiguration,
    render_pipeline: wgpu::RenderPipeline,
    bind_group0: wgpu::BindGroup, // bind group 0 (uniforms)
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    configured: bool,
    background_color: [f32; 4],
    // Nuevos campos para imagen oculta
    hidden_image_bind_group: wgpu::BindGroup, // bind group 1 (siempre existe)
    _hidden_image_texture: Option<wgpu::Texture>,
    _hidden_image_view: Option<wgpu::TextureView>,
    hidden_texture_size: (u32, u32), // NUEVO: almacenar dimensiones de la textura
}

pub struct WaylandRenderer {
    config: Config,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(config: Config, running: Arc<AtomicBool>) -> Self {
        Self { config, running }
    }

    fn build_cava_config(config: &Config) -> String {
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
            output: HashMap::from([
                ("method".to_string(), "raw".to_string()),
                ("raw_target".to_string(), "/dev/stdout".to_string()),
                ("bit_format".to_string(), "16bit".to_string()),
            ]),
        };
        toml::to_string(&cava_config).unwrap()
    }

    fn load_hidden_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &PathBuf,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, u32, u32)> {
        let img = image::open(path)
            .with_context(|| format!("Failed to open hidden image: {:?}", path))?;
        let rgba = img.to_rgba8();
        let dimensions = rgba.dimensions();
        let width = dimensions.0;
        let height = dimensions.1;

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hidden Image"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok((texture, view, width, height))
    }

    pub fn run(self) -> Result<()> {
        info!("Starting cava-bg with wgpu backend");

        let cava_config_str = Self::build_cava_config(&self.config);
        debug!("cava config:\n{}", cava_config_str);

        let mut cmd = Command::new("cava")
            .arg("-p")
            .arg("/dev/stdin")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .context("Failed to spawn cava process")?;

        if let Some(mut stdin) = cmd.stdin.take() {
            stdin.write_all(cava_config_str.as_bytes())?;
            stdin.flush()?;
        }

        let (_framerate_tx, framerate_rx) = channel::<f64>();
        let cava_stdout = cmd.stdout.take().context("Failed to get cava stdout")?;
        let bar_count = self.config.bars.amount as usize;
        let bar_alpha = self.config.bars.bar_alpha;

        let (cava_tx, cava_rx): (Sender<Vec<f32>>, Receiver<Vec<f32>>) = channel();
        thread::spawn(move || {
            let mut reader = BufReader::new(cava_stdout);
            let mut buffer = vec![0u8; bar_count * 2];
            loop {
                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        let mut bar_heights = vec![0.0f32; bar_count];
                        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
                            let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                            bar_heights[i] = (num as f32) / 65530.0;
                        }
                        if cava_tx.send(bar_heights).is_err() { break; }
                    }
                    Err(e) => {
                        error!("Error reading cava data: {}", e);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });

        let use_dynamic = self.config.general.dynamic_colors;
        let (initial_colors, color_receiver) = if use_dynamic {
            let num_colors = if !self.config.colors.is_empty() { self.config.colors.len() } else { 8 };
            match WallpaperAnalyzer::generate_gradient_colors(num_colors) {
                Ok(colors) => {
                    info!("Using dynamic colors from wallpaper");
                    let (color_tx, color_rx) = channel();
                    WallpaperAnalyzer::start_wallpaper_monitor(color_tx, num_colors);
                    (colors, color_rx)
                }
                Err(e) => {
                    error!("Failed to generate colors: {}, using config colors", e);
                    let colors: Vec<[f32; 4]> = self.config.colors.values()
                        .map(|c| array_from_config_color(c.clone())).collect();
                    let (_dummy_tx, dummy_rx) = channel::<Vec<[f32; 4]>>();
                    (colors, dummy_rx)
                }
            }
        } else {
            info!("Using static colors from config");
            let colors: Vec<[f32; 4]> = self.config.colors.values()
                .map(|c| array_from_config_color(c.clone())).collect();
            let (_dummy_tx, dummy_rx) = channel::<Vec<[f32; 4]>>();
            (colors, dummy_rx)
        };
        let background_color = array_from_config_color(self.config.general.background_color.clone());

        let hidden_image_config = self.config.hidden_image.clone();
        let use_hidden_image = hidden_image_config.is_some();
        let use_wallpaper_image = hidden_image_config.as_ref().map(|c| c.use_wallpaper).unwrap_or(false);

        let (_wallpaper_path_tx, wallpaper_path_rx): (Option<Sender<Option<PathBuf>>>, Receiver<Option<PathBuf>>) = if use_wallpaper_image {
            let (tx, rx) = channel();
            let tx_clone = tx.clone();
            WallpaperAnalyzer::start_wallpaper_path_monitor(tx);
            (Some(tx_clone), rx)
        } else {
            let (_dummy_tx, dummy_rx) = channel::<Option<PathBuf>>();
            (None, dummy_rx)
        };

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();

        let mut event_loop: EventLoop<AppState> = EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue).insert(loop_handle)
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;

        let initial_wallpaper_path = if use_wallpaper_image {
            WallpaperAnalyzer::find_wallpaper()
        } else {
            None
        };

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            layer_shell,
            compositor,
            per_output: HashMap::new(),
            bar_count,
            bar_gap: self.config.bars.gap,
            bar_alpha,
            preferred_output_name: self.config.general.preferred_output.clone(),
            cava_data_receiver: cava_rx,
            current_bar_heights: vec![0.0; bar_count],
            last_cava_data: None,
            cava_frame_counter: 0,
            colors: initial_colors,
            background_color,
            conn: conn.clone(),
            qh: qh.clone(),
            running: self.running,
            color_receiver,
            framerate: self.config.general.framerate as f64,
            framerate_receiver: framerate_rx,
            hidden_image_config,
            use_hidden_image,
            use_wallpaper_image,
            wallpaper_path_receiver: wallpaper_path_rx,
            current_wallpaper_path: initial_wallpaper_path,
        };
        let frame_duration = Duration::from_secs_f64(1.0 / app_state.framerate);

        for output in app_state.output_state.outputs() {
            if let Err(e) = app_state.ensure_output(&output) {
                error!("Failed to create initial output: {}", e);
            }
        }

        event_loop.run(Some(frame_duration), &mut app_state, |state| {
            if let Ok(new_framerate) = state.framerate_receiver.try_recv() {
                state.framerate = new_framerate;
                info!("Framerate updated dynamically to {}", new_framerate);
            }

            if !state.running.load(Ordering::SeqCst) {
                std::process::exit(0);
            }

            if let Ok(new_colors) = state.color_receiver.try_recv() {
                info!("Updating gradient colors from wallpaper change");
                state.colors = new_colors;
                for output_state in state.per_output.values_mut() {
                    if output_state.configured {
                        let uniforms = Uniforms::new(
                            &state.colors,
                            output_state.width as f32,
                            output_state.height as f32,
                            state.bar_alpha,
                            state.use_hidden_image,
                            state.hidden_image_config.as_ref().map(|c| c.effect).unwrap_or_default(),
                            output_state.hidden_texture_size.0 as f32,
                            output_state.hidden_texture_size.1 as f32,
                        );
                        output_state.wgpu_queue.write_buffer(&output_state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
                    }
                }
            }

            if state.use_wallpaper_image {
                if let Ok(Some(new_path)) = state.wallpaper_path_receiver.try_recv() {
                    info!("Wallpaper changed to {:?}", new_path);
                    state.current_wallpaper_path = Some(new_path.clone());

                    let xray_path = resolve_xray_path(&new_path, &state.hidden_image_config);
                    let load_path = xray_path.as_ref().unwrap_or(&new_path);

                    let mut outputs_to_update = Vec::new();
                    for output_state in state.per_output.values_mut() {
                        if output_state.configured {
                            outputs_to_update.push(output_state);
                        }
                    }

                    for output_state in outputs_to_update {
                        // Actualizar la textura (esto también guarda el nuevo tamaño en output_state.hidden_texture_size)
                        if let Err(e) = AppState::update_hidden_image_texture(output_state, load_path) {
                            error!("Failed to update hidden image texture: {}", e);
                            continue;
                        }

                        // --- CORRECCIÓN: Actualizar el uniform buffer con las nuevas dimensiones ---
                        let effect = state.hidden_image_config.as_ref().map(|c| c.effect).unwrap_or_default();
                        let uniforms = Uniforms::new(
                            &state.colors,
                            output_state.width as f32,
                            output_state.height as f32,
                            state.bar_alpha,
                            state.use_hidden_image,
                            effect,
                            output_state.hidden_texture_size.0 as f32,
                            output_state.hidden_texture_size.1 as f32,
                        );
                        output_state.wgpu_queue.write_buffer(&output_state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
                        // -------------------------------------------------------------------------
                    }
                }
            }

            state.draw();
        })?;

        Ok(())
    }
}

/// Dada la ruta de un wallpaper y la configuración, devuelve la ruta de la versión "xray"
/// si existe un archivo con el mismo nombre en el directorio `xray_images_dir`.
fn resolve_xray_path(wallpaper_path: &PathBuf, config: &Option<HiddenImageConfig>) -> Option<PathBuf> {
    if let Some(cfg) = config {
        if let Some(xray_dir) = &cfg.xray_images_dir {
            let xray_dir_path = PathBuf::from(xray_dir);
            if let Some(file_name) = wallpaper_path.file_name() {
                let xray_path = xray_dir_path.join(file_name);
                if xray_path.exists() {
                    info!("Found xray image: {:?}", xray_path);
                    return Some(xray_path);
                }
            }
        }
    }
    None
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    layer_shell: LayerShell,
    compositor: CompositorState,
    per_output: HashMap<String, PerOutputState>,
    bar_count: usize,
    bar_gap: f32,
    bar_alpha: f32,
    preferred_output_name: Option<String>,
    cava_data_receiver: Receiver<Vec<f32>>,
    current_bar_heights: Vec<f32>,
    last_cava_data: Option<Vec<f32>>,
    cava_frame_counter: usize,
    colors: Vec<[f32; 4]>,
    background_color: [f32; 4],
    conn: Connection,
    qh: QueueHandle<Self>,
    running: Arc<AtomicBool>,
    color_receiver: Receiver<Vec<[f32; 4]>>,
    framerate: f64,
    framerate_receiver: Receiver<f64>,
    hidden_image_config: Option<HiddenImageConfig>,
    use_hidden_image: bool,
    use_wallpaper_image: bool,
    wallpaper_path_receiver: Receiver<Option<PathBuf>>,
    current_wallpaper_path: Option<PathBuf>,
}

impl AppState {
    fn create_dummy_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
        let texture_size = wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dummy Hidden Image"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let transparent_pixel = [0u8, 0, 0, 0];
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &transparent_pixel,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(4), rows_per_image: Some(1) },
            texture_size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view, 1, 1)
    }

    fn load_or_dummy_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &Option<HiddenImageConfig>,
        use_wallpaper: bool,
        wallpaper_path: Option<&PathBuf>,
    ) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
        if use_wallpaper {
            if let Some(path) = wallpaper_path {
                let xray_path = resolve_xray_path(path, config);
                let load_path = xray_path.as_ref().unwrap_or(path);
                match WaylandRenderer::load_hidden_image(device, queue, load_path) {
                    Ok((t, v, w, h)) => {
                        if xray_path.is_some() {
                            info!("Loaded xray image: {:?} ({}x{})", load_path, w, h);
                        } else {
                            info!("Loaded wallpaper as hidden image: {:?} ({}x{})", path, w, h);
                        }
                        (t, v, w, h)
                    }
                    Err(e) => {
                        warn!("Failed to load image {:?}: {}, using dummy", load_path, e);
                        Self::create_dummy_texture(device, queue)
                    }
                }
            } else {
                warn!("use_wallpaper=true but no wallpaper path available, using dummy");
                Self::create_dummy_texture(device, queue)
            }
        } else {
            if let Some(img_config) = config {
                if let Some(path_str) = &img_config.path {
                    let path = PathBuf::from(path_str);
                    match WaylandRenderer::load_hidden_image(device, queue, &path) {
                        Ok((t, v, w, h)) => {
                            info!("Loaded hidden image: {} ({}x{})", path_str, w, h);
                            (t, v, w, h)
                        }
                        Err(e) => {
                            warn!("Failed to load hidden image: {}, using dummy", e);
                            Self::create_dummy_texture(device, queue)
                        }
                    }
                } else {
                    warn!("No path specified for hidden image, using dummy");
                    Self::create_dummy_texture(device, queue)
                }
            } else {
                Self::create_dummy_texture(device, queue)
            }
        }
    }

    fn update_hidden_image_texture(output_state: &mut PerOutputState, path: &PathBuf) -> Result<()> {
        let (new_texture, new_view, width, height) = WaylandRenderer::load_hidden_image(
            &output_state.wgpu_device, &output_state.wgpu_queue, path
        )?;

        let sampler = output_state.wgpu_device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout = output_state.wgpu_device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hidden Image Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let new_bind_group = output_state.wgpu_device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Hidden Image Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&new_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        output_state._hidden_image_texture = Some(new_texture);
        output_state._hidden_image_view = Some(new_view);
        output_state.hidden_image_bind_group = new_bind_group;
        output_state.hidden_texture_size = (width, height);

        Ok(())
    }

    fn ensure_output(&mut self, output: &wl_output::WlOutput) -> Result<()> {
        let info = match self.output_state.info(output) {
            Some(info) => info,
            None => { debug!("Output info not yet available"); return Ok(()); }
        };
        let name = info.name.clone().unwrap_or_else(|| "unknown".to_string());

        if self.per_output.contains_key(&name) { return Ok(()); }
        if let Some(ref pref) = self.preferred_output_name {
            if &name != pref { debug!("Skipping output {} (preferred is {})", name, pref); return Ok(()); }
        }

        info!("Creating surface for output {}", name);
        let surface = self.compositor.create_surface(&self.qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            &self.qh, surface.clone(), Layer::Bottom, Some("cava-bg"), Some(output),
        );

        let logical_size = info.logical_size.unwrap_or((1920, 1080));
        let width = logical_size.0 as u32;
        let height = logical_size.1 as u32;

        layer_surface.set_size(width, height);
        layer_surface.set_anchor(Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT);
        layer_surface.set_exclusive_zone(-1);
        surface.commit();

        let wl_display = self.conn.display().id().as_ptr();
        let wl_surface_ptr = surface.id().as_ptr();
        let display_ptr = NonNull::new(wl_display as *mut std::ffi::c_void).unwrap();
        let surface_ptr = NonNull::new(wl_surface_ptr as *mut std::ffi::c_void).unwrap();
        let display_handle = WaylandDisplayHandle::new(display_ptr);
        let window_handle = WaylandWindowHandle::new(surface_ptr);
        let raw_display = RawDisplayHandle::Wayland(display_handle);
        let raw_window = RawWindowHandle::Wayland(window_handle);

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let wgpu_surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: raw_display,
                raw_window_handle: raw_window,
            })
        }.context("Failed to create WGPU surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&wgpu_surface),
            force_fallback_adapter: false,
        })).context("Failed to find suitable GPU adapter")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        )).context("Failed to create device")?;

        let surface_caps = wgpu_surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| matches!(f, wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb))
            .unwrap_or(surface_caps.formats[0]);

        let alpha_mode = wgpu::CompositeAlphaMode::PreMultiplied;

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode,
            view_formats: vec![surface_format],
            desired_maximum_frame_latency: 0,
        };
        wgpu_surface.configure(&device, &surface_config);

        let (hidden_texture, hidden_image_view, tex_width, tex_height) = Self::load_or_dummy_texture(
            &device,
            &queue,
            &self.hidden_image_config,
            self.use_wallpaper_image,
            self.current_wallpaper_path.as_ref(),
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let bind_group_layout1 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Hidden Image Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let hidden_image_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Hidden Image Bind Group"),
            layout: &bind_group_layout1,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&hidden_image_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let mut all_indices = Vec::with_capacity(self.bar_count * 6);
        for i in 0..self.bar_count {
            let base = (i * 4) as u16;
            all_indices.extend_from_slice(&[base, base+1, base+2, base+1, base+3, base+2]);
        }
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&all_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: (self.bar_count * 4 * 4 * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let effect = self.hidden_image_config.as_ref().map(|c| c.effect).unwrap_or_default();
        let uniforms = Uniforms::new(
            &self.colors,
            width as f32,
            height as f32,
            self.bar_alpha,
            self.use_hidden_image,
            effect,
            tex_width as f32,
            tex_height as f32,
        );
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout0 = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Uniform Bind Group Layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let bind_group0 = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Uniform Bind Group"),
            layout: &bind_group_layout0,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: uniform_buffer.as_entire_binding() }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout0, &bind_group_layout1],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (4 * std::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 0, shader_location: 0 },
                        wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x2, offset: 8, shader_location: 1 },
                    ],
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: "fs_main",
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_config.format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
        });

        let wgpu_surface_static: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };

        let state = PerOutputState {
            surface,
            layer_surface,
            wgpu_surface: wgpu_surface_static,
            wgpu_device: device,
            wgpu_queue: queue,
            wgpu_config: surface_config,
            render_pipeline,
            bind_group0,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            width,
            height,
            configured: false,
            background_color: self.background_color,
            hidden_image_bind_group,
            _hidden_image_texture: Some(hidden_texture),
            _hidden_image_view: Some(hidden_image_view),
            hidden_texture_size: (tex_width, tex_height),
        };

        self.per_output.insert(name.clone(), state);
        info!("WGPU surface created for {}: {}x{}", name, width, height);
        Ok(())
    }

    fn draw(&mut self) {
        while let Ok(new_heights) = self.cava_data_receiver.try_recv() {
            self.last_cava_data = Some(new_heights);
            self.cava_frame_counter += 1;
        }

        if let Some(ref heights) = self.last_cava_data {
            self.current_bar_heights = heights.clone();
        }

        let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width = bar_width * self.bar_gap;
        let mut vertices = Vec::with_capacity(self.bar_count * 16);
        for i in 0..self.bar_count {
            let h = 2.0 * self.current_bar_heights[i] - 1.0;
            let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
            let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
            vertices.extend_from_slice(&[x0, -1.0, 0.0, 0.0]);
            vertices.extend_from_slice(&[x0, h,     0.0, 1.0]);
            vertices.extend_from_slice(&[x1, -1.0, 1.0, 0.0]);
            vertices.extend_from_slice(&[x1, h,     1.0, 1.0]);
        }

        for state in self.per_output.values_mut() {
            if !state.configured { continue; }

            state.wgpu_queue.write_buffer(&state.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let frame = match state.wgpu_surface.get_current_texture() {
                Ok(f) => f,
                Err(wgpu::SurfaceError::Lost) => {
                    state.wgpu_surface.configure(&state.wgpu_device, &state.wgpu_config);
                    continue;
                }
                Err(e) => { error!("Surface error: {:?}", e); continue; }
            };

            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = state.wgpu_device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut bg = state.background_color;
                if self.use_hidden_image && bg[3] > 0.0 {
                    warn!("background_color alpha is {}, forcing to 0.0 for hidden image mode", bg[3]);
                    bg[3] = 0.0;
                }
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color { r: bg[0] as f64, g: bg[1] as f64, b: bg[2] as f64, a: bg[3] as f64 }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });
                render_pass.set_pipeline(&state.render_pipeline);
                render_pass.set_bind_group(0, &state.bind_group0, &[]);
                render_pass.set_bind_group(1, &state.hidden_image_bind_group, &[]);
                render_pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
                render_pass.set_index_buffer(state.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..(self.bar_count * 6) as u32, 0, 0..1);
            }
            state.wgpu_queue.submit(std::iter::once(encoder.finish()));
            frame.present();
            state.surface.frame(&self.qh, state.surface.clone());
        }
    }
}

// --- Wayland Handlers (sin cambios significativos) ---
impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState { &mut self.output_state }
    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Err(e) = self.ensure_output(&output) { error!("Failed to create output: {}", e); }
    }
    fn update_output(&mut self, conn: &Connection, qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.new_output(conn, qh, output);
    }
    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = match self.output_state.info(&output) { Some(i) => i, None => return };
        let name = info.name.unwrap_or_else(|| "unknown".to_string());
        if self.per_output.remove(&name).is_some() { info!("Output {} removed", name); }
    }
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_registry!(AppState);
delegate_layer!(AppState);

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState { &mut self.registry_state }
    registry_handlers![];
}

impl CompositorHandler for AppState {
    fn scale_factor_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_factor: i32) {}
    fn transform_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_transform: wl_output::Transform) {}
    fn frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {}
    fn surface_enter(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}
    fn configure(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, layer: &LayerSurface, configure: LayerSurfaceConfigure, _serial: u32) {
        for (name, state) in self.per_output.iter_mut() {
            if &state.layer_surface == layer {
                let width = configure.new_size.0;
                let height = configure.new_size.1;
                if width == state.width && height == state.height && state.configured { return; }
                state.width = width;
                state.height = height;
                state.wgpu_config.width = width;
                state.wgpu_config.height = height;
                state.wgpu_surface.configure(&state.wgpu_device, &state.wgpu_config);
                let effect = self.hidden_image_config.as_ref().map(|c| c.effect).unwrap_or_default();
                let uniforms = Uniforms::new(
                    &self.colors,
                    width as f32,
                    height as f32,
                    self.bar_alpha,
                    self.use_hidden_image,
                    effect,
                    state.hidden_texture_size.0 as f32,
                    state.hidden_texture_size.1 as f32,
                );
                state.wgpu_queue.write_buffer(&state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
                state.configured = true;
                info!("Output {} configured: {}x{}", name, width, height);
                break;
            }
        }
    }
}