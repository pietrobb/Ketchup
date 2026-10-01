//! Selections of nested and mixed instance paths: kept as picked, refused by
//! root-only operations with a typed refusal, and never mutating the document.

use super::*;

#[test]
fn nested_and_mixed_selection_paths_are_preserved_and_diagnosed_without_mutation() {
    let mut app = KetchupApp::new();
    assert!(app.create_box());
    app.select_from_outliner(InstancePath::root(OccurrenceId(1)), false);
    app.select_from_outliner(InstancePath::root(OccurrenceId(2)), true);
    assert!(app.group_selected());
    assert!(app.make_component());
    assert!(app.copy_selected(Vec3::new(200.0, 0.0, 0.0)));

    let nested = app
        .active_boxes()
        .into_iter()
        .map(|item| item.instance_path)
        .filter(|path| !path.is_root())
        .collect::<Vec<_>>();
    assert_eq!(nested.len(), 4);
    app.clear_selection();
    let nested_paths = BTreeSet::from([nested[0].clone(), nested[1].clone()]);
    app.selection.occurrences = nested_paths.clone();
    assert_eq!(app.selected_instance_paths(), nested_paths);
    let nested_error = RootOccurrenceSelectionError::Nested {
        paths: nested_paths.clone(),
    };
    assert_eq!(
        app.selected_root_occurrence_ids(),
        Err(nested_error.clone())
    );
    let nested_refusal = app.root_occurrence_selection_error(&nested_error);
    assert_eq!(
        nested_refusal.code(),
        "selection-error-nested-instance-paths"
    );
    let nested_diagnostic = nested_refusal.reason_text().to_owned();
    for path in &nested_paths {
        assert!(
            nested_diagnostic.contains(&KetchupApp::assistant_instance_path_label(path)),
            "diagnostic must identify every selected path: {nested_diagnostic}"
        );
    }
    let context = app.assistant_context_for("inspect selection");
    assert_eq!(context["selection_scope"], "nested_instance_paths");
    assert_eq!(context["selected_occurrence_ids"], serde_json::json!([]));
    assert_eq!(
        context["selected_instance_paths"],
        serde_json::json!(
            nested_paths
                .iter()
                .map(KetchupApp::assistant_instance_path_label)
                .collect::<Vec<_>>()
        )
    );
    assert_eq!(app.assistant_selection_summary(), nested_diagnostic);

    let revision = app.document_revision();
    let digest = app.canonical_digest();
    let undo_steps = app.undo_step_count();
    app.begin_occurrence_align();
    assert_eq!(app.action_digest(), nested_diagnostic);
    assert!(!app.occurrence_align_visible());
    assert!(!app.has_occurrence_operation_preview());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    assert!(!app.prepare_assistant_assembly_joint_from_selection());
    assert!(app.action_digest().contains(&nested_diagnostic));
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);

    let component_root = InstancePath::root(
        nested
            .iter()
            .map(InstancePath::root_occurrence)
            .find(|root| *root != nested[0].root_occurrence())
            .expect("the copied component has a distinct root"),
    );
    app.clear_selection();
    let mixed_paths = BTreeSet::from([component_root, nested[0].clone()]);
    app.selection.occurrences = mixed_paths.clone();
    assert_eq!(app.selected_instance_paths(), mixed_paths);
    let mixed_error = RootOccurrenceSelectionError::Mixed {
        paths: mixed_paths.clone(),
    };
    assert_eq!(app.selected_root_occurrence_ids(), Err(mixed_error.clone()));
    let mixed_refusal = app.root_occurrence_selection_error(&mixed_error);
    assert_eq!(mixed_refusal.code(), "selection-error-mixed-instance-paths");
    let mixed_diagnostic = mixed_refusal.reason_text().to_owned();
    let mixed_context = app.assistant_context_for("inspect mixed selection");
    assert_eq!(mixed_context["selection_scope"], "mixed_instance_paths");
    assert_eq!(
        mixed_context["selected_instance_paths"],
        serde_json::json!(
            mixed_paths
                .iter()
                .map(KetchupApp::assistant_instance_path_label)
                .collect::<Vec<_>>()
        )
    );
    app.begin_occurrence_align();
    assert_eq!(app.action_digest(), mixed_diagnostic);
    assert!(!app.occurrence_align_visible());
    assert_eq!(app.document_revision(), revision);
    assert_eq!(app.canonical_digest(), digest);
    assert_eq!(app.undo_step_count(), undo_steps);
}
