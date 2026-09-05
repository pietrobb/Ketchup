#[path = "support/generic_sketch_pocket.rs"]
mod fixture;
use fixture::*;
use ketchup_core::document::{
    BodyId, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, DocumentStore,
    FeatureKind, ProfileSegment,
};
use ketchup_core::exact_brep_graph::{ExactBRepGraph, ExactBRepGraphError, ExactBRepOperation};
use ketchup_core::persistence;
use ketchup_core::sketch::{PrincipalPlane, SketchEntity, SketchEntityId, WorkplaneFrame};
use std::collections::BTreeSet;

#[test]
fn sketch_pocket_preserves_single_region_identity_and_workplane() {
    for plane in [PrincipalPlane::Xy, PrincipalPlane::Yz, PrincipalPlane::Xz] {
        for offset in [0.0, 35.0] {
            let document = document(plane, offset);
            let snapshot = document.current();
            let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
            let ExactBRepOperation::ProfileCut {
                profile,
                interval,
                depth_bits,
                support_lineage_digest,
                ..
            } = graph.nodes.last().unwrap().operation.clone()
            else {
                panic!("expected cut")
            };
            let compiled = &graph.profiles[profile.0 as usize];
            let FeatureKind::Sketch(sketch) = snapshot.feature(CUT_SKETCH).unwrap().kind() else {
                panic!("expected sketch")
            };
            let frame = WorkplaneFrame::principal(plane).offset(offset);
            assert_eq!(compiled.source_feature_id, CUT_SKETCH.0);
            assert_eq!(
                compiled.region_id,
                Some(sketch.solved_regions().unwrap()[0].id.0)
            );
            let actual = compiled.frame_bits.map(f64::from_bits);
            assert_eq!(&actual[0..3], &frame.origin_mm);
            assert_eq!(&actual[3..6], &frame.x_axis);
            assert_eq!(&actual[6..9], &frame.y_axis);
            assert_eq!(&actual[9..12], &frame.normal);
            assert_eq!(interval.direction(), frame.normal);
            assert_eq!((interval.start_mm(), interval.end_mm()), (0.0, 20.0));
            assert_eq!(depth_bits.map(f64::from_bits), Some(20.0));
            assert_eq!(support_lineage_digest, None);
            assert_eq!(
                ExactBRepGraph::from_bytes(&graph.to_bytes().unwrap()).unwrap(),
                graph
            );
            let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
            assert_eq!(
                ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, POCKET).unwrap(),
                graph
            );
        }
    }
}

#[test]
fn missing_ambiguous_open_and_foreign_sketch_profiles_fail_atomically() {
    for case in 0..4 {
        let mut document = base_document(PrincipalPlane::Xy, 0.0);
        let before = document.current().canonical_digest();
        let mut sketch = rectangle(PLANE, [40.0, 40.0], [80.0, 80.0]);
        let mut commands = vec![];
        match case {
            0 => {} // Missing profile.
            1 => {
                sketch.entities.push(SketchEntity::Circle {
                    id: SketchEntityId(10),
                    center_mm: [150.0, 100.0],
                    radius_mm: 10.0,
                });
                assert_eq!(sketch.solved_regions().unwrap().len(), 2);
                commands.push(feature(CUT_SKETCH, FeatureKind::Sketch(sketch)));
            }
            2 => {
                sketch.entities.truncate(1);
                assert!(sketch.solved_regions().is_err());
                commands.push(feature(CUT_SKETCH, FeatureKind::Sketch(sketch)));
            }
            3 => {
                commands.push(CanonicalCommand::CreateDefinition {
                    id: DefinitionId(2),
                    name: "Other".into(),
                });
                commands.push(CanonicalCommand::CreateFeature {
                    id: CUT_SKETCH,
                    definition_id: DefinitionId(2),
                    name: "Foreign".into(),
                    kind: FeatureKind::Sketch(sketch),
                });
            }
            _ => unreachable!(),
        }
        commands.push(pocket(CUT_SKETCH, 20.0));
        assert!(
            document.apply_batch(&CommandBatch::new(commands)).is_err(),
            "case {case}"
        );
        assert_eq!(document.current().canonical_digest(), before);
    }
}

#[test]
fn suppressed_sketch_dependency_suffix_fails_closed() {
    let mut document = document(PrincipalPlane::Xy, 0.0);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                suppressed_feature_ids: vec![CUT_SKETCH, POCKET],
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, POCKET),
        Err(ExactBRepGraphError::SuppressedFeature(POCKET))
    );
}

#[test]
fn geometry_edit_recomputes_only_the_dependent_cut_graph() {
    let mut document = document(PrincipalPlane::Yz, 35.0);
    let before = document.current();
    let graph = ExactBRepGraph::from_snapshot(&before, DEFINITION, POCKET).unwrap();
    let pad = ExactBRepGraph::from_snapshot(&before, DEFINITION, PAD).unwrap();
    let dependencies = before.feature_dependency_graph().unwrap();
    assert_eq!(
        dependencies.dependent_closure(BTreeSet::from([CUT_SKETCH])),
        BTreeSet::from([CUT_SKETCH, POCKET])
    );
    assert!(
        dependencies
            .dependent_closure(BTreeSet::from([PLANE]))
            .contains(&POCKET)
    );
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::TranslateProfile {
                id: CUT_SKETCH,
                delta_mm: [15.0, 10.0],
            },
        ]))
        .unwrap();
    let after = document.current();
    let recomputed = ExactBRepGraph::from_snapshot(&after, DEFINITION, POCKET).unwrap();
    assert_ne!(recomputed.graph_digest, graph.graph_digest);
    assert_ne!(
        recomputed.canonical_input_digest,
        graph.canonical_input_digest
    );
    assert_eq!(
        recomputed.profiles[1].source_feature_id,
        graph.profiles[1].source_feature_id
    );
    assert_eq!(
        recomputed.profiles[1].region_id,
        graph.profiles[1].region_id
    );
    assert_ne!(recomputed.profiles[1].geometry, graph.profiles[1].geometry);
    assert_eq!(
        ExactBRepGraph::from_snapshot(&after, DEFINITION, PAD)
            .unwrap()
            .graph_digest,
        pad.graph_digest
    );
}

#[test]
fn legacy_profile_and_segment_pockets_keep_blind_xy_behavior() {
    for segments in [false, true] {
        let points = vec![[40.0, 40.0], [80.0, 40.0], [80.0, 80.0], [40.0, 80.0]];
        let cut_kind = if segments {
            FeatureKind::SegmentProfile {
                segments: (0..4)
                    .map(|i| ProfileSegment::Line {
                        start_mm: points[i],
                        end_mm: points[(i + 1) % 4],
                    })
                    .collect(),
                closed: true,
            }
        } else {
            FeatureKind::Profile { points_mm: points }
        };
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DEFINITION,
                    name: "Legacy".into(),
                },
                feature(
                    BASE_SKETCH,
                    FeatureKind::Profile {
                        points_mm: vec![[0.0, 0.0], [400.0, 0.0], [400.0, 200.0], [0.0, 200.0]],
                    },
                ),
                feature(
                    PAD,
                    FeatureKind::Extrusion {
                        profile: BASE_SKETCH,
                        height: dimension(20.0),
                    },
                ),
                feature(CUT_SKETCH, cut_kind),
                pocket(CUT_SKETCH, 10.0),
            ]))
            .unwrap();
        let graph = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, POCKET).unwrap();
        let ExactBRepOperation::ProfileCut {
            profile, interval, ..
        } = graph.nodes.last().unwrap().operation
        else {
            panic!("expected cut")
        };
        assert_eq!(graph.profiles[profile.0 as usize].region_id, None);
        assert_eq!(interval.direction(), [0.0, 0.0, 1.0]);
        assert_eq!((interval.start_mm(), interval.end_mm()), (0.0, 10.0));
        let before = document.current().canonical_digest();
        assert_eq!(
            document
                .apply_batch(&CommandBatch::new(vec![
                    CanonicalCommand::SetFeatureDimension {
                        id: POCKET,
                        dimension: dimension(20.0)
                    }
                ]))
                .err()
                .unwrap(),
            CanonicalError::InvalidFeatureOwnership(POCKET)
        );
        assert_eq!(document.current().canonical_digest(), before);
    }
}
