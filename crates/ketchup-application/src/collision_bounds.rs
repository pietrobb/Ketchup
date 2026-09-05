//! Certified canonical envelopes for rejection only; never collision evidence.
use ketchup_core::exact_brep_graph::{
    ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry, ExactBRepPlanarLoop,
    ExactBRepPlanarSegment,
};

#[derive(Clone, Copy, Debug)]
pub(super) struct Bounds([[f64; 3]; 2]);

// Large guard over the handful of products/sums per operation. Track absolute
// intermediate magnitudes (not just output coordinates) to cover cancellation,
// and propagate accumulated error through every affine transform.
const ROUNDING: f64 = 64.0 * f64::EPSILON;

fn lines_scale(segments: &[ExactBRepPlanarSegment]) -> Option<f64> {
    let mut scale: f64 = 1.0;
    for segment in segments {
        let ExactBRepPlanarSegment::Line {
            start_bits,
            end_bits,
        } = segment
        else {
            return None;
        };
        for value in start_bits
            .iter()
            .chain(end_bits)
            .copied()
            .map(f64::from_bits)
        {
            scale = scale.max(value.abs());
        }
    }
    Some(scale)
}

fn circle_scale(center: [u64; 2], radius: u64) -> f64 {
    center
        .map(f64::from_bits)
        .into_iter()
        .map(f64::abs)
        .fold(1.0, f64::max)
        + f64::from_bits(radius).abs()
}

fn loop_scale(boundary: &ExactBRepPlanarLoop) -> Option<f64> {
    match boundary {
        ExactBRepPlanarLoop::Boundary { segments } => lines_scale(segments),
        ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        } => Some(circle_scale(*center_bits, *radius_bits)),
    }
}

fn profile_scale(geometry: &ExactBRepPlanarGeometry) -> Option<f64> {
    match geometry {
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } => lines_scale(segments),
        ExactBRepPlanarGeometry::Circle {
            center_bits,
            radius_bits,
        } => Some(circle_scale(*center_bits, *radius_bits)),
        ExactBRepPlanarGeometry::Region { outer, holes } => {
            let scale = loop_scale(outer)?;
            for hole in holes {
                loop_scale(hole)?;
            }
            Some(scale)
        }
        _ => None,
    }
}

fn affine_scale(matrix: [f64; 16], magnitude: f64, error: f64) -> Option<(f64, f64)> {
    if !matrix.iter().all(|x| x.is_finite()) || matrix[12..] != [0.0, 0.0, 0.0, 1.0] {
        return None;
    }
    let norm = (0..3)
        .map(|axis| (0..3).map(|j| matrix[4 * axis + j].abs()).sum::<f64>())
        .fold(1.0, f64::max)
        * (1.0 + ROUNDING);
    let translation = [matrix[3], matrix[7], matrix[11]]
        .into_iter()
        .map(f64::abs)
        .fold(0.0, f64::max);
    let magnitude = (norm * magnitude + translation).max(1.0) * (1.0 + ROUNDING);
    let error = norm * error + ROUNDING * magnitude;
    (magnitude.is_finite() && error.is_finite()).then_some((magnitude, error))
}

fn supported_operation(operation: &ExactBRepOperation) -> bool {
    matches!(
        operation,
        ExactBRepOperation::Extrude { .. }
            | ExactBRepOperation::ProfileCut { .. }
            | ExactBRepOperation::RigidTransform { .. }
            | ExactBRepOperation::Boolean { .. }
    )
}

pub(super) fn certified_bounds(graph: &ExactBRepGraph) -> Option<Bounds> {
    if !graph
        .nodes
        .iter()
        .all(|node| supported_operation(&node.operation))
    {
        return None;
    }
    // This API validates the entire graph. Its envelopes alone are NOT safe for
    // every operation: in particular Shell/EdgeFinish/FaceOffset may grow solids.
    let bounds = graph.producer_bounds_mm().ok()??;
    let scales = graph
        .profiles
        .iter()
        .map(|p| profile_scale(&p.geometry))
        .collect::<Option<Vec<_>>>()?;
    let mut errors: Vec<(f64, f64)> = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let (magnitude, error) = match &node.operation {
            ExactBRepOperation::Extrude {
                profile, interval, ..
            } => {
                let frame = graph.profiles[profile.0 as usize]
                    .frame_bits
                    .map(f64::from_bits);
                let scale = scales[profile.0 as usize];
                let distance = interval.start_mm().abs().max(interval.end_mm().abs());
                let magnitude = (0..3)
                    .map(|axis| {
                        frame[axis].abs()
                            + (frame[3 + axis].abs() + frame[6 + axis].abs()) * scale
                            + interval.direction()[axis].abs() * distance
                    })
                    .fold(1.0, f64::max)
                    * (1.0 + ROUNDING);
                (magnitude, ROUNDING * magnitude)
            }
            ExactBRepOperation::ProfileCut { target, .. } => errors[target.0 as usize],
            ExactBRepOperation::Boolean { target, tool, .. } => {
                let l = errors[target.0 as usize];
                let r = errors[tool.0 as usize];
                (l.0.max(r.0), l.1.max(r.1))
            }
            ExactBRepOperation::RigidTransform {
                target,
                matrix_bits,
            } => {
                let (magnitude, error) = errors[target.0 as usize];
                affine_scale(matrix_bits.map(f64::from_bits), magnitude, error)?
            }
            // Reject the entire certification, including unsupported ancestors
            // of otherwise safe cuts/transforms. Native queries remain available.
            _ => return None,
        };
        if !magnitude.is_finite() || !error.is_finite() {
            return None;
        }
        errors.push((magnitude, error));
    }
    let error = errors.last()?.1;
    Bounds::expanded(bounds, error)
}

impl Bounds {
    fn expanded(mut bounds: [[f64; 3]; 2], margin: f64) -> Option<Self> {
        for axis in 0..3 {
            bounds[0][axis] = (bounds[0][axis] - margin).next_down();
            bounds[1][axis] = (bounds[1][axis] + margin).next_up();
        }
        (bounds.iter().flatten().all(|x| x.is_finite())
            && (0..3).all(|axis| bounds[0][axis] <= bounds[1][axis]))
        .then_some(Self(bounds))
    }

    pub(super) fn world(self, matrix: [f64; 16], tolerance: f64) -> Option<Self> {
        if !tolerance.is_finite() || tolerance < 0.0 {
            return None;
        }
        let magnitude = self.0.iter().flatten().map(|x| x.abs()).fold(1.0, f64::max);
        let (_, error) = affine_scale(matrix, magnitude, 0.0)?;
        let mut bounds = [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]];
        for x in [self.0[0][0], self.0[1][0]] {
            for y in [self.0[0][1], self.0[1][1]] {
                for z in [self.0[0][2], self.0[1][2]] {
                    for axis in 0..3 {
                        let i = axis * 4;
                        let value =
                            matrix[i] * x + matrix[i + 1] * y + matrix[i + 2] * z + matrix[i + 3];
                        if !value.is_finite() {
                            return None;
                        }
                        bounds[0][axis] = bounds[0][axis].min(value);
                        bounds[1][axis] = bounds[1][axis].max(value);
                    }
                }
            }
        }
        Self::expanded(bounds, tolerance + error)
    }

    pub(super) fn separated(self, other: Self) -> bool {
        (0..3).any(|axis| self.0[1][axis] < other.0[0][axis] || other.0[1][axis] < self.0[0][axis])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_core::document::*;
    use ketchup_core::exact_brep_graph::{ExactBRepNodeId, ExactBRepProfileId};
    use ketchup_core::prismatic::TolerancePolicy;

    fn graph(points: Vec<[f64; 2]>) -> ExactBRepGraph {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Part".into(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(1),
                    definition_id: DefinitionId(1),
                    name: "Profile".into(),
                    kind: FeatureKind::Profile { points_mm: points },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Solid".into(),
                    kind: FeatureKind::Extrusion {
                        profile: FeatureId(1),
                        height: Dimension::new("10", 10.0).unwrap(),
                    },
                },
            ]))
            .unwrap();
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(1), FeatureId(2)).unwrap()
    }

    fn square() -> ExactBRepGraph {
        graph(vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]])
    }

    fn world(bounds: Bounds, x: f64) -> Bounds {
        bounds
            .world(
                *Transform::from_translation(x, 0.0, 0.0).unwrap().matrix(),
                TolerancePolicy::default().epsilon_mm(),
            )
            .unwrap()
    }

    #[test]
    fn certified_canonical_bounds_safely_reject_separated_solids() {
        let bounds = certified_bounds(&square()).unwrap();
        assert!(world(bounds, 0.0).separated(world(bounds, 20.0)));
        assert!(!world(bounds, 0.0).separated(world(bounds, 9.0)));
    }

    #[test]
    fn sloped_false_positive_remains_a_native_candidate() {
        let left = certified_bounds(&graph(vec![[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]])).unwrap();
        let right = certified_bounds(&graph(vec![[10.0, 10.0], [1.0, 10.0], [10.0, 1.0]])).unwrap();
        assert!(!world(left, 0.0).separated(world(right, 0.0)));
    }

    #[test]
    fn touching_and_tolerance_boundary_are_not_rejected() {
        let bounds = certified_bounds(&square()).unwrap();
        let epsilon = TolerancePolicy::default().epsilon_mm();
        for offset in [10.0, 10.0 + epsilon, 10.0 + 2.0 * epsilon] {
            assert!(!world(bounds, 0.0).separated(world(bounds, offset)));
        }
        assert!(world(bounds, 0.0).separated(world(bounds, 10.0 + 4.0 * epsilon)));
    }

    #[test]
    fn unsupported_operations_and_invalid_graphs_fall_back() {
        for operation in [
            ExactBRepOperation::Shell {
                target: ExactBRepNodeId(0),
                removed_faces: vec![],
                thickness_bits: 1.0_f64.to_bits(),
            },
            ExactBRepOperation::PlanarOffset {
                profile: ExactBRepProfileId(0),
                distance_bits: 1.0_f64.to_bits(),
            },
            ExactBRepOperation::Revolve {
                profile: ExactBRepProfileId(0),
                axis_start_bits: [0.0_f64.to_bits(); 2],
                axis_end_bits: [0.0_f64.to_bits(), 1.0_f64.to_bits()],
                angle_degrees_bits: 360.0_f64.to_bits(),
            },
            ExactBRepOperation::ImportedExact {
                source_sha256: [0; 32],
                source_byte_len: 1,
                result_fingerprint: "unknown".into(),
            },
        ] {
            let mut graph = square();
            assert!(!supported_operation(&operation));
            graph.nodes[0].operation = operation;
            assert!(certified_bounds(&graph).is_none());
        }
        let mut invalid = square();
        invalid.nodes.clear();
        assert!(certified_bounds(&invalid).is_none());
    }

    #[test]
    fn unrecognized_profile_curves_fall_back_but_circles_are_supported() {
        assert!(
            profile_scale(&ExactBRepPlanarGeometry::Circle {
                center_bits: [0; 2],
                radius_bits: 10.0_f64.to_bits()
            })
            .is_some()
        );
        assert!(
            profile_scale(&ExactBRepPlanarGeometry::Boundary {
                closed: true,
                segments: vec![ExactBRepPlanarSegment::CircularArc {
                    start_bits: [0; 2],
                    end_bits: [0; 2],
                    center_bits: [0; 2],
                    clockwise: false
                }]
            })
            .is_none()
        );
        assert!(
            profile_scale(&ExactBRepPlanarGeometry::Spline {
                control_point_bits: vec![]
            })
            .is_none()
        );
    }

    #[test]
    fn occurrence_affine_corners_and_roundoff_are_conservative() {
        let bounds = certified_bounds(&square()).unwrap();
        let matrix = [
            1.0, -1.0, 0.0, 1.0e12, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        let transformed = bounds.world(matrix, 0.0).unwrap();
        assert!(transformed.0[0][0] < 1.0e12 - 10.0);
        assert!(transformed.0[1][0] > 1.0e12 + 10.0);
        assert!(transformed.0[1][1] > 20.0);
        assert!(transformed.0[1][2] > 20.0);
        let mut invalid = matrix;
        invalid[12] = 0.1;
        assert!(bounds.world(invalid, 0.0).is_none());
    }
}
