//! A part that a scene or a layer hides stays selected (Unhide and Show
//! selection need it) but Delete and Cut refuse it: nobody removes parts they
//! cannot see.
use crate::*;

const SOURCE: &str = "box(\"strecha/krokva\", (60, 200, 4000), tags = [\"strecha\"])\nbox(\"prizemie/stena\", (3000, 160, 2500), at = (0, 500, 0), tags = [\"prizemie\"])\n";

fn house() -> (KetchupApp, OccurrenceId, OccurrenceId, TagId) {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "house.star".into(),
            source: SOURCE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let snapshot = app.document.current();
    let named = |name: &str| {
        snapshot
            .occurrences()
            .find(|occurrence| occurrence.name() == name)
            .unwrap()
            .id()
    };
    let roof_tag = snapshot
        .tags()
        .find(|tag| tag.name() == "strecha")
        .unwrap()
        .id();
    (
        app,
        named("strecha/krokva"),
        named("prizemie/stena"),
        roof_tag,
    )
}

#[test]
fn switching_to_a_scene_that_hides_the_roof_keeps_the_rafter_from_being_deleted() {
    let (mut app, rafter, wall, roof) = house();
    assert!(app.set_tag_visibility(roof, false));
    let ground_floor = app.save_view("Prízemie").unwrap();
    assert!(app.set_tag_visibility(roof, true));

    app.select_from_outliner(InstancePath::root(rafter), false);
    app.select_from_outliner(InstancePath::root(wall), true);
    assert_eq!(app.selected_occurrence_ids(), [rafter, wall].into());

    assert!(app.activate_saved_view(ground_floor));
    // Still selected, so Show selection reaches it; Delete and Cut refuse it.
    assert_eq!(app.selected_occurrence_ids(), [rafter, wall].into());
    assert_eq!(app.hidden_selected_count(), 1);
    assert!(!app.command_enabled(AppCommand::Delete));
    assert!(!app.command_enabled(AppCommand::Cut));
    assert!(!app.delete_selected());
    assert_eq!(
        app.action_digest(),
        app.catalog.format(
            "digest-delete-hidden-selected",
            &BTreeMap::from([("count", "1".to_owned())])
        )
    );
    let snapshot = app.document.current();
    assert!(
        snapshot.occurrence(rafter).is_some(),
        "the hidden rafter stays"
    );
    assert!(
        snapshot.occurrence(wall).is_some(),
        "nothing is half-deleted"
    );

    app.select_from_outliner(InstancePath::root(wall), false);
    assert!(app.delete_selected());
    let snapshot = app.document.current();
    assert!(snapshot.occurrence(rafter).is_some());
    assert!(snapshot.occurrence(wall).is_none());
}

#[test]
fn the_layer_rows_reuse_the_scene_the_counts_were_taken_from() {
    let (mut app, _, wall, roof) = house();
    assert!(app.set_tag_visibility(roof, false));
    let snapshot = app.document.current();
    let scene = snapshot.scene_query();
    let rows = app.active_scene_from(&snapshot, Some(&scene));
    assert_eq!(rows, app.active_scene_query());
    let paths = rows.iter().map(|row| row.instance_path.clone());
    assert_eq!(paths.collect::<Vec<_>>(), [InstancePath::root(wall)]);
}

#[test]
fn a_part_hidden_by_a_layer_or_by_redo_is_never_deleted() {
    let (mut app, rafter, wall, roof) = house();
    app.select_from_outliner(InstancePath::root(rafter), false);
    assert!(app.set_tag_visibility(roof, false));
    assert_eq!(app.selected_occurrence_ids(), [rafter].into());
    assert!(!app.delete_selected());
    assert!(app.document.current().occurrence(rafter).is_some());

    // Shown again by Undo and selected, then hidden by Redo.
    assert!(app.undo());
    app.select_from_outliner(InstancePath::root(wall), true);
    assert_eq!(app.hidden_selected_count(), 0);
    assert!(app.redo());
    assert_eq!(app.hidden_selected_count(), 1);
    assert!(!app.delete_selected());
    let snapshot = app.document.current();
    assert!(snapshot.occurrence(rafter).is_some());
    assert!(snapshot.occurrence(wall).is_some());

    // Shown again, the same selection deletes.
    assert!(app.set_tag_visibility(roof, true));
    assert!(app.delete_selected());
    let snapshot = app.document.current();
    assert!(snapshot.occurrence(rafter).is_none());
    assert!(snapshot.occurrence(wall).is_none());
}
