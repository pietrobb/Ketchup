use super::*;

#[test]
fn separate_occurrence_union_keep_tool_preserves_both_occurrence_identities() {
    let mut document = seed_separate_solid_tool_document(80.0, 0.0, 80.0);
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
            separate_solid_tool_plan(BooleanOperation::Union, true),
        )]))
        .unwrap();
    let applied = document.current().canonical_digest();

    let snapshot = document.current();
    let target = snapshot.occurrence(OccurrenceId(301)).unwrap();
    let tool = snapshot.occurrence(OccurrenceId(302)).unwrap();
    assert_eq!(target.id(), OccurrenceId(301));
    assert_eq!(target.definition_id(), DefinitionId(401));
    assert_eq!(tool.id(), OccurrenceId(302));
    assert_eq!(tool.definition_id(), DefinitionId(201));
    assert_eq!(tool.transform().matrix()[3], 80.0);
    assert!(matches!(
        document.current().feature(FeatureId(407)).unwrap().kind(),
        FeatureKind::RigidTransform {
            target: FeatureId(406),
            transform,
        } if transform.matrix()[3] == 80.0
    ));
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Union,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Union,
            ..
        }
    )));
    assert_eq!(document.visible_undo_steps(), 1);

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), applied);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_some());
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);
}

#[test]
fn separate_occurrence_intersect_is_canonical_exact_unique_and_persistent() {
    let mut document = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let before = document.current().canonical_digest();
    let intersect = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Intersect, false),
    )]);
    let union = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Union, false),
    )]);
    assert_ne!(intersect.digest(), union.digest());

    document.apply_batch(&intersect).unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    let state = encode_semantic_state(&document.current());
    assert!(
        state
            .complete()
            .contains("features.408.kind.Boolean.operation=\"Intersect\"")
    );
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Intersect,
            ..
        }
    )));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(301), "Unique intersection")
        .unwrap();
    let unique = document.current();
    let unique_definition = unique
        .occurrence(OccurrenceId(301))
        .unwrap()
        .definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    let [_, _, unique_target, _, _, unique_tool, unique_result] = unique_features else {
        panic!("Intersect Make Unique must preserve the graph-first chain");
    };
    assert!(matches!(
        unique.feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_none());
    assert!(matches!(
        reopened.snapshot().feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Intersect,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let mut touching = seed_separate_solid_tool_document(100.0, 0.0, 30.0);
    let touching_digest = touching.current().canonical_digest();
    assert_eq!(
        touching
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Intersect, true),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(touching.current().canonical_digest(), touching_digest);
    assert_eq!(touching.visible_undo_steps(), 0);
}

#[test]
fn separate_occurrence_split_is_stable_unique_persistent_and_exact_ready() {
    let mut document = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let before = document.current().canonical_digest();
    let split = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Split, true),
    )]);
    let intersect = CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
        separate_solid_tool_plan(BooleanOperation::Intersect, true),
    )]);
    assert_ne!(split.digest(), intersect.digest());

    document.apply_batch(&split).unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(401)
    );
    let applied_snapshot = document.current();
    let splitter = applied_snapshot.occurrence(OccurrenceId(302)).unwrap();
    assert_eq!(splitter.definition_id(), DefinitionId(201));
    assert_eq!(splitter.transform().matrix()[3], 70.0);
    assert_eq!(splitter.transform().matrix()[7], 20.0);
    assert!(matches!(
        document.current().feature(FeatureId(408)).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target: FeatureId(404),
            tool: FeatureId(407),
        }
    ));
    assert!(
        encode_semantic_state(&document.current())
            .complete()
            .contains("features.408.kind.Boolean.operation=\"Split\"")
    );
    let graph =
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408))
            .unwrap();
    assert!(graph.nodes.iter().any(|node| matches!(
        node.operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Split,
            ..
        }
    )));

    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(301), "Unique split")
        .unwrap();
    let unique = document.current();
    let unique_definition = unique
        .occurrence(OccurrenceId(301))
        .unwrap()
        .definition_id();
    let unique_features = unique.definition(unique_definition).unwrap().feature_ids();
    let [_, _, unique_target, _, _, unique_tool, unique_result] = unique_features else {
        panic!("Split Make Unique must preserve the graph-first chain");
    };
    assert!(matches!(
        unique.feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.snapshot().occurrence(OccurrenceId(302)).is_some());
    assert!(matches!(
        reopened.snapshot().feature(*unique_result).unwrap().kind(),
        FeatureKind::Boolean {
            operation: BooleanOperation::Split,
            target,
            tool,
        } if target == unique_target && tool == unique_tool
    ));

    let mut consume = seed_separate_solid_tool_document(70.0, 20.0, 30.0);
    let consume_before = consume.current().canonical_digest();
    assert_eq!(
        consume
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Split, false),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(consume.current().canonical_digest(), consume_before);
    assert_eq!(consume.visible_undo_steps(), 0);

    let mut touching = seed_separate_solid_tool_document(100.0, 0.0, 30.0);
    let touching_digest = touching.current().canonical_digest();
    assert_eq!(
        touching
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
                separate_solid_tool_plan(BooleanOperation::Split, true),
            )]))
            .err()
            .unwrap(),
        CanonicalError::InvalidSolidToolPlan
    );
    assert_eq!(touching.current().canonical_digest(), touching_digest);
    assert_eq!(touching.visible_undo_steps(), 0);
}

#[test]
fn bounded_planar_offset_is_dimensioned_validated_undoable_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(701);
    const PROFILE: FeatureId = FeatureId(702);
    const OFFSET: FeatureId = FeatureId(703);
    const OCCURRENCE: OccurrenceId = OccurrenceId(704);

    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Offset profile".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Source rectangle".to_owned(),
                kind: FeatureKind::polygon(&[
                    [10.0, 20.0],
                    [110.0, 20.0],
                    [110.0, 100.0],
                    [10.0, 100.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Planar offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: PROFILE,
                    distance: Dimension::new("5.000", 5.0).unwrap(),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Offset occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let outward = document.current().canonical_digest();
    assert_ne!(outward, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(OFFSET).unwrap().kind(),
        FeatureKind::PlanarOffset { profile: PROFILE, distance }
            if distance.source_token() == "5.000" && distance.millimetres() == 5.0
    ));
    let state = encode_semantic_state(&document.current());
    assert!(state.complete().contains("features.703.kind.PlanarOffset."));
    assert!(
        state
            .complete()
            .contains("features.703.kind.PlanarOffset.distance.source_token=\"5.000\"")
    );
    assert!(state.agent().contains("kind:{PlanarOffset:"));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("100000.001", 100_000.001).unwrap(),
            },
        ]))
        .unwrap();
    let large_rectangle = offset_graph(&document.current(), DEFINITION);
    assert_eq!(offset_distance(&large_rectangle), 100_000.001);
    assert!(matches!(
        &large_rectangle.profiles[0].geometry,
        ExactBRepPlanarGeometry::Boundary { segments, .. } if segments.len() == 4
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-7.5", -7.5).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    assert_ne!(inward, outward);
    assert_eq!(document.visible_undo_steps(), 2);
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    for invalid_distance in [0.0, 0.009, -0.009, -40.0, 1_000_000.0] {
        let before = document.current().canonical_digest();
        let undo_steps = document.visible_undo_steps();
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new(invalid_distance.to_string(), invalid_distance)
                        .unwrap(),
                },
            ]))
            .err()
            .expect("invalid offset must reject the whole batch");
        assert_eq!(error, CanonicalError::InvalidPlanarOffset);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), undo_steps);
    }

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetProfilePoints {
                id: PROFILE,
                points_mm: vec![
                    [33_839.523_822_158_46, 20.0],
                    [33_842.954_297_017_83, 20.0],
                    [33_842.954_297_017_83, 100.0],
                    [33_839.523_822_158_46, 100.0],
                ],
            },
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-1.7102374296856577", -1.710_237_429_685_657_7).unwrap(),
            },
        ]))
        .err()
        .expect("floating-point boundary mismatch must fail closed");
    assert_eq!(error, CanonicalError::InvalidPlanarOffset);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    document.make_unique(OCCURRENCE, "Unique offset").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_profile, unique_offset] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Planar Offset Make Unique must preserve the two-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_offset).unwrap().kind(),
        FeatureKind::PlanarOffset { profile, distance }
            if profile == unique_profile
                && distance.source_token() == "-7.5"
                && distance.millimetres() == -7.5
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_offset).unwrap().kind(),
        FeatureKind::PlanarOffset { profile, distance }
            if profile == unique_profile
                && distance.source_token() == "-7.5"
                && distance.millimetres() == -7.5
    ));
}

#[test]
fn circular_planar_offset_is_persistent_undoable_and_fail_closed() {
    const DEFINITION: DefinitionId = DefinitionId(706);
    const WORKPLANE: FeatureId = FeatureId(707);
    const CIRCLE: FeatureId = FeatureId(708);
    const OFFSET: FeatureId = FeatureId(709);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Circular offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: CIRCLE,
                definition_id: DEFINITION,
                name: "Circle sketch".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![SketchEntity::Circle {
                        id: SketchEntityId(1),
                        center_mm: [12.0, -8.0],
                        radius_mm: 20.0,
                    }],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Planar offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: CIRCLE,
                    distance: Dimension::new("3.000", 3.0).unwrap(),
                },
            },
        ]))
        .unwrap();
    let outward = document.current().canonical_digest();
    let outward_request = offset_graph(&document.current(), DEFINITION);
    assert_eq!(offset_distance(&outward_request), 3.0);
    assert!(matches!(
        outward_request.profiles[0].geometry,
        ExactBRepPlanarGeometry::Circle { center_bits, radius_bits }
            if center_bits.map(f64::from_bits) == [12.0, -8.0]
                && f64::from_bits(radius_bits) == 20.0
    ));

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-3", -3.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request = offset_graph(&document.current(), DEFINITION);
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("-20", -20.0).unwrap(),
                },
            ]))
            .err()
            .expect("collapsed circular offset must fail closed"),
        CanonicalError::InvalidPlanarOffset
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        offset_graph(&reopened.snapshot(), DEFINITION).canonical_input_digest,
        inward_request.canonical_input_digest
    );
}

#[test]
fn cubic_planar_offset_request_is_persistent_undoable_and_forgery_resistant() {
    const DEFINITION: DefinitionId = DefinitionId(710);
    const WORKPLANE: FeatureId = FeatureId(711);
    const SKETCH: FeatureId = FeatureId(712);
    const OFFSET: FeatureId = FeatureId(713);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Cubic planar offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Line cubic sketch".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -15.0],
                            end_mm: [20.0, -15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -15.0],
                            end_mm: [20.0, 15.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 15.0],
                            control_1_mm: [10.0, 25.0],
                            control_2_mm: [-10.0, 25.0],
                            end_mm: [-20.0, 15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 15.0],
                            end_mm: [-20.0, -15.0],
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Cubic offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("2.000", 2.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let outward = document.current().canonical_digest();
    let outward_request = offset_graph(&document.current(), DEFINITION);
    let ExactBRepPlanarGeometry::Boundary { segments, .. } = &outward_request.profiles[0].geometry
    else {
        panic!("expected a boundary profile");
    };
    assert!(matches!(
        segments.as_slice(),
        [
            ExactBRepPlanarSegment::Line { .. },
            ExactBRepPlanarSegment::Line { .. },
            ExactBRepPlanarSegment::CubicBezier { .. },
            ExactBRepPlanarSegment::Line { .. }
        ]
    ));
    let undo_steps = document.visible_undo_steps();
    assert!(matches!(
        document.apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("0", 0.0).unwrap(),
            },
        ])),
        Err(CanonicalError::InvalidPlanarOffset)
    ));
    assert_eq!(document.current().canonical_digest(), outward);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let mut forged = outward_request.clone();
    let ExactBRepPlanarGeometry::Boundary { segments, .. } = &mut forged.profiles[0].geometry
    else {
        panic!("fixture must preserve its boundary");
    };
    let ExactBRepPlanarSegment::CubicBezier { control_1_bits, .. } = &mut segments[2] else {
        panic!("fixture must preserve its cubic segment");
    };
    control_1_bits[1] = 24.0_f64.to_bits();
    assert!(forged.validate().is_err());

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-2", -2.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request = offset_graph(&document.current(), DEFINITION);
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        offset_graph(&reopened.snapshot(), DEFINITION).canonical_input_digest,
        inward_request.canonical_input_digest
    );
}

#[test]
fn compound_planar_offset_request_is_persistent_undoable_and_forgery_resistant() {
    const DEFINITION: DefinitionId = DefinitionId(714);
    const WORKPLANE: FeatureId = FeatureId(715);
    const SKETCH: FeatureId = FeatureId(716);
    const OFFSET: FeatureId = FeatureId(717);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Compound planar offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Line-cubic region with hole".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -15.0],
                            end_mm: [20.0, -15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -15.0],
                            end_mm: [20.0, 15.0],
                        },
                        SketchEntity::CubicBezier {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 15.0],
                            control_1_mm: [10.0, 25.0],
                            control_2_mm: [-10.0, 25.0],
                            end_mm: [-20.0, 15.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 15.0],
                            end_mm: [-20.0, -15.0],
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(5),
                            center_mm: [0.0, 0.0],
                            radius_mm: 5.0,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Compound offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("2.000", 2.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let outward = document.current().canonical_digest();
    let outward_request = offset_graph(&document.current(), DEFINITION);
    let ExactBRepPlanarGeometry::Region { outer, holes } = &outward_request.profiles[0].geometry
    else {
        panic!("expected a region profile");
    };
    assert!(matches!(
        outer,
        ExactBRepPlanarLoop::Boundary { segments }
            if segments.iter().any(|segment| matches!(
                segment,
                ExactBRepPlanarSegment::CubicBezier { .. }
            ))
    ));
    assert!(matches!(
        holes.as_slice(),
        [ExactBRepPlanarLoop::Circle {
            center_bits,
            radius_bits,
        }] if center_bits.map(f64::from_bits) == [0.0, 0.0]
            && f64::from_bits(*radius_bits) == 5.0
    ));

    let mut forged = outward_request.clone();
    let ExactBRepPlanarGeometry::Region { holes, .. } = &mut forged.profiles[0].geometry else {
        panic!("fixture must preserve its region");
    };
    let ExactBRepPlanarLoop::Circle { radius_bits, .. } = &mut holes[0] else {
        panic!("fixture must preserve its circular hole");
    };
    *radius_bits = 4.0_f64.to_bits();
    assert!(forged.validate().is_err());

    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("5", 5.0).unwrap(),
                },
            ]))
            .err()
            .expect("collapsed hole must fail closed"),
        CanonicalError::InvalidPlanarOffset
    );
    assert_eq!(document.current().canonical_digest(), outward);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: OFFSET,
                dimension: Dimension::new("-2", -2.0).unwrap(),
            },
        ]))
        .unwrap();
    let inward = document.current().canonical_digest();
    let inward_request = offset_graph(&document.current(), DEFINITION);
    assert_ne!(inward, outward);
    assert_ne!(
        inward_request.canonical_input_digest,
        outward_request.canonical_input_digest
    );
    assert_eq!(document.undo().unwrap().canonical_digest(), outward);
    assert_eq!(document.redo().unwrap().canonical_digest(), inward);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert!(reopened.migration_losses().is_empty());
    assert_eq!(reopened.snapshot().canonical_digest(), inward);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert_eq!(
        offset_graph(&reopened.snapshot(), DEFINITION).canonical_input_digest,
        inward_request.canonical_input_digest
    );
}

#[test]
fn compound_planar_offset_rejects_inter_loop_collision_before_history() {
    const DEFINITION: DefinitionId = DefinitionId(718);
    const WORKPLANE: FeatureId = FeatureId(719);
    const SKETCH: FeatureId = FeatureId(720);
    const OFFSET: FeatureId = FeatureId(721);

    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Close-hole compound offset".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".to_owned(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Region with close holes".to_owned(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: WORKPLANE,
                    entities: vec![
                        SketchEntity::Line {
                            id: SketchEntityId(1),
                            start_mm: [-20.0, -20.0],
                            end_mm: [20.0, -20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(2),
                            start_mm: [20.0, -20.0],
                            end_mm: [20.0, 20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(3),
                            start_mm: [20.0, 20.0],
                            end_mm: [-20.0, 20.0],
                        },
                        SketchEntity::Line {
                            id: SketchEntityId(4),
                            start_mm: [-20.0, 20.0],
                            end_mm: [-20.0, -20.0],
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(5),
                            center_mm: [-3.0, 0.0],
                            radius_mm: 2.0,
                        },
                        SketchEntity::Circle {
                            id: SketchEntityId(6),
                            center_mm: [3.0, 0.0],
                            radius_mm: 2.0,
                        },
                    ],
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: OFFSET,
                definition_id: DEFINITION,
                name: "Safe outward material offset".to_owned(),
                kind: FeatureKind::PlanarOffset {
                    profile: SKETCH,
                    distance: Dimension::new("1", 1.0).unwrap(),
                },
            },
        ]))
        .unwrap();

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    assert_eq!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetFeatureDimension {
                    id: OFFSET,
                    dimension: Dimension::new("-1", -1.0).unwrap(),
                },
            ]))
            .err(),
        Some(CanonicalError::InvalidPlanarOffset)
    );
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);
}

#[test]
fn bounded_multisegment_profile_sweep_is_validated_undoable_visible_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(711);
    const PROFILE: FeatureId = FeatureId(712);
    const PATH: FeatureId = FeatureId(713);
    const SWEEP: FeatureId = FeatureId(714);
    const OCCURRENCE: OccurrenceId = OccurrenceId(715);

    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Sweep definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangular section".to_owned(),
                kind: FeatureKind::polygon(&[
                    [-5.0, -10.0],
                    [5.0, -10.0],
                    [5.0, 10.0],
                    [-5.0, 10.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Tangent line-arc path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [50.0, 0.0],
                        },
                        ProfileSegment::CircularArc {
                            start_mm: [50.0, 0.0],
                            end_mm: [75.0, 25.0],
                            center_mm: [50.0, 25.0],
                            clockwise: false,
                        },
                        ProfileSegment::Line {
                            start_mm: [75.0, 25.0],
                            end_mm: [75.0, 50.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Bounded sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Sweep occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();

    let swept = document.current().canonical_digest();
    assert_ne!(swept, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty);
    assert_eq!(document.redo().unwrap().canonical_digest(), swept);
    assert!(matches!(
        document.current().feature(SWEEP).unwrap().kind(),
        FeatureKind::Sweep {
            profile: PROFILE,
            path: PATH,
            ..
        }
    ));
    let state = encode_semantic_state(&document.current());
    assert!(state.complete().contains("features.714.kind.Sweep."));
    assert!(
        state
            .complete()
            .contains("features.714.kind.Sweep.profile=712")
    );
    assert!(
        state
            .complete()
            .contains("features.714.kind.Sweep.path=713")
    );
    assert!(state.agent().contains("kind:{Sweep:"));

    document.make_unique(OCCURRENCE, "Unique sweep").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_profile, unique_path, unique_sweep] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Sweep Make Unique must preserve the three-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_sweep).unwrap().kind(),
        FeatureKind::Sweep { profile, path, .. }
            if profile == unique_profile && path == unique_path
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_sweep).unwrap().kind(),
        FeatureKind::Sweep { profile, path, .. }
            if profile == unique_profile && path == unique_path
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Invalid sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Bent path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [0.0, 50.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [0.0, 50.0],
                            end_mm: [25.0, 50.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Unsupported bent sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
        ]))
        .err()
        .expect("bounded Sweep must reject a multi-segment path atomically");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Degenerate segment sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 20.0], [0.0, 20.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Sub-tolerance path segment".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![
                        ProfileSegment::Line {
                            start_mm: [0.0, 0.0],
                            end_mm: [5.0e-8, 0.0],
                        },
                        ProfileSegment::Line {
                            start_mm: [5.0e-8, 0.0],
                            end_mm: [10.0, 0.0],
                        },
                    ],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Degenerate segment sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
        ]))
        .err()
        .expect("Sweep must reject a sub-tolerance path segment atomically");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let error = invalid
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Unsupported spline sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Spline section".to_owned(),
                kind: FeatureKind::closed_spline(&[
                    [-5.0, -10.0],
                    [5.0, -10.0],
                    [5.0, 10.0],
                    [-5.0, 10.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Straight path".to_owned(),
                kind: FeatureKind::Profile {
                    segments: vec![ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [0.0, 125.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Unsupported spline sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
        ]))
        .err()
        .expect("Sweep must reject a spline profile before exact evaluation");
    assert_eq!(error, CanonicalError::InvalidSweep);
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn bounded_spline_profile_loft_is_validated_undoable_visible_and_persistent() {
    const DEFINITION: DefinitionId = DefinitionId(721);
    const LOWER: FeatureId = FeatureId(722);
    const UPPER: FeatureId = FeatureId(723);
    const LOFT: FeatureId = FeatureId(724);
    const OCCURRENCE: OccurrenceId = OccurrenceId(725);

    let lower_points = vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]];
    let upper_points = vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]];
    let mut document = DocumentStore::new();
    let empty = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Loft definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Lower spline".to_owned(),
                kind: FeatureKind::closed_spline(&lower_points),
            },
            CanonicalCommand::CreateFeature {
                id: UPPER,
                definition_id: DEFINITION,
                name: "Upper spline".to_owned(),
                kind: FeatureKind::closed_spline(&upper_points),
            },
            CanonicalCommand::CreateFeature {
                id: LOFT,
                definition_id: DEFINITION,
                name: "Bounded loft".to_owned(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: LOWER,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: UPPER,
                            elevation_mm: 80.0,
                        },
                    ],
                    guide: None,
                    continuity: LoftContinuity::Position,
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OCCURRENCE,
                definition_id: DEFINITION,
                name: "Loft occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();

    let lofted = document.current().canonical_digest();
    assert_ne!(lofted, empty);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty);
    assert_eq!(document.redo().unwrap().canonical_digest(), lofted);
    let state = encode_semantic_state(&document.current());
    assert!(
        state
            .complete()
            .contains("features.722.kind.Profile.segments.0.Spline.")
    );
    assert!(state.complete().contains("features.724.kind.Loft."));
    assert!(
        state
            .complete()
            .contains("features.724.kind.Loft.sections.1.profile=723")
    );
    assert!(state.agent().contains("{Spline:"));
    assert!(state.agent().contains("kind:{Loft:"));

    document.make_unique(OCCURRENCE, "Unique loft").unwrap();
    let unique = document.current();
    let unique_definition = unique.occurrence(OCCURRENCE).unwrap().definition_id();
    let [unique_lower, unique_upper, unique_loft] =
        unique.definition(unique_definition).unwrap().feature_ids()
    else {
        panic!("Loft Make Unique must preserve the three-feature chain");
    };
    assert!(matches!(
        unique.feature(*unique_lower).unwrap().kind(),
        kind if kind.closed_spline_points() == Some(lower_points.as_slice())
    ));
    assert!(matches!(
        unique.feature(*unique_upper).unwrap().kind(),
        kind if kind.closed_spline_points() == Some(upper_points.as_slice())
    ));
    assert!(matches!(
        unique.feature(*unique_loft).unwrap().kind(),
        FeatureKind::Loft { sections, .. }
            if sections[0].profile == *unique_lower
                && sections[0].elevation_mm == 0.0
                && sections[1].profile == *unique_upper
                && sections[1].elevation_mm == 80.0
    ));

    let unique_digest = unique.canonical_digest();
    let reopened = persistence::load(&persistence::save(&unique)).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), unique_digest);
    assert!(reopened.migration_losses().is_empty());
    assert!(matches!(
        reopened.snapshot().feature(*unique_loft).unwrap().kind(),
        FeatureKind::Loft { sections, .. }
            if sections[0].profile == *unique_lower
                && sections[1].profile == *unique_upper
    ));

    let before = document.current().canonical_digest();
    let undo_steps = document.visible_undo_steps();
    let error = document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: FeatureId(726),
            definition_id: unique_definition,
            name: "Invalid unordered loft".to_owned(),
            kind: FeatureKind::Loft {
                sections: vec![
                    LoftSection {
                        profile: *unique_lower,
                        elevation_mm: 80.0,
                    },
                    LoftSection {
                        profile: *unique_upper,
                        elevation_mm: 0.0,
                    },
                ],
                guide: None,
                continuity: LoftContinuity::Position,
            },
        }]))
        .err()
        .expect("unordered Loft sections must reject atomically");
    assert_eq!(error, CanonicalError::InvalidLoft);
    assert_eq!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), undo_steps);

    let mut invalid_spline = DocumentStore::new();
    let empty = invalid_spline.current().canonical_digest();
    let error = invalid_spline
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Invalid spline definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: LOWER,
                definition_id: DEFINITION,
                name: "Underspecified spline".to_owned(),
                kind: FeatureKind::closed_spline(&[[0.0, 0.0], [10.0, 0.0], [0.0, 10.0]]),
            },
        ]))
        .err()
        .expect("underspecified spline profile must reject atomically");
    assert_eq!(error, CanonicalError::InvalidProfile);
    assert_eq!(invalid_spline.current().canonical_digest(), empty);
    assert_eq!(invalid_spline.visible_undo_steps(), 0);
}

#[test]
fn separate_occurrence_solid_tool_defers_geometry_validity_to_exact_worker() {
    let mut document = seed_separate_solid_tool_document(100.0, 0.0, 80.0);
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::ApplySolidTool(
            separate_solid_tool_plan(BooleanOperation::Union, true),
        )]))
        .unwrap();

    assert_ne!(document.current().canonical_digest(), before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(
        ExactBRepGraph::from_snapshot(&document.current(), DefinitionId(401), FeatureId(408),)
            .is_ok()
    );
    assert_eq!(
        document
            .current()
            .occurrence(OccurrenceId(301))
            .unwrap()
            .definition_id(),
        DefinitionId(401)
    );
    assert!(document.current().occurrence(OccurrenceId(302)).is_some());
}

#[test]
fn segment_profile_is_canonical_undoable_persistent_and_exact_for_circle() {
    let mut document = DocumentStore::new();
    let before = document.current().canonical_digest();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(501),
                name: "Circle definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(502),
                definition_id: DefinitionId(501),
                name: "Exact circle".to_owned(),
                kind: FeatureKind::Profile {
                    segments: circular_profile_segments(false),
                    closed: true,
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(503),
                definition_id: DefinitionId(501),
                name: "Pending exact cylinder".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(502), height("25")),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(504),
                definition_id: DefinitionId(501),
                name: "Circle occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let applied = document.current().canonical_digest();
    assert_ne!(applied, before);
    assert_eq!(document.visible_undo_steps(), 1);
    assert!(matches!(
        document.current().feature(FeatureId(502)).unwrap().kind(),
        FeatureKind::Profile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));
    let exact_graph =
        producer_exact_graph(&document.current(), DefinitionId(501), FeatureId(503)).unwrap();
    assert_eq!(
        exact_graph.producer_bounds_mm().unwrap(),
        Some([[-10.0, -10.0, 0.0], [10.0, 10.0, 25.0]])
    );
    assert_eq!(
        exact_graph.profiles[0].geometry,
        ExactBRepPlanarGeometry::Circle {
            center_bits: [0.0_f64.to_bits(); 2],
            radius_bits: 10.0_f64.to_bits(),
        }
    );

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), applied);
    assert!(matches!(
        reopened.snapshot().feature(FeatureId(502)).unwrap().kind(),
        FeatureKind::Profile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));
    assert_eq!(document.undo().unwrap().canonical_digest(), before);
    assert_eq!(document.redo().unwrap().canonical_digest(), applied);

    document
        .make_unique(OccurrenceId(504), "Unique circle")
        .unwrap();
    let unique_snapshot = document.current();
    let unique_definition_id = unique_snapshot
        .occurrence(OccurrenceId(504))
        .unwrap()
        .definition_id();
    assert_ne!(unique_definition_id, DefinitionId(501));
    let cloned_segment_profile = unique_snapshot
        .definition(unique_definition_id)
        .unwrap()
        .feature_ids()
        .iter()
        .filter_map(|feature_id| unique_snapshot.feature(*feature_id))
        .find(|feature| matches!(feature.kind(), FeatureKind::Profile { .. }))
        .expect("make unique must clone the segment-authoritative profile");
    assert!(matches!(
        cloned_segment_profile.kind(),
        FeatureKind::Profile { segments, closed: true }
            if segments == &circular_profile_segments(false)
    ));

    let mut opposite = DocumentStore::new();
    opposite
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(501),
                name: "Circle definition".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(502),
                definition_id: DefinitionId(501),
                name: "Exact circle".to_owned(),
                kind: FeatureKind::Profile {
                    segments: circular_profile_segments(true),
                    closed: true,
                },
            },
        ]))
        .unwrap();
    assert_ne!(
        opposite.current().canonical_digest(),
        reopened.snapshot().canonical_digest(),
        "arc direction is authoritative canonical geometry"
    );
}

#[test]
fn segment_profile_rejects_discontinuity_and_radius_mismatch_atomically() {
    for segments in [
        vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [6.0, 0.0],
                end_mm: [0.0, 0.0],
            },
        ],
        vec![
            ProfileSegment::Line {
                start_mm: [0.0, 0.0],
                end_mm: [5.0, 0.0],
            },
            ProfileSegment::Line {
                start_mm: [5.0 + 1.0e-12, 0.0],
                end_mm: [0.0, 0.0],
            },
        ],
        vec![
            ProfileSegment::CircularArc {
                start_mm: [10.0, 0.0],
                end_mm: [-9.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
            ProfileSegment::CircularArc {
                start_mm: [-9.0, 0.0],
                end_mm: [10.0, 0.0],
                center_mm: [0.0, 0.0],
                clockwise: false,
            },
        ],
    ] {
        let mut document = DocumentStore::new();
        let before = document.current().canonical_digest();
        let error = document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(601),
                    name: "Invalid profile definition".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(602),
                    definition_id: DefinitionId(601),
                    name: "Invalid segment profile".to_owned(),
                    kind: FeatureKind::Profile {
                        segments,
                        closed: true,
                    },
                },
            ]))
            .err()
            .expect("invalid segment profile must fail");
        assert_eq!(error, CanonicalError::InvalidProfile);
        assert_eq!(document.current().canonical_digest(), before);
        assert_eq!(document.visible_undo_steps(), 0);
    }
}

#[test]
fn v11_cubic_sweep_is_canonical_visible_persistent_and_fail_closed() {
    const DEFINITION: DefinitionId = DefinitionId(800);
    const PROFILE: FeatureId = FeatureId(801);
    const PATH: FeatureId = FeatureId(802);
    const SWEEP: FeatureId = FeatureId(803);

    let path_segments = vec![
        ProfileSegment::Line {
            start_mm: [0.0, 0.0],
            end_mm: [25.0, 0.0],
        },
        ProfileSegment::CubicBezier {
            start_mm: [25.0, 0.0],
            control_1_mm: [35.0, 0.0],
            control_2_mm: [45.0, 10.0],
            end_mm: [45.0, 20.0],
        },
        ProfileSegment::Line {
            start_mm: [45.0, 20.0],
            end_mm: [45.0, 45.0],
        },
    ];
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "V11 cubic sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Small rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Line-cubic-line C1 path".to_owned(),
                kind: FeatureKind::Profile {
                    segments,
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "V11 sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
        ])
    };

    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document
        .apply_batch(&commands(path_segments.clone()))
        .unwrap();
    let swept_digest = document.current().canonical_digest();
    assert_ne!(swept_digest, empty_digest);
    assert_eq!(document.visible_undo_steps(), 1);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), swept_digest);
    assert!(
        encode_semantic_state(&document.current())
            .complete()
            .contains("features.802.kind.Profile.segments.1.CubicBezier.")
    );

    let bytes = persistence::save(&document.current());
    assert_eq!(
        u16::from_le_bytes([bytes[10], bytes[11]]),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), swept_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::Profile { segments, closed: false } if segments == &path_segments
    ));

    let mut invalid_segments = path_segments;
    invalid_segments[1] = ProfileSegment::CubicBezier {
        start_mm: [25.0, 0.0],
        control_1_mm: [25.0, 0.0],
        control_2_mm: [45.0, 10.0],
        end_mm: [45.0, 20.0],
    };
    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    assert!(matches!(
        invalid.apply_batch(&commands(invalid_segments)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let adjacent_self_intersection = vec![
        ProfileSegment::CubicBezier {
            start_mm: [0.0, 0.0],
            control_1_mm: [10.0, -5.0],
            control_2_mm: [9.0, 10.0],
            end_mm: [10.0, 10.0],
        },
        ProfileSegment::CubicBezier {
            start_mm: [10.0, 10.0],
            control_1_mm: [11.0, 10.0],
            control_2_mm: [-10.0, -20.0],
            end_mm: [20.0, 0.0],
        },
    ];
    assert!(matches!(
        invalid.apply_batch(&commands(adjacent_self_intersection)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn spatial_sweep_path_is_canonical_visible_persistent_and_bounded() {
    const DEFINITION: DefinitionId = DefinitionId(900);
    const PATH: FeatureId = FeatureId(902);
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Spatial sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Spatial path".to_owned(),
                kind: FeatureKind::SpatialPath { segments },
            },
        ])
    };
    let path = vec![SpatialPathSegment::Line {
        start_mm: [2.0, 3.0, 4.0],
        end_mm: [2.0, 3.0, 24.0],
    }];
    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document.apply_batch(&commands(path.clone())).unwrap();
    let digest = document.current().canonical_digest();
    assert_ne!(digest, empty_digest);
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), digest);
    assert!(
        encode_semantic_state(&document.current())
            .complete()
            .contains("features.902.kind.SpatialPath.segments.0.Line.")
    );

    let bytes = persistence::save(&document.current());
    assert_eq!(
        u16::from_le_bytes([bytes[10], bytes[11]]),
        persistence::CURRENT_SCHEMA
    );
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.source_schema(), persistence::CURRENT_SCHEMA);
    assert_eq!(reopened.snapshot().canonical_digest(), digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments == &path
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let segments = (0..65)
        .map(|index| SpatialPathSegment::Line {
            start_mm: [f64::from(index), 0.0, 0.0],
            end_mm: [f64::from(index + 1), 0.0, 0.0],
        })
        .collect();
    assert!(matches!(
        invalid.apply_batch(&commands(segments)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn v12_spatial_sweep_is_canonical_persistent_and_rejects_uncompilable_bounds() {
    const DEFINITION: DefinitionId = DefinitionId(910);
    const PROFILE: FeatureId = FeatureId(911);
    const PATH: FeatureId = FeatureId(912);
    const SWEEP: FeatureId = FeatureId(913);
    let commands = |segments| {
        CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "V12 spatial sweep".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Small rectangle".to_owned(),
                kind: FeatureKind::polygon(&[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: PATH,
                definition_id: DEFINITION,
                name: "Spatial path".to_owned(),
                kind: FeatureKind::SpatialPath { segments },
            },
            CanonicalCommand::CreateFeature {
                id: SWEEP,
                definition_id: DEFINITION,
                name: "Spatial sweep".to_owned(),
                kind: FeatureKind::sweep(PROFILE, PATH),
            },
        ])
    };
    let path = vec![
        SpatialPathSegment::Line {
            start_mm: [0.0, 0.0, 0.0],
            end_mm: [10.0, 0.0, 0.0],
        },
        SpatialPathSegment::CircularArc {
            start_mm: [10.0, 0.0, 0.0],
            end_mm: [20.0, 10.0, 0.0],
            center_mm: [10.0, 10.0, 0.0],
            normal: [0.0, 0.0, 1.0],
            clockwise: false,
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [20.0, 10.0, 0.0],
            control_1_mm: [20.0, 15.0, 0.0],
            control_2_mm: [20.0, 20.0, 5.0],
            end_mm: [20.0, 20.0, 10.0],
        },
    ];

    let mut document = DocumentStore::new();
    let empty_digest = document.current().canonical_digest();
    document.apply_batch(&commands(path.clone())).unwrap();
    let sweep_digest = document.current().canonical_digest();
    assert_eq!(document.undo().unwrap().canonical_digest(), empty_digest);
    assert_eq!(document.redo().unwrap().canonical_digest(), sweep_digest);

    let bytes = persistence::save(&document.current());
    let reopened = persistence::load(&bytes).unwrap();
    assert_eq!(reopened.snapshot().canonical_digest(), sweep_digest);
    assert_eq!(persistence::save(&reopened.snapshot()), bytes);
    assert!(matches!(
        reopened.snapshot().feature(PATH).unwrap().kind(),
        FeatureKind::SpatialPath { segments } if segments == &path
    ));
    assert!(matches!(
        reopened.snapshot().feature(SWEEP).unwrap().kind(),
        FeatureKind::Sweep {
            profile: PROFILE,
            path: PATH,
            ..
        }
    ));

    let mut invalid = DocumentStore::new();
    let before = invalid.current().canonical_digest();
    let outside_compilable_envelope = vec![SpatialPathSegment::Line {
        start_mm: [999_999.5, 0.0, 0.0],
        end_mm: [999_999.5, 0.0, 10.0],
    }];
    assert!(matches!(
        invalid.apply_batch(&commands(outside_compilable_envelope)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);

    let adjacent_self_intersection = vec![
        SpatialPathSegment::CubicBezier {
            start_mm: [0.0, 0.0, 0.0],
            control_1_mm: [10.0, -5.0, 0.0],
            control_2_mm: [9.0, 10.0, 0.0],
            end_mm: [10.0, 10.0, 0.0],
        },
        SpatialPathSegment::CubicBezier {
            start_mm: [10.0, 10.0, 0.0],
            control_1_mm: [11.0, 10.0, 0.0],
            control_2_mm: [-10.0, -20.0, 0.0],
            end_mm: [20.0, 0.0, 0.0],
        },
    ];
    assert!(matches!(
        invalid.apply_batch(&commands(adjacent_self_intersection)),
        Err(CanonicalError::InvalidSweep)
    ));
    assert_eq!(invalid.current().canonical_digest(), before);
    assert_eq!(invalid.visible_undo_steps(), 0);
}

#[test]
fn occurrence_color_is_persisted_atomic_and_geometry_independent() {
    let mut document = seed_product_document();
    let before = document.current();
    let color = Some([0, 128, 255]);
    let batch = CommandBatch::new(vec![CanonicalCommand::SetOccurrenceColor {
        id: FIRST,
        color,
    }]);
    assert_ne!(
        batch.digest(),
        CommandBatch::new(vec![CanonicalCommand::SetOccurrenceColor {
            id: FIRST,
            color: None
        }])
        .digest()
    );
    let revision = document.apply_batch(&batch).unwrap();
    assert!(revision.dirty_features().is_empty());
    let colored = document.current();
    assert_ne!(colored.canonical_digest(), before.canonical_digest());
    assert_eq!(colored.occurrence(FIRST).unwrap().color(), color);
    assert_eq!(colored.occurrence(SECOND).unwrap().color(), None);
    assert_eq!(
        colored.features().collect::<Vec<_>>(),
        before.features().collect::<Vec<_>>()
    );
    let bytes = persistence::save(&colored);
    assert_eq!(
        u16::from_le_bytes(bytes[10..12].try_into().unwrap()),
        persistence::CURRENT_SCHEMA
    );
    let loaded = persistence::load(&bytes).unwrap();
    assert_eq!(
        loaded.document().canonical_digest(),
        colored.canonical_digest()
    );
    document.undo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    document.redo().unwrap();
    assert_eq!(
        document.current().canonical_digest(),
        colored.canonical_digest()
    );
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceColor {
                    id: FIRST,
                    color: None
                },
                CanonicalCommand::SetOccurrenceColor {
                    id: OccurrenceId(999),
                    color
                },
            ]))
            .is_err()
    );
    assert_eq!(
        document.current().canonical_digest(),
        colored.canonical_digest()
    );
    assert!(
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceColor {
                    id: FIRST,
                    color: None
                }
            ]))
            .unwrap()
            .dirty_features()
            .is_empty()
    );
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
}

#[test]
fn component_conversion_and_unique_preserve_colors_and_resolve_root_override() {
    let mut document = seed_product_document();
    let color = Some([21, 42, 63]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor { id: FIRST, color },
        ]))
        .unwrap();
    let conversion = document
        .convert_group_to_component(GROUP, "Colored component")
        .unwrap();
    let root = conversion.component_occurrence_id;
    let child_color = |snapshot: &Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .find(|o| o.occurrence_id == root && !o.instance_path.steps().is_empty())
            .unwrap()
            .color()
    };
    assert_eq!(child_color(&document.current()), color);
    document
        .make_unique(root, "Unique colored component")
        .unwrap();
    assert_eq!(child_color(&document.current()), color);
    let override_color = Some([255, 0, 0]);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: root,
                color: override_color,
            },
        ]))
        .unwrap();
    assert_eq!(child_color(&document.current()), override_color);
    let loaded = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(child_color(&loaded.document()), override_color);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceColor {
                id: root,
                color: None,
            },
        ]))
        .unwrap();
    assert_eq!(child_color(&document.current()), color);
}

#[test]
fn a_spatial_sweep_up_persists_and_enters_the_digest() {
    let document_with = |up| {
        let mut document = DocumentStore::new();
        document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(1),
                    name: "Sweep".to_owned(),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(2),
                    definition_id: DefinitionId(1),
                    name: "Profile".to_owned(),
                    kind: FeatureKind::polygon(&[[0.0, -1.0], [1.0, 0.0], [0.0, 1.0]]),
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(3),
                    definition_id: DefinitionId(1),
                    name: "Path".to_owned(),
                    kind: FeatureKind::SpatialPath {
                        segments: vec![SpatialPathSegment::Line {
                            start_mm: [0.0, 0.0, 0.0],
                            end_mm: [10.0, 0.0, 0.0],
                        }],
                    },
                },
                CanonicalCommand::CreateFeature {
                    id: FeatureId(4),
                    definition_id: DefinitionId(1),
                    name: "Sweep".to_owned(),
                    kind: FeatureKind::Sweep {
                        profile: FeatureId(2),
                        path: FeatureId(3),
                        up,
                    },
                },
            ]))
            .map(|_| document)
    };
    let plain = document_with(None).unwrap().current();
    let held = document_with(Some([0.0, 1.0, 1.0])).unwrap().current();
    assert_ne!(plain.canonical_digest(), held.canonical_digest());
    let reopened = persistence::load(&persistence::save(&held)).unwrap();
    assert_eq!(
        reopened.snapshot().canonical_digest(),
        held.canonical_digest()
    );
    assert!(matches!(
        reopened.snapshot().feature(FeatureId(4)).unwrap().kind(),
        FeatureKind::Sweep {
            up: Some([0.0, 1.0, 1.0]),
            ..
        }
    ));
    for up in [[0.0; 3], [f64::NAN, 0.0, 1.0]] {
        assert!(matches!(
            document_with(Some(up)),
            Err(CanonicalError::InvalidSweep)
        ));
    }
}
