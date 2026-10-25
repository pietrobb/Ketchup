//! #2477 step 4: the dock's VARIANTS section, driven through AccessKit.
use super::*;
use crate::program_evaluation::Lookup;
use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable};
use ketchup_model::document::RuleProgramSource;
use live_bridge::variant_window::VariantAction;

type Shell = egui_kittest::Harness<'static, KetchupApp>;

const CABINET: &str = "W = param(\"width\", 600, min = 300, max = 2400, doc = \"Outer width\")\n\
H = param(\"height\", 720)\n\
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

/// Types `text` into the field of parameter `name` and presses Enter.
fn type_parameter(harness: &mut Shell, name: &str, text: &str) {
    let field = named(harness, "params-field", name);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .type_text(text);
    harness.run_steps(1);
    harness.key_press(egui::Key::Enter);
}

fn field_value(harness: &Shell, name: &str) -> Option<String> {
    let field = named(harness, "params-field", name);
    let node = harness.get_by_role_and_label(Role::TextInput, &field);
    node.accesskit_node().value()
}

#[test]
fn a_typed_parameter_value_is_the_set_params_apply_and_one_undo_step() {
    let mut harness = harness();
    assert_eq!(field_value(&harness, "width").as_deref(), Some("600"));
    assert_eq!(field_value(&harness, "height").as_deref(), Some("720"));
    let range = harness.state().catalog.format(
        "params-range",
        &BTreeMap::from([("min", "300".to_owned()), ("max", "2400".to_owned())]),
    );
    harness.get_by_label(&range);

    // An expression is read like every field; the apply runs on its worker.
    let undo = harness.state().undo_step_count();
    type_parameter(&mut harness, "width", "2*500");
    harness.step();
    assert_eq!(
        harness.state().variant_run_is_queued_apply_and_busy(),
        (true, true),
        "the value runs as the set_params queued apply and keeps the window busy"
    );
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (982.0, 720.0));
    assert_eq!(harness.state().undo_step_count(), undo + 1);
    assert_eq!(field_value(&harness, "width").as_deref(), Some("1000"));
    let set = harness.state().catalog.format(
        "params-set",
        &BTreeMap::from([("name", "width".to_owned()), ("value", "1000".to_owned())]),
    );
    assert_eq!(harness.state().live.variants.outcome, Some(set));

    harness.state_mut().undo();
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (582.0, 720.0));
    assert_eq!(field_value(&harness, "width").as_deref(), Some("600"));
}

#[test]
fn an_unreadable_parameter_value_changes_nothing_and_says_why() {
    let mut harness = harness();
    let (stamp, undo) = (
        harness.state().live_bridge_stamp(),
        harness.state().undo_step_count(),
    );
    type_parameter(&mut harness, "width", "wide");
    harness.run_steps(2);
    assert!(!harness.state().live.variants.is_running());
    settle(&mut harness);
    assert_eq!(harness.state().live_bridge_stamp(), stamp);
    assert_eq!(harness.state().undo_step_count(), undo);
    assert_eq!(right_side(harness.state()), (582.0, 720.0));
    let unreadable = text(&harness, "value-unreadable");
    let outcome = harness.state().live.variants.outcome.clone().unwrap();
    assert!(outcome.contains(&unreadable), "{outcome}");
    assert_eq!(field_value(&harness, "width").as_deref(), Some("600"));
}

/// Review 2026-10-24 WB-27: a value typed without Enter was dropped when
/// the field lost focus, and a count read "3 m" as 3000.
#[test]
fn a_value_is_set_when_its_field_loses_focus_and_a_count_takes_no_unit() {
    let mut harness = harness();
    let field = named(&harness, "params-field", "height");
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .type_text("70 cm");
    harness.run_steps(1);
    harness.key_press(egui::Key::Tab);
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (582.0, 700.0));

    // Escape leaves the value as it was.
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .type_text("900");
    harness.run_steps(1);
    harness.key_press(egui::Key::Escape);
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (582.0, 700.0));

    let counted = RuleProgramSource {
        file_name: "shelves.star".into(),
        source: "N = param(\"shelves\", 2, unit = \"\")\n\
                 for i in range(N):\n    box(\"s%d\" % i, (600, 400, 18), at = (0, 0, 300 * i))\n"
            .into(),
        overrides: BTreeMap::new(),
        variants: BTreeMap::new(),
    };
    harness
        .state_mut()
        .apply_program_source(counted, false)
        .unwrap();
    settle(&mut harness);
    let undo = harness.state().undo_step_count();
    type_parameter(&mut harness, "shelves", "3 m");
    settle(&mut harness);
    assert_eq!(harness.state().undo_step_count(), undo);
    let unreadable = text(&harness, "value-unreadable");
    let outcome = harness.state().live.variants.outcome.clone().unwrap();
    assert!(outcome.contains(&unreadable), "{outcome}");
    type_parameter(&mut harness, "shelves", "3");
    settle(&mut harness);
    assert_eq!(field_value(&harness, "shelves").as_deref(), Some("3"));
}

/// Review 2026-10-24 UI-29: an evaluation or action that began while a value was being
/// typed turned the field off, and the typed text was lost; a third of a metre read
/// `333.3333333333333`. The field being typed in stays, its text is set once the window
/// is free, and a value shows with at most six decimals.
#[test]
fn a_value_typed_while_the_dock_waits_is_kept_and_set_when_it_is_free() {
    let mut harness = harness();
    let field = named(&harness, "params-field", "width");
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .focus();
    harness.run_steps(1);
    harness.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    harness
        .get_by_role_and_label(Role::TextInput, &field)
        .type_text("800");
    harness.run_steps(1);

    let reply = harness
        .state_mut()
        .hold_variant_run(VariantAction::Use("none".into()));
    harness.run_steps(3);
    let typed = harness.get_by_role_and_label(Role::TextInput, &field);
    assert!(
        !typed.accesskit_node().is_disabled(),
        "the typed field stays"
    );
    assert_eq!(field_value(&harness, "width").as_deref(), Some("800"));
    harness.key_press(egui::Key::Enter);
    harness.run_steps(2);
    assert_eq!(field_value(&harness, "width").as_deref(), Some("800"));

    drop(reply);
    settle(&mut harness);
    settle(&mut harness);
    assert_eq!(right_side(harness.state()), (782.0, 720.0));

    set_size(&mut harness, 1000.0 / 3.0 + 300.0, 720.0);
    assert_eq!(
        field_value(&harness, "width").as_deref(),
        Some("633.333333")
    );
    let undo = harness.state().undo_step_count();
    type_parameter(&mut harness, "width", "633.333333");
    settle(&mut harness);
    assert_eq!(
        harness.state().undo_step_count(),
        undo,
        "Enter on the value as shown keeps it"
    );
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
    let fields = [
        named(&harness, "params-field", "width"),
        named(&harness, "params-field", "height"),
    ];
    let fields_disabled = |harness: &Shell| {
        fields
            .iter()
            .map(|label| {
                let field = harness.get_by_role_and_label(Role::TextInput, label);
                field.accesskit_node().is_disabled()
            })
            .collect::<Vec<_>>()
    };
    let pending = text(&harness, "variants-pending");
    assert_eq!(disabled(&harness), [false; 5]);
    assert_eq!(fields_disabled(&harness), [false; 2]);
    assert!(harness.query_by_label(&pending).is_none());

    let reply = harness
        .state_mut()
        .hold_variant_run(VariantAction::Use("600".into()));
    harness.run_steps(3);
    assert_eq!(disabled(&harness), [true; 5]);
    assert_eq!(fields_disabled(&harness), [true; 2]);
    harness.get_by_label(&pending);

    drop(reply);
    settle(&mut harness);
    assert_eq!(disabled(&harness), [false; 5]);
    assert_eq!(fields_disabled(&harness), [false; 2]);
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
