//! Underlays on the GPU: every segment of an underlay in one vertex buffer, uploaded once,
//! and one line-list draw per visible layer, painted over the ground grid and under the
//! model. A camera frame only rewrites the camera; showing or hiding a layer only changes
//! which ranges are drawn.

use super::{linear_rgba, single_precision};
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_model::underlay::UnderlayLines;
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

/// The segments of one underlay as GPU vertices: two points per segment, in millimetres
/// in the XY plane, with the vertex range of each layer.
#[derive(Debug, PartialEq)]
pub struct UnderlayGeometry {
    /// SHA-256 of the lines' blob: the same lines upload once.
    key: [u8; 32],
    points: Vec<[f32; 2]>,
    /// In the order of the underlay's layers.
    layers: Vec<Range<u32>>,
}

impl UnderlayGeometry {
    #[must_use]
    pub fn new(key: [u8; 32], lines: &UnderlayLines) -> Self {
        let mut points = Vec::with_capacity(lines.segment_count() * 2);
        let mut layers = Vec::with_capacity(lines.layers().len());
        for layer in lines.layers() {
            let first = vertex_index(points.len());
            for segment in layer.segments() {
                points.extend(segment.map(|[x, y]| {
                    let [x, y, _] = single_precision([x, y, 0.0]);
                    [x, y]
                }));
            }
            layers.push(first..vertex_index(points.len()));
        }
        Self {
            key,
            points,
            layers,
        }
    }

    /// The vertex range of each layer, in the order of the underlay's layers.
    #[must_use]
    pub fn layers(&self) -> &[Range<u32>] {
        &self.layers
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.points.len() / 2
    }
}

/// A vertex count the GPU draws; a buffer this large could not be allocated anyway.
fn vertex_index(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// One underlay of the frame and the vertex ranges of its visible layers (none while it
/// or all its layers are hidden: its upload is kept for when it is shown again).
#[derive(Clone, Debug)]
pub struct UnderlayDraw {
    pub geometry: Arc<UnderlayGeometry>,
    pub visible: Vec<Range<u32>>,
}

/// The GPU state of the underlays, kept in the paint callback resources.
pub struct GpuUnderlayRenderer {
    pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    buffers: BTreeMap<[u8; 32], wgpu::Buffer>,
    draws: Vec<UnderlayDraw>,
    /// How many line buffers were uploaded: once per underlay, not per frame.
    uploads: u64,
}

impl GpuUnderlayRenderer {
    #[must_use]
    pub fn new(device: &wgpu::Device, target_format: wgpu::TextureFormat) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ketchup underlay shader"),
            source: wgpu::ShaderSource::Wgsl(UNDERLAY_SHADER.into()),
        });
        let uniform = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ketchup underlay camera and color"),
            size: 80,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup underlay bind group layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ketchup underlay bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform.as_entire_binding(),
            }],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ketchup underlay pipeline layout"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Ketchup underlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_underlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: 8,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_underlay"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        Self {
            pipeline,
            uniform,
            bind_group,
            buffers: BTreeMap::new(),
            draws: Vec::new(),
            uploads: 0,
        }
    }

    /// Writes the camera and color and uploads the lines of underlays not on the GPU yet;
    /// the lines of underlays no longer drawn are released.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[UnderlayDraw],
        world_to_clip: [f32; 16],
        color: [f32; 4],
    ) {
        let uniform = world_to_clip
            .into_iter()
            .chain(color)
            .flat_map(f32::to_ne_bytes)
            .collect::<Vec<_>>();
        queue.write_buffer(&self.uniform, 0, &uniform);
        self.buffers.retain(|key, _| {
            draws
                .iter()
                .any(|draw| draw.geometry.key == *key && !draw.geometry.points.is_empty())
        });
        for draw in draws {
            let geometry = &draw.geometry;
            if geometry.points.is_empty() || self.buffers.contains_key(&geometry.key) {
                continue;
            }
            let bytes = geometry
                .points
                .iter()
                .flatten()
                .flat_map(|value| value.to_ne_bytes())
                .collect::<Vec<_>>();
            let buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Ketchup underlay lines"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            });
            self.buffers.insert(geometry.key, buffer);
            self.uploads += 1;
        }
        self.draws = draws.to_vec();
    }

    /// Draws the visible layers prepared last.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        for draw in &self.draws {
            let Some(buffer) = self.buffers.get(&draw.geometry.key) else {
                continue;
            };
            pass.set_vertex_buffer(0, buffer.slice(..));
            for range in &draw.visible {
                pass.draw(range.clone(), 0..1);
            }
        }
    }

    #[must_use]
    pub fn uploads(&self) -> u64 {
        self.uploads
    }
}

/// Paints the underlays of a frame under the model.
#[derive(Clone)]
pub struct UnderlayPaintCallback {
    draws: Vec<UnderlayDraw>,
    world_to_clip: [f32; 16],
    color: [f32; 4],
}

impl UnderlayPaintCallback {
    /// `world_to_clip` column-major, as the scene's; `color` opaque sRGB.
    #[must_use]
    pub fn new(draws: Vec<UnderlayDraw>, world_to_clip: [f32; 16], color: [u8; 3]) -> Self {
        Self {
            draws,
            world_to_clip,
            color: linear_rgba(color),
        }
    }
}

impl CallbackTrait for UnderlayPaintCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen_descriptor: &ScreenDescriptor,
        _egui_encoder: &mut wgpu::CommandEncoder,
        callback_resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Some(renderer) = callback_resources.get_mut::<GpuUnderlayRenderer>() {
            renderer.prepare(device, queue, &self.draws, self.world_to_clip, self.color);
        }
        Vec::new()
    }

    fn paint(
        &self,
        _info: eframe::egui::PaintCallbackInfo,
        render_pass: &mut wgpu::RenderPass<'static>,
        callback_resources: &CallbackResources,
    ) {
        if let Some(renderer) = callback_resources.get::<GpuUnderlayRenderer>() {
            renderer.draw(render_pass);
        }
    }
}

const UNDERLAY_SHADER: &str = r"
struct Underlay {
    world_to_clip: mat4x4<f32>,
    color: vec4<f32>,
};

@group(0) @binding(0)
var<uniform> underlay: Underlay;

@vertex
fn vs_underlay(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
    return underlay.world_to_clip * vec4<f32>(position, 0.0, 1.0);
}

@fragment
fn fs_underlay() -> @location(0) vec4<f32> {
    return underlay.color;
}
";

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::ProfileSegment;
    use ketchup_model::underlay::UnderlayLinesBuilder;

    #[test]
    fn each_layer_gets_the_vertex_range_of_its_segments() {
        let mut builder = UnderlayLinesBuilder::default();
        let line = |from: [f64; 2], to: [f64; 2]| ProfileSegment::Line {
            start_mm: from,
            end_mm: to,
        };
        builder.add_loose("walls", &line([0.0, 0.0], [1000.0, 0.0]));
        builder.add_loose("walls", &line([1000.0, 0.0], [1000.0, 500.0]));
        builder.add_loose("axes", &line([0.0, -100.0], [0.0, 600.0]));
        let (names, lines) = builder.finish();
        let geometry = UnderlayGeometry::new([7; 32], &lines);
        assert_eq!(geometry.segment_count(), 3);
        let ranges = names
            .iter()
            .zip(geometry.layers())
            .map(|(name, range)| (name.as_str(), range.end - range.start))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ranges, BTreeMap::from([("axes", 2), ("walls", 4)]));
    }
}
