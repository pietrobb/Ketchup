//! Project drawings of the timber-frame house: the sheet carries the floor plan,
//! both sections and four elevations, and follows the visible layers.
use crate::*;
use ketchup_manufacturing::project_drawings::{ProjectSheet, ProjectView};
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

fn cut_solids(sheet: &ProjectSheet, view: ProjectView) -> usize {
    sheet
        .views
        .iter()
        .find(|summary| summary.view == view)
        .map_or(0, |summary| summary.cut_solids)
}

#[test]
fn the_house_sheet_has_plan_sections_and_elevations_of_the_visible_layers() {
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
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1200.0))
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    let started = Instant::now();
    while harness.state().exact.task.is_some() || harness.state().exact.source.is_none() {
        assert!(
            started.elapsed() < Duration::from_secs(300),
            "exact house never finished"
        );
        harness.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }

    let concept = harness.state().project_drawings().unwrap();
    assert_eq!(concept.views.len(), ProjectView::ALL.len());
    assert!(concept.svg.starts_with("<?xml") || concept.svg.starts_with("<svg"));
    for key in [
        "drawings-view-longitudinal",
        "drawings-view-cross",
        "drawings-view-front",
    ] {
        let title = harness.state().catalog.text(key);
        assert!(concept.svg.contains(&title), "{key} missing");
    }
    for view in [
        ProjectView::Plan,
        ProjectView::LongitudinalSection,
        ProjectView::CrossSection,
    ] {
        assert!(cut_solids(&concept, view) > 0, "{view:?} cuts nothing");
    }

    let app = harness.state_mut();
    let tag = app
        .document
        .current()
        .tags()
        .find(|tag| tag.name() == "koncept")
        .map(|tag| tag.id())
        .unwrap();
    assert!(app.set_tag_visibility(tag, false));
    let construction = app.project_drawings().unwrap();
    let (all, without_concept) = (
        cut_solids(&concept, ProjectView::Plan),
        cut_solids(&construction, ProjectView::Plan),
    );
    assert!(
        without_concept > 50 && without_concept < all,
        "hiding the concept layer leaves the studs and layers: {without_concept} of {all}"
    );

    let path = std::env::temp_dir().join(format!("ketchup-drawings-{}.svg", std::process::id()));
    assert!(app.export_project_drawings_to(&path));
    let written = std::fs::read_to_string(&path).unwrap();
    std::fs::remove_file(&path).ok();
    assert_eq!(written, construction.svg);
    assert!(app.digest.contains("1:"), "{}", app.digest);
}
