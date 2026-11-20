//! Every structural parameter a feature describes can be read and written back.

use super::*;
use ketchup_geometry::sketch::{
    FeatureDirection, SketchConstraint, SketchEntityId, SketchPointRef, SketchRegionId,
};

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn assert_descriptors_have_read_write_accessors(kind: FeatureKind, tolerance_mm: f64) {
    validate_feature_kind(&kind, tolerance_mm).unwrap();
    let descriptors = kind.parameter_descriptors();
    assert!(!descriptors.is_empty());
    for descriptor in descriptors {
        let value = feature_kind_parameter_value(&kind, descriptor.path().as_str())
            .unwrap_or_else(|| panic!("missing reader for {}", descriptor.path().as_str()));
        let target = FeatureParameterTarget::new(
            FeatureId(99),
            descriptor.path().as_str(),
            descriptor.value_type(),
        )
        .unwrap();
        let mut updated = kind.clone();
        assert!(
            set_feature_kind_parameter(&mut updated, &target, &dimension(value), tolerance_mm)
                .unwrap(),
            "missing writer for {}",
            descriptor.path().as_str()
        );
        validate_feature_kind(&updated, tolerance_mm).unwrap_or_else(|error| {
            panic!(
                "writer for {} produced invalid feature: {error}",
                descriptor.path().as_str()
            )
        });
        assert_eq!(
            feature_kind_parameter_value(&updated, descriptor.path().as_str()),
            Some(value)
        );
    }
}

#[test]
fn every_structural_descriptor_has_a_valid_read_write_round_trip() {
    let tolerance_mm = crate::tolerance::DEFAULT_LINEAR_TOLERANCE_MM;
    let sketch = FeatureKind::Sketch(SketchSpec {
        workplane: FeatureId(1),
        entities: vec![SketchEntity::Circle {
            id: SketchEntityId(1),
            center_mm: [3.0, 4.0],
            radius_mm: 2.0,
        }],
        constraints: vec![
            SketchConstraint {
                id: SketchConstraintId(1),
                kind: SketchConstraintKind::Radius {
                    entity: SketchEntityId(1),
                    value: dimension(2.0),
                },
            },
            SketchConstraint {
                id: SketchConstraintId(2),
                kind: SketchConstraintKind::FixedPoint {
                    point: SketchPointRef {
                        entity: SketchEntityId(1),
                        point: SketchPointKind::Center,
                    },
                    position_mm: [3.0, 4.0],
                },
            },
        ],
    });
    let kinds = vec![
        FeatureKind::Workplane(WorkplaneSpec {
            support: WorkplaneSupport::Offset {
                base: FeatureId(1),
                distance: dimension(8.0),
            },
            frame: WorkplaneFrame::principal(PrincipalPlane::Xy),
        }),
        sketch,
        FeatureKind::polygon(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]]),
        FeatureKind::Profile {
            segments: vec![ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [4.0, 2.0],
            }],
            closed: false,
        },
        FeatureKind::closed_spline(&[[0.0, 0.0], [4.0, 0.0], [4.0, 2.0], [0.0, 2.0]]),
        FeatureKind::extrusion(FeatureId(1), dimension(12.0)),
        FeatureKind::Pad(PadSpec {
            profile: PadProfile::SketchRegion {
                sketch: FeatureId(1),
                region: SketchRegionId(1),
            },
            direction: FeatureDirection::AlongNormal,
            extent: FeatureExtent::Bidirectional {
                along: FeatureExtentEnd::Blind(dimension(6.0)),
                opposite: FeatureExtentEnd::Blind(dimension(3.0)),
            },
            operation: PadOperation::NewBody,
        }),
        FeatureKind::Revolve {
            profile: FeatureId(1),
            axis_start_mm: [0.0, 0.0],
            axis_end_mm: [0.0, 1.0],
            angle_degrees: 180.0,
        },
        FeatureKind::pocket(FeatureId(2), FeatureId(1), dimension(5.0)),
        FeatureKind::PlanarOffset {
            profile: FeatureId(1),
            distance: dimension(2.0),
        },
        FeatureKind::Loft {
            sections: vec![
                LoftSection {
                    profile: FeatureId(1),
                    elevation_mm: 0.0,
                },
                LoftSection {
                    profile: FeatureId(2),
                    elevation_mm: 10.0,
                },
            ],
            guide: None,
            continuity: LoftContinuity::Position,
            rail: None,
        },
    ];
    for kind in kinds {
        assert_descriptors_have_read_write_accessors(kind, tolerance_mm);
    }
}
