//! Underlays on the GPU, uploaded once: the segments of an underlay as instances of a quad,
//! drawn per visible layer as wide as its lineweight in its color and dash pattern, and the
//! letters of its texts as instances of the outlines of the glyphs they use (see
//! [`letters`]) in their color. A buffer larger than the device allows is split in pieces.
//! They are painted over the ground grid and under the model. A camera frame rewrites the
//! camera and chooses how each cell of letters is drawn; showing or hiding a layer only
//! changes which segment ranges are drawn and which layers' letters the GPU keeps.

mod letters;
mod text;

use super::{linear_rgba, single_precision};
use eframe::egui_wgpu::{CallbackResources, CallbackTrait, ScreenDescriptor};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::style::LineStyle;
use letters::{CellView, INSTANCE_STRIDE, LetterInstance, Letters};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

/// Pixels a line of a lineweight of one millimetre is wide: as a 96 dpi screen shows it.
const PX_PER_WEIGHT_MM: f32 = 96.0 / 25.4;

/// Bytes of one [`Segment`] on the GPU.
const SEGMENT_STRIDE: u64 = 24;
/// Bytes of one [`GpuStyle`] on the GPU.
const STYLE_STRIDE: usize = 32;

/// A straight piece of a polyline: its ends, how far along its polyline it starts, in
/// millimetres, where its dash pattern starts, and the index of its style.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Segment {
    start: [f32; 2],
    end: [f32; 2],
    from_mm: f32,
    style: u32,
}

impl Segment {
    fn write(&self, bytes: &mut Vec<u8>) {
        for value in self.start.iter().chain(&self.end).chain([&self.from_mm]) {
            bytes.extend(value.to_ne_bytes());
        }
        bytes.extend(self.style.to_ne_bytes());
    }
}

/// A style as the shader reads it: linear RGBA, alpha 0 for the underlay's own color; its
/// width in pixels; its pattern's lengths in [`UnderlayGeometry::patterns`] and how long
/// it repeats, in millimetres.
#[derive(Clone, Debug, PartialEq)]
struct GpuStyle {
    color: [f32; 4],
    width_px: f32,
    pattern: Range<u32>,
    period_mm: f32,
}

impl GpuStyle {
    fn write(&self, bytes: &mut Vec<u8>) {
        for value in self.color.iter().chain([&self.width_px]) {
            bytes.extend(value.to_ne_bytes());
        }
        bytes.extend(self.pattern.start.to_ne_bytes());
        bytes.extend(vertex_index(self.pattern.len()).to_ne_bytes());
        bytes.extend(self.period_mm.to_ne_bytes());
    }
}

/// The segments and letters of one underlay as GPU data, in millimetres in the XY plane.
#[derive(Debug, PartialEq)]
pub struct UnderlayGeometry {
    /// SHA-256 of the lines' blob: the same lines upload once.
    key: [u8; 32],
    segments: Vec<Segment>,
    /// The range of each layer's segments, in the order of the underlay's layers.
    layers: Vec<Range<u32>>,
    styles: Vec<GpuStyle>,
    /// The dash, gap and dot lengths of every style's pattern, in millimetres.
    patterns: Vec<f32>,
    letters: Letters,
    /// Per layer, the vertices its segments and letters draw.
    layer_vertices: Vec<u64>,
}

impl UnderlayGeometry {
    #[must_use]
    pub fn new(key: [u8; 32], lines: &UnderlayLines) -> Self {
        let gpu = |value: f64| single_precision([value, 0.0, 0.0])[0];
        let mut segments = Vec::with_capacity(lines.segment_count());
        let mut layers = Vec::with_capacity(lines.layers().len());
        for layer in lines.layers() {
            let first = vertex_index(segments.len());
            for (style, polyline) in layer.styled_polylines() {
                let mut from_mm = 0.0;
                for pair in polyline.windows(2) {
                    let [start, end] = [pair[0], pair[1]].map(|point| point.map(gpu));
                    segments.push(Segment {
                        start,
                        end,
                        from_mm: gpu(from_mm),
                        style,
                    });
                    from_mm += (pair[1][0] - pair[0][0]).hypot(pair[1][1] - pair[0][1]);
                }
            }
            layers.push(first..vertex_index(segments.len()));
        }
        let mut patterns = Vec::new();
        let mut styles = Vec::new();
        // An underlay saved before styles names style 0, which it does not hold.
        for style in lines
            .styles()
            .iter()
            .chain(lines.styles().is_empty().then(|| lines.style(0)))
        {
            styles.push(gpu_style(style, &mut patterns, gpu));
        }
        let mut layer_vertices = layers
            .iter()
            .map(|range| u64::from(range.end - range.start) * 6)
            .collect::<Vec<_>>();
        let letters = Letters::new(lines, &mut layer_vertices);
        Self {
            key,
            segments,
            layers,
            styles,
            patterns,
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
        self.segments.len()
    }

    /// The letters of its texts that draw something (no spaces).
    #[must_use]
    pub fn letter_count(&self) -> usize {
        self.letters.instances.len()
    }

    fn is_empty(&self) -> bool {
        self.segments.is_empty() && self.letters.instances.is_empty()
    }
}

/// `style` for the shader, its pattern appended to `patterns`.
fn gpu_style(style: &LineStyle, patterns: &mut Vec<f32>, gpu: impl Fn(f64) -> f32) -> GpuStyle {
    let first = vertex_index(patterns.len());
    patterns.extend(style.pattern_mm.iter().map(|length| gpu(*length)));
    GpuStyle {
        color: style.color.map_or([0.0; 4], linear_rgba),
        width_px: (f32::from(style.weight_hmm) / 100.0 * PX_PER_WEIGHT_MM)
            .round()
            .max(1.0),
        pattern: first..vertex_index(patterns.len()),
        period_mm: gpu(style.pattern_mm.iter().map(|length| length.abs()).sum()),
    }
}

/// A vertex or instance count the GPU draws.
fn vertex_index(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// One underlay of the frame, the indices of its visible layers (none while it or all its
/// layers are hidden: its upload is kept for when it is shown again) and its placement,
/// column-major, from its lines to the world.
#[derive(Clone, Debug)]
pub struct UnderlayDraw {
    pub geometry: Arc<UnderlayGeometry>,
    pub visible: Vec<usize>,
    pub placement: [f32; 16],
}

/// A row-major placement column-major in single precision, as the GPU reads it.
#[must_use]
pub fn gpu_placement(row_major: &[f64; 16]) -> [f32; 16] {
    std::array::from_fn(|at| single_precision([row_major[(at % 4) * 4 + at / 4], 0.0, 0.0])[0])
}

/// The column-major product `outer · inner`: `inner` applied first.
fn column_major_product(outer: &[f32; 16], inner: &[f32; 16]) -> [f32; 16] {
    std::array::from_fn(|at| {
        let (column, row) = (at / 4, at % 4);
        (0..4)
            .map(|index| outer[index * 4 + row] * inner[column * 4 + index])
            .sum()
    })
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

/// `items` in vertex buffers no larger than the device allows, `stride` bytes each.
fn upload_chunked<T>(
    device: &wgpu::Device,
    label: &str,
    items: &[T],
    stride: u64,
    write: impl Fn(&T, &mut Vec<u8>),
) -> Vec<Chunk> {
    let per_chunk = usize::try_from(device.limits().max_buffer_size / stride).unwrap_or(usize::MAX);
    items
        .chunks(per_chunk.max(1))
        .enumerate()
        .map(|(index, chunk)| Chunk {
            buffer: device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents: &bytes_of(chunk, &write),
                usage: wgpu::BufferUsages::VERTEX,
            }),
            first: vertex_index(index * per_chunk),
            count: vertex_index(chunk.len()),
        })
        .collect()
}

fn bytes_of<T>(items: &[T], write: impl Fn(&T, &mut Vec<u8>)) -> Vec<u8> {
    let mut bytes = Vec::new();
    for item in items {
        write(item, &mut bytes);
    }
    bytes
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
    segments: Vec<Chunk>,
    outlines: Option<wgpu::Buffer>,
    instances: Vec<Chunk>,
    /// One `u32` per layer, nonzero while its letters are shown.
    shown: wgpu::Buffer,
    /// The shown layers, the styles and their patterns.
    group: wgpu::BindGroup,
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
    underlay_layout: wgpu::BindGroupLayout,
    placement_layout: wgpu::BindGroupLayout,
    /// Per draw of the frame, the uniform of its placement.
    placements: Vec<(wgpu::Buffer, wgpu::BindGroup)>,
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
            label: Some("Ketchup underlay camera, color and viewport"),
            size: 96,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup underlay bind group layout"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform)],
        });
        let placement_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup underlay placement layout"),
            entries: &[entry(0, wgpu::BufferBindingType::Uniform)],
        });
        let storage = wgpu::BufferBindingType::Storage { read_only: true };
        let underlay_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup underlay layers and styles layout"),
            entries: &[entry(0, storage), entry(1, storage), entry(2, storage)],
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
            bind_group_layouts: &[&layout, &underlay_layout, &placement_layout],
            push_constant_ranges: &[],
        });
        let pipeline = |label: &str,
                        (vertex, fragment): (&str, &str),
                        topology: wgpu::PrimitiveTopology,
                        buffers: &[wgpu::VertexBufferLayout<'_>]| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vertex),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    buffers,
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fragment),
                    compilation_options: wgpu::PipelineCompilationOptions::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target_format,
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState {
                    topology,
                    ..wgpu::PrimitiveState::default()
                },
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview: None,
                cache: None,
            })
        };
        let segment = wgpu::vertex_attr_array![
            0 => Float32x2, 1 => Float32x2, 2 => Float32, 3 => Uint32
        ];
        let point = wgpu::vertex_attr_array![0 => Float32x2];
        let instance = wgpu::vertex_attr_array![
            1 => Float32x2, 2 => Float32x2, 3 => Float32x2, 4 => Uint32, 5 => Float32, 6 => Uint32
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
            ("vs_line", "fs_line"),
            wgpu::PrimitiveTopology::TriangleList,
            &[wgpu::VertexBufferLayout {
                array_stride: SEGMENT_STRIDE,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &segment,
            }],
        );
        let letters_pipeline = pipeline(
            "Ketchup underlay letters pipeline",
            ("vs_letter", "fs_letter"),
            wgpu::PrimitiveTopology::LineList,
            &[point_buffer, instance_buffer.clone()],
        );
        let strokes_pipeline = pipeline(
            "Ketchup underlay letter strokes pipeline",
            ("vs_stroke", "fs_letter"),
            wgpu::PrimitiveTopology::LineList,
            &[instance_buffer],
        );
        Self {
            lines_pipeline,
            letters_pipeline,
            strokes_pipeline,
            uniform,
            bind_group,
            underlay_layout,
            placement_layout,
            placements: Vec::new(),
            uploaded: BTreeMap::new(),
            draws: Vec::new(),
            uploads: 0,
        }
    }

    /// Writes the camera, color and viewport, uploads underlays not on the GPU yet, which
    /// of their layers' letters show and how each cell of letters is drawn on a viewport
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
            .chain(viewport_px)
            .chain([0.0; 2])
            .flat_map(f32::to_ne_bytes)
            .collect::<Vec<_>>();
        queue.write_buffer(&self.uniform, 0, &uniform);
        while self.placements.len() < draws.len() {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("Ketchup underlay placement"),
                size: 64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Ketchup underlay placement"),
                layout: &self.placement_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                }],
            });
            self.placements.push((buffer, group));
        }
        for (draw, (buffer, _)) in draws.iter().zip(&self.placements) {
            let bytes = draw.placement.map(f32::to_ne_bytes).concat();
            queue.write_buffer(buffer, 0, &bytes);
        }
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
            let lines_to_clip = column_major_product(&world_to_clip, &draw.placement);
            uploaded.cell_views.clear();
            uploaded.cell_views.extend(
                geometry
                    .letters
                    .cells
                    .iter()
                    .map(|cell| cell.view(&lines_to_clip, viewport_px)),
            );
        }
        self.draws = draws.to_vec();
    }

    fn upload(&self, device: &wgpu::Device, geometry: &UnderlayGeometry) -> Uploaded {
        let letters = &geometry.letters;
        let outlines = (!letters.outlines.is_empty()).then(|| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Ketchup underlay glyph outlines"),
                contents: &bytes_of(&letters.outlines, write_point),
                usage: wgpu::BufferUsages::VERTEX,
            })
        });
        let storage = |label: &str, contents: &[u8]| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            })
        };
        // At least one slot each: a binding is never empty. Every layer starts hidden.
        let layers = geometry.layer_count().max(1);
        let shown = storage("Ketchup underlay shown layers", &vec![0u8; layers * 4]);
        let mut styles = bytes_of(&geometry.styles, GpuStyle::write);
        styles.resize(styles.len().max(STYLE_STRIDE), 0);
        let styles = storage("Ketchup underlay styles", &styles);
        let mut patterns = bytes_of(&geometry.patterns, |length, bytes| {
            bytes.extend(length.to_ne_bytes());
        });
        patterns.resize(patterns.len().max(4), 0);
        let patterns = storage("Ketchup underlay dash patterns", &patterns);
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ketchup underlay layers and styles"),
            layout: &self.underlay_layout,
            entries: &[&shown, &styles, &patterns]
                .into_iter()
                .zip(0..)
                .map(|(buffer, binding)| wgpu::BindGroupEntry {
                    binding,
                    resource: buffer.as_entire_binding(),
                })
                .collect::<Vec<_>>(),
        });
        Uploaded {
            segments: upload_chunked(
                device,
                "Ketchup underlay segments",
                &geometry.segments,
                SEGMENT_STRIDE,
                Segment::write,
            ),
            outlines,
            instances: upload_chunked(
                device,
                "Ketchup underlay letters",
                &letters.instances,
                INSTANCE_STRIDE,
                LetterInstance::write,
            ),
            shown,
            group,
            shown_layers: vec![0; layers],
            cell_views: Vec::new(),
        }
    }

    /// Draws the visible layers prepared last.
    pub fn draw(&self, pass: &mut wgpu::RenderPass<'_>) {
        let drawn = || {
            self.draws
                .iter()
                .zip(&self.placements)
                .filter(|(draw, _)| !draw.visible.is_empty())
                .filter_map(|(draw, (_, placement))| {
                    Some((draw, placement, self.uploaded.get(&draw.geometry.key)?))
                })
        };
        pass.set_pipeline(&self.lines_pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        for (draw, placement, uploaded) in drawn() {
            pass.set_bind_group(1, &uploaded.group, &[]);
            pass.set_bind_group(2, placement, &[]);
            for chunk in &uploaded.segments {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                for range in draw
                    .visible
                    .iter()
                    .filter_map(|&layer| draw.geometry.layers.get(layer))
                {
                    if let Some(local) = local_range(range, chunk.first, chunk.count) {
                        pass.draw(0..6, local);
                    }
                }
            }
        }
        // The cells of letters drawn whole, glyph by glyph.
        pass.set_pipeline(&self.letters_pipeline);
        for (draw, placement, uploaded) in drawn() {
            let Some(outlines) = &uploaded.outlines else {
                continue;
            };
            pass.set_bind_group(1, &uploaded.group, &[]);
            pass.set_bind_group(2, placement, &[]);
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
        for (draw, placement, uploaded) in drawn() {
            let letters = &draw.geometry.letters;
            pass.set_bind_group(1, &uploaded.group, &[]);
            pass.set_bind_group(2, placement, &[]);
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
    /// wide and high; `color` opaque sRGB, of what the drawing draws in its foreground
    /// color.
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
    viewport_px: vec2<f32>,
};

// Linear RGBA, alpha 0 for the underlay's own color; `count` pattern lengths from
// `first`, repeated every `period_mm`.
struct Style {
    color: vec4<f32>,
    width_px: f32,
    first: u32,
    count: u32,
    period_mm: f32,
};

@group(0) @binding(0)
var<uniform> underlay: Underlay;

@group(1) @binding(0)
var<storage, read> layer_shown: array<u32>;

@group(1) @binding(1)
var<storage, read> styles: array<Style>;

@group(1) @binding(2)
var<storage, read> pattern_mm: array<f32>;

// From the underlay's lines to the world.
@group(2) @binding(0)
var<uniform> placement: mat4x4<f32>;

struct Drawn {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) style: u32,
    // How far along its polyline, in millimetres.
    @location(1) along_mm: f32,
};

// Outside the clip volume, where nothing of it is drawn.
const NOWHERE = vec4<f32>(2.0, 2.0, 2.0, 1.0);

// A segment as a quad as wide on screen as its style: two triangles of corners 0, 1, 2
// and 3, 4, 5; 1, 4 and 5 at its end, 2, 3 and 5 on its left.
@vertex
fn vs_line(
    @builtin(vertex_index) corner: u32,
    @location(0) start: vec2<f32>,
    @location(1) end: vec2<f32>,
    @location(2) from_mm: f32,
    @location(3) style: u32,
) -> Drawn {
    var drawn: Drawn;
    drawn.style = style;
    drawn.along_mm = 0.0;
    let a = underlay.world_to_clip * placement * vec4<f32>(start, 0.0, 1.0);
    let b = underlay.world_to_clip * placement * vec4<f32>(end, 0.0, 1.0);
    // What lies behind the camera is cut off before the segment is widened on screen.
    // not a tolerance: the clip-space w where a segment is cut, kept off 0 to divide by.
    let near = 1e-5;
    if (a.w < near && b.w < near) {
        drawn.position = NOWHERE;
        return drawn;
    }
    var cut = vec2<f32>(0.0, 1.0);
    if (a.w < near) {
        cut.x = (near - a.w) / (b.w - a.w);
    }
    if (b.w < near) {
        cut.y = (near - a.w) / (b.w - a.w);
    }
    let first = mix(a, b, cut.x);
    let last = mix(a, b, cut.y);
    let at_end = corner == 1u || corner >= 4u;
    let left = corner == 2u || corner == 3u || corner == 5u;
    let on_screen = (last.xy / last.w - first.xy / first.w) * underlay.viewport_px;
    var normal = vec2<f32>(0.0, 1.0);
    if (length(on_screen) > 0.0) {
        normal = vec2<f32>(-on_screen.y, on_screen.x) / length(on_screen);
    }
    let point = select(first, last, at_end);
    let offset = normal * styles[style].width_px / underlay.viewport_px;
    drawn.position = point + vec4<f32>(offset * select(-1.0, 1.0, left) * point.w, 0.0, 0.0);
    drawn.along_mm = from_mm + distance(start, end) * select(cut.x, cut.y, at_end);
    return drawn;
}

fn color_of(style: Style) -> vec4<f32> {
    return select(style.color, underlay.color, style.color.a == 0.0);
}

@fragment
fn fs_line(drawn: Drawn) -> @location(0) vec4<f32> {
    // Millimetres along the line per pixel, taken before any branch.
    let per_px = fwidth(drawn.along_mm);
    let style = styles[drawn.style];
    // A pattern too fine to tell apart on screen is drawn as a continuous line.
    if (style.count > 0u && style.period_mm > 4.0 * per_px) {
        var at = drawn.along_mm - floor(drawn.along_mm / style.period_mm) * style.period_mm;
        for (var index = 0u; index < style.count; index += 1u) {
            let length = pattern_mm[style.first + index];
            if (length < 0.0 && at < -length) {
                discard;
            }
            // A dot is drawn a pixel long.
            if (length >= 0.0 && at < max(length, per_px)) {
                break;
            }
            at -= abs(length);
        }
    }
    return color_of(style);
}

// A letter of a hidden layer goes outside the clip volume.
fn letter_point(position: vec2<f32>, layer: u32, style: u32) -> Drawn {
    var drawn: Drawn;
    drawn.style = style;
    drawn.along_mm = 0.0;
    drawn.position = select(
        underlay.world_to_clip * placement * vec4<f32>(position, 0.0, 1.0),
        NOWHERE,
        layer_shown[layer] == 0u,
    );
    return drawn;
}

@vertex
fn vs_letter(
    @location(0) outline: vec2<f32>,
    @location(1) origin: vec2<f32>,
    @location(2) x_axis: vec2<f32>,
    @location(3) y_axis: vec2<f32>,
    @location(4) layer: u32,
    @location(6) style: u32,
) -> Drawn {
    return letter_point(origin + x_axis * outline.x + y_axis * outline.y, layer, style);
}

// A letter too small to read: a stroke along its baseline, as long as it advances.
@vertex
fn vs_stroke(
    @builtin(vertex_index) end: u32,
    @location(1) origin: vec2<f32>,
    @location(2) x_axis: vec2<f32>,
    @location(4) layer: u32,
    @location(5) advance: f32,
    @location(6) style: u32,
) -> Drawn {
    return letter_point(origin + x_axis * advance * f32(end), layer, style);
}

// A text takes the color of its style, never its pattern.
@fragment
fn fs_letter(drawn: Drawn) -> @location(0) vec4<f32> {
    return color_of(styles[drawn.style]);
}
";

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::ProfileSegment;
    use ketchup_model::underlay::UnderlayLinesBuilder;

    #[test]
    fn each_layer_gets_the_range_of_its_segments() {
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
        assert_eq!(ranges, BTreeMap::from([("axes", 1), ("walls", 2)]));
        // A segment is a quad of two triangles.
        assert_eq!(geometry.layer_vertices(1), 6);
    }

    #[test]
    fn a_segment_carries_its_style_and_how_far_along_its_polyline_it_starts() {
        let mut builder = UnderlayLinesBuilder::default();
        let line = |from: [f64; 2], to: [f64; 2]| ProfileSegment::Line {
            start_mm: from,
            end_mm: to,
        };
        builder.add(
            "walls",
            &[
                line([0.0, 0.0], [300.0, 0.0]),
                line([300.0, 0.0], [300.0, 400.0]),
            ],
        );
        builder.set_style(&LineStyle {
            color: Some([255, 0, 0]),
            weight_hmm: 50,
            pattern_mm: vec![20.0, -10.0, 0.0, -10.0],
        });
        builder.add("walls", &[line([0.0, 100.0], [100.0, 100.0])]);
        let (_, lines) = builder.finish();
        let geometry = UnderlayGeometry::new([9; 32], &lines);
        let along = geometry
            .segments
            .iter()
            .map(|segment| (segment.from_mm, segment.style))
            .collect::<Vec<_>>();
        assert_eq!(along, [(0.0, 0), (300.0, 0), (0.0, 1)]);
        assert_eq!(
            geometry.styles,
            [
                GpuStyle {
                    color: [0.0; 4],
                    width_px: 1.0,
                    pattern: 0..0,
                    period_mm: 0.0,
                },
                GpuStyle {
                    color: linear_rgba([255, 0, 0]),
                    width_px: 2.0,
                    pattern: 0..4,
                    period_mm: 40.0,
                },
            ]
        );
        assert_eq!(geometry.patterns, [20.0, -10.0, 0.0, -10.0]);
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
        assert_eq!((walls.len(), labels.len()), (1, 0));
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
        assert_eq!(geometry.layer_vertices(0), 6);
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
