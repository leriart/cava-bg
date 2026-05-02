// unused: #![allow(unknown_literals)]

use anyhow::{Context, Result};
use log::{debug, error, info, warn};
use smithay_client_toolkit::reexports::calloop::EventLoop;
use smithay_client_toolkit::reexports::calloop_wayland_source::WaylandSource;
use smithay_client_toolkit::registry::ProvidesRegistryState;
use smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity;
use smithay_client_toolkit::shell::wlr_layer::{
    Anchor, Layer, LayerShell, LayerShellHandler, LayerSurface, LayerSurfaceConfigure,
};
use smithay_client_toolkit::shell::WaylandSurface;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::{
    compositor::{CompositorHandler, CompositorState},
    output::{OutputHandler, OutputState},
    registry::RegistryState,
    seat::{
        pointer::PointerEvent, pointer::PointerEventKind, pointer::PointerHandler, Capability,
        SeatHandler, SeatState,
    },
};
use smithay_client_toolkit::{
    delegate_compositor, delegate_layer, delegate_output, delegate_pointer, delegate_registry,
    delegate_seat, registry_handlers,
};
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::Proxy;
use wayland_client::{
    globals::registry_queue_init,
    protocol::{wl_output, wl_pointer, wl_seat, wl_surface},
    Connection, QueueHandle,
};
use wgpu::util::DeviceExt;

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

fn build_cava_config(config: &Config) -> String {
    let cava_config = CavaConfig {
        general: CavaGeneralConfig {
            framerate: config.general.framerate,
            bars: config.audio.bar_count,
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

fn cava_config_hash(config: &Config) -> u64 {
    let s = build_cava_config(config);
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

use crate::app_config::{
    array_from_config_color, BarShape, BlendMode, CavaConfig, CavaGeneralConfig,
    CavaSmoothingConfig, Config, HiddenImageConfig, HiddenImageEffect, LayerChoice,
    OutputDescriptor, PaletteType, ProfileSource, VisualizationMode, XRayConfig,
};
use crate::bar_geometry;
use crate::parallax_system::{AudioBands, ComputedParallaxLayer, ParallaxSystem};
use crate::perf_monitor::PerfMonitor;
use crate::video_decoder::{VideoDecoder, VideoDecoderConfig};
use crate::wallpaper::WallpaperAnalyzer;
use crate::xray_animator::XRayAnimator;

/// Wallpaper change event carrying both the path and extracted colors.
/// This ensures color extraction and path-based reloads happen atomically.
#[allow(dead_code)]
struct WallpaperEvent {
    path: Option<PathBuf>,
    colors: Option<Vec<[f32; 4]>>,
}

const SHADER_WGSL: &str = include_str!("shader.wgsl");
const PARALLAX_SHADER_WGSL: &str = include_str!("parallax_shader.wgsl");

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    gradient_colors: [[f32; 4]; 32],
    params: [f32; 4],
    window_size: [f32; 2],
    texture_size: [f32; 2],
    crop_scale: [f32; 2],
    crop_offset: [f32; 2],
    extra: [f32; 4],
}

impl Uniforms {
    #[allow(clippy::too_many_arguments)]
    fn new(
        colors: &[[f32; 4]],
        width: f32,
        height: f32,
        bar_alpha: f32,
        use_hidden_image: bool,
        effect: HiddenImageEffect,
        tex_width: f32,
        tex_height: f32,
        crop_scale: [f32; 2],
        crop_offset: [f32; 2],
        gradient_dir: f32, // 0=BottomToTop, 1=TopToBottom, 2=LeftToRight, 3=RightToLeft
        use_gradient: bool,
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
            crop_scale,
            crop_offset,
            extra: [gradient_dir, if use_gradient { 1.0 } else { 0.0 }, 0.0, 0.0],
        }
    }
}

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct ParallaxUniform {
    translation_ndc: [f32; 2],
    scale: f32,
    rotation_rad: f32,
    opacity: f32,
    _pad: f32,
    crop_scale: [f32; 2],
    crop_offset: [f32; 2],
}

impl ParallaxUniform {
    fn from_layer(
        layer: &ComputedParallaxLayer,
        viewport_size: (u32, u32),
        _texture_size: (u32, u32),
    ) -> Self {
        let width = viewport_size.0.max(1) as f32;
        let height = viewport_size.1.max(1) as f32;
        // translation_px now stores NDC offset directly (fraction of screen).
        // compute_transform() outputs in NDC space, layered by z_index exponentially.
        let tx = layer.transform.translation_px[0];
        let ty = -layer.transform.translation_px[1];

        let tw = _texture_size.0.max(1) as f32;
        let th = _texture_size.1.max(1) as f32;

        // Same aspect-ratio preserving transform as X-Ray (cover + center).
        let (mut crop_scale, mut crop_offset) =
            compute_preserve_aspect_crop_transform(width, height, tw, th);

        // Remove Y-axis inversion — parallax shader doesn't flip UVs.
        if crop_scale[1] < 0.0 {
            crop_scale[1] = -crop_scale[1];
            crop_offset[1] = -crop_offset[1] + 1.0;
        }

        // The quad moves in NDC based on parallax translation_px.
        // This shifts the entire scene relative to the viewport,
        // revealing areas the aspect-ratio transform would crop.
        // UVs stay fixed — movement is purely NDC-based.

        Self {
            translation_ndc: [tx, ty],
            scale: layer.transform.scale.max(0.01),
            rotation_rad: layer.transform.rotation_rad,
            opacity: layer.transform.opacity.clamp(0.0, 1.0),
            _pad: 0.0,
            crop_scale,
            crop_offset,
        }
    }
}

struct ParallaxGpuLayer {
    texture: wgpu::Texture,
    _view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    dimensions: (u32, u32),
    last_sequence: Option<u64>,
    /// Each layer gets its own uniform buffer so the render loop
    /// can write per-layer uniforms without GPU race conditions.
    uniform_buffer: wgpu::Buffer,
    uniform_bind_group: wgpu::BindGroup,
}

fn compute_preserve_aspect_crop_transform(
    viewport_width: f32,
    viewport_height: f32,
    texture_width: f32,
    texture_height: f32,
) -> ([f32; 2], [f32; 2]) {
    if viewport_width <= 0.0
        || viewport_height <= 0.0
        || texture_width <= 0.0
        || texture_height <= 0.0
    {
        return ([1.0, -1.0], [0.0, 1.0]);
    }

    let screen_ratio = viewport_width / viewport_height;
    let tex_ratio = texture_width / texture_height;
    if tex_ratio > screen_ratio {
        let scale_factor = viewport_height / texture_height;
        let scaled_width = texture_width * scale_factor;
        let offset_x = (viewport_width - scaled_width) * 0.5;
        let scale_x = viewport_width / scaled_width;
        let offset_norm_x = -offset_x / scaled_width;
        ([scale_x, -1.0], [offset_norm_x, 1.0])
    } else {
        let scale_factor = viewport_width / texture_width;
        let scaled_height = texture_height * scale_factor;
        let offset_y = (viewport_height - scaled_height) * 0.5;
        let scale_y = viewport_height / scaled_height;
        let offset_norm_y = -offset_y / scaled_height;
        ([1.0, -scale_y], [0.0, 1.0 - offset_norm_y])
    }
}

fn is_video_media_path(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "mp4" | "webm" | "mkv" | "mov" | "avi" | "m4v" | "flv" | "wmv" | "gif"
    )
}

fn create_texture_from_rgba_frame(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    frame: &crate::video_decoder::VideoFrame,
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Hidden Video Texture"),
        size: wgpu::Extent3d {
            width: frame.width.max(1),
            height: frame.height.max(1),
            depth_or_array_layers: 1,
        },
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
        &frame.rgba,
        wgpu::ImageDataLayout {
            offset: 0,
            bytes_per_row: Some(frame.width * 4),
            rows_per_image: Some(frame.height),
        },
        wgpu::Extent3d {
            width: frame.width.max(1),
            height: frame.height.max(1),
            depth_or_array_layers: 1,
        },
    );

    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    (texture, view)
}

struct PerOutputState {
    output_name: String,
    output_index: u32,
    logical_position: (i32, i32),
    /// The raw wl_output this renderer is attached to (needed for surface recreation).
    wl_output: wl_output::WlOutput,
    surface: WlSurface,
    layer_surface: LayerSurface,
    /// The Wayland layer this surface was created with.
    /// When changed at runtime, the surface must be destroyed and recreated.
    draw_layer: LayerChoice,
    wgpu_surface: wgpu::Surface<'static>,
    wgpu_device: wgpu::Device,
    wgpu_queue: wgpu::Queue,
    wgpu_config: wgpu::SurfaceConfiguration,
    bar_render_pipeline: wgpu::RenderPipeline,
    bind_group0: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    configured: bool,
    /// When true, this output surface needs to be destroyed and recreated
    /// (e.g. because the Wayland layer changed).
    needs_rebuild: bool,
    background_color: [f32; 4],
    hidden_image_bind_group: wgpu::BindGroup,
    /// Whether a real (non-dummy) hidden image texture is currently loaded.
    /// When false, the render loop will keep use_hidden_image=false even if the config says so.
    hidden_image_loaded: bool,
    _hidden_image_texture: Option<wgpu::Texture>,
    _hidden_image_view: Option<wgpu::TextureView>,
    hidden_texture_size: (u32, u32),
    hidden_video_decoder: Option<VideoDecoder>,
    crop_scale: [f32; 2],
    crop_offset: [f32; 2],
    parallax_bind_group_layout: wgpu::BindGroupLayout,
    #[allow(dead_code)]
    parallax_uniform_buffer: wgpu::Buffer,
    #[allow(dead_code)]
    parallax_uniform_bind_group: wgpu::BindGroup,
    parallax_uniform_layout: wgpu::BindGroupLayout,
    parallax_pipeline_normal: wgpu::RenderPipeline,
    parallax_pipeline_add: wgpu::RenderPipeline,
    parallax_vertex_buffer: wgpu::Buffer,
    parallax_index_buffer: wgpu::Buffer,
    parallax_layers_gpu: HashMap<usize, ParallaxGpuLayer>,
    effective_config: Config,
    xray_animator: XRayAnimator,
    perf_monitor: PerfMonitor,
}

fn map_layer_choice_to_wlr_layer(choice: LayerChoice) -> Layer {
    match choice {
        LayerChoice::Background => Layer::Background,
        LayerChoice::Bottom => Layer::Bottom,
        LayerChoice::Top => Layer::Top,
        LayerChoice::Overlay => Layer::Overlay,
    }
}

/// Compute the Wayland anchor bitmask from the Display position + individual anchor toggles.
fn compute_display_anchor(disp: &crate::app_config::DisplayConfig) -> Anchor {
    let mut anchor = Anchor::empty();
    match disp.position {
        crate::app_config::Position::Fill => {
            anchor |= Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
        }
        crate::app_config::Position::Center => {
            // No anchors → centered floating
        }
        crate::app_config::Position::Top => {
            anchor |= Anchor::TOP | Anchor::LEFT | Anchor::RIGHT;
        }
        crate::app_config::Position::Bottom => {
            anchor |= Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT;
        }
        crate::app_config::Position::Left => {
            anchor |= Anchor::LEFT | Anchor::TOP | Anchor::BOTTOM;
        }
        crate::app_config::Position::Right => {
            anchor |= Anchor::RIGHT | Anchor::TOP | Anchor::BOTTOM;
        }
        crate::app_config::Position::Custom => {
            // Override with individual anchor toggles
            if disp.anchor_top {
                anchor |= Anchor::TOP;
            }
            if disp.anchor_bottom {
                anchor |= Anchor::BOTTOM;
            }
            if disp.anchor_left {
                anchor |= Anchor::LEFT;
            }
            if disp.anchor_right {
                anchor |= Anchor::RIGHT;
            }
        }
    }
    anchor
}

pub struct WaylandRenderer {
    config: Config,
    running: Arc<AtomicBool>,
    config_path: Option<PathBuf>,
    cava_handle: Option<std::process::Child>,
    last_cava_config_hash: u64,
}

impl WaylandRenderer {
    pub fn new(config: Config, running: Arc<AtomicBool>, config_path: Option<PathBuf>) -> Self {
        Self {
            config,
            running,
            config_path,
            cava_handle: None,
            last_cava_config_hash: 0,
        }
    }

    fn spawn_color_update_listener_thread(
        config_path: PathBuf,
        running: Arc<AtomicBool>,
        color_update_tx: Sender<ColorUpdatePayload>,
    ) {
        thread::spawn(move || {
            let config_dir = config_path
                .parent()
                .unwrap_or_else(|| std::path::Path::new("/tmp"))
                .to_path_buf();
            let socket_path = config_dir.join("runtime-color-update.sock");
            let _ = std::fs::remove_file(&socket_path);
            let socket = match std::os::unix::net::UnixDatagram::bind(&socket_path) {
                Ok(s) => s,
                Err(e) => {
                    error!("Failed to bind color update socket: {}", e);
                    return;
                }
            };
            info!(
                "Push directo de color habilitado en socket (datagram) {}",
                socket_path.display()
            );
            socket.set_nonblocking(true).ok();
            let mut buf = vec![0u8; 65536];
            loop {
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                match socket.recv_from(&mut buf) {
                    Ok((n, _addr)) => {
                        let msg = String::from_utf8_lossy(&buf[..n]).trim().to_string();
                        if msg.is_empty() {
                            continue;
                        }
                        info!("Received color update via socket: {} bytes", msg.len());
                        match serde_json::from_str::<ColorUpdatePayload>(&msg) {
                            Ok(payload) => {
                                if color_update_tx.send(payload).is_err() {
                                    break;
                                }
                            }
                            Err(e) => {
                                error!("Failed to parse ColorUpdatePayload: {}", e);
                            }
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(e) => {
                        error!("Socket recv error: {}", e);
                        std::thread::sleep(std::time::Duration::from_millis(100));
                    }
                }
            }
            let _ = std::fs::remove_file(&socket_path);
        });
    }

    fn spawn_config_hot_reload_thread(
        config_path: PathBuf,
        running: Arc<AtomicBool>,
        config_update_tx: Sender<Config>,
    ) {
        thread::spawn(move || {
            let mut last_modified = std::fs::metadata(&config_path)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            loop {
                std::thread::sleep(std::time::Duration::from_secs(1));
                if !running.load(Ordering::SeqCst) {
                    break;
                }
                let modified = match std::fs::metadata(&config_path).and_then(|m| m.modified()) {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if modified > last_modified {
                    info!("Config file modified, reloading…");
                    match std::fs::read_to_string(&config_path) {
                        Ok(content) => match toml::from_str::<Config>(&content) {
                            Ok(new_config) => {
                                if config_update_tx.send(new_config).is_err() {
                                    break;
                                }
                                last_modified = modified;
                                info!("Config hot-reloaded successfully");
                            }
                            Err(e) => {
                                error!("Failed to parse updated config: {}", e);
                            }
                        },
                        Err(e) => {
                            error!("Failed to read updated config: {}", e);
                        }
                    }
                }
            }
        });
    }

    #[allow(dead_code)]
    pub fn push_live_color_update(config: &Config) -> Result<()> {
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("~/.config"))
            .join("cava-bg");
        let socket_path = config_dir.join("runtime-color-update.sock");
        if !socket_path.exists() {
            return Err(anyhow::anyhow!(
                "Socket no encontrado (daemon no corriendo?)"
            ));
        }

        let payload = ColorUpdatePayload {
            colors: if !config.colors.palette.is_empty() {
                config.colors.palette.clone()
            } else {
                Vec::new()
            },
            bar_alpha: Some(config.audio.bar_alpha),
        };

        let message = serde_json::to_vec(&payload)
            .map_err(|e| anyhow::anyhow!("Error serializando payload: {}", e))?;

        let socket = std::os::unix::net::UnixDatagram::unbound()
            .map_err(|e| anyhow::anyhow!("Error creando socket datagram: {}", e))?;

        socket
            .send_to(&message, &socket_path)
            .map_err(|e| anyhow::anyhow!("Error enviando datagrama al socket: {}", e))?;

        Ok(())
    }

    fn load_hidden_image(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        path: &Path,
        target_width: u32,
        target_height: u32,
        prescale_max_dimension: u32,
        generate_mipmaps: bool,
    ) -> Result<(wgpu::Texture, wgpu::TextureView, u32, u32)> {
        let img = image::open(path)
            .with_context(|| format!("Failed to open hidden image: {:?}", path))?;

        let mut rgba = img.to_rgba8();
        let mut width = rgba.width();
        let mut height = rgba.height();

        let max_dim = width.max(height);
        let max_allowed = prescale_max_dimension.max(512);
        if max_dim > max_allowed {
            let scale = (max_allowed as f32) / (max_dim as f32);
            let scaled_w = ((width as f32) * scale).round().max(1.0) as u32;
            let scaled_h = ((height as f32) * scale).round().max(1.0) as u32;
            rgba = image::imageops::resize(
                &rgba,
                scaled_w,
                scaled_h,
                image::imageops::FilterType::Lanczos3,
            );
            width = rgba.width();
            height = rgba.height();
        }

        let output_max = target_width.max(target_height).max(1);
        let tex_max = width.max(height).max(1);
        if tex_max > output_max * 2 {
            let scale = (output_max * 2) as f32 / tex_max as f32;
            let scaled_w = ((width as f32) * scale).round().max(1.0) as u32;
            let scaled_h = ((height as f32) * scale).round().max(1.0) as u32;
            rgba = image::imageops::resize(
                &rgba,
                scaled_w,
                scaled_h,
                image::imageops::FilterType::Triangle,
            );
            width = rgba.width();
            height = rgba.height();
        }

        let mip_level_count = if generate_mipmaps {
            (u32::BITS - width.max(height).max(1).leading_zeros()) as u32
        } else {
            1
        };

        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Hidden Image"),
            size: texture_size,
            mip_level_count,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let mut current_level = rgba;
        for mip in 0..mip_level_count {
            let mip_width = current_level.width().max(1);
            let mip_height = current_level.height().max(1);
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &texture,
                    mip_level: mip,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                current_level.as_raw(),
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(4 * mip_width),
                    rows_per_image: Some(mip_height),
                },
                wgpu::Extent3d {
                    width: mip_width,
                    height: mip_height,
                    depth_or_array_layers: 1,
                },
            );

            if mip + 1 < mip_level_count {
                let next_w = (mip_width / 2).max(1);
                let next_h = (mip_height / 2).max(1);
                current_level = image::imageops::resize(
                    &current_level,
                    next_w,
                    next_h,
                    image::imageops::FilterType::Triangle,
                );
            }
        }

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        Ok((texture, view, width, height))
    }

    pub fn run(mut self) -> Result<()> {
        info!("Starting cava-bg with wgpu backend");

        let cava_config_str = build_cava_config(&self.config);
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
        let (config_update_tx, config_update_rx): (Sender<Config>, Receiver<Config>) = channel();
        let (color_update_tx, color_update_rx): (
            Sender<ColorUpdatePayload>,
            Receiver<ColorUpdatePayload>,
        ) = channel();
        if let Some(ref config_path) = self.config_path {
            Self::spawn_config_hot_reload_thread(
                config_path.clone(),
                self.running.clone(),
                config_update_tx,
            );
            Self::spawn_color_update_listener_thread(
                config_path.clone(),
                self.running.clone(),
                color_update_tx,
            );
        }
        let cava_stdout = cmd.stdout.take().context("Failed to get cava stdout")?;
        let bar_count = self.config.audio.bar_count as usize;
        let bar_alpha = self.config.audio.bar_alpha;
        let idle_cfg = self.config.performance.idle_mode.clone();

        self.cava_handle = Some(cmd);
        self.last_cava_config_hash = cava_config_hash(&self.config);

        let (cava_tx, cava_rx): (Sender<CavaFramePacket>, Receiver<CavaFramePacket>) = channel();
        let reader_cava_tx = cava_tx.clone();
        thread::spawn(move || {
            let mut reader = BufReader::new(cava_stdout);
            let mut buffer = vec![0u8; bar_count * 2];
            let threshold = idle_cfg.audio_threshold.max(0.0);
            loop {
                match reader.read_exact(&mut buffer) {
                    Ok(()) => {
                        let mut bar_heights = vec![0.0f32; bar_count];
                        let mut peak = 0.0f32;
                        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
                            let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                            let value = (num as f32) / 65530.0;
                            peak = peak.max(value);
                            bar_heights[i] = value;
                        }
                        let packet = CavaFramePacket {
                            bars: bar_heights,
                            peak,
                            silent: peak < threshold,
                        };
                        if reader_cava_tx.send(packet).is_err() {
                            break;
                        }
                    }
                    Err(e) => {
                        error!("Error reading cava data: {}", e);
                        std::thread::sleep(Duration::from_millis(10));
                    }
                }
            }
        });

        // Always set up wallpaper monitor so extract_from_wallpaper works
        let num_colors = if !self.config.colors.palette.is_empty() {
            self.config.colors.palette.len()
        } else {
            8
        };
        let extract_colors = self.config.colors.extract_from_wallpaper;
        let extraction_mode = self.config.colors.extraction_mode;
        let (color_tx, color_receiver) = channel();
        let color_tx_for_path = color_tx.clone();
        WallpaperAnalyzer::start_wallpaper_monitor(
            color_tx,
            num_colors,
            extraction_mode,
            extract_colors,
        );

        // External cursor monitor — reads cursor position via hyprctl.
        // This works even when the cursor is over other windows.
        let (cursor_tx, cursor_rx) = channel::<(f32, f32)>();
        AppState::start_cursor_monitor(cursor_tx);

        let use_dynamic = self.config.general.dynamic_colors;
        let initial_colors = if use_dynamic {
            match WallpaperAnalyzer::generate_gradient_colors(num_colors, Some(extraction_mode)) {
                Ok(colors) => {
                    info!("Using dynamic colors from wallpaper");
                    colors
                }
                Err(e) => {
                    error!("Failed to generate colors: {}, using config colors", e);
                    let colors: Vec<[f32; 4]> = if !self.config.colors.palette.is_empty() {
                        self.config.colors.palette.clone()
                    } else {
                        vec![[0.5, 0.3, 0.8, 1.0]; 8]
                    };
                    colors
                }
            }
        } else {
            info!("Using static colors from config");
            let colors: Vec<[f32; 4]> = if !self.config.colors.palette.is_empty() {
                self.config.colors.palette.clone()
            } else {
                vec![[0.5, 0.3, 0.8, 1.0]; 8]
            };
            colors
        };

        let hidden_image_config = self.config.hidden_image.clone();
        let xray_enabled = self.config.xray.enabled;
        let xray_images_dir = self.config.xray.images_dir.as_deref();

        // Always try to detect the wallpaper before deciding whether X-Ray is available.
        // This avoids the chicken-and-egg problem where use_wallpaper=true blocks the
        // path monitor from starting because the wallpaper hadn't been detected yet.
        let detected_wallpaper = if xray_enabled {
            WallpaperAnalyzer::find_wallpaper()
        } else {
            None
        };

        let use_hidden_image = xray_enabled
            && validate_hidden_image_available(
                &hidden_image_config,
                detected_wallpaper.as_ref(),
                xray_images_dir,
            );
        // Determine if we need wallpaper monitoring.
        // Check X-Ray (use_wallpaper) OR Parallax (profile_source == FromWallpaper)
        let xray_needs_wallpaper = hidden_image_config
            .as_ref()
            .map(|c| c.use_wallpaper)
            .unwrap_or(false);
        let parallax_needs_wallpaper = self.config.parallax.enabled
            && self.config.parallax.profile_source == ProfileSource::FromWallpaper
            && self.config.parallax.profiles_dir.is_some();
        let needs_wallpaper_monitoring =
            xray_needs_wallpaper || parallax_needs_wallpaper || extract_colors;

        info!(
            "needs_wallpaper_monitoring={}, xray_needs={}, parallax_needs={}, extract_colors={}",
            needs_wallpaper_monitoring,
            xray_needs_wallpaper,
            parallax_needs_wallpaper,
            extract_colors,
        );

        let (_wallpaper_path_tx, wallpaper_path_rx): (
            Option<Sender<Option<PathBuf>>>,
            Receiver<Option<PathBuf>>,
        ) = if needs_wallpaper_monitoring {
            let (tx, rx) = channel();
            let tx_clone = tx.clone();
            let color_tx = if extract_colors {
                Some(color_tx_for_path.clone())
            } else {
                None
            };
            WallpaperAnalyzer::start_wallpaper_path_monitor(
                tx,
                color_tx,
                num_colors,
                extraction_mode,
            );
            (Some(tx_clone), rx)
        } else {
            let (_dummy_tx, dummy_rx) = channel::<Option<PathBuf>>();
            (None, dummy_rx)
        };

        let use_wallpaper_image = use_hidden_image && xray_needs_wallpaper;

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) =
            registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();

        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let compositor =
            CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;
        let seat_state = SeatState::new(&globals, &qh);

        let initial_wallpaper_path = if needs_wallpaper_monitoring {
            detected_wallpaper
                .clone()
                .or_else(WallpaperAnalyzer::find_wallpaper)
        } else {
            None
        };

        info!("initial_wallpaper_path={:?}", initial_wallpaper_path);

        let telemetry_enabled =
            self.config.advanced.verbose_logging && self.config.performance.telemetry.enabled;
        let parallax_system = if self.config.parallax.enabled {
            info!("Parallax enabled, creating ParallaxSystem...");
            // Pass wallpaper name so rebuild_layers() can auto-match
            let wp_name = initial_wallpaper_path
                .as_ref()
                .and_then(|p| p.file_stem())
                .and_then(|s| s.to_str())
                .map(|s| s.to_string());

            match ParallaxSystem::new(self.config.parallax.clone(), 1920, 1080, wp_name) {
                Ok(system) => {
                    info!("ParallaxSystem created successfully");
                    Some(system)
                }
                Err(err) => {
                    warn!("Parallax disabled due to initialization error: {err:#}");
                    None
                }
            }
        } else {
            None
        };

        let runtime_outputs_path = self
            .config_path
            .as_ref()
            .and_then(|p| p.parent().map(|dir| dir.join("runtime-outputs.json")));

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            seat_state,
            pointer_devices: Vec::new(),
            layer_shell,
            compositor,
            base_config: self.config.clone(),
            per_output: HashMap::new(),
            next_output_index: 0,
            runtime_outputs_path,
            bar_gap: self.config.audio.gap,
            bar_alpha,
            height_scale: self.config.audio.height_scale,
            preferred_output_names: self.config.general.preferred_outputs.clone(),
            render_layer_choice: self.config.display.layer,
            cava_data_receiver: cava_rx,
            current_bar_heights: vec![0.0; bar_count],
            last_cava_peak: 0.0,
            cava_frame_counter: 0,
            is_idle: false,
            last_audio_active_at: Instant::now(),
            idle_transition_start: None,
            idle_audio_threshold: self.config.performance.idle_mode.audio_threshold.max(0.0),
            idle_timeout: Duration::from_secs_f32(
                self.config.performance.idle_mode.timeout_seconds.max(0.1),
            ),
            idle_fps: self.config.performance.idle_mode.idle_fps.max(1) as f64,
            idle_mode_enabled: self.config.performance.idle_mode.enabled,
            colors: initial_colors,
            conn: conn.clone(),
            qh: qh.clone(),
            running: self.running,
            color_receiver,
            framerate: self.config.general.framerate as f64,
            framerate_receiver: framerate_rx,
            config_update_receiver: config_update_rx,
            color_update_receiver: color_update_rx,
            hidden_image_config,
            use_hidden_image,
            use_wallpaper_image,
            bar_shape: self.config.audio.bar_shape,
            corner_radius: self.config.audio.corner_radius,
            corner_segments: self.config.audio.corner_segments,
            xray_config: self.config.xray.clone(),
            xray_animator: XRayAnimator::new(),
            parallax_system,
            cursor_norm: (0.5, 0.5),
            cursor_rx: Some(cursor_rx),
            global_mouse_norm: (0.5, 0.5),
            output_mouse_norm: HashMap::new(),
            wallpaper_path_receiver: wallpaper_path_rx,
            current_wallpaper_path: initial_wallpaper_path,
            last_loaded_wallpaper_path: None,
            perf_monitor: PerfMonitor::new(
                self.config.performance.telemetry.metrics_window,
                self.config.performance.telemetry.log_interval_seconds,
                telemetry_enabled,
            ),
            telemetry_enabled,
            telemetry_decoder_enabled: self.config.advanced.verbose_logging
                && self.config.performance.video_decoder.debug_telemetry,
            next_render_at: Instant::now(),
            idle_exit_transition: Duration::from_millis(
                self.config.performance.idle_mode.exit_transition_ms as u64,
            ),
            xray_prescale_max_dimension: self
                .config
                .performance
                .xray
                .prescale_max_dimension
                .max(512),
            xray_generate_mipmaps: self.config.performance.xray.generate_mipmaps,
            cava_handle: None,
            // cava_tx kept on the channel for restart,
            // no need to store on AppState
            last_cava_config_hash: self.last_cava_config_hash,
            gradient_dir: match self.config.colors.gradient_direction {
                crate::app_config::GradientDirection::BottomToTop => 0.0,
                crate::app_config::GradientDirection::TopToBottom => 1.0,
                crate::app_config::GradientDirection::LeftToRight => 2.0,
                crate::app_config::GradientDirection::RightToLeft => 3.0,
            },
        };
        app_state.xray_animator.on_wallpaper_change(
            app_state.current_wallpaper_path.as_ref(),
            &app_state.xray_config,
        );

        let event_tick = Duration::from_millis(10);

        for output in app_state.output_state.outputs() {
            if let Err(e) = app_state.ensure_output(&output) {
                error!("Failed to create initial output: {}", e);
            }
        }

        event_loop.run(Some(event_tick), &mut app_state, |state| {
            if let Ok(new_framerate) = state.framerate_receiver.try_recv() {
                state.framerate = new_framerate;
                info!("Framerate updated dynamically to {}", new_framerate);
            }

            while let Ok(new_config) = state.config_update_receiver.try_recv() {
                state.apply_runtime_config(new_config);
            }

            // Rebuild surfaces whose Wayland layer changed
            let mut rebuilt: Vec<String> = Vec::new();
            for (output_name, output_state) in &state.per_output {
                if output_state.needs_rebuild {
                    rebuilt.push(output_name.clone());
                }
            }
            for output_name in rebuilt {
                if let Err(e) = state.rebuild_output_surface(&output_name) {
                    error!("Failed to rebuild surface for {}: {}", output_name, e);
                }
            }

            while let Ok(color_update) = state.color_update_receiver.try_recv() {
                state.apply_color_update(color_update);
            }

            if !state.running.load(Ordering::SeqCst) {
                std::process::exit(0);
            }

            if let Ok(new_colors) = state.color_receiver.try_recv() {
                // Only apply wallpaper-extracted colors if extract_from_wallpaper is enabled.
                // Also apply as fallback when parallax/xray have no wallpaper to load.
                let should_use = state.base_config.colors.extract_from_wallpaper
                    || (state.current_wallpaper_path.is_none()
                        && state.last_loaded_wallpaper_path.is_none());

                if should_use {
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
                                state
                                    .hidden_image_config
                                    .as_ref()
                                    .map(|c| c.effect)
                                    .unwrap_or_default(),
                                output_state.hidden_texture_size.0 as f32,
                                output_state.hidden_texture_size.1 as f32,
                                output_state.crop_scale,
                                output_state.crop_offset,
                                state.gradient_dir,
                                state.base_config.colors.use_gradient,
                            );
                            output_state.wgpu_queue.write_buffer(
                                &output_state.uniform_buffer,
                                0,
                                bytemuck::cast_slice(&[uniforms]),
                            );
                        }
                    }
                }
            }

            // Check for wallpaper changes (X-Ray or Parallax)
            if state.use_wallpaper_image
                || (state.parallax_system.is_some()
                    && state.base_config.parallax.profile_source == ProfileSource::FromWallpaper)
            {
                if let Ok(Some(new_path)) = state.wallpaper_path_receiver.try_recv() {
                    if state.current_wallpaper_path.as_ref() == Some(&new_path)
                        && state.last_loaded_wallpaper_path.as_ref() == Some(&new_path)
                    {
                        state.draw();
                        return;
                    }
                    info!("Wallpaper changed to {:?}", new_path);
                    state.current_wallpaper_path = Some(new_path.clone());
                    state.xray_animator.on_wallpaper_change(
                        state.current_wallpaper_path.as_ref(),
                        &state.xray_config,
                    );

                    for output_state in state.per_output.values_mut() {
                        if !output_state.configured {
                            continue;
                        }

                        output_state.xray_animator.on_wallpaper_change(
                            state.current_wallpaper_path.as_ref(),
                            &output_state.effective_config.xray,
                        );

                        let output_hidden_cfg = output_state.effective_config.hidden_image.clone();
                        let use_wallpaper = output_hidden_cfg
                            .as_ref()
                            .map(|cfg| cfg.use_wallpaper)
                            .unwrap_or(false);
                        if !use_wallpaper {
                            continue;
                        }

                        let Some(load_path) = resolve_xray_path(
                            &new_path,
                            &output_hidden_cfg,
                            state.xray_config.images_dir.as_deref(),
                        ) else {
                            warn!(
                                "No matching Xray image for wallpaper {:?}, disabling hidden image",
                                new_path
                            );
                            // Reset to dummy texture so cava renders normally
                            let (dummy_tex, dummy_view, _, _) = AppState::create_dummy_texture(
                                &output_state.wgpu_device,
                                &output_state.wgpu_queue,
                            );
                            output_state._hidden_image_texture = Some(dummy_tex);
                            output_state._hidden_image_view = Some(dummy_view);
                            output_state.hidden_texture_size = (1, 1);
                            output_state.hidden_image_loaded = false;
                            continue;
                        };

                        if let Err(e) = AppState::update_hidden_image_texture(
                            output_state,
                            &load_path,
                            output_state
                                .effective_config
                                .performance
                                .xray
                                .prescale_max_dimension
                                .max(512),
                            output_state
                                .effective_config
                                .performance
                                .xray
                                .generate_mipmaps,
                        ) {
                            error!("Failed to update hidden image texture: {}", e);
                            continue;
                        }
                        output_state.hidden_image_loaded = true;
                    }
                    if let Some(parallax_system) = state.parallax_system.as_mut() {
                        let wp_name = new_path
                            .file_stem()
                            .and_then(|s| s.to_str())
                            .map(|s| s.to_string());
                        info!(
                            "[WALLPAPER] Calling parallax on_wallpaper_change, name={:?}",
                            wp_name
                        );
                        if let Err(e) = parallax_system.on_wallpaper_change(wp_name) {
                            warn!("Failed to rebuild parallax layers after wallpaper change: {e}");
                        }
                        // Clear cached GPU layers so they get recreated
                        for output_state in state.per_output.values_mut() {
                            output_state.parallax_layers_gpu.clear();
                        }
                    }
                    state.last_loaded_wallpaper_path = Some(new_path);
                }
            }

            state.draw();
        })?;

        Ok(())
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
pub struct ColorUpdatePayload {
    pub colors: Vec<[f32; 4]>,
    pub bar_alpha: Option<f32>,
}

#[derive(Clone, Debug)]
struct CavaFramePacket {
    bars: Vec<f32>,
    peak: f32,
    silent: bool,
}

fn resolve_xray_path(
    wallpaper_path: &Path,
    _config: &Option<HiddenImageConfig>,
    xray_images_dir: Option<&str>,
) -> Option<PathBuf> {
    if let Some(xray_dir) = xray_images_dir {
        let xray_dir_path = PathBuf::from(xray_dir);
        if let Some(file_name) = wallpaper_path.file_name() {
            let xray_path = xray_dir_path.join(file_name);
            if xray_path.exists() {
                info!("Found xray image: {:?}", xray_path);
                return Some(xray_path);
            }
        }
    }
    None
}

/// Validate whether the hidden image feature should actually be active.
/// Returns `true` only if:
/// - `config.hidden_image` is Some
/// - When `use_wallpaper=true`: a counterpart with the wallpaper filename exists in xray_images_dir
/// - When `use_wallpaper=false`: the explicit `path` exists
fn validate_hidden_image_available(
    config: &Option<HiddenImageConfig>,
    wallpaper_path: Option<&PathBuf>,
    xray_images_dir: Option<&str>,
) -> bool {
    let Some(cfg) = config else { return false };

    if cfg.use_wallpaper {
        // Wallpaper-relative mode: need a counterpart in xray_images_dir.
        // The explicit path is NOT a fallback — if there's no counterpart,
        // cava should render normally without a hidden image.
        if let Some(wp) = wallpaper_path {
            return resolve_xray_path(wp, config, xray_images_dir).is_some();
        }
        // Wallpaper not detected yet → be optimistic (might arrive later via monitor)
        return true;
    }

    // Direct path mode
    if let Some(path) = &cfg.path {
        return PathBuf::from(path).exists();
    }

    false
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    seat_state: SeatState,
    pointer_devices: Vec<wl_pointer::WlPointer>,
    layer_shell: LayerShell,
    compositor: CompositorState,
    base_config: Config,
    per_output: HashMap<String, PerOutputState>,
    next_output_index: u32,
    runtime_outputs_path: Option<PathBuf>,
    bar_gap: f32,
    bar_alpha: f32,
    height_scale: f32,
    preferred_output_names: Vec<String>,
    render_layer_choice: LayerChoice,
    cava_data_receiver: Receiver<CavaFramePacket>,
    current_bar_heights: Vec<f32>,
    last_cava_peak: f32,
    cava_frame_counter: usize,
    is_idle: bool,
    last_audio_active_at: Instant,
    idle_transition_start: Option<Instant>,
    idle_audio_threshold: f32,
    idle_timeout: Duration,
    idle_fps: f64,
    idle_mode_enabled: bool,
    colors: Vec<[f32; 4]>,
    conn: Connection,
    qh: QueueHandle<Self>,
    running: Arc<AtomicBool>,
    color_receiver: Receiver<Vec<[f32; 4]>>,
    framerate: f64,
    framerate_receiver: Receiver<f64>,
    config_update_receiver: Receiver<Config>,
    color_update_receiver: Receiver<ColorUpdatePayload>,
    hidden_image_config: Option<HiddenImageConfig>,
    use_hidden_image: bool,
    use_wallpaper_image: bool,
    bar_shape: BarShape,
    corner_radius: f32,
    corner_segments: u32,
    xray_config: XRayConfig,
    xray_animator: XRayAnimator,
    parallax_system: Option<ParallaxSystem>,
    /// Cursor position from hyprctl (works across windows)
    cursor_norm: (f32, f32),
    cursor_rx: Option<Receiver<(f32, f32)>>,
    global_mouse_norm: (f32, f32),
    output_mouse_norm: HashMap<String, (f32, f32)>,
    wallpaper_path_receiver: Receiver<Option<PathBuf>>,
    current_wallpaper_path: Option<PathBuf>,
    last_loaded_wallpaper_path: Option<PathBuf>,
    perf_monitor: PerfMonitor,
    telemetry_enabled: bool,
    telemetry_decoder_enabled: bool,
    gradient_dir: f32,
    next_render_at: Instant,
    idle_exit_transition: Duration,
    xray_prescale_max_dimension: u32,
    xray_generate_mipmaps: bool,
    // CAVA process handle for hot-reload
    cava_handle: Option<std::process::Child>,
    last_cava_config_hash: u64,
}

impl AppState {
    /// Starts a background thread that periodically reads the cursor position
    /// via `hyprctl cursorpos`. Works across all windows, not just cava-bg's surface.
    /// Falls back silently if hyprctl isn't available.
    fn start_cursor_monitor(tx: Sender<(f32, f32)>) {
        // Try hyprctl first (Hyprland)
        if Self::try_hyprctl_cursor(tx.clone()).is_some() {
            return;
        }

        // Try wlrctl (wlroots-based compositors: Sway, River, Wayfire, etc.)
        if Self::try_wlrctl_cursor(tx.clone()).is_some() {
            return;
        }

        // Fallback: read from /dev/input/ using evdev-compatible parsing
        // This works with any compositor if the user has input device permissions
        if Self::try_input_device_cursor(tx.clone()).is_some() {
            return;
        }

        info!(
            "[CURSOR] No cursor source available. Install hyprctl (Hyprland) or wlrctl (wlroots)."
        );
    }

    /// Try to get cursor position via `hyprctl cursorpos`
    fn try_hyprctl_cursor(tx: Sender<(f32, f32)>) -> Option<()> {
        let has_hyprctl = std::process::Command::new("which")
            .arg("hyprctl")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !has_hyprctl {
            return None;
        }

        let resolution = std::process::Command::new("hyprctl")
            .args(["monitors", "-j"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    Some(text)
                } else {
                    None
                }
            })
            .and_then(|json| {
                serde_json::from_str::<Vec<serde_json::Value>>(&json)
                    .ok()
                    .and_then(|monitors| {
                        monitors.first().map(|m| {
                            let w = m["width"].as_u64().unwrap_or(1920) as f32;
                            let h = m["height"].as_u64().unwrap_or(1080) as f32;
                            (w, h)
                        })
                    })
            })
            .unwrap_or((1920.0, 1080.0));

        info!(
            "[CURSOR] hyprctl cursor monitor, resolution={}x{}",
            resolution.0, resolution.1
        );

        let (res_w, res_h) = resolution;
        std::thread::spawn(move || loop {
            let output = std::process::Command::new("hyprctl")
                .args(["cursorpos"])
                .output();

            if let Ok(output) = output {
                if output.status.success() {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    if let Some((nx, ny)) = Self::parse_xy_csv(&text, res_w, res_h) {
                        if tx.send((nx, ny)).is_err() {
                            break;
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(16));
        });
        Some(())
    }

    /// Try to get cursor position via `wlrctl pointer` (wlroots compositors)
    fn try_wlrctl_cursor(tx: Sender<(f32, f32)>) -> Option<()> {
        let has_wlrctl = std::process::Command::new("which")
            .arg("wlrctl")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !has_wlrctl {
            return None;
        }

        // Get monitor resolution from environment or swaymsg/hyprctl
        let resolution = std::process::Command::new("swaymsg")
            .args(["-t", "get_outputs"])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    let text = String::from_utf8_lossy(&o.stdout).to_string();
                    Some(text)
                } else {
                    None
                }
            })
            .and_then(|json| {
                serde_json::from_str::<Vec<serde_json::Value>>(&json)
                    .ok()
                    .and_then(|outputs| {
                        outputs.first().map(|m| {
                            let w = m["rect"]["width"].as_u64().unwrap_or(1920) as f32;
                            let h = m["rect"]["height"].as_u64().unwrap_or(1080) as f32;
                            (w, h)
                        })
                    })
            })
            .unwrap_or((1920.0, 1080.0));

        info!(
            "[CURSOR] wlrctl cursor monitor, resolution={}x{}",
            resolution.0, resolution.1
        );

        let (res_w, res_h) = resolution;
        std::thread::spawn(move || {
            loop {
                let output = std::process::Command::new("wlrctl")
                    .args(["pointer"])
                    .output();

                if let Ok(output) = output {
                    let text = String::from_utf8_lossy(&output.stdout).to_string();
                    // wlrctl outputs: "x y" separated by space or tab
                    let parts: Vec<&str> = text.trim().splitn(2, [' ', '\t']).collect();
                    if parts.len() == 2 {
                        let x: f32 = parts[0].trim().parse().unwrap_or(0.0);
                        let y: f32 = parts[1].trim().parse().unwrap_or(0.0);
                        let nx = (x / res_w).clamp(0.0, 1.0);
                        let ny = (y / res_h).clamp(0.0, 1.0);
                        if tx.send((nx, ny)).is_err() {
                            break;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        });
        Some(())
    }

    /// Fallback: read mouse position from /dev/input/ devices
    /// by scanning for absolute-movement devices (touchpads, graphics tablets, etc.)
    /// Parse "x, y" CSV (hyprctl format) into normalized 0-1 coords
    fn parse_xy_csv(text: &str, res_w: f32, res_h: f32) -> Option<(f32, f32)> {
        let parts: Vec<&str> = text.trim().splitn(2, ',').collect();
        if parts.len() == 2 {
            let x: f32 = parts[0].trim().parse().ok()?;
            let y: f32 = parts[1].trim().parse().ok()?;
            Some(((x / res_w).clamp(0.0, 1.0), (y / res_h).clamp(0.0, 1.0)))
        } else {
            None
        }
    }

    fn try_input_device_cursor(_tx: Sender<(f32, f32)>) -> Option<()> {
        // We can't easily read absolute cursor position from evdev without
        // interpreting the protocol and tracking relative motion state.
        // For now, this is a stub that just logs.
        // A full implementation would use `evdev-rs` or `nix` to open
        // input devices and track absolute position from ABS_X/ABS_Y events.
        info!("[CURSOR] Input device fallback not implemented — cursor limited to within-window");
        None
    }

    fn target_frame_duration(&self) -> Duration {
        let fps = if self.is_idle {
            self.idle_fps.max(1.0)
        } else {
            self.framerate.max(1.0)
        };
        Duration::from_secs_f64(1.0 / fps)
    }

    fn update_idle_state(&mut self, had_audio_activity: bool) {
        if !self.idle_mode_enabled {
            self.is_idle = false;
            self.idle_transition_start = None;
            return;
        }

        if had_audio_activity {
            self.last_audio_active_at = Instant::now();
            if self.is_idle {
                self.is_idle = false;
                self.idle_transition_start = Some(Instant::now());
                if self.telemetry_enabled {
                    info!("Idle mode exited");
                }
            }
            return;
        }

        if !self.is_idle && self.last_audio_active_at.elapsed() >= self.idle_timeout {
            self.is_idle = true;
            self.idle_transition_start = None;
            if self.telemetry_enabled {
                info!("Idle mode entered");
            }
        }
    }

    fn should_render_now(&mut self) -> bool {
        let now = Instant::now();
        if now < self.next_render_at {
            return false;
        }
        self.next_render_at = now + self.target_frame_duration();
        true
    }

    fn output_descriptor(&self, output_name: &str, output_index: u32) -> OutputDescriptor {
        OutputDescriptor {
            name: output_name.to_string(),
            connector: Some(output_name.to_string()),
            index: Some(output_index),
        }
    }

    fn persist_runtime_outputs(&self) {
        #[derive(serde::Serialize)]
        struct RuntimeOutputStatus {
            name: String,
            index: u32,
            width: u32,
            height: u32,
            position: [i32; 2],
            configured: bool,
        }

        let Some(path) = &self.runtime_outputs_path else {
            return;
        };

        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }

        let payload = self
            .per_output
            .values()
            .map(|state| RuntimeOutputStatus {
                name: state.output_name.clone(),
                index: state.output_index,
                width: state.width,
                height: state.height,
                position: [state.logical_position.0, state.logical_position.1],
                configured: state.configured,
            })
            .collect::<Vec<_>>();

        match serde_json::to_string_pretty(&payload) {
            Ok(json) => {
                if let Err(err) = std::fs::write(path, json) {
                    warn!(
                        "Failed to write runtime output state {}: {err}",
                        path.display()
                    );
                }
            }
            Err(err) => warn!("Failed to serialize runtime output state: {err}"),
        }
    }

    fn apply_runtime_config(&mut self, new_config: Config) {
        self.base_config = new_config.clone();
        self.bar_gap = new_config.audio.gap;
        self.bar_alpha = new_config.audio.bar_alpha;
        self.height_scale = new_config.audio.height_scale;
        if !new_config.general.dynamic_colors && !new_config.colors.palette.is_empty() {
            self.colors = new_config.colors.palette.clone();
        }

        self.hidden_image_config = new_config.hidden_image.clone();
        self.xray_config = new_config.xray.clone();
        let xray_is_enabled = self.xray_config.enabled;
        self.use_hidden_image = xray_is_enabled
            && validate_hidden_image_available(
                &self.hidden_image_config,
                self.current_wallpaper_path.as_ref(),
                self.xray_config.images_dir.as_deref(),
            );
        self.use_wallpaper_image = self.use_hidden_image
            && self
                .hidden_image_config
                .as_ref()
                .map(|c| c.use_wallpaper)
                .unwrap_or(false);

        // If wallpaper monitoring just became required but wasn't started, start it now.
        // This happens when xray is toggled ON at runtime after init skipped wallpaper monitoring.
        if self.use_wallpaper_image && self.current_wallpaper_path.is_none() {
            if let Some(wp_path) = WallpaperAnalyzer::find_wallpaper() {
                info!("Wallpaper detected on Xray enable: {:?}", wp_path);
                self.current_wallpaper_path = Some(wp_path);
            }
        }
        self.bar_shape = new_config.audio.bar_shape;
        self.corner_radius = new_config.audio.corner_radius;
        self.corner_segments = new_config.audio.corner_segments;

        let new_layer = new_config.display.layer;
        let layer_changed = self.render_layer_choice != new_layer;
        self.render_layer_choice = new_layer;

        if layer_changed {
            info!(
                "Wayland layer changed to {:?}, scheduling surface recreation",
                new_layer
            );
            for state in self.per_output.values_mut() {
                state.draw_layer = new_layer;
                state.needs_rebuild = true;
            }
        }

        self.idle_mode_enabled = new_config.performance.idle_mode.enabled;
        self.idle_audio_threshold = new_config.performance.idle_mode.audio_threshold.max(0.0);
        self.idle_timeout =
            Duration::from_secs_f32(new_config.performance.idle_mode.timeout_seconds.max(0.1));
        self.idle_fps = new_config.performance.idle_mode.idle_fps.max(1) as f64;
        self.idle_exit_transition =
            Duration::from_millis(new_config.performance.idle_mode.exit_transition_ms as u64);
        self.telemetry_enabled =
            new_config.advanced.verbose_logging && new_config.performance.telemetry.enabled;
        self.telemetry_decoder_enabled = new_config.advanced.verbose_logging
            && new_config.performance.video_decoder.debug_telemetry;
        self.perf_monitor.reconfigure(
            new_config.performance.telemetry.metrics_window,
            new_config.performance.telemetry.log_interval_seconds,
            self.telemetry_enabled,
        );
        self.xray_prescale_max_dimension =
            new_config.performance.xray.prescale_max_dimension.max(512);
        self.xray_generate_mipmaps = new_config.performance.xray.generate_mipmaps;

        if new_config.parallax.enabled {
            if let Some(system) = self.parallax_system.as_mut() {
                if let Err(err) = system.set_config(new_config.parallax.clone()) {
                    warn!("Failed to update parallax config: {err:#}");
                    self.parallax_system = None;
                }
            } else {
                match ParallaxSystem::new(new_config.parallax.clone(), 1920, 1080, None) {
                    Ok(system) => self.parallax_system = Some(system),
                    Err(err) => warn!("Failed to initialize parallax system: {err:#}"),
                }
            }
        } else {
            self.parallax_system = None;
        }

        let output_names = self.per_output.keys().cloned().collect::<Vec<_>>();
        for output_name in output_names {
            let descriptor = {
                let Some(current) = self.per_output.get(&output_name) else {
                    continue;
                };
                self.output_descriptor(&output_name, current.output_index)
            };

            let Some(resolved_cfg) = self.base_config.resolve_for_output(&descriptor) else {
                self.per_output.remove(&output_name);
                continue;
            };

            if let Some(state) = self.per_output.get_mut(&output_name) {
                state.effective_config = resolved_cfg;
                state.background_color = array_from_config_color(
                    state.effective_config.general.background_color.clone(),
                );
                state.perf_monitor.reconfigure(
                    state.effective_config.performance.telemetry.metrics_window,
                    state
                        .effective_config
                        .performance
                        .telemetry
                        .log_interval_seconds,
                    state.effective_config.advanced.verbose_logging
                        && state.effective_config.performance.telemetry.enabled,
                );
                state.xray_animator.on_wallpaper_change(
                    self.current_wallpaper_path.as_ref(),
                    &state.effective_config.xray,
                );

                // Re-aplicar display config (anchors, size, margins, exclusive zone) en caliente
                let disp = &state.effective_config.display;
                let anchor = compute_display_anchor(disp);
                let margin_w = disp.margin_left + disp.margin_right;
                let margin_h = disp.margin_top + disp.margin_bottom;
                state.layer_surface.set_anchor(anchor);
                state.layer_surface.set_exclusive_zone(-1);
                let (set_w, set_h) = if disp.width > 0 && disp.height > 0 {
                    if disp.scale_with_resolution {
                        (disp.width.max(1), disp.height.max(1))
                    } else {
                        (
                            state.width.min(disp.width.max(1)),
                            state.height.min(disp.height.max(1)),
                        )
                    }
                } else {
                    (state.width, state.height)
                };
                state.layer_surface.set_size(
                    set_w.saturating_sub(margin_w),
                    set_h.saturating_sub(margin_h),
                );
                state.surface.commit();
            }
        }

        // Hot-reload CAVA if relevant config changed (monstercat, noise_reduction, etc.)
        let new_hash = cava_config_hash(&new_config);
        if new_hash != 0 && new_hash != self.last_cava_config_hash {
            info!("CAVA config changed, restarting cava process");
            if let Some(mut child) = self.cava_handle.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let cava_config_str = build_cava_config(&new_config);
            match Command::new("cava")
                .arg("-p")
                .arg("/dev/stdin")
                .stdout(Stdio::piped())
                .stdin(Stdio::piped())
                .spawn()
            {
                Ok(mut child) => {
                    if let Some(mut stdin) = child.stdin.take() {
                        let _ = stdin.write_all(cava_config_str.as_bytes());
                        let _ = stdin.flush();
                    }
                    // Create new reader thread for the new pipe
                    if let Some(cava_stdout) = child.stdout.take() {
                        let (new_tx, new_rx): (Sender<CavaFramePacket>, Receiver<CavaFramePacket>) =
                            channel();
                        let reader_cava_tx = new_tx.clone();
                        let bar_count = new_config.audio.bar_count as usize;
                        let threshold = new_config.performance.idle_mode.audio_threshold.max(0.0);
                        thread::spawn(move || {
                            let mut reader = BufReader::new(cava_stdout);
                            let mut buffer = vec![0u8; bar_count * 2];
                            loop {
                                match reader.read_exact(&mut buffer) {
                                    Ok(()) => {
                                        let mut bar_heights = vec![0.0f32; bar_count];
                                        let mut peak = 0.0f32;
                                        for (i, chunk) in buffer.chunks_exact(2).enumerate() {
                                            let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                                            let value = (num as f32) / 65530.0;
                                            peak = peak.max(value);
                                            bar_heights[i] = value;
                                        }
                                        let packet = CavaFramePacket {
                                            bars: bar_heights,
                                            peak,
                                            silent: peak < threshold,
                                        };
                                        if reader_cava_tx.send(packet).is_err() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        error!("Error reading cava data: {}", e);
                                        std::thread::sleep(Duration::from_millis(10));
                                    }
                                }
                            }
                        });
                        self.cava_data_receiver = new_rx;
                        // Keep old cava_tx for possible use
                    }
                    self.cava_handle = Some(child);
                    self.last_cava_config_hash = new_hash;
                }
                Err(e) => {
                    error!("Failed to restart cava: {e}");
                }
            }
        }

        // Reload hidden image textures for all outputs when xray/hidden_image config changes.
        // This handles the case where xray is toggled on — the old dummy/texture needs
        // to be replaced with the actual xray image.
        self.reload_hidden_images_for_all_outputs(&new_config);

        self.persist_runtime_outputs();

        info!(
            "Config updated: gap={}, alpha={}, hidden={}, shape={:?}, idle={}, idle_fps={}",
            self.bar_gap,
            self.bar_alpha,
            self.use_hidden_image,
            self.bar_shape,
            new_config.performance.idle_mode.enabled,
            self.idle_fps
        );
    }

    fn rebuild_output_surface(&mut self, output_name: &str) -> anyhow::Result<()> {
        let Some(mut old_state) = self.per_output.remove(output_name) else {
            return Ok(());
        };

        let output = old_state.wl_output.clone();
        let width = old_state.width;
        let height = old_state.height;

        info!(
            "Rebuilding surface for {} on {:?}",
            output_name, old_state.draw_layer
        );

        let surface = self.compositor.create_surface(&self.qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            &self.qh,
            surface.clone(),
            map_layer_choice_to_wlr_layer(old_state.draw_layer),
            Some("cava-visualizer"),
            Some(&output),
        );
        layer_surface.set_input_region(None);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        let disp = &old_state.effective_config.display;
        let anchor = compute_display_anchor(disp);
        let margin_top = disp.margin_top;
        let margin_bottom = disp.margin_bottom;
        let margin_left = disp.margin_left;
        let margin_right = disp.margin_right;
        layer_surface.set_size(
            width.saturating_sub(margin_left + margin_right),
            height.saturating_sub(margin_top + margin_bottom),
        );
        layer_surface.set_anchor(anchor);
        layer_surface.set_exclusive_zone(-1);
        surface.commit();

        // Create new wgpu surface (wgpu::Instance is lightweight, create fresh)
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
        let wl_display_ptr = self.conn.display().id().as_ptr();
        let wl_surface_ptr = surface.id().as_ptr();
        let display_ptr = std::ptr::NonNull::new(wl_display_ptr as *mut std::ffi::c_void).unwrap();
        let surface_ptr = std::ptr::NonNull::new(wl_surface_ptr as *mut std::ffi::c_void).unwrap();
        let display_handle = raw_window_handle::WaylandDisplayHandle::new(display_ptr);
        let window_handle = raw_window_handle::WaylandWindowHandle::new(surface_ptr);

        let wgpu_surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::RawHandle {
                raw_display_handle: raw_window_handle::RawDisplayHandle::Wayland(display_handle),
                raw_window_handle: raw_window_handle::RawWindowHandle::Wayland(window_handle),
            })
        }
        .context("Failed to recreate wgpu surface")?;

        let wgpu_surface_static: wgpu::Surface<'static> =
            unsafe { std::mem::transmute(wgpu_surface) };

        let surface_config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: old_state.wgpu_config.format,
            width: old_state.wgpu_config.width,
            height: old_state.wgpu_config.height,
            present_mode: old_state.wgpu_config.present_mode,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        wgpu_surface_static.configure(&old_state.wgpu_device, &surface_config);

        old_state.surface = surface;
        old_state.layer_surface = layer_surface;
        old_state.wgpu_surface = wgpu_surface_static;
        old_state.wgpu_config = surface_config;
        old_state.configured = true;
        old_state.needs_rebuild = false;

        self.per_output.insert(output_name.to_string(), old_state);
        info!("Surface rebuilt for {}", output_name);
        Ok(())
    }

    fn apply_color_update(&mut self, upd: ColorUpdatePayload) {
        info!(
            "Applying color update from socket: {} colors, alpha={:?}",
            upd.colors.len(),
            upd.bar_alpha
        );
        if !upd.colors.is_empty() {
            self.colors = upd.colors;
        }
        if let Some(alpha) = upd.bar_alpha {
            self.bar_alpha = alpha;
        }
    }

    fn create_dummy_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
        let texture_size = wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        };
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
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(4),
                rows_per_image: Some(1),
            },
            texture_size,
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        (texture, view, 1, 1)
    }

    #[allow(clippy::too_many_arguments)]
    fn load_or_dummy_texture(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        config: &Option<HiddenImageConfig>,
        use_wallpaper: bool,
        wallpaper_path: Option<&PathBuf>,
        target_width: u32,
        target_height: u32,
        prescale_max_dimension: u32,
        generate_mipmaps: bool,
        xray_images_dir: Option<&str>,
    ) -> (
        wgpu::Texture,
        wgpu::TextureView,
        u32,
        u32,
        Option<VideoDecoder>,
    ) {
        let load_path = if use_wallpaper {
            // Wallpaper-relative mode: look for a counterpart in xray_images_dir
            // using the same filename as the current wallpaper.
            // No fallback to explicit path — if no counterpart is found,
            // return a dummy texture so cava renders normally.
            wallpaper_path.and_then(|path| resolve_xray_path(path, config, xray_images_dir))
        } else {
            // Direct image mode: use the explicit path from config, or
            // search the images_dir for a counterpart (no wallpaper involved).
            let from_config = config
                .as_ref()
                .and_then(|img_config| img_config.path.as_ref().map(PathBuf::from));
            from_config.or({
                // No explicit path set; still try xray_images_dir for any candidates?
                None
            })
        };

        let Some(load_path) = load_path else {
            let (t, v, w, h) = Self::create_dummy_texture(device, queue);
            return (t, v, w, h, None);
        };

        if is_video_media_path(&load_path) {
            match VideoDecoder::new(
                &load_path,
                VideoDecoderConfig {
                    target_width,
                    target_height,
                    looping: true,
                    max_buffered_frames: 6,
                    frame_cache_size: 120,
                },
            ) {
                Ok(mut decoder) => {
                    let frame = decoder
                        .poll_latest_for_time(0.0)
                        .or_else(|| decoder.last_frame());
                    if let Some(frame) = frame {
                        let (texture, view) = create_texture_from_rgba_frame(device, queue, &frame);
                        info!(
                            "Loaded hidden video texture {:?} ({}x{})",
                            load_path, frame.width, frame.height
                        );
                        return (texture, view, frame.width, frame.height, Some(decoder));
                    }
                    let (t, v, w, h) = Self::create_dummy_texture(device, queue);
                    return (t, v, w, h, Some(decoder));
                }
                Err(e) => {
                    warn!("Failed to initialize hidden video {:?}: {}", load_path, e);
                }
            }
        }

        match WaylandRenderer::load_hidden_image(
            device,
            queue,
            &load_path,
            target_width,
            target_height,
            prescale_max_dimension,
            generate_mipmaps,
        ) {
            Ok((t, v, w, h)) => {
                info!("Loaded hidden texture {:?} ({}x{})", load_path, w, h);
                (t, v, w, h, None)
            }
            Err(e) => {
                warn!("Failed to load image {:?}: {}, using dummy", load_path, e);
                let (t, v, w, h) = Self::create_dummy_texture(device, queue);
                (t, v, w, h, None)
            }
        }
    }

    /// Reload hidden image textures for all outputs after a config change.
    /// Called from [`apply_runtime_config`] when hidden_image/xray config changes.
    fn reload_hidden_images_for_all_outputs(&mut self, new_config: &Config) {
        info!(
            "reload_hidden_images: xray.enabled={}, wallpaper_path={:?}",
            new_config.xray.enabled, self.current_wallpaper_path
        );

        let xray_is_enabled = new_config.xray.enabled;
        let use_hidden = xray_is_enabled
            && validate_hidden_image_available(
                &new_config.hidden_image,
                self.current_wallpaper_path.as_ref(),
                new_config.xray.images_dir.as_deref(),
            );

        if !use_hidden {
            // Xray disabled — load dummy textures for all outputs
            for state in self.per_output.values_mut() {
                let (tex, view, _, _, _) = Self::load_or_dummy_texture(
                    &state.wgpu_device,
                    &state.wgpu_queue,
                    &None,
                    false,
                    None,
                    state.width,
                    state.height,
                    self.xray_prescale_max_dimension,
                    self.xray_generate_mipmaps,
                    None,
                );
                state._hidden_image_texture = Some(tex);
                state._hidden_image_view = Some(view);
                state.hidden_image_loaded = false;
            }
            return;
        }

        // Xray enabled — figure out which path to load
        // Priority:
        //   1. If use_wallpaper=true: look for counterpart in xray_images_dir
        //   2. Otherwise: use the explicit hidden_image.path
        let load_path: Option<PathBuf> = new_config.hidden_image.as_ref().and_then(|cfg| {
            if cfg.use_wallpaper {
                // Wallpaper-relative: only counterpart search, no path fallback
                self.current_wallpaper_path.as_ref().and_then(|wp| {
                    resolve_xray_path(
                        wp,
                        &new_config.hidden_image,
                        new_config.xray.images_dir.as_deref(),
                    )
                })
            } else {
                // Direct path mode
                cfg.path.as_ref().map(PathBuf::from)
            }
        });

        let Some(ref load_path) = load_path else {
            warn!("Xray enabled but no hidden image path available (wallpaper={:?}, hidden.path={:?})",
                self.current_wallpaper_path,
                new_config.hidden_image.as_ref().and_then(|c| c.path.as_ref()));
            return;
        };

        // Reload texture for every configured output
        for state in self.per_output.values_mut() {
            if !state.configured {
                continue;
            }
            if let Err(e) = Self::update_hidden_image_texture(
                state,
                load_path,
                self.xray_prescale_max_dimension,
                self.xray_generate_mipmaps,
            ) {
                warn!(
                    "Failed to reload hidden image texture for {}: {e}",
                    state.output_name
                );
            } else {
                info!(
                    "Hidden image texture reloaded for {} from {:?}",
                    state.output_name, load_path
                );
            }

            // After successful reload, mark the texture as loaded
            state.hidden_image_loaded = true;
        }
    }

    fn update_hidden_image_texture(
        output_state: &mut PerOutputState,
        path: &Path,
        prescale_max_dimension: u32,
        generate_mipmaps: bool,
    ) -> Result<()> {
        let (new_texture, new_view, width, height, new_decoder_opt) = if is_video_media_path(path) {
            let mut decoder = VideoDecoder::new(
                path,
                VideoDecoderConfig {
                    target_width: output_state.width,
                    target_height: output_state.height,
                    looping: true,
                    max_buffered_frames: 6,
                    frame_cache_size: 120,
                },
            )
            .with_context(|| format!("Failed to initialize hidden video {}", path.display()))?;

            let first = decoder
                .poll_latest_for_time(0.0)
                .or_else(|| decoder.last_frame())
                .ok_or_else(|| anyhow::anyhow!("Hidden video has no decodable frames"))?;
            let (texture, view) = create_texture_from_rgba_frame(
                &output_state.wgpu_device,
                &output_state.wgpu_queue,
                &first,
            );
            (texture, view, first.width, first.height, Some(decoder))
        } else {
            let (texture, view, w, h) = WaylandRenderer::load_hidden_image(
                &output_state.wgpu_device,
                &output_state.wgpu_queue,
                path,
                output_state.width,
                output_state.height,
                prescale_max_dimension,
                generate_mipmaps,
            )?;
            (texture, view, w, h, None)
        };

        let sampler = output_state
            .wgpu_device
            .create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

        let bind_group_layout =
            output_state
                .wgpu_device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("Hidden Image GPU Bind Group Layout"),
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

        let new_bind_group =
            output_state
                .wgpu_device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Hidden Image GPU Bind Group"),
                    layout: &bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&new_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                    ],
                });

        output_state._hidden_image_texture = Some(new_texture);
        output_state._hidden_image_view = Some(new_view);
        output_state.hidden_image_bind_group = new_bind_group;
        output_state.hidden_texture_size = (width, height);
        let (crop_scale, crop_offset) = compute_preserve_aspect_crop_transform(
            output_state.width as f32,
            output_state.height as f32,
            width as f32,
            height as f32,
        );
        output_state.crop_scale = crop_scale;
        output_state.crop_offset = crop_offset;
        output_state.hidden_video_decoder = new_decoder_opt;

        Ok(())
    }

    fn create_parallax_gpu_layer(
        output_state: &mut PerOutputState,
        layer: &ComputedParallaxLayer,
    ) -> Result<ParallaxGpuLayer> {
        let frame = layer
            .frame
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Missing frame for parallax layer {}", layer.id))?;

        let texture_size = wgpu::Extent3d {
            width: frame.width.max(1),
            height: frame.height.max(1),
            depth_or_array_layers: 1,
        };

        let texture = output_state
            .wgpu_device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Parallax Layer Texture"),
                size: texture_size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

        output_state.wgpu_queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            texture_size,
        );

        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = output_state
            .wgpu_device
            .create_sampler(&wgpu::SamplerDescriptor {
                address_mode_u: wgpu::AddressMode::ClampToEdge,
                address_mode_v: wgpu::AddressMode::ClampToEdge,
                address_mode_w: wgpu::AddressMode::ClampToEdge,
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                mipmap_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            });

        let bind_group = output_state
            .wgpu_device
            .create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Parallax Layer Bind Group"),
                layout: &output_state.parallax_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });

        // Per-layer uniform buffer so each layer can have independent displacement
        let uniform_buffer = output_state
            .wgpu_device
            .create_buffer(&wgpu::BufferDescriptor {
                label: Some("Parallax Layer Uniform Buffer"),
                size: std::mem::size_of::<ParallaxUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });

        let uniform_bind_group =
            output_state
                .wgpu_device
                .create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("Parallax Layer Uniform Bind Group"),
                    layout: &output_state.parallax_uniform_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: uniform_buffer.as_entire_binding(),
                    }],
                });

        Ok(ParallaxGpuLayer {
            texture,
            _view: view,
            bind_group,
            dimensions: (frame.width, frame.height),
            last_sequence: layer.frame.as_ref().map(|f| f.sequence),
            uniform_buffer,
            uniform_bind_group,
        })
    }

    fn upload_parallax_frame_if_needed(
        queue: &wgpu::Queue,
        gpu_layer: &mut ParallaxGpuLayer,
        layer: &ComputedParallaxLayer,
    ) {
        let Some(frame) = &layer.frame else {
            return;
        };

        let seq = Some(frame.sequence);
        if gpu_layer.last_sequence == seq && gpu_layer.dimensions == (frame.width, frame.height) {
            return;
        }

        if gpu_layer.dimensions != (frame.width, frame.height) {
            return;
        }

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &gpu_layer.texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.rgba,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(frame.width * 4),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        gpu_layer.last_sequence = seq;
    }

    fn ensure_output(&mut self, output: &wl_output::WlOutput) -> Result<()> {
        let info = match self.output_state.info(output) {
            Some(info) => info,
            None => {
                debug!("Output info not yet available");
                return Ok(());
            }
        };
        let name = info.name.clone().unwrap_or_else(|| "unknown".to_string());

        if self.per_output.contains_key(&name) {
            return Ok(());
        }
        if !self.preferred_output_names.is_empty() && !self.preferred_output_names.contains(&name) {
            debug!("Skipping monitor {} (not in preferred list)", name);
            return Ok(());
        }

        let output_index = self.next_output_index;
        self.next_output_index = self.next_output_index.saturating_add(1);
        let descriptor = self.output_descriptor(&name, output_index);
        let Some(effective_config) = self.base_config.resolve_for_output(&descriptor) else {
            info!("Output {} is disabled by per-output configuration", name);
            return Ok(());
        };

        info!("Creating surface for output {}", name);
        let surface = self.compositor.create_surface(&self.qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            &self.qh,
            surface.clone(),
            map_layer_choice_to_wlr_layer(effective_config.display.layer),
            Some("cava-visualizer"),
            Some(output),
        );

        // Permitir clics a través (input passthrough)
        layer_surface.set_input_region(None);
        layer_surface.set_keyboard_interactivity(KeyboardInteractivity::None);

        let logical_size = info.logical_size.unwrap_or((1920, 1080));
        let logical_position = info.logical_position.unwrap_or((0, 0));
        let mon_width = logical_size.0 as u32;
        let mon_height = logical_size.1 as u32;

        let disp = &effective_config.display;

        // Build anchor mask using position-aware helper
        let anchor = compute_display_anchor(disp);

        // Compute surface size respecting display config
        let (width, height) = if disp.width > 0 && disp.height > 0 {
            if disp.scale_with_resolution {
                (disp.width.max(1), disp.height.max(1))
            } else {
                (
                    mon_width.min(disp.width.max(1)),
                    mon_height.min(disp.height.max(1)),
                )
            }
        } else {
            (mon_width, mon_height)
        };
        let margin_top = disp.margin_top;
        let margin_bottom = disp.margin_bottom;
        let margin_left = disp.margin_left;
        let margin_right = disp.margin_right;

        layer_surface.set_size(
            width.saturating_sub(margin_left + margin_right),
            height.saturating_sub(margin_top + margin_bottom),
        );
        layer_surface.set_anchor(anchor);
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
        }
        .context("Failed to create WGPU surface (wgpu)")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&wgpu_surface),
            force_fallback_adapter: false,
        }))
        .context("Failed to find suitable GPU adapter (wgpu)")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .context("Failed to create wgpu device")?;

        let surface_caps = wgpu_surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| {
                matches!(
                    f,
                    wgpu::TextureFormat::Bgra8UnormSrgb | wgpu::TextureFormat::Rgba8UnormSrgb
                )
            })
            .unwrap_or(surface_caps.formats[0]);

        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|&m| {
                m == wgpu::CompositeAlphaMode::Auto || m == wgpu::CompositeAlphaMode::PreMultiplied
            })
            .or_else(|| surface_caps.alpha_modes.first().copied())
            .unwrap_or(wgpu::CompositeAlphaMode::Opaque);

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

        let output_hidden_image_cfg = effective_config.hidden_image.clone();
        let use_wallpaper_image = output_hidden_image_cfg
            .as_ref()
            .map(|cfg| cfg.use_wallpaper)
            .unwrap_or(false);

        let (hidden_texture, hidden_image_view, tex_width, tex_height, hidden_video_decoder) =
            Self::load_or_dummy_texture(
                &device,
                &queue,
                &output_hidden_image_cfg,
                use_wallpaper_image,
                self.current_wallpaper_path.as_ref(),
                width,
                height,
                effective_config
                    .performance
                    .xray
                    .prescale_max_dimension
                    .max(512),
                effective_config.performance.xray.generate_mipmaps,
                effective_config.xray.images_dir.as_deref(),
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

        let bind_group_layout1 =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Hidden Image GPU Bind Group Layout"),
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
            label: Some("Hidden Image GPU Bind Group"),
            layout: &bind_group_layout1,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&hidden_image_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Bars WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });
        let parallax_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Parallax WGSL Shader"),
            source: wgpu::ShaderSource::Wgsl(PARALLAX_SHADER_WGSL.into()),
        });

        let output_bar_count = effective_config.audio.bar_count.max(1) as usize;
        let mut all_indices = Vec::with_capacity(output_bar_count * 6);
        for i in 0..output_bar_count {
            let base = (i * 4) as u16;
            all_indices.extend_from_slice(&[
                base,
                base + 1,
                base + 2,
                base + 1,
                base + 3,
                base + 2,
            ]);
        }
        // Compute adequate buffer size for the worst-case visualization mode:
        // - Bars/Blocks/InvertedBars: each bar = vertices_per_bar
        // - MirrorBars: 2x bars (mirrored)
        // - Waveform: each bar = 6 verts (quad as 2 tris)
        // - Spectrum: each segment = 6 verts (thick line), with interpolation 2x
        // - Ring: each slice = 6 verts (quad)
        // - Radial/Spiral (deprecated, kept for safety): each = 6 verts (quad)
        let max_verts_per_bar = bar_geometry::vertices_per_bar(
            effective_config.audio.bar_shape,
            effective_config.audio.corner_segments,
        );
        // Worst case: MirrorBars (2x bars) or Spectrum (2x interpolation)
        let worst_case_verts = (output_bar_count * max_verts_per_bar)
            .max(
                output_bar_count * 2 * 6, // Spectrum: 2x interpolated segments, 6 verts each
            )
            .max(
                output_bar_count * 2 * max_verts_per_bar, // MirrorBars: 2x bars
            );
        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Bars GPU Vertex Buffer"),
            size: (worst_case_verts * 4 * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let effect = output_hidden_image_cfg
            .as_ref()
            .map(|c| c.effect)
            .unwrap_or_default();
        let output_colors = if effective_config.general.dynamic_colors
            || effective_config.colors.extract_from_wallpaper
        {
            self.colors.clone()
        } else if !effective_config.colors.palette.is_empty() {
            if effective_config.colors.palette.len() == 1 {
                effective_config.colors.palette.clone()
            } else {
                // always send all palette colors; shader handles gradient vs flat via use_gradient flag
                effective_config.colors.palette.clone()
            }
        } else {
            self.colors.clone()
        };
        let (crop_scale, crop_offset) = compute_preserve_aspect_crop_transform(
            width as f32,
            height as f32,
            tex_width as f32,
            tex_height as f32,
        );
        let gradient_dir = match effective_config.colors.gradient_direction {
            crate::app_config::GradientDirection::BottomToTop => 0.0,
            crate::app_config::GradientDirection::TopToBottom => 1.0,
            crate::app_config::GradientDirection::LeftToRight => 2.0,
            crate::app_config::GradientDirection::RightToLeft => 3.0,
        };
        let uniforms = Uniforms::new(
            &output_colors,
            width as f32,
            height as f32,
            effective_config.audio.bar_alpha,
            self.use_hidden_image,
            effect,
            tex_width as f32,
            tex_height as f32,
            crop_scale,
            crop_offset,
            gradient_dir,
            effective_config.colors.use_gradient,
        );
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Bars GPU Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout0 =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Bars GPU Uniform Bind Group Layout"),
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
            label: Some("Bars GPU Uniform Bind Group"),
            layout: &bind_group_layout0,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let bars_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Bars GPU Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout0, &bind_group_layout1],
            push_constant_ranges: &[],
        });

        let bar_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Bars GPU Render Pipeline"),
            layout: Some(&bars_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (4 * std::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 0,
                            shader_location: 0,
                        },
                        wgpu::VertexAttribute {
                            format: wgpu::VertexFormat::Float32x2,
                            offset: 8,
                            shader_location: 1,
                        },
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

        let parallax_uniform = ParallaxUniform {
            translation_ndc: [0.0, 0.0],
            scale: 1.0,
            rotation_rad: 0.0,
            opacity: 1.0,
            _pad: 0.0,
            crop_scale: [1.0, -1.0],
            crop_offset: [0.0, 1.0],
        };
        let parallax_uniform_buffer =
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Parallax GPU Uniform Buffer"),
                contents: bytemuck::cast_slice(&[parallax_uniform]),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let parallax_uniform_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Parallax GPU Uniform Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let parallax_uniform_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Parallax Unified Bind Group"),
            layout: &parallax_uniform_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: parallax_uniform_buffer.as_entire_binding(),
            }],
        });

        let parallax_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Parallax GPU Pipeline Layout"),
                bind_group_layouts: &[&parallax_uniform_layout, &bind_group_layout1],
                push_constant_ranges: &[],
            });

        let make_parallax_pipeline = |label: &str, blend: wgpu::BlendState| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&parallax_pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &parallax_shader,
                    entry_point: "vs_main",
                    buffers: &[wgpu::VertexBufferLayout {
                        array_stride: (4 * std::mem::size_of::<f32>()) as u64,
                        step_mode: wgpu::VertexStepMode::Vertex,
                        attributes: &[
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 0,
                                shader_location: 0,
                            },
                            wgpu::VertexAttribute {
                                format: wgpu::VertexFormat::Float32x2,
                                offset: 8,
                                shader_location: 1,
                            },
                        ],
                    }],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &parallax_shader,
                    entry_point: "fs_main",
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_config.format,
                        blend: Some(blend),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
            })
        };

        let parallax_pipeline_normal = make_parallax_pipeline(
            "Parallax GPU Pipeline Normal",
            wgpu::BlendState::ALPHA_BLENDING,
        );
        let parallax_pipeline_add = make_parallax_pipeline(
            "Parallax GPU Pipeline Add",
            wgpu::BlendState {
                color: wgpu::BlendComponent {
                    src_factor: wgpu::BlendFactor::One,
                    dst_factor: wgpu::BlendFactor::One,
                    operation: wgpu::BlendOperation::Add,
                },
                alpha: wgpu::BlendComponent::OVER,
            },
        );

        let quad_vertices: [f32; 16] = [
            -1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.0, -1.0, 1.0, 0.0, 0.0,
        ];
        let quad_indices: [u16; 6] = [0, 1, 2, 0, 2, 3];
        let parallax_vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Parallax Quad Vertex Buffer GPU"),
            contents: bytemuck::cast_slice(&quad_vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let parallax_index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Parallax Quad Index Buffer GPU"),
            contents: bytemuck::cast_slice(&quad_indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let wgpu_surface_static: wgpu::Surface<'static> =
            unsafe { std::mem::transmute(wgpu_surface) };

        let mut output_perf_monitor = PerfMonitor::new(
            effective_config.performance.telemetry.metrics_window,
            effective_config.performance.telemetry.log_interval_seconds,
            effective_config.advanced.verbose_logging
                && effective_config.performance.telemetry.enabled,
        );
        output_perf_monitor.record(Duration::from_millis(0));

        let mut xray_animator = XRayAnimator::new();
        xray_animator
            .on_wallpaper_change(self.current_wallpaper_path.as_ref(), &effective_config.xray);

        let state = PerOutputState {
            output_name: name.clone(),
            output_index,
            wl_output: output.clone(),
            logical_position,
            surface,
            layer_surface,
            wgpu_surface: wgpu_surface_static,
            wgpu_device: device,
            wgpu_queue: queue,
            wgpu_config: surface_config,
            bar_render_pipeline,
            bind_group0,
            uniform_buffer,
            vertex_buffer,
            width,
            height,
            configured: false,
            needs_rebuild: false,
            background_color: array_from_config_color(
                effective_config.general.background_color.clone(),
            ),
            hidden_image_bind_group,
            hidden_image_loaded: false, // determined below
            _hidden_image_texture: Some(hidden_texture),
            _hidden_image_view: Some(hidden_image_view),
            hidden_texture_size: (tex_width, tex_height),
            hidden_video_decoder,

            crop_scale,
            crop_offset,
            parallax_bind_group_layout: bind_group_layout1,
            parallax_uniform_buffer,
            parallax_uniform_bind_group,
            parallax_uniform_layout,
            parallax_pipeline_normal,
            parallax_pipeline_add,
            parallax_vertex_buffer,
            parallax_index_buffer,
            parallax_layers_gpu: HashMap::new(),
            draw_layer: effective_config.display.layer,
            effective_config,
            xray_animator,
            perf_monitor: output_perf_monitor,
        };

        // Check if this is a real hidden image (not the 1x1 dummy).
        // Only mark as loaded if the texture is larger than the dummy placeholder.
        let is_real_hidden_image = tex_width > 1 && tex_height > 1;

        self.per_output.insert(name.clone(), state);

        // hidden_image_loaded was initialized to false; set it based on actual texture size
        if is_real_hidden_image {
            if let Some(s) = self.per_output.get_mut(&name) {
                s.hidden_image_loaded = true;
            }
        }

        self.persist_runtime_outputs();
        info!("WGPU surface created for {}: {}x{}", name, width, height);
        Ok(())
    }

    fn draw(&mut self) {
        let frame_start = Instant::now();

        let mut had_activity = false;
        while let Ok(packet) = self.cava_data_receiver.try_recv() {
            self.current_bar_heights = packet.bars;
            self.last_cava_peak = packet.peak;
            had_activity = had_activity || !packet.silent;
            self.cava_frame_counter += 1;
        }

        self.update_idle_state(had_activity || self.last_cava_peak >= self.idle_audio_threshold);

        if !self.should_render_now() {
            return;
        }

        let mut parallax_layers_by_output: HashMap<String, Vec<ComputedParallaxLayer>> =
            HashMap::new();
        if let Some(parallax_system) = self.parallax_system.as_mut() {
            // Read external cursor position (works across windows)
            while let Some(rx) = self.cursor_rx.as_ref() {
                if let Ok(pos) = rx.try_recv() {
                    self.cursor_norm = pos;
                } else {
                    break;
                }
            }

            // Prefer external cursor if available, otherwise fall back to within-window tracking
            if self.cursor_norm != (0.5, 0.5) || self.output_mouse_norm.is_empty() {
                self.global_mouse_norm = self.cursor_norm;
            }

            if !self.output_mouse_norm.is_empty() {
                let (sx, sy, n) = self
                    .output_mouse_norm
                    .values()
                    .fold((0.0, 0.0, 0usize), |acc, p| {
                        (acc.0 + p.0, acc.1 + p.1, acc.2 + 1)
                    });
                if n > 0 {
                    self.global_mouse_norm = (sx / n as f32, sy / n as f32);
                }
            }

            parallax_system.set_mouse_global(self.global_mouse_norm.0, self.global_mouse_norm.1);
            for (output_name, pos) in &self.output_mouse_norm {
                parallax_system.set_mouse_for_output(output_name, pos.0, pos.1);
            }

            let audio_bands = AudioBands::from_bars(&self.current_bar_heights, self.last_cava_peak);
            let output_names = self.per_output.keys().cloned().collect::<Vec<_>>();
            for output_name in output_names {
                let avg_frame_ms = self
                    .per_output
                    .get(&output_name)
                    .and_then(|s| s.perf_monitor.avg_frame_time_ms());
                let audio_cfg = self
                    .per_output
                    .get(&output_name)
                    .map(|s| &s.effective_config.audio);
                let layers = parallax_system.compute_layers(
                    &output_name,
                    audio_bands,
                    audio_cfg,
                    self.is_idle,
                    avg_frame_ms,
                );
                if !layers.is_empty() {
                    parallax_layers_by_output.insert(output_name, layers);
                }
            }
        }

        let playback_seconds = self.cava_frame_counter as f64 / self.framerate.max(1.0);
        for state in self.per_output.values_mut() {
            if !state.configured {
                continue;
            }

            if let Err(e) = update_hidden_video_frame(state, playback_seconds) {
                debug!("Hidden video frame update skipped: {e:#}");
            }

            let cfg = &state.effective_config;
            let output_colors = if cfg.general.dynamic_colors || cfg.colors.extract_from_wallpaper {
                self.colors.clone()
            } else if !cfg.colors.palette.is_empty() {
                if cfg.colors.palette.len() == 1 {
                    cfg.colors.palette.clone()
                } else {
                    // all palette colors; shader handles gradient vs flat via use_gradient flag
                    cfg.colors.palette.clone()
                }
            } else {
                self.colors.clone()
            };
            let effect = cfg
                .hidden_image
                .as_ref()
                .map(|c| c.effect)
                .unwrap_or_default();
            let xray_modulation = state.xray_animator.update(&cfg.xray, self.last_cava_peak);
            let effective_bar_alpha = (cfg.audio.bar_alpha * xray_modulation).clamp(0.0, 1.0);
            let uniforms = Uniforms::new(
                &output_colors,
                state.width as f32,
                state.height as f32,
                effective_bar_alpha,
                cfg.xray.enabled && state.hidden_image_loaded,
                effect,
                state.hidden_texture_size.0 as f32,
                state.hidden_texture_size.1 as f32,
                state.crop_scale,
                state.crop_offset,
                match state.effective_config.colors.gradient_direction {
                    crate::app_config::GradientDirection::BottomToTop => 0.0,
                    crate::app_config::GradientDirection::TopToBottom => 1.0,
                    crate::app_config::GradientDirection::LeftToRight => 2.0,
                    crate::app_config::GradientDirection::RightToLeft => 3.0,
                },
                state.effective_config.colors.use_gradient,
            );
            state.wgpu_queue.write_buffer(
                &state.uniform_buffer,
                0,
                bytemuck::cast_slice(&[uniforms]),
            );

            let target_bar_count = cfg.audio.bar_count.max(1) as usize;
            let mut frame_vertices = Vec::with_capacity(
                target_bar_count
                    * bar_geometry::vertices_per_bar(
                        cfg.audio.bar_shape,
                        cfg.audio.corner_segments,
                    ),
            );
            build_visualizer_vertices(
                &mut frame_vertices,
                &self.current_bar_heights,
                cfg.audio.visualization_mode,
                cfg.audio.bar_shape,
                cfg.audio.corner_radius,
                cfg.audio.corner_segments,
                cfg.audio.polygon_sides,
                false, // line_only removed (use BarShape::Line instead)
                cfg.audio.gap,
                cfg.audio.height_scale,
                target_bar_count,
                cfg.audio.radial_inner_radius,
                cfg.audio.radial_sweep_angle,
                cfg.audio.waveform_line_width,
                cfg.audio.waveform_smoothness,
                cfg.audio.block_size,
                cfg.audio.block_spacing,
                cfg.audio.spiral_turns,
                cfg.audio.mirror_gap,
            );

            state.wgpu_queue.write_buffer(
                &state.vertex_buffer,
                0,
                bytemuck::cast_slice(&frame_vertices),
            );

            let frame = match state.wgpu_surface.get_current_texture() {
                Ok(f) => f,
                Err(wgpu::SurfaceError::Lost) => {
                    state
                        .wgpu_surface
                        .configure(&state.wgpu_device, &state.wgpu_config);
                    continue;
                }
                Err(e) => {
                    error!("Surface error: {:?}", e);
                    continue;
                }
            };

            if let Some(parallax_layers) = parallax_layers_by_output.get(&state.output_name) {
                for layer in parallax_layers {
                    if !state.parallax_layers_gpu.contains_key(&layer.id) {
                        if let Ok(gpu_layer) = Self::create_parallax_gpu_layer(state, layer) {
                            state.parallax_layers_gpu.insert(layer.id, gpu_layer);
                        }
                    }
                    if let Some(gpu_layer) = state.parallax_layers_gpu.get_mut(&layer.id) {
                        Self::upload_parallax_frame_if_needed(&state.wgpu_queue, gpu_layer, layer);
                    }
                }
            }

            let view = frame
                .texture
                .create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = state
                .wgpu_device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let mut bg = state.background_color;
                if state.effective_config.xray.enabled && state.hidden_image_loaded && bg[3] > 0.0 {
                    warn!(
                        "background_color alpha is {}, forcing to 0.0 for hidden image mode (xray)",
                        bg[3]
                    );
                    bg[3] = 0.0;
                }

                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Main Render Pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: bg[0] as f64,
                                g: bg[1] as f64,
                                b: bg[2] as f64,
                                a: bg[3] as f64,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    occlusion_query_set: None,
                    timestamp_writes: None,
                });

                if let Some(parallax_layers) = parallax_layers_by_output.get(&state.output_name) {
                    let viz_idx = state.effective_config.parallax.visualizer_layer_index;
                    for (i, layer) in parallax_layers.iter().enumerate() {
                        // Insert visualizer at the configured position
                        if state.effective_config.audio.show_visualizer
                            && state.effective_config.parallax.visualizer_as_parallax_layer
                            && i == viz_idx
                        {
                            render_pass.set_pipeline(&state.bar_render_pipeline);
                            render_pass.set_bind_group(0, &state.bind_group0, &[]);
                            render_pass.set_bind_group(1, &state.hidden_image_bind_group, &[]);
                            render_pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
                            let vertex_count = frame_vertices.len() as u32 / 4;
                            render_pass.draw(0..vertex_count, 0..1);
                        }

                        let Some(gpu_layer) = state.parallax_layers_gpu.get(&layer.id) else {
                            continue;
                        };

                        let pipeline = match layer.blend_mode {
                            BlendMode::Add => &state.parallax_pipeline_add,
                            BlendMode::Multiply
                            | BlendMode::Screen
                            | BlendMode::Overlay
                            | BlendMode::Reveal
                            | BlendMode::Normal => &state.parallax_pipeline_normal,
                        };

                        // Write per-layer uniform directly to this layer's own buffer
                        // so each layer has its own independent NDC displacement.
                        let uniform = ParallaxUniform::from_layer(
                            layer,
                            (state.width, state.height),
                            (gpu_layer.dimensions.0, gpu_layer.dimensions.1),
                        );
                        state.wgpu_queue.write_buffer(
                            &gpu_layer.uniform_buffer,
                            0,
                            bytemuck::cast_slice(&[uniform]),
                        );

                        render_pass.set_pipeline(pipeline);
                        render_pass.set_bind_group(0, &gpu_layer.uniform_bind_group, &[]);
                        render_pass.set_bind_group(1, &gpu_layer.bind_group, &[]);
                        render_pass.set_vertex_buffer(0, state.parallax_vertex_buffer.slice(..));
                        render_pass.set_index_buffer(
                            state.parallax_index_buffer.slice(..),
                            wgpu::IndexFormat::Uint16,
                        );
                        render_pass.draw_indexed(0..6, 0, 0..1);
                    }
                    // If visualizer should be after all layers, draw it now
                    if state.effective_config.audio.show_visualizer
                        && (!state.effective_config.parallax.visualizer_as_parallax_layer
                            || viz_idx >= parallax_layers.len())
                    {
                        render_pass.set_pipeline(&state.bar_render_pipeline);
                        render_pass.set_bind_group(0, &state.bind_group0, &[]);
                        render_pass.set_bind_group(1, &state.hidden_image_bind_group, &[]);
                        render_pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
                        let vertex_count = frame_vertices.len() as u32 / 4;
                        render_pass.draw(0..vertex_count, 0..1);
                    }
                } else if state.effective_config.audio.show_visualizer {
                    // No parallax layers, just render visualizer
                    render_pass.set_pipeline(&state.bar_render_pipeline);
                    render_pass.set_bind_group(0, &state.bind_group0, &[]);
                    render_pass.set_bind_group(1, &state.hidden_image_bind_group, &[]);
                    render_pass.set_vertex_buffer(0, state.vertex_buffer.slice(..));
                    let vertex_count = frame_vertices.len() as u32 / 4;
                    render_pass.draw(0..vertex_count, 0..1);
                }
            }
            state.wgpu_queue.submit(std::iter::once(encoder.finish()));
            frame.present();
            state.surface.frame(&self.qh, state.surface.clone());
            state.perf_monitor.record(frame_start.elapsed());
            state.perf_monitor.maybe_log();
        }

        if self.telemetry_decoder_enabled && self.cava_frame_counter.is_multiple_of(600) {
            debug!(
                "decoder-telemetry: frames={} idle={} peak={:.4}",
                self.cava_frame_counter, self.is_idle, self.last_cava_peak
            );
        }

        self.perf_monitor.record(frame_start.elapsed());
        self.perf_monitor.maybe_log();
    }
}

fn update_hidden_video_frame(
    output_state: &mut PerOutputState,
    playback_seconds: f64,
) -> Result<()> {
    let Some(decoder) = output_state.hidden_video_decoder.as_mut() else {
        return Ok(());
    };
    let Some(frame) = decoder
        .poll_latest_for_time(playback_seconds)
        .or_else(|| decoder.last_frame())
    else {
        return Ok(());
    };

    let same_size = output_state.hidden_texture_size == (frame.width, frame.height);
    if same_size {
        if let Some(texture) = output_state._hidden_image_texture.as_ref() {
            output_state.wgpu_queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &frame.rgba,
                wgpu::ImageDataLayout {
                    offset: 0,
                    bytes_per_row: Some(frame.width * 4),
                    rows_per_image: Some(frame.height),
                },
                wgpu::Extent3d {
                    width: frame.width.max(1),
                    height: frame.height.max(1),
                    depth_or_array_layers: 1,
                },
            );
            return Ok(());
        }
    }

    let (texture, view) =
        create_texture_from_rgba_frame(&output_state.wgpu_device, &output_state.wgpu_queue, &frame);
    let sampler = output_state
        .wgpu_device
        .create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
    let bind_group = output_state
        .wgpu_device
        .create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Hidden Video GPU Bind Group"),
            layout: &output_state.parallax_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

    output_state._hidden_image_texture = Some(texture);
    output_state._hidden_image_view = Some(view);
    output_state.hidden_image_bind_group = bind_group;
    output_state.hidden_texture_size = (frame.width, frame.height);
    let (crop_scale, crop_offset) = compute_preserve_aspect_crop_transform(
        output_state.width as f32,
        output_state.height as f32,
        frame.width as f32,
        frame.height as f32,
    );
    output_state.crop_scale = crop_scale;
    output_state.crop_offset = crop_offset;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_visualizer_vertices(
    out: &mut Vec<f32>,
    current_bar_heights: &[f32],
    mode: VisualizationMode,
    bar_shape: BarShape,
    corner_radius: f32,
    corner_segments: u32,
    polygon_sides: u32,
    _line_only_deprecated: bool, // removed — use BarShape::Line for line rendering
    gap: f32,
    height_scale: f32,
    target_bar_count: usize,
    radial_inner_radius: f32,
    radial_sweep_angle: f32,
    waveform_line_width: f32,
    waveform_smoothness: f32,
    _block_size: f32,
    _block_spacing: f32,
    _spiral_turns: f32,
    mirror_gap: f32,
) {
    match mode {
        VisualizationMode::Bars => build_bars_layout(
            out,
            current_bar_heights,
            bar_shape,
            corner_radius,
            corner_segments,
            polygon_sides,
            false, // line_only removed
            gap,
            height_scale,
            target_bar_count,
            false,
        ),
        VisualizationMode::Blocks => build_bars_layout(
            out,
            current_bar_heights,
            bar_shape,
            corner_radius,
            corner_segments,
            polygon_sides,
            false, // line_only removed
            gap,
            height_scale,
            target_bar_count,
            true,
        ),
        VisualizationMode::Waveform => {
            let count = target_bar_count.max(2);
            let half_thick = (waveform_line_width / 2000.0).clamp(0.001, 0.05);
            let _smooth = waveform_smoothness.clamp(0.0, 1.0);
            let bar_width = 2.0 / (count as f32 + (count as f32 - 1.0) * gap);
            let bar_gap_width = bar_width * gap;
            for i in 0..count {
                let source_idx = i * current_bar_heights.len().max(1) / count;
                let source = current_bar_heights.get(source_idx).copied().unwrap_or(0.0);
                let y_center = (source * 2.0 - 1.0) * 0.5 * height_scale.clamp(0.1, 2.5);
                let y0 = (y_center - half_thick).clamp(-1.0, 1.0);
                let y1 = (y_center + half_thick).clamp(-1.0, 1.0);
                let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                bar_geometry::build_bar(
                    out,
                    BarShape::Line,
                    x0,
                    y0,
                    x1,
                    y1,
                    corner_radius * bar_width * 0.5,
                    corner_radius * bar_width * 0.5,
                    corner_segments,
                    polygon_sides,
                    true,
                );
            }
        }
        VisualizationMode::MirrorBars => {
            // Bars extending symmetrically up and down from the horizontal
            // center, with a small gutter (mirror_gap) splitting the halves.
            let bar_width = 2.0 / (target_bar_count as f32 + (target_bar_count as f32 - 1.0) * gap);
            let bar_gap_width = bar_width * gap;
            let half_gap = mirror_gap.clamp(0.0, 0.5) * 0.5;
            for i in 0..target_bar_count {
                let source_idx = i * current_bar_heights.len().max(1) / target_bar_count;
                let source = current_bar_heights.get(source_idx).copied().unwrap_or(0.0);
                let half_h = (source * height_scale.clamp(0.1, 3.0)).clamp(0.0, 1.0 - half_gap);
                let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                let radius = corner_radius * bar_width * 0.5;
                // Upper half: from center+gap to center+gap+halfH
                bar_geometry::build_bar(
                    out,
                    bar_shape,
                    x0,
                    half_gap,
                    x1,
                    half_gap + half_h,
                    radius,
                    radius,
                    corner_segments,
                    polygon_sides,
                    false,
                );
                // Lower half: mirrored down
                bar_geometry::build_bar(
                    out,
                    bar_shape,
                    x0,
                    -half_gap - half_h,
                    x1,
                    -half_gap,
                    radius,
                    radius,
                    corner_segments,
                    polygon_sides,
                    false,
                );
            }
        }
        VisualizationMode::InvertedBars => {
            // Bars hanging from the top edge.
            let bar_width = 2.0 / (target_bar_count as f32 + (target_bar_count as f32 - 1.0) * gap);
            let bar_gap_width = bar_width * gap;
            for i in 0..target_bar_count {
                let source_idx = i * current_bar_heights.len().max(1) / target_bar_count;
                let source = current_bar_heights.get(source_idx).copied().unwrap_or(0.0);
                let scaled_height = source * height_scale.clamp(0.1, 3.0);
                let h = 1.0 - 2.0 * scaled_height; // top stays at +1, bottom moves down
                let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
                let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
                let radius = corner_radius * bar_width * 0.5;
                bar_geometry::build_bar(
                    out,
                    bar_shape,
                    x0,
                    h,
                    x1,
                    1.0,
                    radius,
                    radius,
                    corner_segments,
                    polygon_sides,
                    false,
                );
            }
        }
        VisualizationMode::Spectrum => {
            // Smooth thick line connecting the top of each bar — frequency
            // spectrum analyzer style. Uses temporal smoothing weight.
            let count = target_bar_count.max(2);
            let half_thick = (waveform_line_width / 200.0).clamp(0.002, 0.06);
            // Smoothness controls how much rounding we apply between bins
            // (interpolates each segment by inserting an extra midpoint).
            let smooth = waveform_smoothness.clamp(0.0, 1.0);
            let bar_step = 2.0 / count as f32;
            // Pre-compute the (x, y) sample points
            let mut points: Vec<(f32, f32)> = Vec::with_capacity(count);
            for i in 0..count {
                let source_idx = i * current_bar_heights.len().max(1) / count;
                let source = current_bar_heights.get(source_idx).copied().unwrap_or(0.0);
                let x = -1.0 + (i as f32 + 0.5) * bar_step;
                let y = -1.0 + 2.0 * (source * height_scale.clamp(0.1, 3.0)).clamp(0.0, 1.0);
                points.push((x, y));
            }
            // If smoothing > 0, double the resolution by interpolating midpoints.
            let final_points: Vec<(f32, f32)> = if smooth > 0.05 && points.len() >= 3 {
                let mut out = Vec::with_capacity(points.len() * 2);
                for i in 0..points.len() {
                    out.push(points[i]);
                    if i + 1 < points.len() {
                        let a = points[i];
                        let b = points[i + 1];
                        let t = 0.5;
                        // Catmull-rom-like easing using neighbours when available
                        let prev = if i > 0 { points[i - 1] } else { a };
                        let next = if i + 2 < points.len() {
                            points[i + 2]
                        } else {
                            b
                        };
                        let mx = a.0 + (b.0 - a.0) * t;
                        // simple cubic-ish blend on Y, weighted by smoothness
                        let raw_y = a.1 + (b.1 - a.1) * t;
                        let curve_y = 0.5 * (a.1 + b.1)
                            + 0.125 * smooth * (2.0 * (a.1 + b.1) - prev.1 - next.1);
                        let my = raw_y + (curve_y - raw_y) * smooth;
                        out.push((mx, my));
                    }
                }
                out
            } else {
                points
            };
            // Emit thick line segments between consecutive points
            for w in final_points.windows(2) {
                let (a, b) = (w[0], w[1]);
                push_thick_line(out, a.0, a.1, b.0, b.1, half_thick);
            }
        }

        VisualizationMode::Ring => {
            // Continuous filled ring whose thickness at each angular bucket
            // is modulated by the bin amplitude. Approximated with quads
            // covering each angular slice.
            let count = target_bar_count.max(8);
            let inner_r = (radial_inner_radius / 100.0).clamp(0.0, 0.85);
            let sweep = radial_sweep_angle
                .to_radians()
                .clamp(0.1, std::f32::consts::TAU);
            let max_thick = (1.0 - inner_r).max(0.05);
            let h_scale = height_scale.clamp(0.2, 3.0);
            // We use waveform_smoothness to blend each slice with its
            // neighbours so the ring looks continuous instead of jagged.
            let smooth = waveform_smoothness.clamp(0.0, 1.0);
            let mut amps: Vec<f32> = (0..count)
                .map(|i| {
                    let source_idx = i * current_bar_heights.len().max(1) / count;
                    current_bar_heights.get(source_idx).copied().unwrap_or(0.0)
                })
                .collect();
            if smooth > 0.05 && amps.len() >= 3 {
                let mut blurred = amps.clone();
                for i in 0..amps.len() {
                    let prev = amps[(i + amps.len() - 1) % amps.len()];
                    let next = amps[(i + 1) % amps.len()];
                    blurred[i] =
                        amps[i] * (1.0 - smooth * 0.6) + (prev + next) * 0.5 * smooth * 0.6;
                }
                amps = blurred;
            }
            for i in 0..count {
                let a0 = (i as f32 / count as f32) * sweep;
                let a1 = ((i + 1) as f32 / count as f32) * sweep;
                let amp = amps[i];
                let amp_next = amps[(i + 1) % amps.len()];
                let r_outer_0 = inner_r + (max_thick * amp * h_scale).clamp(0.0, max_thick);
                let r_outer_1 = inner_r + (max_thick * amp_next * h_scale).clamp(0.0, max_thick);
                let (c0, s0) = (a0.cos(), a0.sin());
                let (c1, s1) = (a1.cos(), a1.sin());
                let v_inner_0 = [c0 * inner_r, s0 * inner_r];
                let v_inner_1 = [c1 * inner_r, s1 * inner_r];
                let v_outer_0 = [c0 * r_outer_0, s0 * r_outer_0];
                let v_outer_1 = [c1 * r_outer_1, s1 * r_outer_1];
                push_quad_vertices(out, v_inner_0, v_inner_1, v_outer_0, v_outer_1);
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn build_bars_layout(
    out: &mut Vec<f32>,
    current_bar_heights: &[f32],
    bar_shape: BarShape,
    corner_radius: f32,
    corner_segments: u32,
    polygon_sides: u32,
    _line_only_deprecated: bool, // removed — use BarShape::Line
    gap: f32,
    height_scale: f32,
    target_bar_count: usize,
    quantized_blocks: bool,
) {
    let bar_width = 2.0 / (target_bar_count as f32 + (target_bar_count as f32 - 1.0) * gap);
    let bar_gap_width = bar_width * gap;
    for i in 0..target_bar_count {
        let source_idx = i * current_bar_heights.len().max(1) / target_bar_count;
        let source = current_bar_heights.get(source_idx).copied().unwrap_or(0.0);
        let mut scaled_height = source * height_scale;
        if quantized_blocks {
            let steps = 12.0;
            scaled_height = (scaled_height * steps).round() / steps;
        }
        let h = 2.0 * scaled_height - 1.0;
        let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
        let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
        bar_geometry::build_bar(
            out,
            bar_shape,
            x0,
            -1.0,
            x1,
            h,
            corner_radius * bar_width * 0.5,
            corner_radius * bar_width * 0.5,
            corner_segments,
            polygon_sides,
            false, // line_only removed
        );
    }
}

fn push_quad_vertices(out: &mut Vec<f32>, v0: [f32; 2], v1: [f32; 2], v2: [f32; 2], v3: [f32; 2]) {
    let uv = |x: f32, y: f32| ((x + 1.0) * 0.5, (y + 1.0) * 0.5);
    let (u0, w0) = uv(v0[0], v0[1]);
    let (u1, w1) = uv(v1[0], v1[1]);
    let (u2, w2) = uv(v2[0], v2[1]);
    let (u3, w3) = uv(v3[0], v3[1]);

    out.extend_from_slice(&[v0[0], v0[1], u0, w0]);
    out.extend_from_slice(&[v1[0], v1[1], u1, w1]);
    out.extend_from_slice(&[v2[0], v2[1], u2, w2]);
    out.extend_from_slice(&[v1[0], v1[1], u1, w1]);
    out.extend_from_slice(&[v3[0], v3[1], u3, w3]);
    out.extend_from_slice(&[v2[0], v2[1], u2, w2]);
}

/// Emit a thick line segment (a quad) between (ax, ay) and (bx, by) with
/// the given half-thickness in clip space. The two corners on either side
/// of the segment are offset perpendicular to the line direction.
fn push_thick_line(out: &mut Vec<f32>, ax: f32, ay: f32, bx: f32, by: f32, half_thick: f32) {
    let dx = bx - ax;
    let dy = by - ay;
    let len = (dx * dx + dy * dy).sqrt().max(1e-6);
    let nx = -dy / len * half_thick;
    let ny = dx / len * half_thick;
    let v0 = [ax + nx, ay + ny];
    let v1 = [ax - nx, ay - ny];
    let v2 = [bx + nx, by + ny];
    let v3 = [bx - nx, by - ny];
    push_quad_vertices(out, v0, v1, v2, v3);
}

// --- Wayland Handlers ---
impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }
    fn new_output(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Err(e) = self.ensure_output(&output) {
            error!("Failed to create output: {}", e);
        }
    }
    fn update_output(
        &mut self,
        conn: &Connection,
        qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        if let Some(info) = self.output_state.info(&output) {
            if let Some(name) = info.name.clone() {
                if let Some(state) = self.per_output.get_mut(&name) {
                    state.logical_position = info.logical_position.unwrap_or((0, 0));
                    self.persist_runtime_outputs();
                    return;
                }
            }
        }
        self.new_output(conn, qh, output);
    }
    fn output_destroyed(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        output: wl_output::WlOutput,
    ) {
        let info = match self.output_state.info(&output) {
            Some(i) => i,
            None => return,
        };
        let name = info.name.unwrap_or_else(|| "unknown".to_string());
        if self.per_output.remove(&name).is_some() {
            self.output_mouse_norm.remove(&name);
            self.persist_runtime_outputs();
            info!("Output {} disconnected", name);
        }
    }
}

delegate_compositor!(AppState);
delegate_output!(AppState);
delegate_registry!(AppState);
delegate_layer!(AppState);
delegate_seat!(AppState);
delegate_pointer!(AppState);

impl ProvidesRegistryState for AppState {
    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry_state
    }
    registry_handlers![OutputState, SeatState];
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
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _surface: &wl_surface::WlSurface,
        _time: u32,
    ) {
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

impl SeatHandler for AppState {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, seat: wl_seat::WlSeat) {
        debug!("New seat detected: {:?}", seat.id());
    }

    fn new_capability(
        &mut self,
        _conn: &Connection,
        qh: &QueueHandle<Self>,
        seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            match self.seat_state.get_pointer(qh, &seat) {
                Ok(pointer) => self.pointer_devices.push(pointer),
                Err(err) => warn!("Unable to acquire wl_pointer: {err}"),
            }
        }
    }

    fn remove_capability(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _seat: wl_seat::WlSeat,
        capability: Capability,
    ) {
        if capability == Capability::Pointer {
            self.pointer_devices.clear();
            self.output_mouse_norm.clear();
        }
    }

    fn remove_seat(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _seat: wl_seat::WlSeat) {
        self.pointer_devices.clear();
        self.output_mouse_norm.clear();
    }
}

impl PointerHandler for AppState {
    fn pointer_frame(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        _pointer: &wl_pointer::WlPointer,
        events: &[PointerEvent],
    ) {
        for event in events {
            let output_hit = self
                .per_output
                .iter()
                .find(|(_, state)| state.surface == event.surface)
                .map(|(name, state)| (name.clone(), state.width.max(1), state.height.max(1)));

            let Some((output_name, width, height)) = output_hit else {
                continue;
            };

            match event.kind {
                PointerEventKind::Enter { .. } | PointerEventKind::Motion { .. } => {
                    let nx = (event.position.0 as f32 / width as f32).clamp(0.0, 1.0);
                    let ny = (event.position.1 as f32 / height as f32).clamp(0.0, 1.0);
                    self.output_mouse_norm.insert(output_name, (nx, ny));
                }
                PointerEventKind::Leave { .. } => {
                    self.output_mouse_norm.remove(&output_name);
                }
                _ => {}
            }
        }
    }
}

impl LayerShellHandler for AppState {
    fn closed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _layer: &LayerSurface) {}
    fn configure(
        &mut self,
        _conn: &Connection,
        _qh: &QueueHandle<Self>,
        layer: &LayerSurface,
        configure: LayerSurfaceConfigure,
        _serial: u32,
    ) {
        let mut configured_output: Option<(String, u32, u32)> = None;

        for (name, state) in self.per_output.iter_mut() {
            if &state.layer_surface != layer {
                continue;
            }

            let width = configure.new_size.0;
            let height = configure.new_size.1;
            if width == state.width && height == state.height && state.configured {
                return;
            }

            state.width = width;
            state.height = height;
            state.wgpu_config.width = width;
            state.wgpu_config.height = height;
            state
                .wgpu_surface
                .configure(&state.wgpu_device, &state.wgpu_config);

            let effect = state
                .effective_config
                .hidden_image
                .as_ref()
                .map(|c| c.effect)
                .unwrap_or_default();
            let output_colors = if state.effective_config.general.dynamic_colors
                || state.effective_config.colors.extract_from_wallpaper
            {
                self.colors.clone()
            } else if !state.effective_config.colors.palette.is_empty() {
                if state.effective_config.colors.palette.len() == 1 {
                    state.effective_config.colors.palette.clone()
                } else {
                    // all palette colors; shader handles gradient vs flat via use_gradient flag
                    state.effective_config.colors.palette.clone()
                }
            } else {
                self.colors.clone()
            };

            let (crop_scale, crop_offset) = compute_preserve_aspect_crop_transform(
                width as f32,
                height as f32,
                state.hidden_texture_size.0 as f32,
                state.hidden_texture_size.1 as f32,
            );
            state.crop_scale = crop_scale;
            state.crop_offset = crop_offset;
            let uniforms = Uniforms::new(
                &output_colors,
                width as f32,
                height as f32,
                state.effective_config.audio.bar_alpha,
                state.effective_config.xray.enabled && state.hidden_image_loaded,
                effect,
                state.hidden_texture_size.0 as f32,
                state.hidden_texture_size.1 as f32,
                state.crop_scale,
                state.crop_offset,
                match state.effective_config.colors.gradient_direction {
                    crate::app_config::GradientDirection::BottomToTop => 0.0,
                    crate::app_config::GradientDirection::TopToBottom => 1.0,
                    crate::app_config::GradientDirection::LeftToRight => 2.0,
                    crate::app_config::GradientDirection::RightToLeft => 3.0,
                },
                state.effective_config.colors.use_gradient,
            );
            state.wgpu_queue.write_buffer(
                &state.uniform_buffer,
                0,
                bytemuck::cast_slice(&[uniforms]),
            );
            state.configured = true;
            configured_output = Some((name.clone(), width, height));
            break;
        }

        if let Some((name, width, height)) = configured_output {
            self.persist_runtime_outputs();
            info!("Output {} ready: {}x{}", name, width, height);
        }
    }
}
