//! Turning a layer of a small house on or off through its dock checkbox
//! repaints at once: the exact solids are reused, not evaluated or published again.
use crate::tests::shell::step_until;
use crate::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../tests/fixtures/small-house.star");

#[test]
fn toggling_a_house_layer_repaints_without_exact_reevaluation() {
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "small-house.star".into(),
            source: HOUSE.into(),
            overrides: BTreeMap::new(),
        },
        true,
    )
    .unwrap();
    app.headless_force_exact_worker_path(&worker);
    app.enable_headless_instanced_scene();
    let snapshot = app.document.current();
    let tag = snapshot
        .tags()
        .find(|tag| tag.name() == "koncept")
        .expect("the house has a concept layer");
    let (tag_id, mut visible) = (tag.id(), tag.visible());
    let members = snapshot
        .occurrences()
        .filter(|occurrence| occurrence.tags().contains(&tag_id))
        .count();
    let row = app.catalog.format(
        "tags-row",
        &BTreeMap::from([
            ("name", "koncept".to_owned()),
            ("count", members.to_string()),
        ]),
    );
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    harness.run_steps(1);
    step_until(
        &mut harness,
        Duration::from_secs(300),
        "exact house never finished",
        |app| app.exact.task.is_none() && app.exact.source.is_some(),
    );
    harness.run_steps(2);
    let results = harness.state().exact.results.len();
    assert!(results > 20, "{results} exact bodies");
    assert!(harness.state().exact.producers_requested > 0);

    for _ in 0..2 {
        let bodies = harness.state().exact_render_body_count();
        let triangles = harness.state().instanced_scene_triangle_count();
        let requested = harness.state().exact.producers_requested;
        let clicked = Instant::now();
        harness.get_by_role_and_label(Role::CheckBox, &row).click();
        harness.run_steps(2);
        step_until(
            &mut harness,
            Duration::from_secs(300),
            "the toggle's exact pass never finished",
            |app| app.exact.task.is_none(),
        );
        harness.run_steps(1);
        let elapsed = clicked.elapsed();
        visible = !visible;
        let app = harness.state();
        assert_eq!(app.tag_visibility(tag_id), Some(visible));
        assert_eq!(
            app.exact.producers_requested, requested,
            "the toggle sent producers to the exact worker"
        );
        assert_eq!(app.exact.results.len(), results);
        assert_ne!(app.exact_render_body_count(), bodies);
        assert_ne!(app.instanced_scene_triangle_count(), triangles);
        eprintln!("layer toggle to {visible} painted in {elapsed:?}");
    }
}
