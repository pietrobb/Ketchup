use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, FeatureId, FeatureKind,
    OccurrenceId, ProfileSegment, Transform,
};
use ketchup_core::dxf_export::{DxfProfileExportError, export_visible_profiles_dxf};
use ketchup_core::import::{DxfImportOptions, inspect_dxf, plan_dxf_import};
use ketchup_core::sketch::{
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
};

fn representative_dxf() -> Vec<u8> {
    b"0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n\
0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n\
0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n10\n0\n20\n0\n10\n4\n20\n0\n42\n1\n10\n4\n20\n4\n10\n0\n20\n4\n\
0\nCIRCLE\n8\nbores\n10\n20\n20\n20\n40\n2\n\
0\nENDSEC\n0\nEOF\n"
        .to_vec()
}

fn commit(document: &mut DocumentStore, batch: CommandBatch) {
    let proposal = document.prepare_proposal(batch).unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
}

#[test]
fn visible_profile_dxf_round_trip_is_deterministic_layered_and_non_mutating() {
    let source = representative_dxf();
    let mut document = DocumentStore::new();
    let batch = plan_dxf_import(
        &document.current(),
        &source,
        "representative.dxf",
        DxfImportOptions::new(None),
    )
    .unwrap();
    commit(&mut document, batch);
    commit(
        &mut document,
        CommandBatch::new(vec![CanonicalCommand::SetOccurrenceTransform {
            id: OccurrenceId(1),
            transform: Transform::from_translation(50.0, 25.0, 0.0).unwrap(),
        }]),
    );
    let snapshot = document.current();
    let before = snapshot.canonical_digest();

    let exported = export_visible_profiles_dxf(&snapshot).unwrap();
    let repeated = export_visible_profiles_dxf(&snapshot).unwrap();
    assert_eq!(exported, repeated);
    assert_eq!(snapshot.canonical_digest(), before);
    assert!(exported.dxf.starts_with(b"0\nSECTION\n2\nHEADER\n"));
    assert!(
        exported
            .loss_report
            .contains(&format!("source_digest={before}"))
    );
    assert!(exported.loss_report.contains("unit=millimetre"));
    assert!(exported.loss_report.contains("dwg=unsupported"));

    let original = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    let round_trip = inspect_dxf(&exported.dxf, DxfImportOptions::new(None)).unwrap();
    assert_eq!(round_trip.layers(), original.layers());
    assert_eq!(round_trip.profiles().len(), original.profiles().len());
    assert_eq!(
        round_trip
            .profiles()
            .iter()
            .map(|profile| (profile.layer(), profile.closed(), profile.segments().len()))
            .collect::<Vec<_>>(),
        vec![("bores", true, 2), ("cut", true, 4), ("outline", false, 2),]
    );
    let moved = round_trip
        .profiles()
        .iter()
        .find(|profile| profile.layer() == "bores")
        .unwrap();
    assert!(moved.segments()[0].start_mm()[0] >= 68.0);
    assert!(moved.segments()[0].start_mm()[1] >= 43.0);
}

#[test]
fn solved_sketch_profiles_export_line_arc_circle_and_composed_workplane_frame() {
    let definition = DefinitionId(1);
    let workplane = FeatureId(1);
    let sketch = FeatureId(2);
    let occurrence = OccurrenceId(1);
    let mut document = DocumentStore::new();
    commit(
        &mut document,
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: definition,
                name: "modern-sketch".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: definition,
                name: "rotated-plane".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [10.0, 20.0, 0.0],
                        [0.0, 1.0, 0.0],
                        [-1.0, 0.0, 0.0],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: definition,
                name: "solved-profiles".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            end_mm: [10.0, 0.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [10.0, 0.0],
                            end_mm: [10.0, 10.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(3),
                            start_mm: [10.0, 10.0],
                            end_mm: [0.0, 10.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [0.0, 10.0],
                            end_mm: [0.0, 0.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(5),
                            start_mm: [20.0, 0.0],
                            end_mm: [30.0, 0.0],
                        },
                        SketchEntity::Arc {
                            id: SketchEntityId(6),
                            start_mm: [20.0, 0.0],
                            end_mm: [30.0, 0.0],
                            center_mm: [25.0, 0.0],
                            clockwise: true,
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(7),
                            center_mm: [45.0, 0.0],
                            radius_mm: 2.0,
                        },
                    ],
                    constraints: vec![SketchConstraint {
                        id: SketchConstraintId(1),
                        kind: SketchConstraintKind::Radius {
                            entity: SketchEntityId(7),
                            value: Dimension::from_decimal("3").unwrap(),
                        },
                    }],
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: occurrence,
                definition_id: definition,
                name: "placed-sketch".to_owned(),
                transform: Transform::from_translation(100.0, 200.0, 0.0).unwrap(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]),
    );
    let snapshot = document.current();
    let before = snapshot.canonical_digest();

    let exported = export_visible_profiles_dxf(&snapshot).unwrap();
    assert_eq!(snapshot.canonical_digest(), before);
    assert!(exported.loss_report.contains("profile_count=3"));
    assert!(exported.loss_report.contains("segment_count=8"));
    assert!(
        exported
            .loss_report
            .contains("omitted_non_profile_feature_count=1")
    );

    let inspected = inspect_dxf(&exported.dxf, DxfImportOptions::new(None)).unwrap();
    assert_eq!(inspected.layers(), &["modern-sketch"]);
    assert_eq!(inspected.profiles().len(), 3);
    let circle = inspected
        .profiles()
        .iter()
        .find(|profile| {
            profile.segments().len() == 2
                && profile
                    .segments()
                    .iter()
                    .all(|segment| matches!(segment, ProfileSegment::CircularArc { .. }))
        })
        .expect("the solved circle must remain two exact circular arcs");
    for segment in circle.segments() {
        let ProfileSegment::CircularArc {
            start_mm,
            center_mm,
            ..
        } = segment
        else {
            unreachable!();
        };
        assert!((center_mm[0] - 110.0).abs() < 1.0e-9);
        assert!((center_mm[1] - 265.0).abs() < 1.0e-9);
        assert!(
            ((start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]) - 3.0).abs() < 1.0e-9
        );
    }
    let line_arc = inspected
        .profiles()
        .iter()
        .find(|profile| {
            profile
                .segments()
                .iter()
                .any(|segment| matches!(segment, ProfileSegment::Line { .. }))
                && profile
                    .segments()
                    .iter()
                    .any(|segment| matches!(segment, ProfileSegment::CircularArc { .. }))
        })
        .expect("the solved line/arc boundary must remain mixed exact geometry");
    let arc = line_arc
        .segments()
        .iter()
        .find_map(|segment| match segment {
            ProfileSegment::CircularArc {
                center_mm,
                start_mm,
                ..
            } => Some((*center_mm, *start_mm)),
            _ => None,
        })
        .unwrap();
    assert!((arc.0[0] - 110.0).abs() < 1.0e-9);
    assert!((arc.0[1] - 245.0).abs() < 1.0e-9);
    assert!(((arc.1[0] - arc.0[0]).hypot(arc.1[1] - arc.0[1]) - 5.0).abs() < 1.0e-9);
    let rectangle = inspected
        .profiles()
        .iter()
        .find(|profile| profile.segments().len() == 4)
        .expect("the solved line rectangle must remain four lines");
    assert!(rectangle.segments().iter().any(|segment| {
        let start = segment.start_mm();
        (start[0] - 100.0).abs() < 1.0e-9 && (start[1] - 230.0).abs() < 1.0e-9
    }));
}

#[test]
fn unsupported_curves_and_non_planar_occurrences_fail_closed() {
    let mut bezier_document = DocumentStore::new();
    commit(
        &mut bezier_document,
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "curves".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 1.0, 0.0],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "curve".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: FeatureId(1),
                    entities: vec![
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(1),
                            start_mm: [0.0, 0.0],
                            control_1_mm: [1.0, 2.0],
                            control_2_mm: [2.0, 2.0],
                            end_mm: [3.0, 0.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [3.0, 0.0],
                            end_mm: [0.0, 0.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "curve".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]),
    );
    assert_eq!(
        export_visible_profiles_dxf(&bezier_document.current()),
        Err(DxfProfileExportError::UnsupportedCurve)
    );

    let mut non_planar_document = DocumentStore::new();
    commit(
        &mut non_planar_document,
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "non-planar-sketch".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(1),
                definition_id: DefinitionId(1),
                name: "XZ".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::Free,
                    frame: WorkplaneFrame::from_axes(
                        [0.0, 0.0, 0.0],
                        [1.0, 0.0, 0.0],
                        [0.0, 0.0, 1.0],
                    )
                    .unwrap(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(2),
                definition_id: DefinitionId(1),
                name: "circle".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: FeatureId(1),
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [0.0, 0.0],
                        radius_mm: 5.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "non-planar-sketch".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
        ]),
    );
    assert_eq!(
        export_visible_profiles_dxf(&non_planar_document.current()),
        Err(DxfProfileExportError::UnsupportedTransform)
    );
}
