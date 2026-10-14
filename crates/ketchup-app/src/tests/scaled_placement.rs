//! A document saved before Scale was refused can hold a part whose placement
//! scales it. Whatever reads the part's definition geometry (its drawing
//! sheet, a STEP file, its volume) refuses that part and names the placement,
//! instead of answering for the unscaled part.

use super::*;
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::drawn_shape::tests::evaluate_exact;
use ketchup_model::document::ScaledPlacement;
use ketchup_model::drawing::DrawingSheetId;

const PART: OccurrenceId = OccurrenceId(1);

fn reason() -> String {
    ScaledPlacement {
        occurrence_id: PART,
    }
    .to_string()
}

/// The default part with a drawing sheet of it, then placed scaled by 1.5
/// along x only, saved and opened again as an older document would be.
fn reopened_scaled_document(directory: &Path) -> (KetchupApp, DrawingSheetId) {
    let mut app = KetchupApp::new();
    evaluate_exact(&mut app);
    app.selection.select_occurrence(PART, false);
    assert!(app.preview_selection_drawing(), "{}", app.action_digest());
    assert!(app.confirm_assembly_preview(), "{}", app.action_digest());
    let placement = *app
        .document
        .current()
        .occurrence(PART)
        .unwrap()
        .transform()
        .matrix();
    let mut scaled = placement;
    scaled[0] *= 1.5;
    app.document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetOccurrenceTransform {
                id: PART,
                transform: Transform::from_matrix(scaled).unwrap(),
            },
        ]))
        .unwrap();
    let path = directory.join("scaled.ketchup");
    assert!(app.save_document_to(&path), "{}", app.action_digest());

    let mut reopened = KetchupApp::new().with_dialogs(Box::new(
        dialogs::ScriptedFileDialogs::new().always_confirm_high_risk_as(62),
    ));
    assert!(
        reopened.open_document_from(&path),
        "{}",
        reopened.action_digest()
    );
    evaluate_exact(&mut reopened);
    let sheet = reopened
        .document
        .current()
        .drawing_sheets()
        .next()
        .unwrap()
        .id();
    (reopened, sheet)
}

#[test]
fn a_scaled_part_refuses_its_drawing_sheet_as_dxf_and_dwg_and_writes_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, sheet) = reopened_scaled_document(directory.path());
    let digest = app.canonical_digest();
    for (format, name) in [
        (SheetFileFormat::Dxf, "sheet.dxf"),
        (SheetFileFormat::Dwg, "sheet.dwg"),
    ] {
        let path = directory.path().join(name);
        assert!(!app.export_drawing_sheet_to(sheet, &path, format));
        assert!(
            app.action_digest().contains(&reason()),
            "{}",
            app.action_digest()
        );
        assert!(!path.exists(), "{name} was written");
    }

    // A new drawing of the scaled part is refused the same way.
    app.selection.select_occurrence(PART, false);
    assert!(!app.preview_selection_drawing());
    assert!(
        app.action_digest().contains(&reason()),
        "{}",
        app.action_digest()
    );
    assert_eq!(app.canonical_digest(), digest);
}

#[test]
fn a_scaled_part_shows_no_volume_in_the_dock_or_the_mass_report() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, _) = reopened_scaled_document(directory.path());
    app.selection.select_occurrence(PART, false);

    let measures = app.selection_measures().unwrap();
    assert_eq!(measures.volume_mm3, None);
    assert_eq!(
        app.selection_measure_lines(&measures).last(),
        Some(&format!("Volume: not measured, {}", reason()))
    );
    // The whole line is Slovak in the Slovak interface, the reason too.
    app.set_ui_language(crate::language::UiLanguage::Slovak);
    assert_eq!(
        app.selection_measure_lines(&measures)
            .last()
            .map(String::as_str),
        Some(
            "Objem: nemeria sa, umiestnenie dielu 1 mení jeho mierku alebo ho skosí; \
             diel sa smie len posunúť a otočiť"
        )
    );
    // Never "being computed": the mass report says why there is no mass.
    assert!(matches!(
        app.selection_mass(&measures),
        crate::selection_mass_ui::SelectionMass::Ready(_)
    ));

    let snapshot = app.document.current();
    let report = ketchup_application::mass_properties::mass_report(
        &snapshot,
        app.exact_results_for_snapshot(&snapshot),
        None,
        ketchup_application::validation_rules::ValidationRules::library(),
        None,
    );
    let part = &report["parts"][0];
    assert_eq!(part["occurrence_id"], PART.0, "{report}");
    assert!(part.get("volume_mm3").is_none(), "{report}");
    assert!(part.get("mass_kg").is_none(), "{report}");
    assert_eq!(part["status"], "not_verified");
    assert_eq!(part["reason"], reason());
}

#[test]
fn a_scaled_part_refuses_step_export_and_writes_no_file() {
    let directory = tempfile::tempdir().unwrap();
    let (mut app, _) = reopened_scaled_document(directory.path());
    let path = directory.path().join("model.step");
    let error = app.export_current_model_step_to(&path).unwrap_err();
    assert!(error.to_string().contains(&reason()), "{error}");
    assert!(!path.exists());
    assert!(!path.with_extension("step.loss.txt").exists());
}
