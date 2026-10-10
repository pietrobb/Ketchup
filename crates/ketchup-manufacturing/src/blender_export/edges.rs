//! Presentation edges for the GLB export: face boundaries and creases of the
//! tessellation, as the desktop renderer draws them.

use super::{ExactProductError, MeshExportSource, Vec3};
use std::collections::BTreeMap;

/// How the triangles sharing one edge meet.
struct EdgeUse {
    count: usize,
    face: Option<u32>,
    normal: Vec3,
    all_faces_known: bool,
    different_faces: bool,
    crease: bool,
}

// The same face-boundary / 0.95 normal-dot rule used by the desktop renderer.
// These are presentation segments of the accepted tessellation, not exact CAD edges.
pub(super) fn feature_edges(
    source: MeshExportSource<'_>,
) -> Result<Vec<[u32; 2]>, ExactProductError> {
    let mut uses = BTreeMap::<[u32; 2], EdgeUse>::new();
    for index in 0..source.triangle_count() {
        let triangle = source
            .triangle_indices(index)
            .ok_or(ExactProductError::InvalidMeshExport)?;
        let [a, b, c] =
            triangle.map(|vertex| {
                source
                    .vertex_position_mm(usize::try_from(vertex).map_err(
                        |_: std::num::TryFromIntError| ExactProductError::InvalidMeshExport,
                    )?)
                    .map(Vec3::from)
                    .filter(|point| point.is_finite())
                    .ok_or(ExactProductError::InvalidMeshExport)
            });
        let [a, b, c] = [a?, b?, c?];
        let normal = (b - a).cross(c - a);
        let normal = normal.normalized().unwrap_or(Vec3::ZERO);
        let face = match source {
            MeshExportSource::Exact(package) => package.triangle_face_ordinal(index),
            MeshExportSource::Canonical { .. } => None,
        };
        for mut edge in [
            [triangle[0], triangle[1]],
            [triangle[1], triangle[2]],
            [triangle[2], triangle[0]],
        ] {
            edge.sort_unstable();
            if edge[0] == edge[1] {
                continue;
            }
            uses.entry(edge)
                .and_modify(|usage| {
                    usage.count += 1;
                    usage.all_faces_known &= face.is_some();
                    usage.different_faces |= face != usage.face;
                    usage.crease |= usage.normal.dot(normal) < 0.95;
                })
                .or_insert(EdgeUse {
                    count: 1,
                    face,
                    normal,
                    all_faces_known: face.is_some(),
                    different_faces: false,
                    crease: false,
                });
        }
    }
    Ok(uses
        .into_iter()
        .filter(|(_, usage)| {
            usage.count == 1
                || if usage.all_faces_known {
                    usage.different_faces
                } else {
                    usage.crease
                }
        })
        .map(|(edge, _)| edge)
        .collect())
}
