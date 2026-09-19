//! Canonical profile surfaces shared by viewport rendering and picking.
use crate::mesh_projection::{CanonicalPlanarProfileMesh, canonical_profile_feature_mesh};
use ketchup_core::document::{DefinitionId, FeatureKind, Snapshot};
use std::collections::BTreeMap;

pub type SurfaceMesh = (Vec<[f64; 3]>, Vec<[u32; 3]>);

pub fn canonical_definition_surface(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
) -> Option<CanonicalPlanarProfileMesh> {
    let definition = snapshot.definition(definition_id)?;
    let feature_id = *definition.feature_ids().last()?;
    let feature = snapshot.feature(feature_id)?;
    match feature.kind() {
        FeatureKind::Extrusion { profile, height } => {
            let (_, positions, triangles) = canonical_profile_feature_mesh(snapshot, *profile)?;
            let normal = match snapshot.feature(*profile)?.kind() {
                FeatureKind::Sketch(sketch) => {
                    let FeatureKind::Workplane(workplane) =
                        snapshot.feature(sketch.workplane)?.kind()
                    else {
                        return None;
                    };
                    workplane.frame.normal
                }
                _ => [0.0, 0.0, 1.0],
            };
            let (positions, triangles) = extrude_profile_surface(
                (positions, triangles),
                normal.map(|v| v * height.millimetres()),
            )?;
            Some((feature_id, positions, triangles))
        }
        _ => canonical_profile_feature_mesh(snapshot, feature_id),
    }
}

pub fn extrude_profile_surface(
    (positions, face_triangles): SurfaceMesh,
    offset: [f64; 3],
) -> Option<SurfaceMesh> {
    if face_triangles.is_empty() || offset.iter().any(|v| !v.is_finite()) {
        return None;
    }
    let count = u32::try_from(positions.len()).ok()?;
    let first = face_triangles[0].map(|i| positions[i as usize]);
    let u = std::array::from_fn::<_, 3, _>(|i| first[1][i] - first[0][i]);
    let v = std::array::from_fn::<_, 3, _>(|i| first[2][i] - first[0][i]);
    let normal = [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ];
    let direction: f64 = normal.iter().zip(offset).map(|(n, d)| n * d).sum();
    if direction.abs() <= 1.0e-12 {
        return None;
    }
    let mut solid = positions.clone();
    solid.extend(
        positions
            .iter()
            .map(|p| std::array::from_fn(|i| p[i] + offset[i])),
    );
    let mut triangles = Vec::new();
    let mut edges = BTreeMap::<[u32; 2], Vec<[u32; 2]>>::new();
    for face in face_triangles {
        let top = if direction > 0.0 {
            face
        } else {
            [face[0], face[2], face[1]]
        };
        triangles.push([top[0], top[2], top[1]]);
        triangles.push(top.map(|i| i + count));
        for edge in [[top[0], top[1]], [top[1], top[2]], [top[2], top[0]]] {
            let key = if edge[0] < edge[1] {
                edge
            } else {
                [edge[1], edge[0]]
            };
            edges.entry(key).or_default().push(edge);
        }
    }
    // Derive the perimeter from oriented cap triangles, not vertex storage order.
    for uses in edges.values() {
        if let [edge] = uses.as_slice() {
            let [a, b] = *edge;
            triangles.push([a, b, a + count]);
            triangles.push([b, b + count, a + count]);
        }
    }
    Some((solid, triangles))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picking_uses_transformed_prism_faces_and_misses_empty_bounding_box_corners() {
        use crate::mesh_projection::MeshInteractionProjection;
        use crate::{Ray, Vec3};
        use ketchup_core::document::{
            CanonicalCommand as C, CommandBatch, Dimension, DocumentStore, FeatureId, OccurrenceId,
            ProfileSegment, Transform,
        };
        for height in [-8.0, 8.0] {
            let mut store = DocumentStore::new();
            let points = [[0.0, 0.0], [30.0, 0.0], [5.0, 20.0]];
            store
                .apply_batch(&CommandBatch::new(vec![
                    C::CreateDefinition {
                        id: DefinitionId(1),
                        name: "Triangle".into(),
                    },
                    C::CreateFeature {
                        id: FeatureId(1),
                        definition_id: DefinitionId(1),
                        name: "Profile".into(),
                        kind: FeatureKind::SegmentProfile {
                            closed: true,
                            segments: (0..3)
                                .map(|i| ProfileSegment::Line {
                                    start_mm: points[i],
                                    end_mm: points[(i + 1) % 3],
                                })
                                .collect(),
                        },
                    },
                    C::CreateFeature {
                        id: FeatureId(2),
                        definition_id: DefinitionId(1),
                        name: "Prism".into(),
                        kind: FeatureKind::Extrusion {
                            profile: FeatureId(1),
                            height: Dimension::new(height.to_string(), height).unwrap(),
                        },
                    },
                    C::CreateOccurrence {
                        id: OccurrenceId(1),
                        definition_id: DefinitionId(1),
                        name: "Translated".into(),
                        transform: Transform::from_translation(100.0, 50.0, 3.0).unwrap(),
                        parent: None,
                        tag: None,
                        visible: true,
                    },
                ]))
                .unwrap();
            let snapshot = store.current();
            let projection = MeshInteractionProjection::from_snapshot(&snapshot);
            let ray = |x, y, z, dz| {
                Ray::new(Vec3::new(100.0 + x, 50.0 + y, z), Vec3::new(0.0, 0.0, dz)).unwrap()
            };
            assert!(
                projection
                    .exact_surface_pick(ray(25.0, 18.0, 40.0, -1.0))
                    .is_none()
            );
            let top = projection
                .exact_surface_pick(ray(10.0, 5.0, 40.0, -1.0))
                .unwrap();
            assert!((top.position_mm.z - (3.0 + height.max(0.0))).abs() < 1.0e-9);
            assert_eq!(top.outward_normal, Vec3::new(0.0, 0.0, 1.0));
            let bottom = projection
                .exact_surface_pick(ray(10.0, 5.0, -40.0, 1.0))
                .unwrap();
            assert!((bottom.position_mm.z - (3.0 + height.min(0.0))).abs() < 1.0e-9);
            assert_eq!(bottom.outward_normal, Vec3::new(0.0, 0.0, -1.0));
            let side = projection
                .exact_surface_pick(
                    Ray::new(
                        Vec3::new(115.0, 30.0, 3.0 + height * 0.5),
                        Vec3::new(0.0, 1.0, 0.0),
                    )
                    .unwrap(),
                )
                .unwrap();
            assert_eq!(side.outward_normal, Vec3::new(0.0, -1.0, 0.0));
        }
    }

    #[test]
    fn both_windings_and_signed_extents_form_one_outward_closed_prism() {
        for reverse in [false, true] {
            for height in [-8.0, 8.0] {
                let positions = vec![[0.0, 0.0, 0.0], [30.0, 0.0, 0.0], [5.0, 20.0, 0.0]];
                let face = if reverse { [0, 2, 1] } else { [0, 1, 2] };
                let (positions, triangles) =
                    extrude_profile_surface((positions, vec![face]), [0.0, 0.0, height]).unwrap();
                assert_eq!(triangles.len(), 8);
                let mut edges = BTreeMap::<[u32; 2], Vec<[u32; 2]>>::new();
                let mut volume = 0.0;
                for triangle in triangles {
                    let [a, b, c] = triangle.map(|i| positions[i as usize]);
                    volume += (a[0] * (b[1] * c[2] - b[2] * c[1])
                        + a[1] * (b[2] * c[0] - b[0] * c[2])
                        + a[2] * (b[0] * c[1] - b[1] * c[0]))
                        / 6.0;
                    for edge in [
                        [triangle[0], triangle[1]],
                        [triangle[1], triangle[2]],
                        [triangle[2], triangle[0]],
                    ] {
                        let mut key = edge;
                        key.sort();
                        edges.entry(key).or_default().push(edge);
                    }
                }
                assert!(
                    (volume - 2400.0).abs() < 1e-9,
                    "volume={volume}, reverse={reverse}, height={height}"
                );
                for uses in edges.values() {
                    assert_eq!(uses.len(), 2);
                    assert_eq!(uses[0], [uses[1][1], uses[1][0]]);
                }
            }
        }
    }
}
