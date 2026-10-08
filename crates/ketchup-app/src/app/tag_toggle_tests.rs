//! Turning a layer of the timber-frame house on or off through its dock checkbox
//! repaints at once: the exact solids are reused, not evaluated or published again.
use crate::tests::shell::step_until;
use crate::*;
use eframe::egui::accesskit::Role;
use egui_kittest::kittest::Queryable;
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

/// Debug builds are several times slower than release, where a toggle takes about
/// 0.13 s alone (and up to ~0.45 s beside the parallel test suite); before the
/// fix it took seconds in release.
const TOGGLE_BUDGET: Duration =
    Duration::from_millis(if cfg!(debug_assertions) { 2_500 } else { 800 });

#[test]
fn toggling_a_house_layer_repaints_without_exact_reevaluation() {
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    let mut app = KetchupApp::new();
    app.apply_program_source(
        ketchup_model::document::RuleProgramSource {
            file_name: "tiny-house.star".into(),
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
    assert!(results > 300, "{results} exact bodies");

    for _ in 0..2 {
        let bodies = harness.state().exact_render_body_count();
        let triangles = harness.state().instanced_scene_triangle_count();
        let clicked = Instant::now();
        harness.get_by_role_and_label(Role::CheckBox, &row).click();
        harness.run_steps(2);
        while harness.state().exact.task.is_some() {
            assert!(clicked.elapsed() < Duration::from_secs(30));
            harness.run_steps(1);
        }
        harness.run_steps(1);
        let elapsed = clicked.elapsed();
        visible = !visible;
        let app = harness.state();
        assert_eq!(app.tag_visibility(tag_id), Some(visible));
        assert_eq!(app.exact.results.len(), results);
        assert_ne!(app.exact_render_body_count(), bodies);
        assert_ne!(app.instanced_scene_triangle_count(), triangles);
        eprintln!("layer toggle to {visible} painted in {elapsed:?}");
        assert!(
            elapsed < TOGGLE_BUDGET,
            "layer toggle took {elapsed:?}, budget {TOGGLE_BUDGET:?}"
        );
    }
}
