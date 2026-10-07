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

#[test]
fn escape_keeps_the_old_scene_name() {
    let mut harness = harness();
    let id = harness.state_mut().save_view("Strecha").unwrap();
    harness.state_mut().saved_views_ui.renaming = Some((id, "Roof".into()));
    harness.run_steps(2);
    harness.key_press(egui::Key::Escape);
    harness.run_steps(2);
    assert_eq!(
        harness.state().saved_views(),
        vec![(id, "Strecha".to_owned())]
    );
    assert!(harness.state().saved_views_ui.renaming.is_none());
}
