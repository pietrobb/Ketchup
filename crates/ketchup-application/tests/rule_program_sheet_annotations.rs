//! Product contract for sheet-bound Starlark declarations (R1).
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::RuleProgramSource;

fn source(text: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "sheet.star".into(),
        source: text.into(),
        overrides: Default::default(),
    }
}

const PART: &str = "box('bracket', (40, 30, 10))\n";
const SHEET: &str = r#"
drawing_sheet('Detail', source='bracket')
sheet_angular('corner', sheet='Detail', view='front',
    lines=[{'direction': [1, 0], 'through_mm': [20, 0]},
           {'direction': [0, 1], 'through_mm': [0, 5]}],
    arc_radius_page_mm=8, tolerance=0.5)
sheet_datum('base', sheet='Detail', view='front',
    line={'direction': [1, 0], 'through_mm': [20, 0]},
    label='A', offset_page_mm=[0, -8])
sheet_feature_control_frame('flatness', sheet='Detail', view='front',
    line={'direction': [1, 0], 'through_mm': [20, 0]},
    characteristic='flatness', tolerance_mm=0.1, offset_page_mm=[10, -12])
"#;

#[test]
fn r1_sheet_annotations_reconcile_and_survive_undo_and_save() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let program = format!("{PART}{SHEET}");
    session.apply_rule_program(source(&program), false).unwrap();
    let snapshot = session.snapshot();
    assert!(
        snapshot.annotations().is_empty(),
        "sheet annotations are not document marks"
    );
    assert_eq!(snapshot.drawing_sheets().count(), 1);
    let original = snapshot.drawing_sheets().next().unwrap().clone();
    assert_eq!(original.angular_dimensions().len(), 1);
    assert_eq!(original.datum_symbols().len(), 1);
    assert_eq!(original.feature_control_frames().len(), 1);

    // A changed source forces reconciliation rather than the identical-source shortcut.
    session
        .apply_rule_program(source(&format!("{program}\n# apply again\n")), false)
        .unwrap();
    assert_eq!(session.snapshot().drawing_sheets().count(), 1);
    assert_eq!(
        session.snapshot().drawing_sheets().next().unwrap(),
        &original
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("detail.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened.snapshot().drawing_sheets().next().unwrap(),
        &original
    );

    session.apply_rule_program(source(PART), false).unwrap();
    assert_eq!(session.snapshot().drawing_sheets().count(), 0);
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().drawing_sheets().next().unwrap(),
        &original
    );
}

#[test]
fn r1_sheet_annotations_refuse_ambiguous_geometry_without_publishing() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    session.apply_rule_program(source(PART), false).unwrap();
    let ambiguous = format!(
        "{PART}\ndrawing_sheet('Detail', source='bracket')\n\
        sheet_datum('base', sheet='Detail', view='front', \
        line={{'direction': [1, 0]}}, label='A', offset_page_mm=[0, -8])\n"
    );
    let error = session
        .apply_rule_program(source(&ambiguous), false)
        .err()
        .unwrap();
    let rejection = error.rejection();
    let message = rejection.to_string();
    assert!(message.contains("ambiguous"), "{message}");
    assert!(message.contains("through_mm"), "{message}");
    assert_eq!(session.snapshot().drawing_sheets().count(), 0);
    assert_eq!(session.snapshot().occurrences().count(), 1);
}

#[test]
fn r1_sheet_annotations_keep_manual_sheets_and_refuse_name_collisions() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    use ketchup_model::drawing::{DrawingSheet, DrawingSheetId, DrawingSource};
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    session.apply_rule_program(source(PART), false).unwrap();
    let definition = session
        .snapshot()
        .occurrences()
        .next()
        .unwrap()
        .definition_id();
    let manual = DrawingSheet::new(
        DrawingSheetId(50),
        "Manual",
        DrawingSource::Definition(definition),
    )
    .unwrap();
    session
        .apply_rule_commands_with_source(
            CommandBatch::new(vec![CanonicalCommand::CreateDrawingSheet(manual.clone())]),
            source(PART),
        )
        .unwrap();
    let program = format!("{PART}{SHEET}");
    session.apply_rule_program(source(&program), false).unwrap();
    assert_eq!(session.snapshot().drawing_sheets().count(), 2);
    session.apply_rule_program(source(PART), false).unwrap();
    assert_eq!(session.snapshot().drawing_sheets().next().unwrap(), &manual);
    let error = session
        .apply_rule_program(
            source(&format!(
                "{PART}\ndrawing_sheet('Manual', source='bracket')\n"
            )),
            false,
        )
        .err()
        .unwrap();
    assert!(error.to_string().contains("manually created"), "{error}");
    assert_eq!(session.snapshot().drawing_sheets().next().unwrap(), &manual);
}

#[test]
fn r1_sheet_annotations_duplicate_manual_name_never_loses_a_sheet() {
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    use ketchup_model::drawing::{DrawingSheet, DrawingSheetId, DrawingSource};
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let program = format!("{PART}{SHEET}");
    session.apply_rule_program(source(&program), false).unwrap();
    let definition = session
        .snapshot()
        .occurrences()
        .next()
        .unwrap()
        .definition_id();
    let manual = DrawingSheet::new(
        DrawingSheetId(50),
        "Detail",
        DrawingSource::Definition(definition),
    )
    .unwrap();
    session
        .apply_rule_commands_with_source(
            CommandBatch::new(vec![CanonicalCommand::CreateDrawingSheet(manual.clone())]),
            source(&program),
        )
        .unwrap();
    let before = session
        .snapshot()
        .drawing_sheets()
        .cloned()
        .collect::<Vec<_>>();
    let result = session.apply_rule_program(source(&format!("{program}\n# changed\n")), false);
    assert!(
        result.is_err(),
        "ambiguous ownership must be rejected, not delete the manual sheet"
    );
    assert_eq!(
        session
            .snapshot()
            .drawing_sheets()
            .cloned()
            .collect::<Vec<_>>(),
        before
    );
    assert_eq!(session.snapshot().drawing_sheet(manual.id()), Some(&manual));
}

#[test]
fn r1_sheet_annotations_parameter_change_reselects_edges_and_undo_restores_sheet() {
    let _turn = crate::integration_support::file_turn();
    let program = |width: u32| format!("box('bracket', ({width}, 30, 10))\n{SHEET}");
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(source(&program(40)), false)
        .unwrap();
    let before = session.snapshot().drawing_sheets().next().unwrap().clone();
    session
        .apply_rule_program(source(&program(60)), false)
        .unwrap();
    let snapshot = session.snapshot();
    let after = snapshot.drawing_sheets().next().unwrap();
    assert_eq!(after.id(), before.id());
    assert_eq!(after.angular_dimensions().len(), 1);
    assert_eq!(after.datum_symbols().len(), 1);
    session.undo().unwrap();
    assert_eq!(session.snapshot().drawing_sheets().next().unwrap(), &before);
}

#[test]
fn r1_sheet_annotations_declare_radius_and_diameter_on_exact_circles() {
    let _turn = crate::integration_support::file_turn();
    let program = r#"
revolve('pin', axis=[[0, 0], [0, 1]], profile=[['bottom', [0, 0], [6, 0]], ['side', [6, 0], [6, 20]], ['top', [6, 20], [0, 20]], ['axis', [0, 20], [0, 0]]])
drawing_sheet('Pin', source='pin')
sheet_radius('radius', sheet='Pin', view='front',
    circle={'center_mm': [0, 0], 'radius_mm': 6},
    leader_angle_degrees=45, offset_page_mm=8, tolerance=0.1)
sheet_diameter('diameter', sheet='Pin', view='front',
    circle={'center_mm': [0, 0], 'radius_mm': 6},
    leader_angle_degrees=135, offset_page_mm=8, tolerance=0.2)
"#;
    let mut session = DocumentSession::default();
    session.apply_rule_program(source(program), false).unwrap();
    let snapshot = session.snapshot();
    let sheet = snapshot.drawing_sheets().next().unwrap();
    assert_eq!(sheet.circular_dimensions().len(), 2);
    assert!(snapshot.annotations().is_empty());
}

#[test]
fn r1_sheet_annotations_group_equal_holes_and_label_real_cosmetic_threads() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let program = r#"
p = box('plate', (60, 40, 20))
for x in [10, 20, 30, 40]:
    hole(p, 'z+', at=(x, 10), diameter=6, depth=12)
hole(p, 'z+', at=(50, 30), depth=14, thread=('M5', 8), modeled_thread=False)
drawing_sheet('Bores', source='plate', hole_callouts=True)
"#;
    session.apply_rule_program(source(program), false).unwrap();
    let snapshot = session.snapshot();
    let sheet = snapshot.drawing_sheets().next().unwrap();
    let labels = sheet
        .notes()
        .iter()
        .map(|note| note.text_template())
        .collect::<Vec<_>>();
    assert_eq!(labels.len(), 2, "{labels:?}");
    assert!(
        labels.contains(&"4× BORE Ø6 mm; CYLINDRICAL DEPTH 12 mm"),
        "{labels:?}"
    );
    assert!(
        labels.contains(&"M5 x 0.8 THREAD DEPTH 8 mm; BORE Ø4.2 mm"),
        "{labels:?}"
    );
    let original = sheet.clone();
    session
        .apply_rule_program(
            source(&program.replace(
                "diameter=6, depth=12",
                "diameter=6, depth=8 if x == 10 else 12",
            )),
            false,
        )
        .unwrap();
    let changed = session.snapshot();
    let changed_sheet = changed.drawing_sheets().next().unwrap();
    assert_eq!(changed_sheet.notes().len(), 3);
    assert!(
        changed_sheet
            .notes()
            .iter()
            .any(|note| note.text_template() == "3× BORE Ø6 mm; CYLINDRICAL DEPTH 12 mm")
    );
    use ketchup_application::evaluation;
    use ketchup_model::exact_product::ExactResultRegistry;
    let empty = ExactResultRegistry::default();
    let task = evaluation::start_exact_evaluation_scoped(
        changed.clone(),
        &Default::default(),
        &empty,
        &empty,
        evaluation::configured_exact_worker().or_else(|| {
            evaluation::exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        }),
        None,
        || {},
    );
    let products = task.wait(std::time::Duration::from_secs(60)).unwrap();
    let (exact, _, report) =
        evaluation::materialize_exact_products(&changed, &empty, &empty, &task, products).unwrap();
    assert!(report.complete && report.topology_complete);
    use ketchup_model::drawing::{DrawingError, project_orthographic_drawing};
    assert_eq!(
        project_orthographic_drawing(&changed, &exact, &original).unwrap_err(),
        DrawingError::AnnotationSourceLost
    );
    let drawing = project_orthographic_drawing(&changed, &exact, changed_sheet).unwrap();
    let bundle = ketchup_model::drawing_export::export_drawing(&changed, &drawing).unwrap();
    assert!(String::from_utf8_lossy(bundle.svg()).contains("3× BORE"));
    assert!(String::from_utf8_lossy(bundle.dxf()).contains("CYLINDRICAL DEPTH 12"));
    assert!(bundle.pdf().is_ok());
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().drawing_sheets().next().unwrap(),
        &original
    );
}

#[test]
fn r1_sheet_annotations_split_bore_is_refused_instead_of_counted_twice() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(
            source(
                r#"
p = box('plate', (40, 30, 20))
hole(p, 'z+', at=(10, 10), diameter=6, depth=8)
hole(p, 'z+', at=(10, 10), diameter=6, depth=12)
drawing_sheet('Bores', source='plate', hole_callouts=True)
"#,
            ),
            false,
        )
        .err()
        .unwrap();
    assert_eq!(session.snapshot().drawing_sheets().count(), 0);
}

#[test]
fn r1_sheet_annotations_generate_real_exact_hole_callouts_and_update_depth() {
    let _turn = crate::integration_support::file_turn();
    let program = |depth: u32| {
        format!(
            "p = box('plate', (40, 30, 20))\n\
         hole(p, 'z+', at=(10, 10), diameter=6, depth={depth})\n\
         drawing_sheet('Bores', source='plate', hole_callouts=True)\n"
        )
    };
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(source(&program(12)), false)
        .unwrap();
    let original = session.snapshot().drawing_sheets().next().unwrap().clone();
    assert_eq!(original.notes().len(), 1);
    assert_eq!(
        original.notes()[0].text_template(),
        "BORE Ø6 mm; CYLINDRICAL DEPTH 12 mm"
    );
    session
        .apply_rule_program(source(&program(8)), false)
        .unwrap();
    let changed = session.snapshot().drawing_sheets().next().unwrap().clone();
    assert_eq!(
        changed.notes()[0].text_template(),
        "BORE Ø6 mm; CYLINDRICAL DEPTH 8 mm"
    );
    session.undo().unwrap();
    assert_eq!(
        session.snapshot().drawing_sheets().next().unwrap(),
        &original
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bores.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened.snapshot().drawing_sheets().next().unwrap(),
        &original
    );
}
