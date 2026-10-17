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
}
