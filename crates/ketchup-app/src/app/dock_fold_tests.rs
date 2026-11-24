use crate::*;
use egui_kittest::kittest::Queryable;

/// The right dock took 380 px or more on every screen. Folded, it leaves a
/// strip of its tab icons and the model gets the width; a tab icon on the
/// strip opens the dock on that page.
#[test]
fn the_folded_dock_gives_its_width_to_the_model_and_a_tab_icon_opens_it() {
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1280.0, 800.0))
        .build_state(
            |context, app: &mut KetchupApp| app.ui(context),
            KetchupApp::new(),
        );
    harness.run_steps(2);
    let catalog = harness.state().catalog.clone();
    let open_width = harness.state().viewport_rect().unwrap().width();

    harness.get_by_label(&catalog.text("dock-fold")).click();
    harness.run_steps(2);
    let folded_width = harness.state().viewport_rect().unwrap().width();
    assert!(
        folded_width > open_width + 300.0,
        "{open_width} -> {folded_width}"
    );
    assert!(harness.query_by_label(&catalog.text("dock-fold")).is_none());

    harness
        .get_by_label(&catalog.text(DockTab::Checks.label_key()))
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().dock_tab(), DockTab::Checks);
    assert!(
        harness
            .query_by_label(&catalog.text("dock-unfold"))
            .is_none()
    );
    let reopened_width = harness.state().viewport_rect().unwrap().width();
    assert!(
        (reopened_width - open_width).abs() < 1.0,
        "{reopened_width}"
    );
}
