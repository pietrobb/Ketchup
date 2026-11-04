//! Raster underlays on the GPU (Měchenice MECH-10(a)): each image one texture, uploaded
//! once and kept while an underlay shows it, drawn as the quad of its four corners under
//! the fills and lines of every underlay. Its corners are split points relative to the
//! eye, as the lines' are, so an orthophoto in national grid coordinates lies where its
//! drawing does to a fraction of a pixel.

use super::SplitPoint;
use ketchup_model::underlay::raster::{RasterFormat, UnderlayRaster};
use std::collections::BTreeMap;
use std::sync::Arc;
use wgpu::util::DeviceExt as _;

/// Bytes of one corner of a quad on the GPU: its point, split, and where in the image.
const CORNER_STRIDE: u64 = 24;

/// The pixels of a raster underlay's file, decoded once.
pub struct RasterImage {
    /// SHA-256 of the file: the same image uploads once.
    pub(crate) key: [u8; 32],
    pixels: image::RgbaImage,
}

impl std::fmt::Debug for RasterImage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RasterImage")
            .field("width_px", &self.pixels.width())
            .field("height_px", &self.pixels.height())
            .finish_non_exhaustive()
    }
}

impl RasterImage {
    /// Decodes the image file `bytes` of `raster`, which must have its pixels.
    ///
    /// # Errors
    ///
    /// When the bytes are no image of that format or have other pixels than `raster` says.
    pub fn decode(bytes: &[u8], raster: &UnderlayRaster) -> Result<Self, image::ImageError> {
        let format = match raster.format {
            RasterFormat::Png => image::ImageFormat::Png,
            RasterFormat::Jpeg => image::ImageFormat::Jpeg,
        };
        let mut reader = image::ImageReader::with_format(std::io::Cursor::new(bytes), format);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(raster.width_px);
        limits.max_image_height = Some(raster.height_px);
        reader.limits(limits);
        let pixels = reader.decode()?.into_rgba8();
        if pixels.dimensions() != (raster.width_px, raster.height_px) {
            return Err(image::ImageError::Limits(
                image::error::LimitError::from_kind(image::error::LimitErrorKind::DimensionError),
            ));
        }
        Ok(Self {
            key: raster.image_sha256,
            pixels,
        })
    }

    #[must_use]
    pub fn dimensions(&self) -> [u32; 2] {
        [self.pixels.width(), self.pixels.height()]
    }

    /// The colour of the pixel `[column, row]`, sRGB and alpha.
    #[must_use]
    pub fn pixel(&self, [column, row]: [u32; 2]) -> Option<[u8; 4]> {
        self.pixels
            .get_pixel_checked(column, row)
            .map(|pixel| pixel.0)
    }
}

/// The image of a raster underlay this frame: its pixels and its corners in the
/// underlay's own coordinates, upper left, upper right, lower right, lower left.
#[derive(Clone, Debug)]
pub struct RasterDraw {
    pub image: Arc<RasterImage>,
    pub corners_mm: [[f64; 2]; 4],
}

impl RasterDraw {
    /// The corners as a strip of two triangles: upper left, upper right, lower left,
    /// lower right; each its point split relative to nothing yet, and where in the image.
    fn corner_bytes(&self) -> Vec<u8> {
        let [upper_left, upper_right, lower_right, lower_left] = self.corners_mm;
        [
            (upper_left, [0.0_f32, 0.0]),
            (upper_right, [1.0, 0.0]),
            (lower_left, [0.0, 1.0]),
            (lower_right, [1.0, 1.0]),
        ]
        .into_iter()
        .flat_map(|(point, uv)| {
            let split = SplitPoint::new(point);
            split
                .high
                .into_iter()
                .chain(split.low)
                .chain(uv)
                .flat_map(f32::to_ne_bytes)
                .collect::<Vec<_>>()
        })
        .collect()
    }
}

/// The images of the raster underlays on the GPU and the pipeline that draws them.
pub(super) struct GpuRasters {
    pipeline: wgpu::RenderPipeline,
    texture_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    /// Per image, the bind group of its texture.
    textures: BTreeMap<[u8; 32], wgpu::BindGroup>,
    /// Per draw of the frame, the corners of its quad.
    corners: Vec<wgpu::Buffer>,
    uploads: u64,
}

impl GpuRasters {
    /// The pipeline, drawing into `target_format` with the placement of a draw in
    /// `placement_layout`'s group 1.
    pub(super) fn new(
        device: &wgpu::Device,
        target_format: wgpu::TextureFormat,
        placement_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ketchup raster underlay shader"),
            source: wgpu::ShaderSource::Wgsl(RASTER_SHADER.into()),
        });
        let texture_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ketchup raster underlay texture layout"),
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
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Ketchup raster underlay pipeline layout"),
            bind_group_layouts: &[&texture_layout, placement_layout],
            push_constant_ranges: &[],
        });
        let attributes = wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2, 2 => Float32x2];
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Ketchup raster underlay pipeline"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_raster"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                buffers: &[wgpu::VertexBufferLayout {
                    array_stride: CORNER_STRIDE,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attributes,
                }],
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_raster"),
                compilation_options: wgpu::PipelineCompilationOptions::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleStrip,
                ..wgpu::PrimitiveState::default()
            },
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview: None,
            cache: None,
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Ketchup raster underlay sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            ..wgpu::SamplerDescriptor::default()
        });
        Self {
            pipeline,
            texture_layout,
            sampler,
            textures: BTreeMap::new(),
            corners: Vec::new(),
            uploads: 0,
        }
    }

    /// Writes the corners of each draw's image, uploads images not on the GPU yet and
    /// releases those no longer drawn.
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        rasters: &[Option<&RasterDraw>],
    ) {
        self.textures.retain(|key, _| {
            rasters
                .iter()
                .flatten()
                .any(|raster| raster.image.key == *key)
        });
        while self.corners.len() < rasters.len() {
            self.corners
                .push(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Ketchup raster underlay corners"),
                    size: CORNER_STRIDE * 4,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }
        for (raster, buffer) in rasters.iter().zip(&self.corners) {
            let Some(raster) = raster else {
                continue;
            };
            queue.write_buffer(buffer, 0, &raster.corner_bytes());
            if !self.textures.contains_key(&raster.image.key) {
                let group = self.upload(device, queue, &raster.image);
                self.textures.insert(raster.image.key, group);
                self.uploads += 1;
            }
        }
    }

    /// `image` as a texture no larger than the device allows, scaled down to it.
    fn upload(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        image: &RasterImage,
    ) -> wgpu::BindGroup {
        let largest = device.limits().max_texture_dimension_2d.max(1);
        let [width, height] = image.dimensions();
        let scaled;
        let longest = width.max(height);
        let pixels = if longest > largest {
            // Whole pixels: `length <= longest`, so the side never exceeds `largest`.
            let side = |length: u32| {
                let fitted = u64::from(length) * u64::from(largest) / u64::from(longest);
                u32::try_from(fitted).map_or(largest, |fitted| fitted.clamp(1, largest))
            };
            scaled = image::imageops::resize(
                &image.pixels,
                side(width),
                side(height),
                image::imageops::FilterType::Triangle,
            );
            &scaled
        } else {
            &image.pixels
        };
        let texture = device.create_texture_with_data(
            queue,
            &wgpu::TextureDescriptor {
                label: Some("Ketchup raster underlay image"),
                size: wgpu::Extent3d {
                    width: pixels.width(),
                    height: pixels.height(),
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            pixels.as_raw(),
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ketchup raster underlay texture"),
            layout: &self.texture_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        })
    }

    /// Draws the image of each draw `rasters` shows, with its placement.
    pub(super) fn draw<'a>(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        rasters: impl Iterator<Item = (usize, &'a RasterDraw, &'a wgpu::BindGroup)>,
    ) {
        pass.set_pipeline(&self.pipeline);
        for (index, raster, placement) in rasters {
            let (Some(texture), Some(corners)) = (
                self.textures.get(&raster.image.key),
                self.corners.get(index),
            ) else {
                continue;
            };
            pass.set_bind_group(0, texture, &[]);
            pass.set_bind_group(1, placement, &[]);
            pass.set_vertex_buffer(0, corners.slice(..));
            pass.draw(0..4, 0..1);
        }
    }

    pub(super) fn uploads(&self) -> u64 {
        self.uploads
    }
}

const RASTER_SHADER: &str = r"
@group(0) @binding(0)
var image: texture_2d<f32>;

@group(0) @binding(1)
var image_sampler: sampler;

// From the point of the underlay's lines the camera looks at to clip space, and that
// point split in a high and a low part, as for its lines.
struct Lines {
    eye_to_clip: mat4x4<f32>,
    eye_high: vec2<f32>,
    eye_low: vec2<f32>,
};

@group(1) @binding(0)
var<uniform> lines: Lines;

struct Corner {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_raster(
    @location(0) high: vec2<f32>,
    @location(1) low: vec2<f32>,
    @location(2) uv: vec2<f32>,
) -> Corner {
    var corner: Corner;
    let point = (high - lines.eye_high) + (low - lines.eye_low);
    corner.position = lines.eye_to_clip * vec4<f32>(point, 0.0, 1.0);
    corner.uv = uv;
    return corner;
}

@fragment
fn fs_raster(corner: Corner) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, corner.uv);
}
";
