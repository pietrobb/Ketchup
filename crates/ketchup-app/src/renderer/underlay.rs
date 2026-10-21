//! Underlays on the GPU, uploaded once: the segments of an underlay in a vertex buffer
//! drawn as one line list per visible layer, and the letters of its texts as instances of
//! the outlines of the glyphs they use (see [`letters`]). A buffer larger than the device
//! allows is split in pieces. They are painted over the ground grid and under the model.
//! A camera frame rewrites the camera and chooses how each cell of letters is drawn;
//! showing or hiding a layer only changes which line ranges are drawn and which layers'
//! letters the GPU keeps.

mod letters;
mod text;

use super::{linear_rgba, single_precision};
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_model::underlay::UnderlayLines;
use letters::{CellView, INSTANCE_STRIDE, LetterInstance, Letters};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

/// The segments and letters of one underlay as GPU data, in millimetres in the XY plane.
#[derive(Debug, PartialEq)]
pub struct UnderlayGeometry {
    /// SHA-256 of the lines' blob: the same lines upload once.
    key: [u8; 32],
    /// Two points per segment.
    points: Vec<[f32; 2]>,
    /// The vertex range of each layer's segments, in the order of the underlay's layers.
    layers: Vec<Range<u32>>,
    letters: Letters,
    /// Per layer, the vertices its segments and letters draw.
    layer_vertices: Vec<u64>,
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
        let mut layer_vertices = layers
            .iter()
            .map(|range| u64::from(range.end - range.start))
            .collect::<Vec<_>>();
        let letters = Letters::new(lines, &mut layer_vertices);
        Self {
            key,
            points,
            layers,
            letters,
            layer_vertices,
        }
    }

    #[must_use]
    pub fn layer_count(&self) -> usize {
        self.layers.len()
    }

    /// The vertices the segments and letters of one layer draw.
    #[must_use]
    pub fn layer_vertices(&self, layer: usize) -> u64 {
        self.layer_vertices.get(layer).copied().unwrap_or(0)
    }

    #[must_use]
    pub fn segment_count(&self) -> usize {
        self.points.len() / 2
    }

    /// The letters of its texts that draw something (no spaces).
    #[must_use]
    pub fn letter_count(&self) -> usize {
        self.letters.instances.len()
    }

    fn is_empty(&self) -> bool {
        self.points.is_empty() && self.letters.instances.is_empty()
    }
}

/// A vertex or instance count the GPU draws.
fn vertex_index(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// One underlay of the frame and the indices of its visible layers (none while it or all
/// its layers are hidden: its upload is kept for when it is shown again).
#[derive(Clone, Debug)]
pub struct UnderlayDraw {
    pub geometry: Arc<UnderlayGeometry>,
    pub visible: Vec<usize>,
}

/// A piece of a buffer too large for one: the elements `first..first + count`.
struct Chunk {
    buffer: wgpu::Buffer,
    first: u32,
    count: u32,
}

/// The part of `range` (of all elements) inside the chunk `first..first + count`, counted
/// from the chunk's start.
fn local_range(range: &Range<u32>, first: u32, count: u32) -> Option<Range<u32>> {
    let start = range.start.max(first);
    let end = range.end.min(first.saturating_add(count));
    (start < end).then(|| start - first..end - first)
}

/// `items` in buffers no larger than the device allows, `stride` bytes each; a buffer
/// holds a multiple of `multiple` items, so no segment is split.
fn upload_chunked<T>(
    device: &wgpu::Device,
    label: &str,
    items: &[T],
    stride: u64,
    multiple: usize,
    write: impl Fn(&T, &mut Vec<u8>),
) -> Vec<Chunk> {
    let fit = usize::try_from(device.limits().max_buffer_size / stride).unwrap_or(usize::MAX);
    let per_chunk = (fit / multiple * multiple).max(multiple);
    items
        .chunks(per_chunk)
        .enumerate()
        .map(|(index, chunk)| {
            let mut bytes = Vec::with_capacity(
                chunk
                    .len()
                    .saturating_mul(usize::try_from(stride).unwrap_or(0)),
            );
            for item in chunk {
                write(item, &mut bytes);
            }
            Chunk {
                buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(label),
                    contents: &bytes,
                    usage: wgpu::BufferUsages::VERTEX,
                }),
                first: vertex_index(index * per_chunk),
                count: vertex_index(chunk.len()),
            }
        })
        .collect()
}

fn write_point(point: &[f32; 2], bytes: &mut Vec<u8>) {
    bytes.extend(point.iter().flat_map(|value| value.to_ne_bytes()));
}

/// The cells of letters this frame draws as `view`.
fn cells_drawn<'a>(
    uploaded: &'a Uploaded,
    letters: &'a Letters,
    view: CellView,
) -> impl Iterator<Item = &'a letters::LetterCell> {
    letters
        .cells
        .iter()
        .zip(&uploaded.cell_views)
        .filter(move |(_, drawn)| **drawn == view)
        .map(|(cell, _)| cell)
}

/// One underlay on the GPU.
struct Uploaded {
    lines: Vec<Chunk>,
    outlines: Option<wgpu::Buffer>,
    instances: Vec<Chunk>,
    /// One `u32` per layer, nonzero while its letters are shown.
    shown: wgpu::Buffer,
    shown_group: wgpu::BindGroup,
    shown_layers: Vec<u32>,
    /// How this frame draws each cell of letters.
    cell_views: Vec<CellView>,
}

/// The GPU state of the underlays, kept in the paint callback resources.
pub struct GpuUnderlayRenderer {
    lines_pipeline: wgpu::RenderPipeline,
    letters_pipeline: wgpu::RenderPipeline,
    strokes_pipeline: wgpu::RenderPipeline,
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    shown_layout: wgpu::BindGroupLayout,
    uploaded: BTreeMap<[u8; 32], Uploaded>,
    draws: Vec<UnderlayDraw>,
    /// How many underlays were uploaded: once per underlay, not per frame.
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
        let shown_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup underlay shown layers layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Storage { read_only: true },
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
        let pipeline = |label: &str,
                        layouts: &[&wgpu::BindGroupLayout],
                        entry_point: &str,
                        buffers: &[wgpu::VertexBufferLayout<'_>]| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: layouts,
                push_constant_ranges: &[],
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(entry_point),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers,
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
            })
        };
        let point = wgpu::vertex_attr_array![0 => Float32x2];
        let instance = wgpu::vertex_attr_array![
            1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Uint32, 5 => Float32
        ];
        let point_buffer = wgpu::VertexBufferLayout {
            array_stride: 8,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &point,
        };
        let instance_buffer = wgpu::VertexBufferLayout {
            array_stride: INSTANCE_STRIDE,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &instance,
        };
        let lines_pipeline = pipeline(
            "Ketchup underlay lines pipeline",
            &[&layout],
            "vs_underlay",
            std::slice::from_ref(&point_buffer),
        );
        let letters_pipeline = pipeline(
            "Ketchup underlay letters pipeline",
            &[&layout, &shown_layout],
            "vs_letter",
            &[point_buffer, instance_buffer.clone()],
        );
        let strokes_pipeline = pipeline(
            "Ketchup underlay letter strokes pipeline",
            &[&layout, &shown_layout],
            "vs_stroke",
            &[instance_buffer],
        );
        Self {
            lines_pipeline,
            letters_pipeline,
            strokes_pipeline,
            uniform,
            bind_group,
            shown_layout,
            uploaded: BTreeMap::new(),
            draws: Vec::new(),
            uploads: 0,
        }
    }

    /// Writes the camera and color, uploads underlays not on the GPU yet, which of their
    /// layers' letters show and how each cell of letters is drawn on a viewport
    /// `viewport_px` pixels wide and high; underlays no longer drawn are released.
    pub fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        draws: &[UnderlayDraw],
        world_to_clip: [f32; 16],
        viewport_px: [f32; 2],
        color: [f32; 4],
    ) {
        let uniform = world_to_clip
            .into_iter()
            .chain(color)
            .flat_map(f32::to_ne_bytes)
            .collect::<Vec<_>>();
        queue.write_buffer(&self.uniform, 0, &uniform);
        self.uploaded.retain(|key, _| {
            draws
                .iter()
                .any(|draw| draw.geometry.key == *key && !draw.geometry.is_empty())
        });
        for draw in draws {
            let geometry = &draw.geometry;
            if geometry.is_empty() {
                continue;
            }
            if !self.uploaded.contains_key(&geometry.key) {
                let uploaded = self.upload(device, geometry);
                self.uploaded.insert(geometry.key, uploaded);
                self.uploads += 1;
            }
            let Some(uploaded) = self.uploaded.get_mut(&geometry.key) else {
                continue;
            };
            let mut shown = vec![0u32; uploaded.shown_layers.len()];
            for &layer in &draw.visible {
                if let Some(slot) = shown.get_mut(layer) {
                    *slot = 1;
                }
            }
            if shown != uploaded.shown_layers {
                let bytes = shown
                    .iter()
                    .flat_map(|value| value.to_ne_bytes())
                    .collect::<Vec<_>>();
                queue.write_buffer(&uploaded.shown, 0, &bytes);
                uploaded.shown_layers = shown;
            }
            uploaded.cell_views.clear();
            uploaded.cell_views.extend(
                geometry
                    .letters
                    .cells
                    .iter()
                    .map(|cell| cell.view(&world_to_clip, viewport_px)),
            );
        }
        self.draws = draws.to_vec();
    }

    fn upload(&self, device: &wgpu::Device, geometry: &UnderlayGeometry) -> Uploaded {
        let letters = &geometry.letters;
        let outlines = (!letters.outlines.is_empty()).then(|| {
            let mut bytes = Vec::with_capacity(letters.outlines.len() * 8);
            for point in &letters.outlines {
                write_point(point, &mut bytes);
            }
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Ketchup underlay glyph outlines"),
                contents: &bytes,
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        // At least one slot: a binding is never empty. Every layer starts hidden.
        let layers = geometry.layer_count().max(1);
        let shown = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Ketchup underlay shown layers"),
            contents: &vec![0u8; layers * 4],
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        });
        let shown_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ketchup underlay shown layers"),
            layout: &self.shown_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: shown.as_entire_binding(),
            }],
        });
        Uploaded {
            lines: upload_chunked(
                device,
                "Ketchup underlay lines",
                &geometry.points,
                8,
                2,
                write_point,
            ),
            outlines,
            instances: upload_chunked(
                device,
                "Ketchup underlay letters",
                &letters.instances,
                INSTANCE_STRIDE,
                1,
                LetterInstance::write,
            ),
            shown,
            shown_group,
            shown_layers: vec![0; layers],
            cell_views: Vec::new(),
        }
    }

    /// Draws the visible layers prepared last.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let drawn = || {
            self.draws
                .iter()
                .filter(|draw| !draw.visible.is_empty())
                .filter_map(|draw| Some((draw, self.uploaded.get(&draw.geometry.key)?)))
        };
        pass.set_pipeline(&self.lines_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        for (draw, uploaded) in drawn() {
            for chunk in &uploaded.lines {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                for range in draw
                    .visible
                    .iter()
                    .filter_map(|&layer| draw.geometry.layers.get(layer))
                {
                    if let Some(local) = local_range(range, chunk.first, chunk.count) {
                        pass.draw(local, 0..1);
                    }
                }
            }
        }
        // The cells of letters drawn whole, glyph by glyph.
        pass.set_pipeline(&self.letters_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        for (draw, uploaded) in drawn() {
            let Some(outlines) = &uploaded.outlines else {
                continue;
            };
            pass.set_bind_group(1, &uploaded.shown_group, &[]);
            pass.set_vertex_buffer(0, outlines.slice(..));
            let letters = &draw.geometry.letters;
            for chunk in &uploaded.instances {
                pass.set_vertex_buffer(1, chunk.buffer.slice(..));
                for cell in cells_drawn(uploaded, letters, CellView::Whole) {
                    for run in &letters.runs[cell.runs.clone()] {
                        if let Some(local) = local_range(&run.instances, chunk.first, chunk.count) {
                            pass.draw(run.vertices.clone(), local);
                        }
                    }
                }
            }
        }
        // The cells of letters too small to read, one stroke per letter.
        pass.set_pipeline(&self.strokes_pipeline);
        for (draw, uploaded) in drawn() {
            let letters = &draw.geometry.letters;
            pass.set_bind_group(1, &uploaded.shown_group, &[]);
            for chunk in &uploaded.instances {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                for cell in cells_drawn(uploaded, letters, CellView::Strokes) {
                    if let Some(local) = local_range(&cell.instances, chunk.first, chunk.count) {
                        pass.draw(0..2, local);
                    }
                }
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
    viewport_px: [f32; 2],
    color: [f32; 4],
}

impl UnderlayPaintCallback {
    /// `world_to_clip` column-major, as the scene's, onto a viewport `viewport_px` pixels
    /// wide and high; `color` opaque sRGB.
    #[must_use]
    pub fn new(
        draws: Vec<UnderlayDraw>,
        world_to_clip: [f32; 16],
        viewport_px: [f32; 2],
        color: [u8; 3],
    ) -> Self {
        Self {
            draws,
            world_to_clip,
            viewport_px,
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
            renderer.prepare(
                device,
                queue,
                &self.draws,
                self.world_to_clip,
                self.viewport_px,
                self.color,
            );
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

@group(1) @binding(0)
var<storage, read> layer_shown: array<u32>;

@vertex
fn vs_underlay(@location(0) position: vec2<f32>) -> @builtin(position) vec4<f32> {
    return underlay.world_to_clip * vec4<f32>(position, 0.0, 1.0);
}

// A letter of a hidden layer goes outside the clip volume, where nothing of it is drawn.
fn letter_point(position: vec2<f32>, layer: u32) -> vec4<f32> {
    if (layer_shown[layer] == 0u) {
        return vec4<f32>(2.0, 2.0, 2.0, 1.0);
    }
    return underlay.world_to_clip * vec4<f32>(position, 0.0, 1.0);
}

@vertex
fn vs_letter(
    @location(0) outline: vec2<f32>,
    @location(1) origin: vec2<f32>,
    @location(2) x_axis: vec2<f32>,
    @location(3) y_axis: vec2<f32>,
    @location(4) layer: u32,
) -> @builtin(position) vec4<f32> {
    return letter_point(origin + x_axis * outline.x + y_axis * outline.y, layer);
}

// A letter too small to read: a stroke along its baseline, as long as it advances.
@vertex
fn vs_stroke(
    @builtin(vertex_index) end: u32,
    @location(1) origin: vec2<f32>,
    @location(2) x_axis: vec2<f32>,
    @location(4) layer: u32,
    @location(5) advance: f32,
) -> @builtin(position) vec4<f32> {
    return letter_point(origin + x_axis * advance * f32(end), layer);
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
            .zip(&geometry.layers)
            .map(|(name, range)| (name.as_str(), range.end - range.start))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ranges, BTreeMap::from([("axes", 2), ("walls", 4)]));
        assert_eq!(geometry.layer_vertices(1), 2);
    }

    #[test]
    fn a_range_split_over_chunks_draws_each_part_from_its_chunks_start() {
        assert_eq!(local_range(&(4..10), 0, 6), Some(4..6));
        assert_eq!(local_range(&(4..10), 6, 6), Some(0..4));
        assert_eq!(local_range(&(4..10), 12, 6), None);
        assert_eq!(local_range(&(0..0), 0, 6), None);
    }

    #[test]
    fn letters_are_instances_of_one_outline_per_glyph_on_their_layer() {
        use ketchup_model::underlay::text::UnderlayText;
        let mut builder = UnderlayLinesBuilder::default();
        builder.add_loose(
            "walls",
            &ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [1000.0, 0.0],
            },
        );
        builder.add_text(
            "labels",
            UnderlayText::new("Hall Hall", [0.0, 500.0], 100.0),
        );
        let (names, lines) = builder.finish();
        assert_eq!(names, ["walls", "labels"]);
        let geometry = UnderlayGeometry::new([8; 32], &lines);
        let [walls, labels] = &geometry.layers[..] else {
            panic!("two layers")
        };
        assert_eq!((walls.len(), labels.len()), (2, 0));
        // Eight letters (not the space) of three glyphs: H, a and l, each outline once.
        let letters = &geometry.letters;
        assert_eq!(geometry.letter_count(), 8);
        assert_eq!(letters.runs.len(), 3);
        let per_run = letters
            .runs
            .iter()
            .map(|run| run.instances.len())
            .collect::<Vec<_>>();
        assert_eq!(per_run.iter().sum::<usize>(), 8);
        assert!(letters.instances.iter().all(|letter| letter.layer == 1));
        let outline_vertices = letters
            .runs
            .iter()
            .map(|run| (run.vertices.len() * run.instances.len()) as u64)
            .sum::<u64>();
        assert_eq!(geometry.layer_vertices(1), outline_vertices);
        assert_eq!(geometry.layer_vertices(0), 2);
        // Every letter's outline lies in the text's box: 100 mm capitals on y = 500, the
        // l a little taller, the a a little below the baseline.
        for run in &letters.runs {
            let outline = &letters.outlines[run.vertices.start as usize..run.vertices.end as usize];
            for letter in
                &letters.instances[run.instances.start as usize..run.instances.end as usize]
            {
                for [x, y] in outline {
                    let point = [0, 1].map(|axis| {
                        letter.origin[axis] + letter.x_axis[axis] * x + letter.y_axis[axis] * y
                    });
                    assert!(
                        (0.0..1000.0).contains(&point[0]) && (490.0..640.0).contains(&point[1]),
                        "{point:?}"
                    );
                }
            }
        }
    }
}
