//! Where an imported mesh shows a picture (U4e): a SketchUp or glTF texture.
//!
//! The picture's bytes are a blob of the document, named by their SHA-256; the mesh body
//! names the blob and says, for each of its triangles, which picture it shows and where
//! its three corners lie on it. A triangle without one shows its part's colour.

use super::{FeatureKind, MeshAuthority, MeshBodySpec};

impl FeatureKind {
    /// The blobs of the document this feature reads: the source of an imported body or the
    /// pictures of a mesh, by the SHA-256 hex of their bytes.
    #[must_use]
    pub fn blobs(&self) -> Vec<String> {
        match self {
            Self::ImportedExactBody(spec) => vec![
                spec.source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect(),
            ],
            Self::MeshBody(mesh) => mesh
                .look
                .iter()
                .flat_map(MeshLook::blobs)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        }
    }
}

impl MeshBodySpec {
    /// A mesh body that shows no picture.
    #[must_use]
    pub fn new(
        schema: String,
        vertices_mm: Vec<[f64; 3]>,
        triangles: Vec<[u32; 3]>,
        authority: MeshAuthority,
    ) -> Self {
        Self {
            schema,
            vertices_mm,
            triangles,
            authority,
            look: None,
        }
    }
}

/// The pictures a mesh body shows and where each triangle lies on them.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshLook {
    pub images: Vec<MeshImage>,
    /// One per triangle of the mesh body, in its order.
    pub triangles: Vec<Option<TexturedTriangle>>,
}

/// A picture, PNG or JPEG, as the imported file carried it.
#[derive(Clone, Debug, Eq, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct MeshImage {
    /// The SHA-256 hex of the picture's bytes, a blob of the document.
    pub blob: String,
    /// `image/png` or `image/jpeg`.
    pub mime_type: String,
}

/// The picture a triangle shows and the point of the picture at each of its corners, in
/// glTF texture coordinates: `u` to the right and `v` down, one tile per unit, repeating.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TexturedTriangle {
    pub image: u32,
    pub uv: [[f32; 2]; 3],
}

impl TexturedTriangle {
    /// [`Self::image`] as an index into the pictures; one past any list when it does not fit.
    #[must_use]
    pub fn image_index(&self) -> usize {
        usize::try_from(self.image).unwrap_or(usize::MAX)
    }
}

/// The MIME types of a picture a mesh can show.
pub const MESH_IMAGE_TYPES: [&str; 2] = ["image/png", "image/jpeg"];

/// Why a [`MeshLook`] does not fit its mesh body.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MeshLookError {
    /// It does not have one entry per triangle.
    TriangleCount,
    /// It names no picture, a picture no triangle shows or one it does not have.
    Images,
    /// A picture's blob is no SHA-256 hex or its type is not PNG or JPEG.
    Image,
    /// A texture coordinate is not a finite number.
    Coordinates,
}

impl MeshLook {
    /// Whether this look fits a mesh body of `triangle_count` triangles.
    pub fn validate(&self, triangle_count: usize) -> Result<(), MeshLookError> {
        if self.triangles.len() != triangle_count {
            return Err(MeshLookError::TriangleCount);
        }
        if self.images.iter().any(|image| {
            !crate::graph::is_sha256_hex(&image.blob)
                || !MESH_IMAGE_TYPES.contains(&image.mime_type.as_str())
        }) {
            return Err(MeshLookError::Image);
        }
        let mut shown = vec![false; self.images.len()];
        for triangle in self.triangles.iter().flatten() {
            let slot = shown
                .get_mut(triangle.image_index())
                .ok_or(MeshLookError::Images)?;
            *slot = true;
            if !triangle.uv.iter().flatten().all(|value| value.is_finite()) {
                return Err(MeshLookError::Coordinates);
            }
        }
        if shown.is_empty() || shown.contains(&false) {
            return Err(MeshLookError::Images);
        }
        Ok(())
    }

    /// The blobs of its pictures.
    pub fn blobs(&self) -> impl Iterator<Item = &str> {
        self.images.iter().map(|image| image.blob.as_str())
    }
}
