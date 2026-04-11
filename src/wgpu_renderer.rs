use anyhow::{Context, Result};
use bytemuck_derive::{Pod, Zeroable};
use log::info;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Receiver;
use std::sync::{Arc, Mutex};
use winit::dpi::PhysicalSize;
use winit::event::{Event, WindowEvent};
use winit::event_loop::EventLoop;
use winit::window::WindowBuilder;
use wgpu::util::DeviceExt;

use crate::app_config::Config;

#[repr(C)]
#[derive(Debug, Copy, Clone, Zeroable, Pod)]
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

pub struct WgpuRenderer {
    config: Config,
    audio_rx: Receiver<Vec<f32>>,
    color_rx: Arc<Mutex<Receiver<Vec<[f32; 4]>>>>,
    running: Arc<AtomicBool>,
}

impl WgpuRenderer {
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
        info!("Starting Wgpu renderer");
        let event_loop = EventLoop::new().context("Failed to create event loop")?;
        let window = WindowBuilder::new()
            .with_title("cava-bg")
            .with_inner_size(PhysicalSize::new(1920, 1080))
            .with_decorations(false)
            .with_transparent(true)
            .build(&event_loop)
            .context("Failed to create window")?;

        let bar_count = self.config.bars.amount as usize;
        let bar_gap = self.config.bars.gap;
        let running = self.running.clone();
        let audio_rx = self.audio_rx;
        let color_rx = self.color_rx;
        let initial_colors: Vec<[f32; 4]> = self
            .config
            .colors
            .values()
            .map(|c| crate::app_config::array_from_config_color(c.clone()))
            .collect();

        event_loop.run(move |event, control_flow| {
            if !running.load(Ordering::SeqCst) {
                control_flow.exit();
                return;
            }

            use std::sync::OnceLock;
            static WG: OnceLock<(wgpu::Surface, wgpu::Device, wgpu::Queue, wgpu::SurfaceConfiguration, wgpu::Buffer, wgpu::Buffer, wgpu::Buffer, wgpu::BindGroup, wgpu::RenderPipeline)> = OnceLock::new();

            let (surface, device, queue, mut surface_config, index_buffer, vertex_buffer, uniform_buffer, bind_group, render_pipeline) = WG.get_or_init(|| {
                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::default());
                let surface = unsafe { instance.create_surface(&window) }.unwrap();
                let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
                    power_preference: wgpu::PowerPreference::HighPerformance,
                    compatible_surface: Some(&surface),
                    force_fallback_adapter: false,
                })).unwrap();
                let (device, queue) = pollster::block_on(adapter.request_device(
                    &wgpu::DeviceDescriptor {
                        label: None,
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::default(),
                    },
                    None,
                )).unwrap();

                let size = window.inner_size();
                let mut surface_config = surface.get_default_config(&adapter, size.width, size.height).unwrap();
                surface_config.present_mode = wgpu::PresentMode::Fifo;
                let caps = surface.get_capabilities(&adapter);
                surface_config.alpha_mode = caps.alpha_modes.iter().find(|m| **m != wgpu::CompositeAlphaMode::Opaque).copied().unwrap_or(wgpu::CompositeAlphaMode::Auto);
                surface.configure(&device, &surface_config);

                let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("Shader"),
                    source: wgpu::ShaderSource::Wgsl(include_str!("shader.wgsl").into()),
                });

                let mut indices = Vec::with_capacity(bar_count * 6);
                for i in 0..bar_count {
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
                    size: (bar_count * 8 * std::mem::size_of::<f32>()) as u64,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });

                let uniforms = Uniforms::new(&initial_colors, size.width as f32, size.height as f32);
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

                (surface, device, queue, surface_config, index_buffer, vertex_buffer, uniform_buffer, bind_group, render_pipeline)
            });

            let size = window.inner_size();
            if surface_config.width != size.width || surface_config.height != size.height {
                surface_config.width = size.width;
                surface_config.height = size.height;
                surface.configure(&device, &surface_config);
            }

            match event {
                Event::WindowEvent { event, .. } => match event {
                    WindowEvent::CloseRequested => control_flow.exit(),
                    WindowEvent::RedrawRequested => {
                        if let Ok(guard) = color_rx.lock() {
                            if let Ok(new_colors) = guard.try_recv() {
                                let uniforms = Uniforms::new(&new_colors, surface_config.width as f32, surface_config.height as f32);
                                queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
                            }
                        }

                        let mut bar_heights = vec![0.0; bar_count];
                        if let Ok(new_heights) = audio_rx.try_recv() {
                            bar_heights = new_heights;
                        }

                        let bar_width = 2.0 / (bar_count as f32 + (bar_count as f32 - 1.0) * bar_gap);
                        let bar_gap_width = bar_width * bar_gap;
                        let mut vertices = vec![0.0f32; bar_count * 8];
                        for i in 0..bar_count {
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
                        queue.write_buffer(&vertex_buffer, 0, bytemuck::cast_slice(&vertices));

                        let frame = match surface.get_current_texture() {
                            Ok(frame) => frame,
                            Err(_) => return,
                        };
                        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());
                        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None });
                        {
                            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: Some("Render Pass"),
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: None,
                                occlusion_query_set: None,
                                timestamp_writes: None,
                            });
                            render_pass.set_pipeline(render_pipeline);
                            render_pass.set_bind_group(0, bind_group, &[]);
                            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
                            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                            render_pass.draw_indexed(0..(bar_count * 6) as u32, 0, 0..1);
                        }
                        queue.submit(std::iter::once(encoder.finish()));
                        frame.present();
                    }
                    WindowEvent::Resized(new_size) => {
                        if new_size.width > 0 && new_size.height > 0 {
                            surface_config.width = new_size.width;
                            surface_config.height = new_size.height;
                            surface.configure(&device, &surface_config);
                        }
                    }
                    _ => {}
                },
                Event::AboutToWait => {
                    window.request_redraw();
                }
                _ => {}
            }
        });

        // Necesario para cumplir con el tipo de retorno Result<()>.
        // Esta línea nunca se ejecuta porque event_loop.run no retorna.
        #[allow(unreachable_code)]
        Ok(())
    }
}