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
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::app_config::{array_from_config_color, Config};

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
    let y = coord.y;
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
    frame_count: u64,
    // Se almacena el color de fondo por si se quiere usar en clear
    background_color: [f32; 4],
}

pub struct WaylandRenderer {
    config: Config,
    audio_rx: Receiver<Vec<f32>>,
    color_rx: Arc<Mutex<Receiver<Vec<[f32; 4]>>>>,
    running: Arc<AtomicBool>,
}

impl WaylandRenderer {
    pub fn new(
        config: Config,
        audio_rx: Receiver<Vec<f32>>,
        color_rx: Arc<Mutex<Receiver<Vec<[f32; 4]>>>>,
        running: Arc<AtomicBool>,
    ) -> Self {
        Self {
            config,
            audio_rx,
            color_rx,
            running,
        }
    }

    pub fn run(self) -> Result<()> {
        info!("Iniciando renderizador Wayland con WGPU");
        std::env::set_var("EGL_PLATFORM", "wayland");

        let conn = Connection::connect_to_env().context("Failed to connect to Wayland")?;
        let (globals, event_queue) = registry_queue_init(&conn).context("Failed to init registry")?;
        let qh = event_queue.handle();
        let mut event_loop: EventLoop<AppState> =
            EventLoop::try_new().context("Failed to create event loop")?;
        let loop_handle = event_loop.handle();
        WaylandSource::new(conn.clone(), event_queue)
            .insert(loop_handle.clone())
            .map_err(|e| anyhow::anyhow!("Wayland source error: {:?}", e))?;

        let compositor = CompositorState::bind(&globals, &qh).context("wl_compositor not available")?;
        let layer_shell = LayerShell::bind(&globals, &qh).context("layer shell not available")?;

        let bar_count = self.config.bars.amount as usize;
        let bar_gap = self.config.bars.gap;
        let running = self.running.clone();
        let audio_rx = self.audio_rx;
        let color_rx = self.color_rx;
        let initial_colors: Vec<[f32; 4]> = self
            .config
            .colors
            .values()
            .map(|c| array_from_config_color(c.clone()))
            .collect();

        let background_color = array_from_config_color(self.config.general.background_color.clone());

        let mut app_state = AppState {
            registry_state: RegistryState::new(&globals),
            output_state: OutputState::new(&globals, &qh),
            layer_shell,
            compositor,
            per_output: HashMap::new(),
            bar_count,
            bar_gap,
            preferred_output_name: self.config.general.preferred_output.clone(),
            audio_rx,
            color_rx,
            running,
            current_colors: initial_colors,
            background_color,
            conn: conn.clone(),
            qh: qh.clone(),
        };

        // El bucle de eventos se ejecuta sin timeout; los frames son impulsados por surface.frame()
        event_loop
            .run(None, &mut app_state, |_| {})
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
    bar_count: usize,
    bar_gap: f32,
    preferred_output_name: Option<String>,
    audio_rx: Receiver<Vec<f32>>,
    color_rx: Arc<Mutex<Receiver<Vec<[f32; 4]>>>>,
    running: Arc<AtomicBool>,
    current_colors: Vec<[f32; 4]>,
    background_color: [f32; 4],
    conn: Connection,
    qh: QueueHandle<Self>,
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
                debug!("Omitiendo output {} (preferido es {})", name, pref);
                return Ok(());
            }
        }

        info!("Creando superficie para output {}", name);

        let surface = self.compositor.create_surface(&self.qh);
        let layer_surface = self.layer_shell.create_layer_surface(
            &self.qh,
            surface.clone(),
            Layer::Background,
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

        let uniforms = Uniforms::new(&self.current_colors, width as f32, height as f32);
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

        // La superficie debe vivir 'static, pero aquí sabemos que el programa no terminará antes que ella.
        let static_surface = unsafe { std::mem::transmute::<_, wgpu::Surface<'static>>(wgpu_surface) };

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
            frame_count: 0,
            background_color: self.background_color,
        };

        self.per_output.insert(name.clone(), state);
        info!("Superficie WGPU creada para {}: {}x{}", name, width, height);
        Ok(())
    }

    fn draw_output(&mut self, name: &str) {
        let state = match self.per_output.get_mut(name) {
            Some(s) if s.configured => s,
            Some(_) => {
                debug!("Output {} no configurado aún", name);
                return;
            }
            None => return,
        };

        // Actualizar colores desde el watcher (si hay nuevos)
        if let Ok(guard) = self.color_rx.lock() {
            if let Ok(new_colors) = guard.try_recv() {
                info!("Actualizando colores del degradado ({} colores)", new_colors.len());
                self.current_colors = new_colors.clone();
                let uniforms = Uniforms::new(&self.current_colors, state.width as f32, state.height as f32);
                state.wgpu_queue.write_buffer(&state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
            }
        }

        // Leer datos de audio (o generar onda de prueba)
        let mut bar_heights = vec![0.0; self.bar_count];
        if let Ok(new_heights) = self.audio_rx.try_recv() {
            bar_heights = new_heights;
            debug!("Usando datos reales de audio: {} barras", bar_heights.len());
        } else {
            // Modo de prueba: onda sinusoidal
            let phase = (std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f32())
                * 5.0;
            for i in 0..self.bar_count {
                bar_heights[i] = ((phase + i as f32 * 0.3).sin() * 0.5 + 0.5).clamp(0.0, 1.0);
            }
            if state.frame_count % 60 == 0 {
                warn!("Usando datos de prueba para visualización");
            }
        }

        // Calcular vértices
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
        state.wgpu_queue.write_buffer(&state.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

        // Renderizar
        let frame = match state.wgpu_surface.get_current_texture() {
            Ok(frame) => frame,
            Err(e) => {
                error!("Error obteniendo textura de la superficie: {:?}", e);
                return;
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

        state.frame_count += 1;
        // Solicitar el siguiente frame callback para mantener el ciclo
        state.surface.frame(&self.qh, state.surface.clone());
    }

    pub fn draw(&mut self) {
        if !self.running.load(Ordering::SeqCst) {
            info!("Apagando graceful...");
            std::process::exit(0);
        }

        let names: Vec<String> = self.per_output.keys().cloned().collect();
        for name in names {
            self.draw_output(&name);
        }
    }
}

// --- Implementaciones de handlers ---

impl OutputHandler for AppState {
    fn output_state(&mut self) -> &mut OutputState {
        &mut self.output_state
    }

    fn new_output(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, output: wl_output::WlOutput) {
        if let Err(e) = self.ensure_output(&output) {
            error!("Fallo al crear output: {}", e);
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
        if let Some(_state) = self.per_output.remove(&name) {
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
    fn scale_factor_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_factor: i32) {}
    fn transform_changed(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _new_transform: wl_output::Transform) {}
    fn frame(&mut self, _conn: &Connection, _qh: &QueueHandle<Self>, _surface: &wl_surface::WlSurface, _time: u32) {
        // Este callback se invoca cuando el compositor está listo para un nuevo frame
        self.draw();
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
                state.wgpu_config.width = width;
                state.wgpu_config.height = height;
                state.wgpu_surface.configure(&state.wgpu_device, &state.wgpu_config);

                let uniforms = Uniforms::new(&self.current_colors, width as f32, height as f32);
                state.wgpu_queue.write_buffer(&state.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

                state.configured = true;
                target_name = Some(name.clone());
                info!("Output {} configurado: {}x{}", name, width, height);
                break;
            }
        }
        if let Some(name) = target_name {
            // Cuando la superficie está configurada, solicitamos el primer frame
            if let Some(state) = self.per_output.get(&name) {
                state.surface.frame(&self.qh, state.surface.clone());
            }
            self.draw_output(&name);
        }
    }
}