//! #2477 step 4: the dock's VARIANTS section, driven through AccessKit.
use super::*;
use crate::program_evaluation::Lookup;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable};
use ketchup_model::document::RuleProgramSource;
use live_bridge::variant_window::VariantAction;

type Shell = egui_kittest::Harness<'static, KetchupApp>;

const CABINET: &str = "W = param(\"width\", 600)\nH = param(\"height\", 720)\n\
box(\"left\", (18, 400, H), material = \"oak\")\n\
box(\"right\", (18, 400, H), at = (W - 18, 0, 0), material = \"oak\")\n\
box(\"top\", (W - 36, 400, 18), at = (18, 0, H - 18), material = \"oak\")\n";

/// The window's cabinet program with these values, keeping its variants.
fn cabinet(app: &KetchupApp, width: f64, height: f64) -> RuleProgramSource {
    let variants = app
        .document
        .current_rule_program()
        .map(|program| program.variants.clone())
        .unwrap_or_default();
    RuleProgramSource {
        file_name: "cabinet.star".into(),
        source: CABINET.into(),
        overrides: BTreeMap::from([("width".into(), width), ("height".into(), height)]),
        variants,
    }
}

fn harness() -> Shell {
    let mut app = KetchupApp::new();
    let program = cabinet(&app, 600.0, 720.0);
    app.apply_program_source(program, true).unwrap();
    let mut harness = egui_kittest::Harness::builder()
        .with_size(Vec2::new(1600.0, 1400.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    settle(&mut harness);
    harness
}

/// Frames until the program evaluation and the dock's action are done.
fn settle(harness: &mut Shell) {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        harness.run_steps(2);
        let app = harness.state();
        // The dock's own lookup, so its buttons are enabled when this returns.
        let evaluated = app.document.current_rule_program().is_none_or(|program| {
            !matches!(app.program_evaluations.try_get(program), Lookup::Pending)
        });
        if evaluated && !app.program_is_planning() && !app.live.variants.is_running() {
            harness.run_steps(1);
            return;
        }
        assert!(Instant::now() < deadline, "the dock did not settle");
        std::thread::yield_now();
    }
}

/// Where the right side stands and how tall it is.
fn right_side(app: &KetchupApp) -> (f64, f64) {
    let program = app.document.current_rule_program().unwrap();
    let evaluated = app.program_evaluations.get_blocking(program).unwrap();
    let (min, max) = evaluated.model.part("right").unwrap().world_bounds();
    (min[0], max[2] - min[2])
}

fn saved(app: &KetchupApp) -> Vec<String> {
    let program = app.document.current_rule_program().unwrap();
    program.variants.keys().cloned().collect()
}

fn named(harness: &Shell, key: &str, name: &str) -> String {
    let arguments = BTreeMap::from([("name", name.to_owned())]);
    harness.state().catalog.format(key, &arguments)
}

fn text(harness: &Shell, key: &str) -> String {
    harness.state().catalog.text(key)
}

fn save_as(harness: &mut Shell, name: &str) {
    let field = text(harness, "variants-name");
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .type_text(name);
    harness.run_steps(2);
    let save = text(harness, "variants-save");
    harness.get_by_role_and_label(Role::Button, &save).click();
    settle(harness);
    let outcome = harness.state().live.variants.outcome.clone();
    assert_eq!(outcome, Some(named(harness, "variants-saved", name)));
}

fn set_size(harness: &mut Shell, width: f64, height: f64) {
    let program = cabinet(harness.state(), width, height);
    harness
        .state_mut()
        .apply_program_source(program, false)
        .unwrap();
    settle(harness);
}

#[test]
fn the_dock_saves_uses_and_deletes_variants_without_waiting_for_them() {
    let mut harness = harness();
    save_as(&mut harness, "600");
    set_size(&mut harness, 1000.0, 900.0);
    save_as(&mut harness, "1000");
    assert_eq!(saved(harness.state()), ["1000", "600"]);
    let use_1000 = named(&harness, "variants-use", "1000");
    let active = harness.get_by_role_and_label(Role::Button, &use_1000);
    assert!(active.accesskit_node().is_disabled(), "1000 is in use");

    // Use starts the set_params apply on its worker: the click's frame ends
    // with the job still running, and the frames after it only poll.
    let use_600 = named(&harness, "variants-use", "600");
    let undo = harness.state().undo_step_count();
    harness
        .get_by_role_and_label(Role::Button, &use_600)
        .click();
    harness.step();
    assert!(harness.state().live.variants.is_running());
    assert_eq!(
        harness.state().variant_run_is_queued_apply_and_busy(),
        (true, true),
        "Use runs as the set_params queued apply and keeps the window busy"
    );
    settle(&mut harness);
    assert_eq!(
        harness.state().variant_run_is_queued_apply_and_busy(),
        (false, false)
    );
    assert_eq!(right_side(harness.state()), (582.0, 720.0));
    assert_eq!(harness.state().undo_step_count(), undo + 1);
    let outcome = harness.state().live.variants.outcome.clone();
    assert_eq!(outcome, Some(named(&harness, "variants-used", "600")));

    // Width and height changed together; one Undo restores both.
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (982.0, 900.0));

    let delete = named(&harness, "variants-delete", "1000");
    harness.get_by_role_and_label(Role::Button, &delete).click();
    settle(&mut harness);
    assert_eq!(saved(harness.state()), ["600"]);
    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(saved(harness.state()), ["1000", "600"]);
}

#[test]
fn every_dock_button_waits_while_an_action_runs() {
    let mut harness = harness();
    save_as(&mut harness, "600");
    set_size(&mut harness, 1000.0, 900.0);
    save_as(&mut harness, "1000");
    harness.state_mut().live.variants.name = "800".into();
    harness.run_steps(2);
    let buttons = [
        named(&harness, "variants-use", "600"),
        named(&harness, "variants-delete", "600"),
        named(&harness, "variants-delete", "1000"),
        text(&harness, "variants-save"),
        text(&harness, "variants-export"),
    ];
    let disabled = |harness: &Shell| {
        buttons
            .iter()
            .map(|label| {
                let button = harness.get_by_role_and_label(Role::Button, label);
                button.accesskit_node().is_disabled()
            })
            .collect::<Vec<_>>()
    };
    let pending = text(&harness, "variants-pending");
    assert_eq!(disabled(&harness), [false; 5]);
    assert!(harness.query_by_label(&pending).is_none());

    let reply = harness
        .state_mut()
        .hold_variant_run(VariantAction::Use("600".into()));
    harness.run_steps(3);
    assert_eq!(disabled(&harness), [true; 5]);
    harness.get_by_label(&pending);

    drop(reply);
    settle(&mut harness);
    assert_eq!(disabled(&harness), [false; 5]);
    assert_eq!(saved(harness.state()), ["1000", "600"]);
}

#[test]
fn the_dock_exports_every_variant_and_leaves_the_document_as_it_is() {
    let mut harness = harness();
    save_as(&mut harness, "600");
    set_size(&mut harness, 1000.0, 900.0);
    save_as(&mut harness, "1000");
    let folder = tempfile::tempdir().unwrap();
    let target = folder.path().join("skrina.xlsx");
    let (stamp, undo) = (
        harness.state().live_bridge_stamp(),
        harness.state().undo_step_count(),
    );
    let context = harness.ctx.clone();
    let export = live_bridge::variant_window::VariantAction::Export(target.clone());
    harness.state_mut().start_variant_action(&context, export);
    settle(&mut harness);
    let outcome = harness.state().live.variants.outcome.clone().unwrap();
    assert!(outcome.contains(": 2,"), "{outcome}");
    assert!(target.exists());
    assert!(folder.path().join("skrina.takeoff.xlsx").exists());
    assert_eq!(harness.state().live_bridge_stamp(), stamp);
    assert_eq!(harness.state().undo_step_count(), undo);
    assert_eq!(right_side(harness.state()), (982.0, 900.0));

    // A second export to the same file is refused by name and keeps the file.
    let before = std::fs::read(&target).unwrap();
    let export = live_bridge::variant_window::VariantAction::Export(target.clone());
    harness.state_mut().start_variant_action(&context, export);
    settle(&mut harness);
    let outcome = harness.state().live.variants.outcome.clone().unwrap();
    assert!(outcome.starts_with("export_target_exists"), "{outcome}");
    assert_eq!(std::fs::read(&target).unwrap(), before);
}
