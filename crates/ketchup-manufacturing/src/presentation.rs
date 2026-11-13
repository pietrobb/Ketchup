//! A presentation image of any model: a still picture for a client or a quotation, separate from
//! the modelling viewport, which it leaves unchanged.
//!
//! The renderer runs on the CPU in `f64` so every machine and CI runner produces the same pixels:
//! smoothed edges (supersampling), soft sun shadows (on the parts and on an implied ground under
//! the model), shading in corners (screen-space ambient occlusion), outlines at the model's edges,
//! see-through parts by their opacity, and a white, transparent or coloured background.

use crate::blender_export::{MeshGlbInstance, validate_instances};
use ketchup_model::annotation::DecimalMark;
use ketchup_model::document::{SceneOccurrence, Snapshot};
use ketchup_model::exact_product::{ExactProductError, MeshExportSource};
use ketchup_model::persistence::Blobs;
use ketchup_model::tolerance::limits;
use std::collections::BTreeMap;
use std::fmt;
use std::sync::atomic::AtomicBool;

mod raster;
mod texture;

pub use texture::{
    MAX_SOURCE_PIXELS, MAX_SOURCE_SIDE, MAX_TEXTURE_SIDE, PresentationTexture, TextureError,
};

/// The longest side of a presentation image, in pixels.
pub const MAX_PRESENTATION_SIDE: u32 = 8192;
/// Samples a render holds at once: 24 bytes each, about 580 MB at the limit. An image whose
/// smoothed samples do not fit is made in bands of rows (review 2026-11-13 P3), and one too
/// large even for that is smoothed less (review 2026-11-12 R1112-IMG-501). This is also the
/// most pixels an image has.
pub const MAX_PRESENTATION_SAMPLES: u64 = 24_000_000;
/// Farthest a point of the model may lie from the origin along an axis, in millimetres (a
/// million kilometres): past it the renderer's arithmetic overflows (review 2026-11-12 P3).
pub const MAX_PRESENTATION_COORDINATE_MM: f64 = 1e12;
/// Colour of a part with no colour of its own.
pub const DEFAULT_PART_RGB: [u8; 3] = [206, 201, 192];
/// Labels one image may carry.
pub const MAX_PRESENTATION_LABELS: usize = 200;
/// Characters of one label.
pub const MAX_LABEL_CHARS: usize = 120;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Background {
    White,
    /// The model and its ground shadow over full transparency (PNG alpha).
    Transparent,
    Color([u8; 3]),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationOptions {
    pub width: u32,
    pub height: u32,
    pub background: Background,
    /// Direction from the model towards the eye, Kečup axes (Z up). The camera frames the model.
    pub view_from: [f64; 3],
    /// Most samples per pixel along each axis, 1–4: how smooth the edges are. An image too
    /// large for them within [`MAX_PRESENTATION_SAMPLES`] is smoothed less.
    pub supersample: u32,
    pub shadows: bool,
    pub ambient_occlusion: bool,
    pub outlines: bool,
    /// The model's overall width, depth and height as dimension lines beside it.
    pub dimensions: bool,
    /// What the dimensions write before a decimal, as the drawings do.
    pub decimal_mark: DecimalMark,
}

impl Default for PresentationOptions {
    fn default() -> Self {
        Self {
            width: 1920,
            height: 1080,
            background: Background::White,
            view_from: [-1.0, -1.6, 1.1],
            supersample: 3,
            shadows: true,
            ambient_occlusion: true,
            outlines: true,
            dimensions: false,
            decimal_mark: DecimalMark::Point,
        }
    }
}

/// One triangle in Kečup world millimetres with its sRGB colour and opacity (255 = opaque).
/// A textured triangle shows its picture instead of the colour, with the same opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PresentationTriangle {
    pub corners_mm: [[f64; 3]; 3],
    pub rgba: [u8; 4],
    pub texture: Option<TriangleTexture>,
}

/// The picture of the scene a triangle shows, by its index in
/// [`PresentationScene::textures`], and the point of the picture at each corner (glTF texture
/// coordinates: `u` right, `v` down, one picture per unit, repeating).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TriangleTexture {
    pub texture: usize,
    pub uv: [[f64; 2]; 3],
}

/// Text beside a point of the model, joined to it by a leader line.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationLabel {
    pub text: String,
    pub at_mm: [f64; 3],
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct PresentationScene {
    pub triangles: Vec<PresentationTriangle>,
    pub labels: Vec<PresentationLabel>,
    pub textures: Vec<PresentationTexture>,
}

/// Straight (not premultiplied) sRGB RGBA pixels, rows from the top.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PresentationImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

#[derive(Debug)]
pub enum PresentationError {
    Size {
        width: u32,
        height: u32,
        max_side: u32,
    },
    Supersample {
        supersample: u32,
    },
    /// The image has more pixels than [`MAX_PRESENTATION_SAMPLES`].
    TooManyPixels {
        pixels: u64,
        limit: u64,
    },
    /// Whoever wanted the image stopped waiting for it.
    Cancelled,
    /// `view_from` is zero or not finite.
    ViewDirection,
    /// No visible triangle with an area.
    EmptyModel,
    /// A corner coordinate is not finite or past [`MAX_PRESENTATION_COORDINATE_MM`].
    NonFiniteGeometry {
        triangle: usize,
    },
    TooManyTriangles {
        triangles: usize,
        limit: usize,
    },
    /// Label `label` is blank, has a line break or is longer than `limit` characters.
    LabelText {
        label: usize,
        limit: usize,
    },
    /// The point of label `label` is not finite.
    LabelPoint {
        label: usize,
    },
    /// The point of label `label` is farther from the model than the model is large, where
    /// framing it would leave the model a speck (review 2026-11-12 R1112-IMG-503).
    LabelAway {
        label: usize,
    },
    TooManyLabels {
        labels: usize,
        limit: usize,
    },
    /// The built-in typeface of labels and dimensions cannot be read.
    Font,
    /// The picture `blob` of the document a textured face shows cannot be read.
    Texture {
        blob: String,
        source: TextureError,
    },
    /// Triangle `triangle` names a picture the scene does not have or a texture coordinate
    /// that is not finite.
    TextureReference {
        triangle: usize,
    },
    Source(ExactProductError),
    Png(png::EncodingError),
}

impl fmt::Display for PresentationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Size {
                width,
                height,
                max_side,
            } => write!(
                formatter,
                "a presentation image of {width} × {height} px is outside 1–{max_side} px per side"
            ),
            Self::Supersample { supersample } => write!(
                formatter,
                "edge smoothing {supersample} is outside 1–4 samples per pixel side"
            ),
            Self::TooManyPixels { pixels, limit } => write!(
                formatter,
                "a presentation image of {pixels} pixels is more than the limit of {limit}"
            ),
            Self::Cancelled => write!(formatter, "the presentation image was cancelled"),
            Self::ViewDirection => write!(
                formatter,
                "the view direction is zero or not a finite number"
            ),
            Self::EmptyModel => write!(formatter, "the model has no visible surface to show"),
            Self::NonFiniteGeometry { triangle } => write!(
                formatter,
                "triangle {triangle} of the model has a corner that is not a finite number or is more than {MAX_PRESENTATION_COORDINATE_MM} mm from the origin"
            ),
            Self::TooManyTriangles { triangles, limit } => write!(
                formatter,
                "the model has {triangles} triangles, more than the limit of {limit}"
            ),
            Self::LabelText { label, limit } => write!(
                formatter,
                "label {label} is blank, has a line break or is longer than {limit} characters"
            ),
            Self::LabelPoint { label } => write!(
                formatter,
                "label {label} points at a coordinate that is not a finite number"
            ),
            Self::LabelAway { label } => write!(
                formatter,
                "label {label} points farther from the model than the model is large"
            ),
            Self::TooManyLabels { labels, limit } => write!(
                formatter,
                "the image has {labels} labels, more than the limit of {limit}"
            ),
            Self::Font => write!(formatter, "the built-in label typeface cannot be read"),
            Self::Texture { blob, source } => write!(
                formatter,
                "the picture {blob} on a textured face cannot be read: {source}"
            ),
            Self::TextureReference { triangle } => write!(
                formatter,
                "triangle {triangle} of the model names a picture it does not have or a texture coordinate that is not a finite number"
            ),
            Self::Source(error) => write!(formatter, "the model cannot be read: {error}"),
            Self::Png(error) => write!(formatter, "the PNG cannot be written: {error}"),
        }
    }
}

impl std::error::Error for PresentationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Source(error) => Some(error),
            Self::Png(error) => Some(error),
            Self::Texture { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// World triangles of the visible instances, each coloured by its occurrence and given the
/// opacity `opacity` returns for it (255 = opaque, e.g. lower for a membrane). A triangle of
/// an imported mesh with a picture shows that picture, read from the document's `blobs`.
pub fn presentation_scene(
    snapshot: &Snapshot,
    instances: &[MeshGlbInstance<'_>],
    blobs: &Blobs,
    opacity: impl Fn(&SceneOccurrence) -> u8,
) -> Result<PresentationScene, PresentationError> {
    let visible = instances
        .iter()
        .filter(|instance| instance.occurrence.visible)
        .copied()
        .collect::<Vec<_>>();
    validate_instances(snapshot, &visible).map_err(PresentationError::Source)?;
    let total = visible
        .iter()
        .map(|instance| instance.source.triangle_count())
        .sum::<usize>();
    if total > limits::EXPORT_TRIANGLES {
        return Err(PresentationError::TooManyTriangles {
            triangles: total,
            limit: limits::EXPORT_TRIANGLES,
        });
    }
    let mut triangles = Vec::with_capacity(total);
    let mut textures = Vec::new();
    let mut texture_of_blob = BTreeMap::new();
    for instance in visible {
        let [r, g, b] = instance.occurrence.color.unwrap_or(DEFAULT_PART_RGB);
        let rgba = [r, g, b, opacity(instance.occurrence)];
        let transform = instance.occurrence.transform;
        let look = match instance.source {
            MeshExportSource::Canonical { mesh, .. } => mesh.look.as_ref(),
            MeshExportSource::Exact(_) => None,
        };
        let mut scene_texture = Vec::new();
        for image in look.map_or(&[][..], |look| &look.images) {
            let at = match texture_of_blob.get(&image.blob) {
                Some(at) => *at,
                None => {
                    let texture = blobs
                        .get(&image.blob)
                        .ok_or(TextureError::Missing)
                        .and_then(|bytes| PresentationTexture::decode(bytes, &image.mime_type))
                        .map_err(|source| PresentationError::Texture {
                            blob: image.blob.clone(),
                            source,
                        })?;
                    textures.push(texture);
                    texture_of_blob.insert(image.blob.clone(), textures.len() - 1);
                    textures.len() - 1
                }
            };
            scene_texture.push(at);
        }
        for index in 0..instance.source.triangle_count() {
            let indices =
                instance
                    .source
                    .triangle_indices(index)
                    .ok_or(PresentationError::Source(
                        ExactProductError::InvalidMeshExport,
                    ))?;
            let mut corners_mm = [[0.0; 3]; 3];
            for (corner, vertex) in corners_mm.iter_mut().zip(indices) {
                let point = usize::try_from(vertex)
                    .ok()
                    .and_then(|vertex| instance.source.vertex_position_mm(vertex))
                    .ok_or(PresentationError::Source(
                        ExactProductError::InvalidMeshExport,
                    ))?;
                *corner = transform.transform_point(point);
            }
            let texture = look
                .and_then(|look| *look.triangles.get(index)?)
                .map(|textured| {
                    Some(TriangleTexture {
                        texture: *scene_texture.get(textured.image_index())?,
                        uv: textured.uv.map(|[u, v]| [f64::from(u), f64::from(v)]),
                    })
                })
                .map(|texture| {
                    texture.ok_or(PresentationError::Source(
                        ExactProductError::InvalidMeshExport,
                    ))
                })
                .transpose()?;
            triangles.push(PresentationTriangle {
                corners_mm,
                rgba,
                texture,
            });
        }
    }
    Ok(PresentationScene {
        triangles,
        labels: Vec::new(),
        textures,
    })
}

/// A label for each component name among the visible instances (the occurrence name when the
/// component has none), at the top centre of the first instance of that name.
pub fn part_labels(instances: &[MeshGlbInstance<'_>]) -> Vec<PresentationLabel> {
    let mut named = std::collections::BTreeSet::new();
    let mut labels = Vec::new();
    for instance in instances
        .iter()
        .filter(|instance| instance.occurrence.visible)
    {
        let occurrence = instance.occurrence;
        let name = [&occurrence.definition_name, &occurrence.occurrence_name]
            .into_iter()
            .map(|name| name.trim())
            .find(|name| !name.is_empty());
        let Some(name) = name else {
            continue;
        };
        let text = name
            .chars()
            .map(|letter| if letter.is_control() { ' ' } else { letter })
            .take(MAX_LABEL_CHARS)
            .collect::<String>();
        if labels.len() == MAX_PRESENTATION_LABELS || named.contains(&text) {
            continue;
        }
        let source = instance.source;
        let points = (0..source.vertex_count())
            .filter_map(|vertex| source.vertex_position_mm(vertex))
            .map(|point| occurrence.transform.transform_point(point));
        let Some((low, high)) = points.fold(None, |bounds: Option<([f64; 3], [f64; 3])>, point| {
            let (low, high) = bounds.unwrap_or((point, point));
            Some((
                [0, 1, 2].map(|axis| low[axis].min(point[axis])),
                [0, 1, 2].map(|axis| high[axis].max(point[axis])),
            ))
        }) else {
            continue;
        };
        named.insert(text.clone());
        labels.push(PresentationLabel {
            text,
            at_mm: [(low[0] + high[0]) * 0.5, (low[1] + high[1]) * 0.5, high[2]],
        });
    }
    labels
}

/// Samples per pixel side an image of `width` × `height` is rendered with: `supersample` when
/// bands of its rows fit [`MAX_PRESENTATION_SAMPLES`] with them, else the most that fit. Refuses a size
/// or smoothing out of range and an image of more pixels than the limit.
pub fn presentation_supersample(
    width: u32,
    height: u32,
    supersample: u32,
) -> Result<u32, PresentationError> {
    if !(1..=MAX_PRESENTATION_SIDE).contains(&width)
        || !(1..=MAX_PRESENTATION_SIDE).contains(&height)
    {
        return Err(PresentationError::Size {
            width,
            height,
            max_side: MAX_PRESENTATION_SIDE,
        });
    }
    if !(1..=4).contains(&supersample) {
        return Err(PresentationError::Supersample { supersample });
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PRESENTATION_SAMPLES {
        return Err(PresentationError::TooManyPixels {
            pixels,
            limit: MAX_PRESENTATION_SAMPLES,
        });
    }
    let fits = |factor: u32| {
        let side = |pixels: u32| usize::try_from(pixels * factor).unwrap_or(usize::MAX);
        let factor = usize::try_from(factor).unwrap_or(usize::MAX);
        raster::band_rows(side(width), side(height), factor, MAX_PRESENTATION_SAMPLES).is_some()
    };
    Ok((1..=supersample)
        .rev()
        .find(|factor| fits(*factor))
        .unwrap_or(1))
}

/// Renders `scene` as `options` say.
pub fn render_presentation(
    scene: &PresentationScene,
    options: &PresentationOptions,
) -> Result<PresentationImage, PresentationError> {
    render_presentation_until(scene, options, &AtomicBool::new(false))
}

/// Renders `scene` as `options` say unless `cancelled` is set first; it is looked at once a
/// row of samples (review 2026-11-12 R1112-IMG-502).
pub fn render_presentation_until(
    scene: &PresentationScene,
    options: &PresentationOptions,
    cancelled: &AtomicBool,
) -> Result<PresentationImage, PresentationError> {
    let supersample = presentation_supersample(options.width, options.height, options.supersample)?;
    let options = PresentationOptions {
        supersample,
        ..*options
    };
    if scene.triangles.len() > limits::EXPORT_TRIANGLES {
        return Err(PresentationError::TooManyTriangles {
            triangles: scene.triangles.len(),
            limit: limits::EXPORT_TRIANGLES,
        });
    }
    if let Some(triangle) = scene.triangles.iter().position(|triangle| {
        triangle
            .corners_mm
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > MAX_PRESENTATION_COORDINATE_MM)
    }) {
        return Err(PresentationError::NonFiniteGeometry { triangle });
    }
    if let Some(triangle) = scene.triangles.iter().position(|triangle| {
        triangle.texture.is_some_and(|texture| {
            texture.texture >= scene.textures.len()
                || !texture.uv.iter().flatten().all(|value| value.is_finite())
        })
    }) {
        return Err(PresentationError::TextureReference { triangle });
    }
    check_labels(&scene.labels)?;
    raster::render(scene, &options, cancelled, MAX_PRESENTATION_SAMPLES)
}

/// Refuses more than [`MAX_PRESENTATION_LABELS`] labels, a blank, multi-line or overlong text
/// and a point that is not finite.
pub fn check_labels(labels: &[PresentationLabel]) -> Result<(), PresentationError> {
    if labels.len() > MAX_PRESENTATION_LABELS {
        return Err(PresentationError::TooManyLabels {
            labels: labels.len(),
            limit: MAX_PRESENTATION_LABELS,
        });
    }
    for (label, PresentationLabel { text, at_mm }) in labels.iter().enumerate() {
        if text.trim().is_empty()
            || text.chars().any(char::is_control)
            || text.chars().count() > MAX_LABEL_CHARS
        {
            return Err(PresentationError::LabelText {
                label,
                limit: MAX_LABEL_CHARS,
            });
        }
        if !at_mm.iter().all(|value| value.is_finite()) {
            return Err(PresentationError::LabelPoint { label });
        }
    }
    Ok(())
}

impl PresentationImage {
    /// The image as a PNG (RGBA, 8 bits per channel).
    pub fn png(&self) -> Result<Vec<u8>, PresentationError> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_source_srgb(png::SrgbRenderingIntent::Perceptual);
        let mut writer = encoder.write_header().map_err(PresentationError::Png)?;
        writer
            .write_image_data(&self.rgba)
            .map_err(PresentationError::Png)?;
        writer.finish().map_err(PresentationError::Png)?;
        Ok(bytes)
    }
}
