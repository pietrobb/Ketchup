use ketchup_model::document::{CanonicalCommand, CommandBatch, Dimension, DocumentStore, NodeId};
use ketchup_model::persistence::{self, ContainerData};
use ketchup_program::document::{PartChanges, ProgramDocument, ProgramSource, UNDO_LIMIT};
use std::collections::BTreeMap;

const SOURCE: &str = r#"
W = param("width", 100)
box("fixed", [10, 10, 10], at = [0, 0, 50])
box("moving", [W, 20, 10])
box("dependent", [W - 20, 20, 10], at = [10, 0, 10])
"#;

fn source(width: f64) -> ProgramSource {
    ProgramSource {
        file_name: "history.star".to_owned(),
        source: SOURCE.to_owned(),
        overrides: BTreeMap::from([("width".to_owned(), width)]),
    }
}

fn width(document: &ProgramDocument) -> f64 {
    document.evaluated().model.part("moving").unwrap().size_mm[0]
}

#[test]
fn changing_a_parameter_updates_dependents_but_not_the_fixed_part() {
    let original = source(100.0);
    let mut document = ProgramDocument::new(original.clone()).unwrap();
    let before = document.evaluated().clone();
    let changes = document.replace(source(140.0)).unwrap();
    assert_eq!(changes.modified, ["dependent", "moving"]);
    assert!(changes.added.is_empty());
    assert!(changes.removed.is_empty());
    assert_eq!(width(&document), 140.0);
    assert_eq!(
        document
            .evaluated()
            .model
            .part("dependent")
            .unwrap()
            .size_mm[0],
        120.0
    );
    let after = document.evaluated().clone();
    let after_report = document.report().clone();
    assert_eq!(document.undo().unwrap(), Some(changes.clone()));
    assert_eq!(document.source(), &original);
    assert_eq!(document.evaluated(), &before);
    assert_eq!(document.redo().unwrap(), Some(changes));
    assert_eq!(document.evaluated(), &after);
    assert_eq!(document.report(), &after_report);
}

#[test]
fn only_the_last_ten_changes_can_be_undone_and_redone() {
    let mut document = ProgramDocument::new(source(100.0)).unwrap();
    for width in 101..=125 {
        document.replace(source(f64::from(width))).unwrap();
    }
    for expected in (115..125).rev() {
        assert!(document.undo().unwrap().is_some());
        assert_eq!(width(&document), f64::from(expected));
    }
    assert_eq!(UNDO_LIMIT, 10);
    assert!(document.undo().unwrap().is_none());
    for expected in 116..=125 {
        assert!(document.redo().unwrap().is_some());
        assert_eq!(width(&document), f64::from(expected));
    }
    assert!(document.redo().unwrap().is_none());
}

#[test]
fn failed_edit_and_noop_preserve_the_current_model_and_redo_branch() {
    let mut document = ProgramDocument::new(source(100.0)).unwrap();
    document.replace(source(120.0)).unwrap();
    document.undo().unwrap().unwrap();
    let before = document.evaluated().clone();
    let report = document.report().clone();
    let mut invalid = source(130.0);
    invalid.source.push_str("\nunknown_operation()\n");
    let error = document.replace(invalid).unwrap_err();
    assert!(error.message.contains("unknown_operation"), "{error}");
    assert_eq!(document.evaluated(), &before);
    assert_eq!(document.report(), &report);
    assert_eq!(document.source(), &source(100.0));
    assert_eq!(
        document.replace(source(100.0)).unwrap(),
        PartChanges::default()
    );
    assert!(document.can_redo());
    document.redo().unwrap().unwrap();
    assert_eq!(width(&document), 120.0);
}

#[test]
fn editing_after_undo_replaces_only_the_redo_branch() {
    let mut document = ProgramDocument::new(source(100.0)).unwrap();
    document.replace(source(120.0)).unwrap();
    document.replace(source(140.0)).unwrap();
    document.undo().unwrap().unwrap();
    document.replace(source(180.0)).unwrap();
    assert!(!document.can_redo());
    document.undo().unwrap().unwrap();
    assert_eq!(width(&document), 120.0);
    document.undo().unwrap().unwrap();
    assert_eq!(width(&document), 100.0);
    assert!(!document.can_undo());
}

#[test]
fn reordering_parts_keeps_identity_and_source_edits_are_reversible() {
    let mut document = ProgramDocument::new(source(100.0)).unwrap();
    let mut reordered = source(100.0);
    reordered.source = r#"
W = param("width", 100)
box("dependent", [W - 20, 20, 10], at = [10, 0, 10])
box("moving", [W, 20, 10])
box("fixed", [10, 10, 10], at = [0, 0, 50])
"#
    .to_owned();
    assert_eq!(
        document.replace(reordered.clone()).unwrap(),
        PartChanges::default()
    );
    let mut removed = reordered.clone();
    removed.source = removed
        .source
        .replace("box(\"moving\", [W, 20, 10])", "box(\"new\", [W, 20, 10])");
    let changes = document.replace(removed).unwrap();
    assert_eq!(changes.added, ["new"]);
    assert_eq!(changes.removed, ["moving"]);
    assert!(changes.modified.is_empty());
    let undo = document.undo().unwrap().unwrap();
    assert_eq!(undo.added, ["moving"]);
    assert_eq!(undo.removed, ["new"]);
    assert_eq!(document.source(), &reordered);
}

#[test]
fn changing_joinery_marks_both_machined_parts_and_undo_restores_holes() {
    let cabinet = include_str!("../../../examples/programs/cabinet.star");
    let original = ProgramSource {
        file_name: "cabinet.star".to_owned(),
        source: cabinet.to_owned(),
        overrides: BTreeMap::new(),
    };
    let mut document = ProgramDocument::new(original.clone()).unwrap();
    let before = document.evaluated().clone();
    let mut moved = original;
    moved.source = moved.source.replace(
        "dowels(panel, side, dowel = \"8x30\", margin = 50)",
        "dowels(panel, side, dowel = \"8x30\", margin = 80)",
    );
    let changes = document.replace(moved).unwrap();
    for part in [
        "carcass/bottom",
        "carcass/left",
        "carcass/right",
        "carcass/top",
    ] {
        assert!(
            changes.modified.iter().any(|name| name == part),
            "{changes:?}"
        );
    }
    assert!(
        !changes
            .modified
            .iter()
            .any(|name| name.starts_with("shelves/"))
    );
    assert!(document.report().ok);
    document.undo().unwrap().unwrap();
    assert_eq!(document.evaluated(), &before);
}

#[test]
fn source_only_change_keeps_geometry_and_ids_in_one_canonical_history() {
    let original = source(100.0);
    let mut changed = original.clone();
    changed.source.push_str("\n# same model, edited source\n");
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "width".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    assert!(store.bind_rule_program(original.clone()));
    let before = store.current();
    let before_ids = before
        .definitions()
        .map(|definition| definition.id())
        .collect::<Vec<_>>();
    assert!(store.replace_rule_program_source(changed.clone()));
    assert_eq!(
        store.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(
        store
            .current()
            .definitions()
            .map(|definition| definition.id())
            .collect::<Vec<_>>(),
        before_ids
    );
    assert_ne!(store.current().revision_id(), before.revision_id());
    let revision = store.current().revision_id();
    assert!(store.replace_rule_program_source(changed.clone()));
    assert_eq!(store.current().revision_id(), revision);
    store.undo().unwrap();
    assert_eq!(store.current_rule_program(), Some(&original));
    store.redo().unwrap();
    assert_eq!(store.current_rule_program(), Some(&changed));
    let bytes = persistence::save_document_store(&store, &ContainerData::default()).unwrap();
    let (mut opened, _) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("source-only revision must remain editable"));
    assert_eq!(opened.current_rule_program(), Some(&changed));
    opened.undo().unwrap();
    assert_eq!(opened.current_rule_program(), Some(&original));
}

#[test]
fn source_belongs_to_canonical_revision_through_save_open_and_manual_edit() {
    let original = source(100.0);
    let mut store = DocumentStore::new();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(1),
                name: "width".into(),
                dimension: Dimension::new("100", 100.0).unwrap(),
                dependencies: vec![],
            },
        ]))
        .unwrap();
    assert!(store.bind_rule_program(original.clone()));
    let authored_id = store.current().revision_id();
    store
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetEvaluatorDimension {
                id: NodeId(1),
                dimension: Dimension::new("120", 120.0).unwrap(),
            },
        ]))
        .unwrap();
    assert!(
        store.current_rule_program().is_none(),
        "manual edit must not claim stale source"
    );
    store.undo().unwrap();
    assert_eq!(store.current_rule_program(), Some(&original));
    store.redo().unwrap();
    let bytes = persistence::save_document_store(&store, &ContainerData::default()).unwrap();
    let (mut reopened, _) = persistence::load(&bytes)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("rule document must remain editable"));
    assert!(reopened.current_rule_program().is_none());
    reopened.undo().unwrap();
    assert_eq!(reopened.current().revision_id(), authored_id);
    assert_eq!(reopened.current_rule_program(), Some(&original));
    let compact =
        persistence::save_document_store_current_snapshot(&reopened, &ContainerData::default())
            .unwrap();
    let (only_current, _) = persistence::load(&compact)
        .unwrap()
        .into_editable_with_container()
        .unwrap_or_else(|_| panic!("current rule snapshot must remain editable"));
    assert_eq!(only_current.current_rule_program(), Some(&original));
    assert_eq!(only_current.visible_undo_steps(), 0);
    reopened.redo().unwrap();
    assert!(reopened.current_rule_program().is_none());
}
