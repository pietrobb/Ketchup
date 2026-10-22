use super::*;

/// Review 2026-10-17 MF-6: a part and its mirrored copy are two hands, so the
/// assembly drawing gives each a position of its own with QTY 1, as the
/// fabrication bill of materials does, instead of one position with QTY 2.
#[test]
fn an_assembly_drawing_lists_a_mirrored_copy_as_a_position_of_its_own() {
    const DEFINITION: DefinitionId = DefinitionId(100);
    const PROFILE: FeatureId = FeatureId(100);
    const EXTRUSION: FeatureId = FeatureId(101);
    const LEFT: OccurrenceId = OccurrenceId(100);
    const RIGHT: OccurrenceId = OccurrenceId(101);
    let occurrence = |id, name: &str, transform| CanonicalCommand::CreateOccurrence {
        id,
        definition_id: DEFINITION,
        name: name.into(),
        transform,
        parent: None,
        tags: Default::default(),
        visible: true,
    };
    let mirrored = Transform::from_matrix([
        -1.0, 0.0, 0.0, 100.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ])
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Side".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [30.0, 0.0], [30.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Side body".into(),
                kind: FeatureKind::extrusion(PROFILE, Dimension::from_decimal("10").unwrap()),
            },
            occurrence(LEFT, "Left side", Transform::identity()),
            occurrence(RIGHT, "Right side", mirrored),
            CanonicalCommand::SetOccurrenceGrounded {
                id: LEFT,
                grounded: true,
            },
            CanonicalCommand::SetOccurrenceGrounded {
                id: RIGHT,
                grounded: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let paths = [InstancePath::root(LEFT), InstancePath::root(RIGHT)];
    assert!(
        !snapshot
            .resolve_instance_path(&paths[0])
            .unwrap()
            .mirrored()
    );
    assert!(
        snapshot
            .resolve_instance_path(&paths[1])
            .unwrap()
            .mirrored()
    );

    let exact = ExactResultRegistry::accept(
        &snapshot,
        [exact_box_package(&snapshot, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let drawing_program = program(vec![AssistantCadEditOperation::CreateDrawing {
        name: "Sides".into(),
        instance_paths: paths
            .iter()
            .map(|path| assistant_instance_path(&snapshot, path))
            .collect(),
    }]);
    drop(snapshot);
    let batch = plan(&document, &BTreeSet::new(), &exact, &drawing_program).unwrap();
    document.apply_batch(&batch).unwrap();
    let completed = document.current();
    let sheet = completed.drawing_sheets().next().unwrap();
    let mut positions = sheet
        .bom_balloons()
        .iter()
        .map(|balloon| (balloon.instance_path().clone(), balloon.position()))
        .collect::<Vec<_>>();
    positions.sort();
    assert_eq!(positions, [(paths[0].clone(), 1), (paths[1].clone(), 2)]);

    let exact = ExactResultRegistry::accept(
        &completed,
        [exact_box_package(&completed, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let drawing = project_orthographic_drawing(&completed, &exact, sheet).unwrap();
    assert_eq!(
        drawing
            .layout
            .bom_rows
            .iter()
            .map(|row| (row.position, row.mirrored, row.quantity, row.label.as_str()))
            .collect::<Vec<_>>(),
        [
            (1, false, 1, "1 | Side | QTY 1"),
            (2, true, 1, "2 | Side (mirrored) | QTY 1"),
        ]
    );
    let svg = export_drawing(&completed, &drawing).unwrap();
    let svg = std::str::from_utf8(svg.svg()).unwrap();
    assert!(!svg.contains("QTY 2"), "{svg}");

    // Review 2026-10-21 MF-21: the mirrored copy drawn alone is still the
    // opposite hand; only a part in its own hand is drawn as its definition.
    for (path, expected) in [
        (&paths[0], DrawingSource::Definition(DEFINITION)),
        (
            &paths[1],
            DrawingSource::RigidAssemblyInstances {
                instance_paths: vec![paths[1].clone()],
            },
        ),
    ] {
        let single = program(vec![AssistantCadEditOperation::CreateDrawing {
            name: "Side".into(),
            instance_paths: vec![assistant_instance_path(&completed, path)],
        }]);
        let current = document.current();
        let exact = ExactResultRegistry::accept(
            &current,
            [exact_box_package(&current, DEFINITION, EXTRUSION)],
        )
        .unwrap();
        drop(current);
        let batch = plan(&document, &BTreeSet::new(), &exact, &single).unwrap();
        document.apply_batch(&batch).unwrap();
        let drawn = document.current();
        let sheet = drawn.drawing_sheets().last().unwrap();
        assert_eq!(sheet.source(), &expected);
        if path == &paths[1] {
            let exact = ExactResultRegistry::accept(
                &drawn,
                [exact_box_package(&drawn, DEFINITION, EXTRUSION)],
            )
            .unwrap();
            let drawing = project_orthographic_drawing(&drawn, &exact, sheet).unwrap();
            assert_eq!(
                drawing
                    .layout
                    .bom_rows
                    .iter()
                    .map(|row| (row.mirrored, row.label.as_str()))
                    .collect::<Vec<_>>(),
                [(true, "1 | Side (mirrored) | QTY 1")]
            );
        }
    }
}

/// Review 2026-10-21 MF-21 (capability review run 184): a fresh mirror copy is
/// not grounded, and drawing it alone is still the opposite hand rather than a
/// refusal to draw a part that is not a rigid assembly.
#[test]
fn a_fresh_ungrounded_mirror_copy_drawn_alone_is_the_opposite_hand() {
    const DEFINITION: DefinitionId = DefinitionId(100);
    const PROFILE: FeatureId = FeatureId(100);
    const EXTRUSION: FeatureId = FeatureId(101);
    const LEFT: OccurrenceId = OccurrenceId(100);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Side".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PROFILE,
                definition_id: DEFINITION,
                name: "Profile".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [30.0, 0.0], [30.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: EXTRUSION,
                definition_id: DEFINITION,
                name: "Side body".into(),
                kind: FeatureKind::extrusion(PROFILE, Dimension::from_decimal("10").unwrap()),
            },
            CanonicalCommand::CreateOccurrence {
                id: LEFT,
                definition_id: DEFINITION,
                name: "Left side".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let mirror = program(vec![AssistantCadEditOperation::Mirror {
        selector: explicit(LEFT.0),
        plane_origin_mm: [50.0, 0.0, 0.0],
        plane_normal: [1.0, 0.0, 0.0],
    }]);
    let batch = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &mirror,
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let snapshot = document.current();
    let copy = snapshot
        .occurrences()
        .map(|occurrence| occurrence.id())
        .find(|id| *id != LEFT)
        .unwrap();
    let path = InstancePath::root(copy);
    assert!(snapshot.resolve_instance_path(&path).unwrap().mirrored());
    assert!(!snapshot.occurrence_is_grounded(copy));

    let exact = ExactResultRegistry::accept(
        &snapshot,
        [exact_box_package(&snapshot, DEFINITION, EXTRUSION)],
    )
    .unwrap();
    let single = program(vec![AssistantCadEditOperation::CreateDrawing {
        name: "Side".into(),
        instance_paths: vec![assistant_instance_path(&snapshot, &path)],
    }]);
    drop(snapshot);
    let batch = plan(&document, &BTreeSet::new(), &exact, &single).unwrap();
    document.apply_batch(&batch).unwrap();
    let drawn = document.current();
    let sheet = drawn.drawing_sheets().last().unwrap();
    assert_eq!(
        sheet.source(),
        &DrawingSource::RigidAssemblyInstances {
            instance_paths: vec![path],
        }
    );
    let exact =
        ExactResultRegistry::accept(&drawn, [exact_box_package(&drawn, DEFINITION, EXTRUSION)])
            .unwrap();
    let drawing = project_orthographic_drawing(&drawn, &exact, sheet).unwrap();
    assert_eq!(
        drawing
            .layout
            .bom_rows
            .iter()
            .map(|row| (row.mirrored, row.label.as_str()))
            .collect::<Vec<_>>(),
        [(true, "1 | Side (mirrored) | QTY 1")]
    );
}
