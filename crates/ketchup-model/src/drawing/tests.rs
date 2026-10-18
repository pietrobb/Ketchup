use super::*;

#[test]
fn view_frames_are_orthonormal_and_invalid_auxiliary_frames_fail_closed() {
    for kind in [
        OrthographicViewKind::Front,
        OrthographicViewKind::Top,
        OrthographicViewKind::Right,
        OrthographicViewKind::Isometric,
    ] {
        let frame = kind.frame();
        assert!(
            (ketchup_geometry::linalg::length_squared(frame.horizontal()) - 1.0).abs() <= 1.0e-12
        );
        assert!(
            (ketchup_geometry::linalg::length_squared(frame.vertical()) - 1.0).abs() <= 1.0e-12
        );
        assert!(
            (ketchup_geometry::linalg::length_squared(frame.direction()) - 1.0).abs() <= 1.0e-12
        );
        assert!(dot(frame.horizontal(), frame.vertical()).abs() <= 1.0e-12);
        assert!(dot(frame.horizontal(), frame.direction()).abs() <= 1.0e-12);
        assert!(dot(frame.vertical(), frame.direction()).abs() <= 1.0e-12);
    }
    assert_eq!(
        OrthographicViewKind::auxiliary([0.0; 3], [0.0, 1.0, 0.0]),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::auxiliary([1.0, 0.0, 0.0], [2.0, 0.0, 0.0]),
        Err(DrawingError::InvalidView)
    );
    assert_eq!(
        OrthographicViewKind::auxiliary([f64::NAN, 0.0, 0.0], [0.0, 1.0, 0.0]),
        Err(DrawingError::InvalidView)
    );
}

#[test]
fn typed_angular_radius_and_diameter_dimensions_layout_export_and_tamper_fail_closed() {
    let view_kind = OrthographicViewKind::Front;
    let horizontal_id = "sheet-9/view-front/definition-1:edge:1".to_owned();
    let vertical_id = "sheet-9/view-front/definition-1:edge:2".to_owned();
    let circle_id = "sheet-9/view-front/definition-1:circle:7".to_owned();
    assert_eq!(
        DrawingCircularDimension::new(
            DrawingDimensionId(1),
            view_kind,
            circle_id.clone(),
            DrawingCircularDimensionKind::Radius,
            360.0,
            8.0,
            DrawingDimensionTolerance::None,
        ),
        Err(DrawingError::InvalidDimension)
    );
    assert_eq!(
        DrawingAngularDimension::new(
            DrawingDimensionId(1),
            view_kind,
            [horizontal_id.clone(), vertical_id.clone()],
            12.0,
            DrawingDimensionTolerance::symmetric(180.0).unwrap(),
        ),
        Err(DrawingError::InvalidDimension)
    );
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        DrawingSheetId(9),
        "Typed dimensions",
        DrawingSource::Definition(DefinitionId(1)),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Typed dimensions", "TD-9", "A", "Kečup").unwrap(),
        vec![view_kind],
        DrawingAnnotations::with_typed_dimensions(
            Vec::new(),
            vec![
                DrawingAngularDimension::new(
                    DrawingDimensionId(1),
                    view_kind,
                    [horizontal_id.clone(), vertical_id.clone()],
                    12.0,
                    DrawingDimensionTolerance::symmetric(0.5).unwrap(),
                )
                .unwrap(),
            ],
            vec![
                DrawingCircularDimension::new(
                    DrawingDimensionId(2),
                    view_kind,
                    circle_id.clone(),
                    DrawingCircularDimensionKind::Radius,
                    45.0,
                    8.0,
                    DrawingDimensionTolerance::None,
                )
                .unwrap(),
                DrawingCircularDimension::new(
                    DrawingDimensionId(3),
                    view_kind,
                    circle_id.clone(),
                    DrawingCircularDimensionKind::Diameter,
                    135.0,
                    8.0,
                    DrawingDimensionTolerance::bilateral(0.2, 0.1).unwrap(),
                )
                .unwrap(),
            ],
            Vec::new(),
        ),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".into(),
            },
            CanonicalCommand::CreateDrawingSheet(sheet),
        ]))
        .unwrap();
    let snapshot = document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(9)).unwrap();
    let views = vec![OrthographicView {
        kind: view_kind,
        stable_view_id: "sheet-9/view-front".into(),
        bounds_mm: [[-15.0, -15.0], [15.0, 15.0]],
        visible_lines: vec![
            ProjectedVisibleLine {
                stable_line_id: horizontal_id,
                start_mm: [-10.0, 0.0],
                end_mm: [10.0, 0.0],
                foreshortened: false,
            },
            ProjectedVisibleLine {
                stable_line_id: vertical_id,
                start_mm: [0.0, -10.0],
                end_mm: [0.0, 10.0],
                foreshortened: false,
            },
        ],
        hidden_lines: Vec::new(),
        circles: vec![ProjectedCircle {
            stable_circle_id: circle_id,
            center_mm: [0.0, 0.0],
            radius_mm: 5.0,
            start_rad: 0.0,
            span_rad: std::f64::consts::TAU,
        }],
        polyline_curves: Vec::new(),
    }];
    let mut parallel_views = views.clone();
    parallel_views[0].visible_lines[1].end_mm = [10.0, -10.0];
    assert_eq!(
        layout_drawing_sheet(&snapshot, sheet, &parallel_views),
        Err(DrawingError::InvalidGeometry)
    );
    let mut missing_circle_views = views.clone();
    missing_circle_views[0].circles.clear();
    assert_eq!(
        layout_drawing_sheet(&snapshot, sheet, &missing_circle_views),
        Err(DrawingError::DimensionSourceLost)
    );
    let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
    assert_eq!(layout.angular_dimensions[0].value_degrees, 90.0);
    assert_eq!(layout.angular_dimensions[0].label, "90 ±0.5°");
    assert_eq!(layout.circular_dimensions[0].label, "R5 mm");
    assert_eq!(layout.circular_dimensions[1].label, "⌀10 +0.2/-0.1 mm");
    assert_eq!(
        sheet.angular_dimensions()[0].unit(),
        DrawingDimensionUnit::Degrees
    );
    assert_eq!(
        sheet.circular_dimensions()[0].unit(),
        DrawingDimensionUnit::Millimetres
    );
    let stable_source_identity = "definition-1".to_owned();
    let drawing = OrthographicDrawing {
        schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        result_digest: drawing_result_digest(&stable_source_identity, &views),
        stable_source_identity,
        views,
        layout,
    };
    let exported = crate::drawing_export::export_drawing(&snapshot, &drawing).unwrap();
    let svg = std::str::from_utf8(exported.svg()).unwrap();
    assert!(svg.contains("90 ±0.5°"));
    assert!(svg.contains("R5 mm"));
    assert!(svg.contains("⌀10 +0.2/-0.1 mm"));
    let mut tampered = drawing;
    tampered.layout.angular_dimensions[0].value_degrees = 91.0;
    assert_eq!(
        crate::drawing_export::export_drawing(&snapshot, &tampered),
        Err(crate::drawing_export::DrawingExportError::InvalidDrawing)
    );
}

#[test]
fn typed_gdt_datums_layout_export_and_tamper_fail_closed() {
    let view = OrthographicViewKind::Front;
    let horizontal_id = "sheet-12/view-front/definition-1:edge:1".to_owned();
    let vertical_id = "sheet-12/view-front/definition-1:edge:2".to_owned();
    let datum_a = DrawingDatumReference::new("A", DrawingMaterialCondition::None).unwrap();
    assert_eq!(
        DrawingDatumReference::new("a", DrawingMaterialCondition::None),
        Err(DrawingError::InvalidAnnotation)
    );
    assert_eq!(
        DrawingFeatureControlFrame::new(
            DrawingFeatureControlFrameId(1),
            view,
            horizontal_id.clone(),
            DrawingGeometricCharacteristic::Flatness,
            0.1,
            false,
            DrawingMaterialCondition::None,
            vec![datum_a.clone()],
            [20.0, 20.0],
        ),
        Err(DrawingError::InvalidAnnotation)
    );
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        DrawingSheetId(12),
        "GD&T sheet",
        DrawingSource::Definition(DefinitionId(1)),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("GD&T sheet", "GDT-12", "A", "Kečup").unwrap(),
        vec![view],
        DrawingAnnotations::with_manufacturing_annotations(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![
                DrawingDatumSymbol::new(
                    DrawingDatumId(1),
                    view,
                    horizontal_id.clone(),
                    "A",
                    [25.0, 20.0],
                )
                .unwrap(),
                DrawingDatumSymbol::new(
                    DrawingDatumId(2),
                    view,
                    vertical_id.clone(),
                    "B",
                    [-25.0, 20.0],
                )
                .unwrap(),
            ],
            vec![
                DrawingFeatureControlFrame::new(
                    DrawingFeatureControlFrameId(1),
                    view,
                    horizontal_id.clone(),
                    DrawingGeometricCharacteristic::Position,
                    0.1,
                    true,
                    DrawingMaterialCondition::MaximumMaterial,
                    vec![
                        datum_a,
                        DrawingDatumReference::new("B", DrawingMaterialCondition::MaximumMaterial)
                            .unwrap(),
                    ],
                    [35.0, 35.0],
                )
                .unwrap(),
            ],
            Vec::new(),
        ),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".into(),
            },
            CanonicalCommand::CreateDrawingSheet(sheet),
        ]))
        .unwrap();
    let snapshot = document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(12)).unwrap();
    let views = vec![OrthographicView {
        kind: view,
        stable_view_id: "sheet-12/view-front".into(),
        bounds_mm: [[-15.0, -15.0], [15.0, 15.0]],
        visible_lines: vec![
            ProjectedVisibleLine {
                stable_line_id: horizontal_id,
                start_mm: [-10.0, 0.0],
                end_mm: [10.0, 0.0],
                foreshortened: false,
            },
            ProjectedVisibleLine {
                stable_line_id: vertical_id,
                start_mm: [0.0, -10.0],
                end_mm: [0.0, 10.0],
                foreshortened: false,
            },
        ],
        hidden_lines: Vec::new(),
        circles: Vec::new(),
        polyline_curves: Vec::new(),
    }];
    let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
    assert_eq!(layout.datum_symbols[0].label, "A");
    assert_eq!(
        layout.feature_control_frames[0].label,
        "POSITION | DIA 0.1 MMC | A | B MMC"
    );
    let stable_source_identity = "definition-1".to_owned();
    let drawing = OrthographicDrawing {
        schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        result_digest: drawing_result_digest(&stable_source_identity, &views),
        stable_source_identity,
        views: views.clone(),
        layout,
    };
    let exported = crate::drawing_export::export_drawing(&snapshot, &drawing).unwrap();
    let svg = std::str::from_utf8(exported.svg()).unwrap();
    assert!(svg.contains("sheet-12/datum-1/triangle-0"));
    assert!(svg.contains("POSITION | DIA 0.1 MMC | A | B MMC"));

    let mut missing_source_views = views;
    missing_source_views[0].visible_lines.clear();
    assert_eq!(
        layout_drawing_sheet(&snapshot, sheet, &missing_source_views),
        Err(DrawingError::AnnotationSourceLost)
    );
    let mut tampered = drawing;
    tampered.layout.feature_control_frames[0].tolerance_mm = 0.2;
    assert_eq!(
        crate::drawing_export::export_drawing(&snapshot, &tampered),
        Err(crate::drawing_export::DrawingExportError::InvalidDrawing)
    );
}

#[test]
fn bom_balloon_flips_preferred_offset_inward_at_view_cell_boundary() {
    let path = InstancePath::root(OccurrenceId(1));
    let balloon = DrawingBomBalloon::new(
        DrawingBomBalloonId(1),
        OrthographicViewKind::Front,
        path.clone(),
        1,
        [8.0, 8.0],
    )
    .unwrap();
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        DrawingSheetId(13),
        "Boundary BOM",
        DrawingSource::RigidAssemblyInstances {
            instance_paths: vec![path.clone()],
        },
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Boundary BOM", "BOM-13", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::with_bom_annotations(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            vec![balloon],
            Vec::new(),
        ),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Boundary part".into(),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(1),
                definition_id: DefinitionId(1),
                name: "Boundary instance".into(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let source_line_id = "sheet-13/view-front/instance-1:edge:1".to_owned();
    let views = vec![OrthographicView {
        kind: OrthographicViewKind::Front,
        stable_view_id: "sheet-13/view-front".into(),
        bounds_mm: [[0.0, 0.0], [185.0, 10.0]],
        visible_lines: vec![ProjectedVisibleLine {
            stable_line_id: source_line_id,
            start_mm: [185.0, 0.0],
            end_mm: [185.0, 10.0],
            foreshortened: false,
        }],
        hidden_lines: Vec::new(),
        circles: Vec::new(),
        polyline_curves: Vec::new(),
    }];

    let layout = layout_drawing_sheet(&snapshot, &sheet, &views).unwrap();
    let balloon = &layout.bom_balloons[0];
    assert!(balloon.circle_center_mm[0] < balloon.leader_line_mm[0][0]);
    assert_eq!(sheet.bom_balloons()[0].offset_page_mm(), [8.0, 8.0]);
}

/// Every R12 TEXT entity of `dxf` whose KETCHUP id starts with `prefix`, as
/// (id, x, baseline y, height, escaped value).
fn dxf_texts(dxf: &str, prefix: &str) -> Vec<(String, f64, f64, f64, String)> {
    let lines: Vec<&str> = dxf.lines().collect();
    let mut texts = Vec::new();
    let (mut entity, mut x, mut y, mut height, mut value) = ("", 0.0, 0.0, 0.0, "");
    for pair in lines.chunks(2) {
        let [code, data] = pair else { break };
        match code.trim() {
            "0" => entity = data,
            "10" if entity == "TEXT" => x = data.parse().unwrap(),
            "20" if entity == "TEXT" => y = data.parse().unwrap(),
            "40" if entity == "TEXT" => height = data.parse().unwrap(),
            "1" if entity == "TEXT" => value = data,
            "1000" if entity == "TEXT" && data.starts_with(prefix) => {
                texts.push(((*data).to_owned(), x, y, height, value.to_owned()));
            }
            _ => {}
        }
    }
    texts
}

/// The vertical extent of one R12 text line, from its descenders to its cap.
fn text_span(y: f64, height: f64) -> [f64; 2] {
    [y - 0.25 * height, y + height]
}

/// A part name long enough that its parts-list row needs two R12 strings: the
/// row's second line takes the next row's place in the table, which moves down
/// by one line, in the DXF and in the SVG/PDF alike, instead of being printed
/// over by it.
#[test]
fn a_wrapped_parts_list_row_pushes_the_next_row_down() {
    let long_name = ["Čšťž"; 10].join(" ");
    let paths = [1, 2].map(|id| InstancePath::root(OccurrenceId(id)));
    let balloons = [1, 2].map(|id| {
        DrawingBomBalloon::new(
            DrawingBomBalloonId(id),
            OrthographicViewKind::Front,
            paths[usize::try_from(id - 1).unwrap()].clone(),
            u32::try_from(id).unwrap(),
            [8.0, 8.0],
        )
        .unwrap()
    });
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        DrawingSheetId(14),
        "Long part name",
        DrawingSource::RigidAssemblyInstances {
            instance_paths: paths.to_vec(),
        },
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Long part name", "BOM-14", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::with_bom_annotations(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            balloons.to_vec(),
            Vec::new(),
        ),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    let mut commands = Vec::new();
    for (id, name) in [(1, long_name.as_str()), (2, "Krátky diel")] {
        commands.push(CanonicalCommand::CreateDefinition {
            id: DefinitionId(id),
            name: name.into(),
        });
        commands.push(CanonicalCommand::CreateOccurrence {
            id: OccurrenceId(id),
            definition_id: DefinitionId(id),
            name: format!("Instance {id}"),
            transform: Transform::identity(),
            parent: None,
            tags: Default::default(),
            visible: true,
        });
        commands.push(CanonicalCommand::SetOccurrenceGrounded {
            id: OccurrenceId(id),
            grounded: true,
        });
    }
    commands.push(CanonicalCommand::CreateDrawingSheet(sheet));
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let snapshot = document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(14)).unwrap();
    let views = vec![OrthographicView {
        kind: OrthographicViewKind::Front,
        stable_view_id: "sheet-14/view-front".into(),
        bounds_mm: [[0.0, 0.0], [40.0, 10.0]],
        visible_lines: [1, 2]
            .map(|id| ProjectedVisibleLine {
                stable_line_id: format!("sheet-14/view-front/instance-{id}:edge:1"),
                start_mm: [f64::from(id) * 10.0, 0.0],
                end_mm: [f64::from(id) * 10.0, 10.0],
                foreshortened: false,
            })
            .to_vec(),
        hidden_lines: Vec::new(),
        circles: Vec::new(),
        polyline_curves: Vec::new(),
    }];
    let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
    let stable_source_identity = "assembly".to_owned();
    let drawing = OrthographicDrawing {
        schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        result_digest: drawing_result_digest(&stable_source_identity, &views),
        stable_source_identity,
        views,
        layout,
    };
    let exported = crate::drawing_export::export_drawing(&snapshot, &drawing).unwrap();
    let dxf = std::str::from_utf8(exported.dxf()).unwrap();
    let rows = dxf_texts(dxf, "bom-row-");
    assert_eq!(
        rows.iter().map(|row| row.0.as_str()).collect::<Vec<_>>(),
        ["bom-row-1", "bom-row-1", "bom-row-2"],
        "the long row needs two R12 strings"
    );
    for (index, upper) in rows.iter().enumerate() {
        for lower in &rows[index + 1..] {
            let [upper_bottom, _] = text_span(upper.2, upper.3);
            let [_, lower_top] = text_span(lower.2, lower.3);
            assert!(
                upper.1 != lower.1 || lower_top <= upper_bottom,
                "{} at {} overlaps {} at {}",
                lower.0,
                lower.2,
                upper.0,
                upper.2
            );
        }
    }
    let svg = std::str::from_utf8(exported.svg()).unwrap();
    let row = svg
        .lines()
        .find(|line| line.contains("id=\"bom-row-1\""))
        .unwrap();
    assert_eq!(row.matches("<tspan").count(), 2, "{row}");
}

/// The sheet of `notes` on one front view, laid out and exported.
fn export_notes(
    notes: Vec<DrawingNote>,
) -> Result<crate::drawing_export::DrawingExportBundle, crate::drawing_export::DrawingExportError> {
    let sheet = DrawingSheet::with_contract_views_and_annotations(
        DrawingSheetId(15),
        "Long note",
        DrawingSource::Definition(DefinitionId(1)),
        DrawingPageTemplate::default(),
        DrawingTitleBlock::new("Long note", "N-15", "A", "Kečup").unwrap(),
        vec![OrthographicViewKind::Front],
        DrawingAnnotations::new(Vec::new(), notes),
    )
    .unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Part".into(),
            },
            CanonicalCommand::CreateDrawingSheet(sheet),
        ]))
        .unwrap();
    let snapshot = document.current();
    let sheet = snapshot.drawing_sheet(DrawingSheetId(15)).unwrap();
    let views = vec![OrthographicView {
        kind: OrthographicViewKind::Front,
        stable_view_id: "sheet-15/view-front".into(),
        bounds_mm: [[0.0, 0.0], [10.0, 10.0]],
        visible_lines: vec![ProjectedVisibleLine {
            stable_line_id: "sheet-15/view-front/definition-1:edge:1".into(),
            start_mm: [0.0, 0.0],
            end_mm: [10.0, 0.0],
            foreshortened: false,
        }],
        hidden_lines: Vec::new(),
        circles: Vec::new(),
        polyline_curves: Vec::new(),
    }];
    let layout = layout_drawing_sheet(&snapshot, sheet, &views).unwrap();
    let stable_source_identity = "definition-1".to_owned();
    let drawing = OrthographicDrawing {
        schema: ORTHOGRAPHIC_LINEWORK_SCHEMA_V2,
        sheet_id: sheet.id(),
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        source_digest: snapshot.canonical_digest(),
        result_digest: drawing_result_digest(&stable_source_identity, &views),
        stable_source_identity,
        views,
        layout,
    };
    crate::drawing_export::export_drawing(&snapshot, &drawing)
}

/// The longest note the model accepts (128 × "Č") needs four R12 strings and
/// continues on lines below its position. Where those lines would print over
/// the next note or into the title block the export says so; where there is
/// room they print clear of the other texts, the same in the DXF and the SVG.
#[test]
fn a_wrapped_note_never_prints_over_the_text_below_it() {
    use crate::drawing_export::DrawingExportError;
    let long = "Č".repeat(128);
    let note = |id, position: [f64; 2], text: &str| {
        DrawingNote::new(DrawingNoteId(id), position, text).unwrap()
    };

    let roomy = export_notes(vec![
        note(1, [200.0, 150.0], &long),
        note(2, [200.0, 128.0], "Druhá poznámka"),
    ])
    .unwrap();
    let dxf = std::str::from_utf8(roomy.dxf()).unwrap();
    let texts = dxf_texts(dxf, "sheet-15/note-");
    assert_eq!(texts.len(), 5);
    for (index, upper) in texts.iter().enumerate() {
        for lower in &texts[index + 1..] {
            assert!(
                text_span(lower.2, lower.3)[1] <= text_span(upper.2, upper.3)[0],
                "{} at {} overlaps {} at {}",
                lower.0,
                lower.2,
                upper.0,
                upper.2
            );
        }
    }
    let svg = std::str::from_utf8(roomy.svg()).unwrap();
    let wrapped = svg
        .lines()
        .find(|line| line.contains("id=\"sheet-15/note-1\""))
        .unwrap();
    assert_eq!(wrapped.matches("<tspan").count(), 4, "{wrapped}");

    // One-line notes 6 mm apart are clear of each other; the long one's
    // second line would land 5.25 mm below its first, on the next note.
    assert_eq!(
        export_notes(vec![
            note(1, [200.0, 150.0], &long),
            note(2, [200.0, 144.0], "Druhá poznámka"),
        ])
        .map(|_| ()),
        Err(DrawingExportError::WrappedNoteOverlaps)
    );
    // Placed just above the drawing area's floor, its last line would sit in
    // the title block.
    assert_eq!(
        export_notes(vec![note(1, [200.0, 55.0], &long)]).map(|_| ()),
        Err(DrawingExportError::WrappedNoteOverlaps)
    );
}

#[test]
fn exact_circle_evidence_projects_only_in_axis_normal_views() {
    let edge = ExactBRepGraphEdgeEvidence {
        edge_ordinal: 7,
        curve_kind: "circle".into(),
        length_mm: 31.415_926_535_897_93,
        centroid_mm: [2.0, 3.0, 4.0],
        bounds_mm: [[-3.0, 3.0, -1.0], [7.0, 3.0, 9.0]],
        closed: true,
        circle_radius_mm: Some(5.0),
        axis_origin_mm: Some([2.0, 3.0, 4.0]),
        unit_axis_direction: Some([0.0, -1.0, 0.0]),
        adjacent_face_ordinals: vec![1],
    };
    let transform = Transform::from_translation(10.0, 20.0, 30.0).unwrap();

    assert_eq!(
        project_instance_circles(
            DrawingSheetId(9),
            OrthographicViewKind::Front,
            "instance-4/g5/o6",
            transform,
            std::slice::from_ref(&edge),
        )
        .unwrap(),
        vec![ProjectedCircle {
            stable_circle_id: "sheet-9/view-front/instance-4/g5/o6:circle:7".into(),
            center_mm: [12.0, 34.0],
            radius_mm: 5.0,
            start_rad: 0.0,
            span_rad: std::f64::consts::TAU,
        }]
    );
    assert!(
        project_instance_circles(
            DrawingSheetId(9),
            OrthographicViewKind::Top,
            "instance-4/g5/o6",
            transform,
            &[edge],
        )
        .unwrap()
        .is_empty()
    );
}
