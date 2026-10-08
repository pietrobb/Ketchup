//! The scene tabs above the viewport.
use crate::*;
use egui_kittest::kittest::Queryable;

fn harness() -> egui_kittest::Harness<'static, KetchupApp> {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.run_steps(2);
    harness
}

fn tab_label(harness: &egui_kittest::Harness<'_, KetchupApp>, name: &str) -> String {
    harness
        .state()
        .catalog
        .format("scenes-tab", &BTreeMap::from([("name", name.to_owned())]))
}

#[test]
fn a_double_clicked_scene_tab_is_renamed_in_place_from_its_current_name() {
    let mut harness = harness();
    let id = harness.state_mut().save_view("Celý dom").unwrap();
    harness.run_steps(2);

    let label = tab_label(&harness, "Celý dom");
    harness.get_by_label(&label).click();
    harness.step();
    harness.get_by_label(&label).click();
    harness.run_steps(2);
    let field = harness.state().catalog.text("scenes-rename");
    assert_eq!(
        harness.get_by_label(&field).value().as_deref(),
        Some("Celý dom"),
        "the name field starts from the current name"
    );

    harness
        .state_mut()
        .saved_views_ui
        .renaming
        .as_mut()
        .unwrap()
        .1 = "Whole house".into();
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(
        harness.state().saved_views(),
        vec![(id, "Whole house".to_owned())]
    );
    assert!(harness.state().saved_views_ui.renaming.is_none());
    harness.get_by_label(&tab_label(&harness, "Whole house"));
}

/// A double click renames without showing the scene first; one click shows
/// it once the double-click delay has passed.
#[test]
fn a_double_click_renames_without_showing_the_scene_and_a_click_shows_it() {
    let mut harness = harness();
    let whole = harness.state_mut().save_view("Celý dom").unwrap();
    let roof = harness.state_mut().save_view("Strecha").unwrap();
    harness.run_steps(2);
    assert_eq!(harness.state().saved_views_ui.active, Some(roof));

    let label = tab_label(&harness, "Celý dom");
    harness.get_by_label(&label).click();
    harness.step();
    harness.get_by_label(&label).click();
    harness.run_steps(40);
    assert_eq!(
        harness
            .state()
            .saved_views_ui
            .renaming
            .as_ref()
            .map(|(id, _)| *id),
        Some(whole)
    );
    assert_eq!(harness.state().saved_views_ui.active, Some(roof));
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);

    harness.get_by_label(&label).click();
    harness.run_steps(40);
    assert_eq!(harness.state().saved_views_ui.active, Some(whole));
    assert!(harness.state().saved_views_ui.renaming.is_none());
}

#[test]
fn escape_keeps_the_old_scene_name_and_the_selection() {
    let mut harness = harness();
    let id = harness.state_mut().save_view("Strecha").unwrap();
    let part = harness
        .state()
        .document
        .current()
        .occurrences()
        .next()
        .unwrap()
        .id();
    harness
        .state_mut()
        .select_from_outliner(InstancePath::root(part), false);
    harness.state_mut().saved_views_ui.renaming = Some((id, "Roof".into()));
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().saved_views(),
        vec![(id, "Strecha".to_owned())]
    );
    assert!(harness.state().saved_views_ui.renaming.is_none());
    assert_eq!(harness.state().selected_occurrence_ids(), [part].into());
}

#[test]
fn a_name_another_scene_has_is_refused_with_a_message() {
    let mut harness = harness();
    let first = harness.state_mut().save_view("Prízemie").unwrap();
    let second = harness.state_mut().save_view("Strecha").unwrap();
    harness.state_mut().saved_views_ui.renaming = Some((second, "Prízemie".into()));
    harness.run_steps(2);
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(
        harness.state().saved_views(),
        vec![
            (first, "Prízemie".to_owned()),
            (second, "Strecha".to_owned())
        ]
    );
    let message = harness.state().catalog.format(
        "digest-saved-view-name-taken",
        &BTreeMap::from([("name", "Prízemie".to_owned())]),
    );
    assert_eq!(harness.state().digest, message);
}

/// Undo of a scene's layer switch shows the old layers, so the scene's tab no
/// longer claims the display.
#[test]
fn undoing_a_scene_switch_leaves_no_tab_active() {
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "house.star".into(),
            source: "box(\"krokva\", (60, 200, 4000), tags = [\"strecha\"])\n".into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    let roof = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "strecha")
        .unwrap()
        .id();
    assert!(app.set_tag_visibility(roof, false));
    let ground_floor = app.save_view("Prízemie").unwrap();
    assert!(app.set_tag_visibility(roof, true));
    assert_eq!(
        app.saved_views_ui.active, None,
        "a layer switch leaves the scene"
    );

    assert!(app.activate_saved_view(ground_floor));
    assert_eq!(app.saved_views_ui.active, Some(ground_floor));
    assert!(app.undo());
    assert_eq!(app.saved_views_ui.active, None);
    assert!(app.redo());
    assert_eq!(
        app.saved_views_ui.active, None,
        "Redo is not a click on the tab"
    );
}
