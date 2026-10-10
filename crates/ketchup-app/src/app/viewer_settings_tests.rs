use crate::*;
use egui_kittest::{Harness, kittest::Queryable as _};
use ketchup_model::document::RuleProgramSource;

fn viewport() -> Rect {
    Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))
}

/// Two root instances of one assembly, each with a nested HPL board.
fn program() -> RuleProgramSource {
    RuleProgramSource {
        file_name: "viewer.star".into(),
        source: "a=box(\"board\", (600,18,400), material=\"HPL\")\ng=group(\"inner\",[a])\nc=component(\"cabinet\",[g])\ninstance(\"second\",c,at=(1000,0,0))".into(),
        overrides: BTreeMap::new(),
    }
}

fn install_geometry(app: &mut KetchupApp) {
    let snapshot = app.document.current();
    let definitions = snapshot
        .scene_query()
        .into_iter()
        .map(|item| item.definition_id)
        .collect::<BTreeSet<_>>();
    for definition in definitions {
        for producer in exact_body_terminal_features(&snapshot, definition)
            .expect("terminals")
            .values()
        {
            let package = ketchup_model::testing::box_package(
                &snapshot,
                definition,
                *producer,
                "viewer-notes-fixture",
                &[],
            )
            .expect("geometry fixture");
            assert!(app.headless_install_exact_package(package));
        }
    }
}

fn cabinet() -> KetchupApp {
    let mut app = KetchupApp::new();
    app.apply_program_source(program(), true).expect("program");
    install_geometry(&mut app);
    app
}

fn body_paths(app: &KetchupApp) -> Vec<InstancePath> {
    app.document
        .current()
        .scene_query()
        .into_iter()
        .map(|item| item.instance_path)
        .filter(|path| !path.is_root())
        .collect()
}

fn export(app: &KetchupApp) -> ketchup_manufacturing::viewer_export::ViewerExport {
    let settings = app.stored_viewer_settings().expect("settings");
    app.current_model_viewer_export(&app.viewer_export_scenes(&settings), viewport())
        .expect("export")
}

#[test]
fn notes_tell_definition_from_occurrence_and_survive_save_reopen_and_reexport() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cabinet.ketchup");
    let mut app = cabinet();
    app.save_view("Front").expect("scene");
    let paths = body_paths(&app);
    assert_eq!(paths.len(), 2);
    let board = app
        .document
        .current()
        .resolve_instance_path(&paths[0])
        .unwrap()
        .definition_id;
    assert!(app.save_document_to(&path));
    let cad = |app: &KetchupApp| {
        let snapshot = app.document.current();
        (
            snapshot.canonical_digest(),
            snapshot.revision_id(),
            app.undo_step_count(),
        )
    };
    let before = cad(&app);
    assert!(app.set_viewer_definition_note(board, "ABS edge 2 mm on all sides"));
    assert!(app.set_viewer_occurrence_note(&paths[0], "Next to the window"));
    assert_eq!(before, cad(&app), "notes never change CAD or Undo");
    assert!(app.is_dirty(), "unsaved notes must not be lost on close");

    let check = |app: &KetchupApp| {
        let manifest = export(app).package.manifest;
        let definition = manifest
            .definitions
            .iter()
            .find(|d| d.id == board.0)
            .expect("board definition");
        assert_eq!(
            definition.note.as_deref(),
            Some("ABS edge 2 mm on all sides")
        );
        let notes = manifest
            .occurrences
            .iter()
            .filter(|o| o.definition_id == board.0)
            .map(|o| o.note.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(notes.len(), 2);
        assert!(notes.contains(&Some("Next to the window")));
        assert!(notes.contains(&None), "the other occurrence keeps no note");
        assert!(
            manifest
                .occurrences
                .iter()
                .all(|o| o.definition_id != board.0 || o.material.as_deref() == Some("HPL"))
        );
    };
    check(&app);
    assert!(app.save_document_to(&path));
    assert!(!app.is_dirty());

    let mut reopened = KetchupApp::new();
    assert!(reopened.open_document_path(&path));
    assert_eq!(
        reopened.stored_viewer_settings().unwrap(),
        app.stored_viewer_settings().unwrap()
    );
    reopened.wait_for_program_evaluation();
    install_geometry(&mut reopened);
    check(&reopened);
    assert!(reopened.set_viewer_occurrence_note(&paths[0], "  "));
    assert_eq!(
        reopened
            .stored_viewer_settings()
            .unwrap()
            .occurrence_note(&paths[0]),
        ""
    );
}

#[test]
fn sharing_choices_pick_scenes_start_and_drop_unshared_components() {
    let mut app = cabinet();
    let front = app.save_view("Front").expect("scene");
    let back = app.save_view("Back").expect("scene");
    let scenes =
        |app: &KetchupApp| app.viewer_export_scenes(&app.stored_viewer_settings().unwrap());
    assert_eq!(scenes(&app), [front, back]);
    assert!(app.set_viewer_scene_shared(front, false));
    assert_eq!(scenes(&app), [back]);
    assert!(app.set_viewer_start_scene(back));
    assert!(app.set_viewer_scene_shared(front, true));
    assert_eq!(scenes(&app), [back, front], "opening scene first");
    assert!(app.stored_viewer_settings().unwrap().scenes.is_none());
    let manifest = export(&app).package.manifest;
    assert_eq!(manifest.start_scene, Some(back.0));
    assert_eq!(manifest.scenes.len(), 2);

    let paths = body_paths(&app);
    let excluded = paths[0].root_occurrence();
    assert!(app.set_viewer_component_shared(excluded, false));
    let shared = export(&app);
    assert!(
        shared
            .package
            .manifest
            .occurrences
            .iter()
            .all(|o| o.path.root_occurrence_id != excluded.0),
        "an unshared component is not in the package, not just hidden"
    );
    assert!(shared.loss_report.contains("viewer_excluded_components=1"));
    assert!(app.set_viewer_component_shared(paths[1].root_occurrence(), false));
    let settings = app.stored_viewer_settings().unwrap();
    assert!(
        app.current_model_viewer_export(&app.viewer_export_scenes(&settings), viewport())
            .is_err()
    );
}

#[test]
fn unreadable_stored_notes_are_kept_and_stop_the_export() {
    let mut app = cabinet();
    app.save_view("Front").expect("scene");
    let corrupt = b"{\"definition_notes\": 7".to_vec();
    app.file.container_data.set_extension(
        ketchup_model::persistence::ExtensionEntry::new(
            "org.ketchup.viewer",
            "sharing-v1.json",
            false,
            corrupt.clone(),
        )
        .unwrap(),
    );
    let paths = body_paths(&app);
    assert!(!app.set_viewer_occurrence_note(&paths[0], "lost?"));
    let stored = app
        .file
        .container_data
        .extensions()
        .find(|entry| entry.namespace() == "org.ketchup.viewer")
        .map(|entry| entry.bytes().to_vec());
    assert_eq!(stored, Some(corrupt));
    let directory = tempfile::tempdir().unwrap();
    assert!(!app.export_current_model_viewer_to(&directory.path().join("x.ketchup-view")));
}

#[test]
fn inspector_note_field_stores_the_occurrence_note_without_editing_cad() {
    let app = cabinet();
    let root = body_paths(&app)[0].root_occurrence();
    let revision = app.document_revision();
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run();
    harness.state_mut().selection.select_occurrence(root, false);
    harness.run();
    let title = harness.state().catalog.text("viewer-notes-title");
    harness.get_by_label(&title).click();
    harness.run();
    let label = harness.state().catalog.text("viewer-note-occurrence");
    let field = harness.get_by_role_and_label(egui::accesskit::Role::TextInput, &label);
    field.focus();
    field.type_text("Customer sample");
    harness.run();
    let app = harness.state();
    assert_eq!(
        app.stored_viewer_settings()
            .unwrap()
            .occurrence_note(&InstancePath::root(root)),
        "Customer sample"
    );
    assert_eq!(app.document_revision(), revision);
    assert!(app.is_dirty());
}
