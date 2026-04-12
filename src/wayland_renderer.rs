use anyhow::{Context, Result};
use log::{debug, error, info};
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
use std::time::Duration;

use crate::app_config::{array_from_config_color, Config, CavaConfig, CavaGeneralConfig, CavaSmoothingConfig};
use crate::wallpaper::WallpaperAnalyzer;

const SHADER_WGSL: &str = r#"
struct VertexInput {
    @location(0) position: vec2<f32>,
};

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.position = vec4<f32>(in.position, 0.0, 1.0);
    return out;
}

struct Uniforms {
    gradient_colors: array<vec4<f32>, 32>,
    colors_count: i32,
    _padding1: i32,
    _padding2: i32,
    _padding3: i32,
    window_size: vec2<f32>,
    _padding4: vec2<f32>,
}

@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@fragment
fn fs_main(@builtin(position) coord: vec4<f32>) -> @location(0) vec4<f32> {
    // Invertir Y para que el degradado vaya de abajo hacia arriba como en OpenGL
    let y = uniforms.window_size.y - coord.y;
    let height = uniforms.window_size.y;
    if (uniforms.colors_count == 1) {
        return uniforms.gradient_colors[0];
    } else {
        let findex = (y * f32(uniforms.colors_count - 1)) / height;
        let index = i32(findex);
        let step = findex - f32(index);
        var idx = index;
        if (idx == uniforms.colors_count - 1) {
            idx = idx - 1;
        }
        return mix(uniforms.gradient_colors[idx], uniforms.gradient_colors[idx + 1], step);
    }
}
"#;

#[repr(C)]
#[derive(Debug, Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    gradient_colors: [[f32; 4]; 32],
    colors_count: i32,
    _padding: [i32; 3],
    window_size: [f32; 2],
    _pad2: [f32; 2],
}

impl Uniforms {
    fn new(colors: &[[f32; 4]], width: f32, height: f32) -> Self {
        let mut grad = [[0.0; 4]; 32];
        for (i, c) in colors.iter().enumerate().take(32) {
            grad[i] = *c;
        }
        Self {
            gradient_colors: grad,
            colors_count: colors.len() as i32,
            _padding: [0, 0, 0],
            window_size: [width, height],
            _pad2: [0.0, 0.0],
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
    bind_group: wgpu::BindGroup,
    uniform_buffer: wgpu::Buffer,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    width: u32,
    height: u32,
    configured: bool,
    background_color: [f32; 4],
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

    pub fn run(self) -> Result<()> {
        info!("Starting cava-bg with wgpu backend");

        // Spawn cava
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

        let cava_stdout = cmd.stdout.take().context("Failed to get cava stdout")?;
        let cava_reader = BufReader::new(cava_stdout);
        let bar_count = self.config.bars.amount as usize;

        // Obtener colores del gradiente: dinámicos o de la configuración
        let use_dynamic = self.config.general.dynamic_colors.unwrap_or(true);
        let gradient_colors: Vec<[f32; 4]> = if use_dynamic {
            let num_colors = if !self.config.colors.is_empty() {
                self.config.colors.len()
            } else {
                8
            };
            match WallpaperAnalyzer::generate_gradient_colors(num_colors) {
                Ok(colors) => {
                    info!("Using dynamic colors from wallpaper");
                    colors
                }
                Err(e) => {
                    error!("Failed to generate colors from wallpaper: {}, using config colors", e);
                    self.config.colors.values()
                        .map(|c| array_from_config_color(c.clone()))
                        .collect()
                }
            }
        } else {
            info!("Using static colors from config");
            self.config.colors.values()
                .map(|c| array_from_config_color(c.clone()))
                .collect()
        };

        let background_color = array_from_config_color(self.config.general.background_color.clone());

        // Wayland connection
        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();

        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle)
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;

        let frame_duration = Duration::from_secs(1) / self.config.general.framerate;

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            layer_shell,
            compositor,
            per_output: HashMap::new(),
            bar_count,
            bar_gap: self.config.bars.gap,
            preferred_output_name: self.config.general.preferred_output.clone(),
            cava_reader,
            colors: gradient_colors,
            background_color,
            conn: conn.clone(),
            qh: qh.clone(),
            running: self.running,
        };

        // Enumerate existing outputs
        for output in app_state.output_state.outputs() {
            if let Err(e) = app_state.ensure_output(&output) {
                error!("Failed to create initial output: {}", e);
            }
        }

        // Bucle principal con timer
        event_loop.run(Some(frame_duration), &mut app_state, |state| {
            if !state.running.load(Ordering::SeqCst) {
                std::process::exit(0);
            }
            state.draw();
        })?;

        Ok(())
    }
}

struct AppState {
    registry_state: RegistryState,
    output_state: OutputState,
    layer_shell: LayerShell,
    compositor: CompositorState,
    per_output: HashMap<String, PerOutputState>,
    bar_count: usize,
    bar_gap: f32,
    preferred_output_name: Option<String>,
    cava_reader: BufReader<std::process::ChildStdout>,
    colors: Vec<[f32; 4]>,
    background_color: [f32; 4],
    conn: Connection,
    qh: QueueHandle<Self>,
    running: Arc<AtomicBool>,
}

impl AppState {
    fn ensure_output(&mut self, output: &wl_output::WlOutput) -> Result<()> {
        let info = self.output_state.info(output).context("Failed to get output info")?;
        let name = info.name.clone().unwrap_or_else(|| "unknown".to_string());

        if self.per_output.contains_key(&name) {
            return Ok(());
        }

        if let Some(ref pref) = self.preferred_output_name {
            if &name != pref {
                debug!("Skipping output {} (preferred is {})", name, pref);
                return Ok(());
            }
        }

        info!("Creating surface for output {}", name);

        let surface = self.compositor.create_surface(&self.qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            &self.qh,
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

        // Crear superficie WGPU
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
        .context("Failed to create WGPU surface")?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&wgpu_surface),
            force_fallback_adapter: false,
        }))
        .context("Failed to find suitable GPU adapter")?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: None,
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
            },
            None,
        ))
        .context("Failed to create device")?;

        let mut surface_config = wgpu_surface
            .get_default_config(&adapter, width, height)
            .unwrap();
        surface_config.present_mode = wgpu::PresentMode::Fifo;
        let caps = wgpu_surface.get_capabilities(&adapter);
        surface_config.alpha_mode = caps
            .alpha_modes
            .iter()
            .find(|m| **m != wgpu::CompositeAlphaMode::Opaque)
            .copied()
            .unwrap_or(wgpu::CompositeAlphaMode::Auto);
        wgpu_surface.configure(&device, &surface_config);

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER_WGSL.into()),
        });

        let mut indices = Vec::with_capacity(self.bar_count * 6);
        for i in 0..self.bar_count {
            let base = (i * 4) as u16;
            indices.extend_from_slice(&[base, base + 1, base + 2, base + 1, base + 2, base + 3]);
        }
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Vertex Buffer"),
            size: (self.bar_count * 8 * std::mem::size_of::<f32>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniforms = Uniforms::new(&self.colors, width as f32, height as f32);
        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Bind Group Layout"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Bind Group"),
            layout: &bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Pipeline Layout"),
            bind_group_layouts: &[&bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: "vs_main",
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: (2 * std::mem::size_of::<f32>()) as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &[wgpu::VertexAttribute {
                        format: wgpu::VertexFormat::Float32x2,
                        offset: 0,
                        shader_location: 0,
                    }],
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

        let static_surface: wgpu::Surface<'static> = unsafe { std::mem::transmute(wgpu_surface) };

        let state = PerOutputState {
            surface,
            layer_surface,
            wgpu_surface: static_surface,
            wgpu_device: device,
            wgpu_queue: queue,
            wgpu_config: surface_config,
            render_pipeline,
            bind_group,
            uniform_buffer,
            vertex_buffer,
            index_buffer,
            width,
            height,
            configured: false,
            background_color: self.background_color,
        };

        self.per_output.insert(name.clone(), state);
        info!("WGPU surface created for {}: {}x{}", name, width, height);
        Ok(())
    }

    fn read_cava_data(&mut self) -> Vec<f32> {
        let mut cava_buffer = vec![0u8; self.bar_count * 2];
        let mut bar_heights = vec![0.0f32; self.bar_count];
        match self.cava_reader.read_exact(&mut cava_buffer) {
            Ok(()) => {
                for (i, chunk) in cava_buffer.chunks_exact(2).enumerate() {
                    let num = u16::from_le_bytes([chunk[0], chunk[1]]);
                    bar_heights[i] = (num as f32) / 65530.0;
                }
            }
            Err(e) => {
                error!("Failed to read from cava: {}", e);
            }
        }
        bar_heights
    }

    fn draw(&mut self) {
        let bar_heights = self.read_cava_data();

        let bar_width = 2.0 / (self.bar_count as f32 + (self.bar_count as f32 - 1.0) * self.bar_gap);
        let bar_gap_width = bar_width * self.bar_gap;
        let mut vertices = vec![0.0f32; self.bar_count * 8];
        for i in 0..self.bar_count {
            let h = 2.0 * bar_heights[i] - 1.0;
            let x0 = bar_gap_width * i as f32 + bar_width * i as f32 - 1.0;
            let x1 = bar_gap_width * i as f32 + bar_width * (i + 1) as f32 - 1.0;
            vertices[i * 8] = x0;
            vertices[i * 8 + 1] = h;
            vertices[i * 8 + 2] = x1;
            vertices[i * 8 + 3] = h;
            vertices[i * 8 + 4] = x0;
            vertices[i * 8 + 5] = -1.0;
            vertices[i * 8 + 6] = x1;
            vertices[i * 8 + 7] = -1.0;
        }

        for state in self.per_output.values_mut() {
            if !state.configured {
                continue;
            }

            state.wgpu_queue.write_buffer(&state.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

            let frame = match state.wgpu_surface.get_current_texture() {
                Ok(frame) => frame,
                Err(wgpu::SurfaceError::Lost) => {
                    state.wgpu_surface.configure(&state.wgpu_device, &state.wgpu_config);
                    continue;
                }
                Err(e) => {
                    error!("Error acquiring surface texture: {:?}", e);
                    continue;
                }
            };

            let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
            let mut encoder = state.wgpu_device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
            {
                let bg = state.background_color;
                let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Render Pass"),
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
                render_pass.set_pipeline(&state.render_pipeline);
                render_pass.set_bind_group(0, &state.bind_group, &[]);
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

// --- Wayland Handlers ---

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Err(e) = self.ensure_output(&output) {
            error!("Failed to create output: {}", e);
        }
    }

    fn update_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        self.new_output(_conn, _qh, output);
    }

    fn output_destroyed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        let info = match self.output_state.info(&output) {
            Some(i) => i,
            None => return,
        };
        let name = info.name.unwrap_or_else(|| "unknown".to_string());
        if self.per_output.remove(&name).is_some() {
            info!("Output {} removed", name);
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
    fn scale_factor_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_factor: i32) {}
    fn transform_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_transform: wl_output::Transform) {}

    fn frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {
        // El renderizado ya se hace en el timer
    }

    fn surface_enter(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
    fn surface_leave(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _output: &wl_output::WlOutput) {}
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
        for (name, state) in self.per_output.iter_mut() {
            if &state.layer_surface == layer {
                let width = configure.new_size.0;
                let height = configure.new_size.1;
                if width == state.width && height == state.height && state.configured {
                    return;
                }
                state.width = width;
                state.height = height;
                state.wgpu_config.width = width;
                state.wgpu_config.height = height;
                state.wgpu_surface.configure(&state.wgpu_device, &state.wgpu_config);

                let uniforms = Uniforms::new(&self.colors, width as f32, height as f32);
                state.wgpu_queue.write_buffer(&state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

                state.configured = true;
                info!("Output {} configured: {}x{}", name, width, height);
                break;
            }
        }
    }
}