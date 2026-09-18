use super::{Transform, Vec3, cross, dot, vector_length};

/// Represent coplanar world points in a rigid local XY frame, without flattening them.
pub(super) fn planar_points(points: &[Vec3]) -> Option<(Transform, Vec<Vec3>)> {
    let origin = *points.first()?;
    if points.len() < 2
        || points
            .iter()
            .any(|point| !point.x.is_finite() || !point.y.is_finite() || !point.z.is_finite())
    {
        return None;
    }
    let (x, y, normal) = if points
        .iter()
        .all(|point| (point.z - origin.z).abs() <= 1.0e-7)
    {
        (
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
        )
    } else {
        let direction = points
            .iter()
            .map(|point| *point - origin)
            .find(|direction| vector_length(*direction) > 0.01)?;
        let x = direction * (1.0 / vector_length(direction));
        let normal = points
            .iter()
            .map(|point| cross(x, *point - origin))
            .find(|normal| vector_length(*normal) > 1.0e-7)
            .unwrap_or_else(|| {
                let reference = if x.x.abs() < 0.9 {
                    Vec3::new(1.0, 0.0, 0.0)
                } else {
                    Vec3::new(0.0, 1.0, 0.0)
                };
                cross(x, reference)
            });
        let normal = normal * (1.0 / vector_length(normal));
        (x, cross(normal, x), normal)
    };
    let transform = Transform::from_matrix([
        x.x, y.x, normal.x, origin.x, x.y, y.y, normal.y, origin.y, x.z, y.z, normal.z, origin.z,
        0.0, 0.0, 0.0, 1.0,
    ])
    .ok()?;
    transform.rigid_inverse()?;
    let local = points
        .iter()
        .map(|point| {
            let delta = *point - origin;
            let distance = dot(delta, normal);
            (distance.is_finite() && distance.abs() <= 1.0e-7)
                .then(|| Vec3::new(dot(delta, x), dot(delta, y), 0.0))
        })
        .collect::<Option<Vec<_>>>()?;
    Some((transform, local))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FeatureKind, KetchupApp, ProfileSegment};

    fn selected_line_points(app: &KetchupApp) -> Vec<Vec3> {
        let selected = app.selected_reference().unwrap();
        let snapshot = app.document_snapshot();
        let transform = snapshot
            .world_transform_for_occurrence(selected.instance_path.root_occurrence())
            .unwrap();
        let matrix = transform.matrix();
        let world = |point: [f64; 2]| {
            Vec3::new(
                matrix[0] * point[0] + matrix[1] * point[1] + matrix[3],
                matrix[4] * point[0] + matrix[5] * point[1] + matrix[7],
                matrix[8] * point[0] + matrix[9] * point[1] + matrix[11],
            )
        };
        let segments = snapshot
            .features()
            .find_map(|feature| {
                if feature.definition_id() != selected.definition_id {
                    return None;
                }
                match feature.kind() {
                    FeatureKind::SegmentProfile { segments, .. } => Some(segments),
                    _ => None,
                }
            })
            .unwrap();
        segments
            .iter()
            .flat_map(|segment| match segment {
                ProfileSegment::Line { start_mm, end_mm } => [world(*start_mm), world(*end_mm)],
                _ => panic!("expected a line"),
            })
            .collect()
    }

    fn assert_point(actual: Vec3, expected: Vec3) {
        assert!(
            vector_length(actual - expected) < 1.0e-8,
            "{actual:?} != {expected:?}"
        );
    }

    #[test]
    fn spatial_line_keeps_clicked_endpoints_and_one_undo_step() {
        for delta in [
            Vec3::new(0.0, 0.0, 30.0),
            Vec3::new(30.0, -20.0, 40.0),
            Vec3::new(-8.0, 12.0, -24.0),
        ] {
            let mut app = KetchupApp::new();
            let start = Vec3::new(13.0, -7.0, 11.0);
            let end = start + delta;
            let before = app.canonical_digest();
            let undo = app.undo_step_count();
            assert!(app.complete_line_sketch(start, end));
            let after = app.canonical_digest();
            let points = selected_line_points(&app);
            assert_point(points[0], start);
            assert_point(points[1], end);
            assert_eq!(app.sketch_start, Some(end));
            assert_eq!(app.undo_step_count(), undo + 1);
            assert!(app.undo());
            assert_eq!(app.canonical_digest(), before);
            assert!(app.redo());
            assert_eq!(app.canonical_digest(), after);
        }
    }

    #[test]
    fn spatial_line_exact_length_preserves_three_dimensional_direction() {
        for delta in [Vec3::new(0.0, 0.0, -10.0), Vec3::new(3.0, 4.0, 12.0)] {
            let mut app = KetchupApp::new();
            let start = Vec3::new(2.0, 3.0, 5.0);
            app.sketch_start = Some(start);
            app.sketch_cursor = Some(start + delta);
            app.value_input = "26".into();
            assert!(app.complete_exact_line());
            let points = selected_line_points(&app);
            assert_point(points[0], start);
            assert_point(points[1], start + delta * (26.0 / vector_length(delta)));
        }
    }

    #[test]
    fn spatial_line_chain_closes_on_its_actual_plane() {
        for points in [
            [
                Vec3::new(3.0, 4.0, 5.0),
                Vec3::new(3.0, 4.0, 25.0),
                Vec3::new(3.0, 24.0, 25.0),
            ],
            [
                Vec3::new(3.0, 4.0, 5.0),
                Vec3::new(13.0, 4.0, 15.0),
                Vec3::new(3.0, 24.0, 25.0),
            ],
        ] {
            let mut app = KetchupApp::new();
            app.line_chain_origin = Some(points[0]);
            app.line_chain_points.push(points[0]);
            assert!(app.complete_line_sketch(points[0], points[1]));
            assert!(app.complete_line_sketch(points[1], points[2]));
            let open = app.canonical_digest();
            assert!(app.complete_line_sketch(points[2], points[0]));
            let world = selected_line_points(&app);
            for (actual, expected) in world.into_iter().zip([
                points[0], points[1], points[1], points[2], points[2], points[0],
            ]) {
                assert_point(actual, expected);
            }
            let closed = app.canonical_digest();
            assert!(app.undo());
            assert_eq!(app.canonical_digest(), open);
            assert!(app.redo());
            assert_eq!(app.canonical_digest(), closed);
        }
    }

    #[test]
    fn nonplanar_line_chain_does_not_silently_flatten_into_a_face() {
        let mut app = KetchupApp::new();
        let points = [
            Vec3::ZERO,
            Vec3::new(20.0, 0.0, 0.0),
            Vec3::new(20.0, 20.0, 0.0),
            Vec3::new(0.0, 20.0, 10.0),
        ];
        app.line_chain_origin = Some(points[0]);
        app.line_chain_points.push(points[0]);
        for pair in points.windows(2) {
            assert!(app.complete_line_sketch(pair[0], pair[1]));
        }
        let before = (app.canonical_digest(), app.undo_step_count());
        assert!(!app.close_line_chain());
        assert_eq!((app.canonical_digest(), app.undo_step_count()), before);
    }
}
