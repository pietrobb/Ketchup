//! wgpu rendering of a package: one shared upload per GLB buffer, one draw per
//! body instance, colors, selection, visibility, display style and section.

use crate::{camera::Camera, guides::Guides, model::Model};
use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{DisplayStyle, PrimitiveTopology, Section};
use num_traits::ToPrimitive;
use std::collections::BTreeMap;
use wgpu::util::DeviceExt as _;

/// GPU resources of one opened model, created once and drawn every frame.
pub struct MeshRenderer {
    triangles: wgpu::RenderPipeline,
    lines: wgpu::RenderPipeline,
    vertices: BTreeMap<usize, wgpu::Buffer>,
    indices: BTreeMap<usize, wgpu::Buffer>,
    draws: Vec<Draw>,
    guides: Vec<GuideDraw>,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
}

/// One colour of the axes and ground grid, drawn as plain line segments.
struct GuideDraw {
    vertices: wgpu::Buffer,
    count: u32,
    bind_group: wgpu::BindGroup,
}

/// One primitive of one body: its shared buffers, owner and placement uniforms.
struct Draw {
    positions: usize,
    indices: usize,
    count: u32,
    occurrence: usize,
    topology: PrimitiveTopology,
    bind_group: wgpu::BindGroup,
}

/// Offscreen color and depth textures the 3D view is drawn into.
pub struct RenderTarget {
    pub color: wgpu::Texture,
    pub view: wgpu::TextureView,
    depth: wgpu::TextureView,
    pub size: [u32; 2],
}

impl RenderTarget {
    /// Textures of `size` pixels, usable by egui and readable back in tests.
    pub fn new(device: &wgpu::Device, size: [u32; 2]) -> Self {
        let extent = wgpu::Extent3d {
            width: size[0],
            height: size[1],
            depth_or_array_layers: 1,
        };
        let color = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Viewer color"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Viewer depth"),
                size: extent,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Depth32Float,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        // The shader writes gamma-encoded bytes, which egui samples through `view`.
        let view = color.create_view(&wgpu::TextureViewDescriptor::default());
        Self {
            color,
            view,
            depth,
            size,
        }
    }

    /// Tightly packed RGBA8 rows of the last frame, for thumbnails and pixel tests.
    pub fn read_rgba(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Result<Vec<u8>, Rejection> {
        let row = self.size[0] * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Viewer readback"),
            size: u64::from(padded) * u64::from(self.size[1]),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.color,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(self.size[1]),
                },
            },
            self.color.size(),
        );
        queue.submit([encoder.finish()]);
        let (sender, receiver) = std::sync::mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                // The receiver below outlives this callback; a failed send cannot occur.
                sender.send(result).ok();
            });
        let completed = device.poll(wgpu::PollType::Wait).is_ok()
            && receiver.recv().is_ok_and(|result| result.is_ok());
        if !completed {
            return Err(Rejection::new("viewer.gpu.readback", RejectionPhase::Io)
                .reason("The rendered frame could not be read back from the GPU.")
                .fix_hint("Retry; if it persists, update the graphics driver."));
        }
        let [row, padded] = [row, padded].map(|v| v.to_usize().expect("u32 fits usize"));
        let mapped = buffer.slice(..).get_mapped_range();
        let pixels = mapped
            .chunks_exact(padded)
            .flat_map(|line| &line[..row])
            .copied()
            .collect();
        drop(mapped);
        buffer.unmap();
        Ok(pixels)
    }
}

impl MeshRenderer {
    /// Upload the shared buffers once; refused before allocating when over budget.
    pub fn new(device: &wgpu::Device, model: &Model) -> Result<Self, Rejection> {
        let geometry = model.geometry()?;
        validate_upload(&geometry, &model.bodies, device.limits().max_buffer_size)?;
        let layout = uniform_layout(device);
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Viewer camera, selection and section"),
            size: 112,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = uniform_group(device, &layout, &uniform);
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Viewer shared geometry pipeline"),
            bind_group_layouts: &[&layout, &layout],
            push_constant_ranges: &[],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Viewer package shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("mesh.wgsl").into()),
        });
        let triangles = pipeline(device, &pipeline_layout, &shader, false);
        let lines = pipeline(device, &pipeline_layout, &shader, true);
        let mut vertices = BTreeMap::new();
        let mut indices = BTreeMap::new();
        // Upload each accessor once, including sharing across material variants.
        for primitive in geometry.meshes.iter().flatten() {
            vertices
                .entry(primitive.position_accessor)
                .or_insert_with(|| {
                    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Viewer shared positions"),
                        contents: primitive.position_bytes(),
                        usage: wgpu::BufferUsages::VERTEX,
                    })
                });
            indices.entry(primitive.index_accessor).or_insert_with(|| {
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Viewer shared indices"),
                    contents: primitive.index_bytes(),
                    usage: wgpu::BufferUsages::INDEX,
                })
            });
        }
        let mut draws = Vec::new();
        let definitions: BTreeMap<_, _> = model
            .manifest()
            .definitions
            .iter()
            .map(|d| (d.id, d))
            .collect();
        for body in &model.bodies {
            let occurrence = &model.manifest().occurrences[body.occurrence];
            let definition = definitions[&occurrence.definition_id];
            for primitive in &geometry.meshes[body.mesh] {
                let color = if primitive.topology == PrimitiveTopology::Lines {
                    [0.025, 0.025, 0.025, 1.0]
                } else if let Some(color) = occurrence.color_srgb.or(definition.color_srgb) {
                    let [r, g, b] = color.map(srgb_linear);
                    [r, g, b, 1.0]
                } else {
                    primitive
                        .color_linear
                        .unwrap_or([0.25, 0.65, 0.85, 1.0])
                        .map(|v| v.to_f32().expect("bounded color"))
                };
                let bytes: Vec<_> = body
                    .world_matrix_mm
                    .iter()
                    .copied()
                    .chain(color)
                    .chain([
                        body.occurrence.to_f32().expect("bounded occurrence"),
                        0.0,
                        0.0,
                        0.0,
                    ])
                    .flat_map(f32::to_le_bytes)
                    .collect();
                let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Viewer instance placement and material"),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::UNIFORM,
                });
                draws.push(Draw {
                    positions: primitive.position_accessor,
                    indices: primitive.index_accessor,
                    count: u32::try_from(primitive.indices().len()).expect("bounded indices"),
                    occurrence: body.occurrence,
                    topology: primitive.topology,
                    bind_group: uniform_group(device, &layout, &buffer),
                });
            }
        }
        let guides = guide_draws(device, &layout, &Guides::new(model.bounds_mm));
        Ok(Self {
            triangles,
            lines,
            vertices,
            indices,
            draws,
            guides,
            uniform,
            bind_group,
        })
    }

    /// Draw one frame of the model as `view` describes it.
    pub fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        target: &RenderTarget,
        camera: &Camera,
        view: &RenderView<'_>,
    ) {
        let matrix = camera.matrix(
            target.size[0].to_f32().expect("GPU width")
                / target.size[1].to_f32().expect("GPU height"),
        );
        let plane = view.section.map_or([0.0; 4], |section| {
            [
                section.normal[0],
                section.normal[1],
                section.normal[2],
                section.offset_mm,
            ]
            .map(|v| v.to_f32().expect("bounded section"))
        });
        let bytes: Vec<_> = matrix
            .iter()
            .flatten()
            .copied()
            .chain([
                view.selected
                    .map_or(-1.0, |i| i.to_f32().expect("bounded occurrence")),
                0.0,
                0.0,
                0.0,
            ])
            .chain(plane)
            .chain([
                if view.section.is_some() { 1.0 } else { 0.0 },
                0.0,
                0.0,
                0.0,
            ])
            .flat_map(f32::to_le_bytes)
            .collect();
        queue.write_buffer(&self.uniform, 0, &bytes);
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Viewer frame"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Viewer package geometry"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target.view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: linear_srgb(0.025),
                            g: linear_srgb(0.035),
                            b: linear_srgb(0.05),
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &target.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });
            pass.set_bind_group(0, &self.bind_group, &[0]);
            // Fill depth for every body before drawing any display edges.
            for draw in [PrimitiveTopology::Triangles, PrimitiveTopology::Lines]
                .into_iter()
                .flat_map(|topology| {
                    self.draws
                        .iter()
                        .filter(move |draw| draw.topology == topology)
                })
            {
                if !view.visible[draw.occurrence]
                    || match draw.topology {
                        PrimitiveTopology::Triangles => {
                            matches!(view.style, DisplayStyle::Wireframe)
                        }
                        PrimitiveTopology::Lines => matches!(view.style, DisplayStyle::Shaded),
                    }
                {
                    continue;
                }
                pass.set_pipeline(if draw.topology == PrimitiveTopology::Lines {
                    &self.lines
                } else {
                    &self.triangles
                });
                pass.set_bind_group(1, &draw.bind_group, &[0]);
                pass.set_vertex_buffer(0, self.vertices[&draw.positions].slice(..));
                pass.set_index_buffer(
                    self.indices[&draw.indices].slice(..),
                    wgpu::IndexFormat::Uint32,
                );
                pass.draw_indexed(0..draw.count, 0, 0..1);
            }
            if view.guides {
                // After the bodies, so the model hides the grid and axes behind it.
                pass.set_pipeline(&self.lines);
                for guide in &self.guides {
                    pass.set_bind_group(1, &guide.bind_group, &[0]);
                    pass.set_vertex_buffer(0, guide.vertices.slice(..));
                    pass.draw(0..guide.count, 0..1);
                }
            }
        }
        queue.submit([encoder.finish()]);
    }
}

/// The view state of a frame: selection, visibility, style, section and
/// whether the orientation axes and ground grid are shown.
pub struct RenderView<'a> {
    pub selected: Option<usize>,
    pub visible: &'a [bool],
    pub style: &'a DisplayStyle,
    pub section: Option<&'a Section>,
    pub guides: bool,
}

/// Upload the guides once per model, one draw per colour.
fn guide_draws(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    guides: &Guides,
) -> Vec<GuideDraw> {
    const X: [f32; 3] = [0.85, 0.06, 0.05];
    const Y: [f32; 3] = [0.05, 0.62, 0.08];
    const Z: [f32; 3] = [0.08, 0.2, 0.95];
    let dim = |c: [f32; 3]| c.map(|v| v * 0.55);
    let groups = [
        (&guides.grid, [0.11, 0.13, 0.16]),
        (&guides.axes[0], X),
        (&guides.axes[1], Y),
        (&guides.axes[2], Z),
        (&guides.negative_axes[0], dim(X)),
        (&guides.negative_axes[1], dim(Y)),
        (&guides.negative_axes[2], dim(Z)),
    ];
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    groups
        .into_iter()
        .filter(|(segments, _)| !segments.is_empty())
        .map(|(segments, [r, g, b])| {
            // Package geometry is glTF metres with Y up; the shader converts back.
            let bytes: Vec<u8> = segments
                .iter()
                .flatten()
                .flat_map(|[x, y, z]| [x / 1000.0, z / 1000.0, -y / 1000.0])
                .flat_map(f32::to_le_bytes)
                .collect();
            let vertices = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Viewer axes and grid"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
            // Never selectable (-2) and never cut by the scene section (1).
            let uniform: Vec<u8> = identity
                .into_iter()
                .chain([r, g, b, 1.0, -2.0, 1.0, 0.0, 0.0])
                .flat_map(f32::to_le_bytes)
                .collect();
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Viewer guide colour"),
                contents: &uniform,
                usage: wgpu::BufferUsages::UNIFORM,
            });
            GuideDraw {
                vertices,
                count: u32::try_from(segments.len() * 2).expect("bounded guide count"),
                bind_group: uniform_group(device, layout, &buffer),
            }
        })
        .collect()
}

/// Refuse geometry over the device buffer limit or the upload and draw budget.
fn validate_upload(
    geometry: &ketchup_view_format::Geometry<'_>,
    bodies: &[crate::model::Body],
    max_buffer: u64,
) -> Result<(), Rejection> {
    let mut sizes = BTreeMap::new();
    for primitive in geometry.meshes.iter().flatten() {
        sizes.insert(
            (0, primitive.position_accessor),
            u64::try_from(primitive.position_bytes().len()).expect("bounded bytes"),
        );
        sizes.insert(
            (1, primitive.index_accessor),
            u64::try_from(primitive.index_bytes().len()).expect("bounded bytes"),
        );
    }
    let draws: u64 = bodies
        .iter()
        .map(|body| u64::try_from(geometry.meshes[body.mesh].len()).expect("bounded primitives"))
        .sum();
    if sizes.values().any(|&size| size > max_buffer)
        || sizes.values().sum::<u64>() > 256 * 1024 * 1024
        || draws > 100_000
    {
        return Err(Rejection::new("viewer.gpu.limit", RejectionPhase::Request)
            .target("display geometry")
            .reason("Display geometry exceeds the device buffer limit, 256 MiB shared upload budget, or 100,000 instance primitives.")
            .fix_hint("Export a smaller model or fewer display instances for this device."));
    }
    Ok(())
}

fn uniform_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Viewer uniforms"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                // Root CBVs avoid static descriptor aliasing observed on DX12/WARP.
                has_dynamic_offset: true,
                min_binding_size: None,
            },
            count: None,
        }],
    })
}

fn uniform_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Viewer uniform group"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}

/// Linear light as an sRGB-encoded channel, like the shader's `encode`.
fn linear_srgb(value: f64) -> f64 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

/// An 8-bit sRGB channel as linear light.
fn srgb_linear(value: u8) -> f32 {
    let v = f32::from(value) / 255.0;
    if v <= 0.04045 {
        v / 12.92
    } else {
        ((v + 0.055) / 1.055).powf(2.4)
    }
}

/// The depth-tested pipeline for triangles or lines.
fn pipeline(
    device: &wgpu::Device,
    layout: &wgpu::PipelineLayout,
    shader: &wgpu::ShaderModule,
    lines: bool,
) -> wgpu::RenderPipeline {
    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Viewer depth-tested primitive"),
        layout: Some(layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vertex_main"),
            compilation_options: Default::default(),
            buffers: &[wgpu::VertexBufferLayout {
                array_stride: 12,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &[wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x3,
                    offset: 0,
                    shader_location: 0,
                }],
            }],
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(if lines {
                "fragment_edge"
            } else {
                "fragment_main"
            }),
            compilation_options: Default::default(),
            targets: &[Some(wgpu::ColorTargetState {
                format: wgpu::TextureFormat::Rgba8Unorm,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
        }),
        primitive: wgpu::PrimitiveState {
            topology: if lines {
                wgpu::PrimitiveTopology::LineList
            } else {
                wgpu::PrimitiveTopology::TriangleList
            },
            ..Default::default()
        },
        depth_stencil: Some(wgpu::DepthStencilState {
            format: wgpu::TextureFormat::Depth32Float,
            depth_write_enabled: !lines,
            depth_compare: wgpu::CompareFunction::LessEqual,
            stencil: Default::default(),
            // Faces are pushed back so their edges stay visible; WebGPU allows
            // depth bias only on triangles, never on lines.
            bias: wgpu::DepthBiasState {
                constant: if lines { 0 } else { 1 },
                ..Default::default()
            },
        }),
        multisample: Default::default(),
        multiview: None,
        cache: None,
    })
}
