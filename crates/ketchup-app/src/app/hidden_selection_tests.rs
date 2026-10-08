//! A part that a scene or a layer hides leaves the selection, so Delete never
//! removes parts nobody sees.
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
fn switching_to_a_scene_that_hides_the_roof_drops_the_rafter_from_the_selection() {
    let (mut app, rafter, wall, roof) = house();
    assert!(app.set_tag_visibility(roof, false));
    let ground_floor = app.save_view("Prízemie").unwrap();
    assert!(app.set_tag_visibility(roof, true));

    app.select_from_outliner(InstancePath::root(rafter), false);
    app.select_from_outliner(InstancePath::root(wall), true);
    assert_eq!(app.selected_occurrence_ids(), [rafter, wall].into());

    assert!(app.activate_saved_view(ground_floor));
    assert_eq!(app.selected_occurrence_ids(), [wall].into());
    assert!(app.delete_selected());
    let snapshot = app.document.current();
    assert!(
        snapshot.occurrence(rafter).is_some(),
        "the hidden rafter stays"
    );
    assert!(snapshot.occurrence(wall).is_none());
}

#[test]
fn hiding_a_layer_or_undoing_its_display_deselects_only_what_it_hides() {
    let (mut app, rafter, wall, roof) = house();
    app.select_from_outliner(InstancePath::root(rafter), false);
    assert!(app.set_tag_visibility(roof, false));
    assert!(app.selected_occurrence_ids().is_empty());
    assert!(!app.delete_selected());
    assert!(app.document.current().occurrence(rafter).is_some());

    // Shown again by Undo and selected, then hidden by Redo.
    assert!(app.undo());
    app.select_from_outliner(InstancePath::root(rafter), false);
    app.select_from_outliner(InstancePath::root(wall), true);
    assert!(app.redo());
    assert_eq!(app.selected_occurrence_ids(), [wall].into());
}
