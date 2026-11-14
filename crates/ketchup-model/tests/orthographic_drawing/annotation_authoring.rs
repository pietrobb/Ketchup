use super::*;
use ketchup_model::drawing::annotation::{DrawingAnnotation, annotate_sheet};

#[test]
fn annotations_are_appended_atomically_and_survive_export_undo_and_save() {
    let mut document = seeded_document();
    let exact = graph_registry_with_circle(&document.current(), "annotation-authoring", 5.0);
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        SHEET,
        "Annotated part",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Annotated part", "PART-1", "A", "Designer").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(
            vec![],
            vec![DrawingNote::new(DrawingNoteId(1), [30.0, 60.0], "Keep this note").unwrap()],
        ),
    )
    .unwrap();
    let (create, plain) = prepare_create_drawing_sheet(&document, &exact, sheet).unwrap();
    document.commit_proposal(&create).unwrap();
    let exact = ExactResultRegistry::carried_forward(&document.current(), &exact);
    let lines = &plain.views[0].visible_lines;
    let pair = lines
        .iter()
        .enumerate()
        .find_map(|(index, a)| {
            lines.iter().skip(index + 1).find_map(|b| {
                let da = [a.end_mm[0] - a.start_mm[0], a.end_mm[1] - a.start_mm[1]];
                let db = [b.end_mm[0] - b.start_mm[0], b.end_mm[1] - b.start_mm[1]];
                (ketchup_geometry::linalg::dot2(da, db).abs() < 1e-9)
                    .then(|| [a.stable_line_id.clone(), b.stable_line_id.clone()])
            })
        })
        .unwrap();
    let line = pair[0].clone();
    let circle = plain.views[0].circles[0].stable_circle_id.clone();
    let requests: Vec<DrawingAnnotation> = serde_json::from_value(serde_json::json!([
        {"kind": "angular", "view": "front", "source_line_ids": pair, "arc_radius_page_mm": 8.0},
        {"kind": "radius", "view": "front", "source_circle_id": circle, "leader_angle_degrees": 45.0, "offset_page_mm": 6.0},
        {"kind": "diameter", "view": "front", "source_circle_id": circle, "leader_angle_degrees": 135.0, "offset_page_mm": 6.0, "tolerance": {"kind": "symmetric", "deviation": 0.1}},
        {"kind": "datum", "view": "front", "source_line_id": line, "label": "A", "offset_page_mm": [15.0, 15.0]},
        {"kind": "feature_control_frame", "view": "front", "source_line_id": line,
         "characteristic": "Parallelism", "tolerance_mm": 0.1, "datum_labels": ["A"], "offset_page_mm": [20.0, 20.0]}
    ])).unwrap();
    let snapshot = document.current();
    let original = snapshot.drawing_sheet(SHEET).unwrap();
    let annotated = annotate_sheet(original, &requests).unwrap();
    let (edit, drawing) = prepare_edit_drawing_sheet(&document, &exact, annotated).unwrap();
    assert_eq!(drawing.layout.angular_dimensions[0].label, "90°");
    assert_eq!(drawing.layout.circular_dimensions[0].label, "R5 mm");
    assert_eq!(drawing.layout.circular_dimensions[1].label, "⌀10 ±0.1 mm");
    let candidate = document
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpdateDrawingSheet(annotate_sheet(original, &requests).unwrap()),
        ]))
        .unwrap();
    let exported = export_drawing(&candidate, &drawing).unwrap();
    assert!(
        std::str::from_utf8(exported.svg())
            .unwrap()
            .contains("R5 mm")
    );
    assert!(!exported.pdf().unwrap().is_empty());
    assert!(
        std::str::from_utf8(exported.dxf())
            .unwrap()
            .contains("PARALLELISM | 0.1 | A")
    );
    let undo = document.visible_undo_steps();
    document.commit_proposal(&edit).unwrap();
    assert_eq!(document.visible_undo_steps(), undo + 1);
    let after = document.current().drawing_sheet(SHEET).unwrap().clone();
    assert_eq!(after.notes(), original.notes());
    assert_eq!(after.title_block(), original.title_block());
    assert_eq!(after.datum_symbols()[0].label(), "A");
    assert_eq!(
        after.feature_control_frames()[0].datum_references()[0].label(),
        "A"
    );
    let more = annotate_sheet(&after, &requests[..3]).unwrap();
    assert_eq!(more.angular_dimensions()[1].id(), DrawingDimensionId(4));
    assert_eq!(more.circular_dimensions()[2].id(), DrawingDimensionId(5));
    assert_eq!(more.circular_dimensions()[3].id(), DrawingDimensionId(6));
    document.undo().unwrap();
    assert_eq!(document.current().drawing_sheet(SHEET).unwrap(), original);
    document.redo().unwrap();
    let reopened = persistence::load(&persistence::save(&document.current()))
        .unwrap()
        .into_editable()
        .ok()
        .unwrap();
    assert_eq!(reopened.current().drawing_sheet(SHEET).unwrap(), &after);
}

#[test]
fn invalid_annotation_batches_leave_existing_sheet_untouched() {
    let mut document = seeded_document();
    let exact = graph_registry_with_circle(&document.current(), "annotation-refusal", 5.0);
    let sheet = DrawingSheet::with_contract_and_views(
        SHEET,
        "Part",
        DrawingSource::Definition(DEFINITION),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Part", "", "", "").unwrap(),
        vec![OrthographicViewKind::Front],
    )
    .unwrap();
    let (create, plain) = prepare_create_drawing_sheet(&document, &exact, sheet).unwrap();
    document.commit_proposal(&create).unwrap();
    let exact = ExactResultRegistry::carried_forward(&document.current(), &exact);
    let snapshot = document.current();
    let sheet = snapshot.drawing_sheet(SHEET).unwrap();
    let before = sheet.clone();
    let valid = serde_json::json!({"kind": "radius", "view": "front", "source_circle_id": plain.views[0].circles[0].stable_circle_id,
        "leader_angle_degrees": 45.0, "offset_page_mm": 6.0});
    for patch in [
        serde_json::json!({"view": "missing"}),
        serde_json::json!({"offset_page_mm": -1.0}),
        serde_json::json!({"source_circle_id": format!("sheet-{}/view-front/missing", SHEET.0)}),
    ] {
        let mut bad = valid.clone();
        bad.as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let requests: Vec<DrawingAnnotation> =
            serde_json::from_value(serde_json::json!([valid, bad])).unwrap();
        let result = annotate_sheet(sheet, &requests)
            .map_err(DrawingAuthoringError::from)
            .and_then(|edited| prepare_edit_drawing_sheet(&document, &exact, edited));
        assert!(result.is_err());
        assert_eq!(document.current().drawing_sheet(SHEET).unwrap(), &before);
    }
}
