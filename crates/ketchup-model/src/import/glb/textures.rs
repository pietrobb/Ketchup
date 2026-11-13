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

/// Whether a picture can be partly transparent: a PNG with an alpha channel or a `tRNS`
/// chunk before its pixels, or one whose chunks do not read; a JPEG never is.
fn may_have_alpha(image: &GlbImage) -> bool {
    if image.mime_type != "image/png" {
        return false;
    }
    let mut rest = image.bytes.get(8..).unwrap_or(&[]);
    while let Some((head, body)) = rest.split_first_chunk::<8>() {
        let length = usize::try_from(u32::from_be_bytes([head[0], head[1], head[2], head[3]]))
            .unwrap_or(usize::MAX);
        let Some(data) = body.get(..length) else {
            return true;
        };
        match &head[4..] {
            b"IHDR" if data.get(9).is_none_or(|colour| matches!(colour, 4 | 6)) => return true,
            b"tRNS" => return true,
            b"IDAT" | b"IEND" => return false,
            _ => {}
        }
        rest = body.get(length.saturating_add(4)..).unwrap_or(&[]);
    }
    true
}

/// Whether a material looks different from opaque: glTF's `alphaMode` with the base
/// colour's alpha and, for `MASK`, its cutoff, through the picture's own alpha. Before,
/// `MASK` was always a loss, and SketchUp writes it for every textured material (review
/// 2026-11-12 P3), so every textured model said its transparency was ignored.
pub(super) fn shows_transparency(
    mode: Option<&str>,
    cutoff: f64,
    alpha: f64,
    texture: Option<MaterialTexture>,
    images: &[Option<GlbImage>],
) -> bool {
    let picture = texture.is_some_and(|texture| {
        images
            .get(texture.image)
            .and_then(Option::as_ref)
            .is_none_or(may_have_alpha)
    });
    match mode {
        Some("BLEND") => alpha < 1.0 || picture,
        Some("MASK") => alpha < cutoff || picture,
        _ => false,
    }
}

/// The place in `images` of a picture of one pixel of `colour`, added when not there yet:
/// how a piece of a joined body keeps its own colour, as a body has one colour and each
/// of its triangles may show a picture (review 2026-11-12 P3: a cube of two materials
/// came in the first one's colour alone).
pub(super) fn colour_picture(
    colour: [u8; 3],
    images: &mut Vec<Option<GlbImage>>,
) -> Result<u32, GlbImportError> {
    let pixels =
        miniz_oxide::deflate::compress_to_vec_zlib(&[0, colour[0], colour[1], colour[2]], 6);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 2, 0, 0, 0][..]),
        (b"IDAT", pixels.as_slice()),
        (b"IEND", &[][..]),
    ] {
        let length = u32::try_from(data.len())
            .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidGeometry)?;
        bytes.extend(length.to_be_bytes());
        let start = bytes.len();
        bytes.extend(kind);
        bytes.extend(data);
        let crc = crc32(&bytes[start..]);
        bytes.extend(crc.to_be_bytes());
    }
    let picture = Some(GlbImage {
        bytes: Arc::new(bytes),
        mime_type: "image/png",
        blob: OnceLock::new(),
    });
    let place = match images.iter().position(|image| *image == picture) {
        Some(place) => place,
        None => {
            images.push(picture);
            images.len() - 1
        }
    };
    u32::try_from(place)
        .map_err(|_out_of_range: std::num::TryFromIntError| GlbImportError::InvalidReference)
}

/// PNG's CRC-32 of a chunk's type and data.
fn crc32(bytes: &[u8]) -> u32 {
    !bytes.iter().fold(u32::MAX, |crc, byte| {
        (0..8).fold(crc ^ u32::from(*byte), |crc, _| {
            if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            }
        })
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(mime_type: &'static str, bytes: Vec<u8>) -> Option<GlbImage> {
        Some(GlbImage {
            bytes: Arc::new(bytes),
            mime_type,
            blob: OnceLock::new(),
        })
    }

    /// A PNG of `colour` type with its `chunks` before the pixels (the CRCs are not read).
    fn png(colour: u8, chunks: &[&[u8; 4]]) -> Option<GlbImage> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8; 4], data: &[u8]| {
            bytes.extend(u32::try_from(data.len()).unwrap().to_be_bytes());
            bytes.extend(kind);
            bytes.extend(data);
            bytes.extend([0; 4]);
        };
        chunk(b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, colour, 0, 0, 0]);
        for kind in chunks {
            chunk(kind, &[0, 0]);
        }
        chunk(b"IDAT", &[0; 6]);
        chunk(b"IEND", &[]);
        picture("image/png", bytes)
    }

    #[test]
    fn a_cut_out_material_notes_its_transparency_only_when_it_can_cut_anything() {
        let images = [
            picture("image/jpeg", vec![0xff, 0xd8, 0xff, 0xe0]),
            png(2, &[b"sRGB"]),
            png(6, &[]),
            png(3, &[b"PLTE", b"tRNS"]),
            picture("image/png", b"\x89PNG\r\n\x1a\n\0\0".to_vec()),
        ];
        let shows = |mode, alpha, image| {
            let texture = Some(MaterialTexture { image, set: 0 });
            shows_transparency(Some(mode), 0.5, alpha, texture, &images)
        };
        // SketchUp's MASK on a photo or an opaque PNG cuts nothing.
        assert!(!shows("MASK", 1.0, 0));
        assert!(!shows("MASK", 1.0, 1));
        assert!(!shows("BLEND", 1.0, 1));
        // A picture with alpha, a palette with tRNS and an unreadable PNG may.
        assert!(shows("MASK", 1.0, 2));
        assert!(shows("MASK", 1.0, 3));
        assert!(shows("MASK", 1.0, 4));
        // The base colour's own alpha: below the cutoff the whole material is cut away.
        assert!(shows("MASK", 0.4, 1));
        assert!(shows("BLEND", 0.9, 1));
        assert!(!shows_transparency(Some("MASK"), 0.5, 0.6, None, &images));
        assert!(!shows_transparency(Some("OPAQUE"), 0.5, 0.2, None, &images));
        assert!(!shows_transparency(None, 0.5, 0.2, None, &images));
    }
}
