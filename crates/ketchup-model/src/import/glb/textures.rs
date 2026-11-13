//! The pictures of a GLB (U4e): a material's base colour texture, its image and where
//! each triangle corner lies on it. A picture comes into the document when it is PNG or
//! JPEG bytes in the GLB's binary chunk, a material shows it and its mesh says where
//! (`TEXCOORD_n`); anything else stays a note, as before.

use super::{
    Accessor, AccessorKind, BufferView, FLOAT, GlbImportError, bump, object, optional_array,
    optional_u64, optional_usize, read_f32,
};
use crate::document::TexturedTriangle;
use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, OnceLock};

/// glTF's `REPEAT` wrap mode, the default and SketchUp's tiling.
const REPEAT: u64 = 10_497;

/// A picture of the GLB as its bytes.
#[derive(Clone, Debug)]
pub(super) struct GlbImage {
    pub(super) bytes: Arc<Vec<u8>>,
    pub(super) mime_type: &'static str,
    /// The bytes' SHA-256, the blob they are kept as: made once, however many meshes
    /// show the picture (review 2026-11-12 R1112-SKP-501: 1 000 meshes sharing a 16 MB
    /// picture hashed it 1 000 times, 12.7 s).
    blob: OnceLock<String>,
}

/// The same picture whether or not its blob was made yet.
impl PartialEq for GlbImage {
    fn eq(&self, other: &Self) -> bool {
        self.bytes == other.bytes && self.mime_type == other.mime_type
    }
}

impl Eq for GlbImage {}

impl GlbImage {
    pub(super) fn blob(&self) -> &str {
        self.blob
            .get_or_init(|| crate::graph::sha256_hex(&self.bytes))
    }
}

/// What a material shows: its colour and, with a usable base colour texture, its image.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct MaterialLook {
    pub(super) color: Option<[u8; 3]>,
    pub(super) texture: Option<MaterialTexture>,
}

/// The image (an index of the GLB's images) and the `TEXCOORD_n` set a material shows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MaterialTexture {
    pub(super) image: usize,
    pub(super) set: u64,
}

/// The PNG or JPEG type of `bytes` from their first bytes; `None` for any other picture,
/// which glTF does not carry either.
fn sniff(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else {
        None
    }
}

/// The GLB's images: each that is PNG or JPEG bytes of a buffer view, else `None`.
pub(super) fn parse_images(
    root: &Map<String, Value>,
    binary: &[u8],
    views: &[BufferView],
) -> Result<Vec<Option<GlbImage>>, GlbImportError> {
    optional_array(root, "images")?
        .iter()
        .map(|image| {
            let image = object(image)?;
            let Some(view) = optional_usize(image, "bufferView")? else {
                return Ok(None);
            };
            let view = views.get(view).ok_or(GlbImportError::InvalidReference)?;
            if view.stride.is_some() {
                return Err(GlbImportError::InvalidAccessor);
            }
            let bytes = binary
                .get(view.offset..view.offset + view.length)
                .ok_or(GlbImportError::InvalidAccessor)?;
            Ok(sniff(bytes).map(|mime_type| GlbImage {
                bytes: Arc::new(bytes.to_vec()),
                mime_type,
                blob: OnceLock::new(),
            }))
        })
        .collect()
}

/// The image of the base colour texture `info` names, when the GLB holds it as a picture
/// Kečup shows; a wrap other than repeat is noted, the picture is still shown tiled.
pub(super) fn base_color_texture(
    root: &Map<String, Value>,
    info: &Value,
    images: &[Option<GlbImage>],
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<Option<MaterialTexture>, GlbImportError> {
    let info = object(info)?;
    let index = optional_usize(info, "index")?.ok_or(GlbImportError::InvalidJson)?;
    let texture = object(
        optional_array(root, "textures")?
            .get(index)
            .ok_or(GlbImportError::InvalidReference)?,
    )?;
    let Some(image) = optional_usize(texture, "source")? else {
        return Ok(None);
    };
    if images
        .get(image)
        .ok_or(GlbImportError::InvalidReference)?
        .is_none()
    {
        return Ok(None);
    }
    if let Some(sampler) = optional_usize(texture, "sampler")? {
        let sampler = object(
            optional_array(root, "samplers")?
                .get(sampler)
                .ok_or(GlbImportError::InvalidReference)?,
        )?;
        for wrap in ["wrapS", "wrapT"] {
            if optional_u64(sampler, wrap)?.is_some_and(|mode| mode != REPEAT) {
                bump(diagnostics, "glb_texture_wrap_ignored", 1)?;
            }
        }
    }
    Ok(Some(MaterialTexture {
        image,
        set: optional_u64(info, "texCoord")?.unwrap_or(0),
    }))
}

/// Notes the images, textures and samplers no imported material shows.
pub(super) fn record_unshown(
    root: &Map<String, Value>,
    looks: &[MaterialLook],
    diagnostics: &mut BTreeMap<&'static str, u32>,
) -> Result<(), GlbImportError> {
    let shown: BTreeSet<usize> = looks
        .iter()
        .filter_map(|look| look.texture.map(|texture| texture.image))
        .collect();
    let images = optional_array(root, "images")?.len();
    let textures = optional_array(root, "textures")?;
    let mut shown_textures = 0_usize;
    for texture in textures {
        if optional_usize(object(texture)?, "source")?.is_some_and(|image| shown.contains(&image)) {
            shown_textures += 1;
        }
    }
    let count = |count: usize| {
        u32::try_from(count)
            .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::UnsupportedFeature)
    };
    bump(
        diagnostics,
        "glb_images_ignored",
        count(images - shown.len())?,
    )?;
    bump(
        diagnostics,
        "glb_textures_ignored",
        count(textures.len() - shown_textures)?,
    )?;
    if shown.is_empty() {
        bump(
            diagnostics,
            "glb_samplers_ignored",
            count(optional_array(root, "samplers")?.len())?,
        )?;
    }
    Ok(())
}

/// The texture coordinates of each of `vertex_count` vertices, `FLOAT` pairs.
pub(super) fn decode_uvs(
    accessor_index: usize,
    vertex_count: usize,
    binary: &[u8],
    views: &[BufferView],
    accessors: &[Accessor],
) -> Result<Vec<[f32; 2]>, GlbImportError> {
    let accessor = accessors
        .get(accessor_index)
        .ok_or(GlbImportError::InvalidReference)?;
    if accessor.kind != AccessorKind::Other(2)
        || accessor.component_type != FLOAT
        || accessor.count != vertex_count
    {
        return Err(GlbImportError::InvalidAccessor);
    }
    let view = views
        .get(accessor.view)
        .ok_or(GlbImportError::InvalidReference)?;
    let stride = view.stride.unwrap_or(8);
    let start = view
        .offset
        .checked_add(accessor.offset)
        .ok_or(GlbImportError::InvalidAccessor)?;
    (0..accessor.count)
        .map(|index| {
            let offset = index
                .checked_mul(stride)
                .and_then(|offset| offset.checked_add(start))
                .ok_or(GlbImportError::InvalidAccessor)?;
            let uv = [read_f32(binary, offset)?, read_f32(binary, offset + 4)?];
            if uv.iter().all(|value| value.is_finite()) {
                Ok(uv)
            } else {
                Err(GlbImportError::InvalidGeometry)
            }
        })
        .collect()
}

/// Each of `triangles` showing `image` with the texture coordinates `uvs` of its corners.
pub(super) fn textured_triangles(
    image: usize,
    uvs: &[[f32; 2]],
    triangles: &[[u32; 3]],
) -> Result<Vec<Option<TexturedTriangle>>, GlbImportError> {
    let image = u32::try_from(image)
        .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidReference)?;
    triangles
        .iter()
        .map(|triangle| {
            let mut uv = [[0.0; 2]; 3];
            for (corner, index) in uv.iter_mut().zip(triangle) {
                *corner = *uvs
                    .get(usize::try_from(*index).unwrap_or(usize::MAX))
                    .ok_or(GlbImportError::InvalidGeometry)?;
            }
            Ok(Some(TexturedTriangle { image, uv }))
        })
        .collect()
}
