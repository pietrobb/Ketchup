//! Drawing a rectangle or an ellipse beside the timber-frame house and pulling
//! it up stays interactive: no frame of the drag re-plans the whole house, and
//! the release turns the shape into a part of the house program.
use crate::*;
use egui_kittest::Harness;
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

/// One frame of the drag or of the held button. In release a frame takes about
/// 20-50 ms; before the fix every drag step and every held frame re-planned the
/// house for about a second.
const FRAME_BUDGET: Duration =
    Duration::from_millis(if cfg!(debug_assertions) { 3_000 } else { 400 });

/// From release to the committed part.
const COMMIT_BUDGET: Duration = Duration::from_millis(if cfg!(debug_assertions) {
    20_000
} else {
    5_000
});

fn settle(harness: &mut Harness<'_, KetchupApp>) {
    let started = Instant::now();
    loop {
        harness.step();
        let app = harness.state();
        if app.exact.task.is_none() && app.push_pull.face_offset_evaluation.is_none() {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(600),
            "exact evaluation never settled"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn event(harness: &mut Harness<'_, KetchupApp>, event: egui::Event) -> Duration {
    harness.input_mut().events.push(event);
    let started = Instant::now();
    harness.step();
    started.elapsed()
}

fn move_to(harness: &mut Harness<'_, KetchupApp>, world: Vec3) -> (Pos2, Duration) {
    let screen = harness.state().viewport_position(world).unwrap();
    (screen, event(harness, egui::Event::PointerMoved(screen)))
}

fn button(harness: &mut Harness<'_, KetchupApp>, screen: Pos2, pressed: bool) -> Duration {
    event(
        harness,
        egui::Event::PointerButton {
            pos: screen,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        },
    )
}

fn click(harness: &mut Harness<'_, KetchupApp>, world: Vec3) {
    let (screen, _) = move_to(harness, world);
    button(harness, screen, true);
    button(harness, screen, false);
}

fn pull_shape_beside_house(ellipse: bool) {
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
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    settle(&mut harness);

    let (start, end) = (
        Vec3::new(-2500.0, -2500.0, 0.0),
        Vec3::new(-1500.0, -1800.0, 0.0),
    );
    harness.state_mut().dispatch_command(if ellipse {
        AppCommand::Ellipse
    } else {
        AppCommand::Rectangle
    });
    harness.step();
    let revision = harness.state().document_revision();
    click(&mut harness, start);
    for step in 1..=3 {
        move_to(&mut harness, start + (end - start) * (step as f64 / 4.0));
    }
    click(&mut harness, end);
    if ellipse {
        // The second radius.
        click(&mut harness, Vec3::new(end.x, start.y - 300.0, 0.0));
    }
    assert_ne!(
        harness.state().document_revision(),
        revision,
        "no shape was drawn"
    );
    settle(&mut harness);

    harness.state_mut().dispatch_command(AppCommand::PushPull);
    harness.step();
    let center = if ellipse { start } else { (start + end) * 0.5 };
    move_to(&mut harness, center + Vec3::new(5.0, 5.0, 0.0));
    let (screen, _) = move_to(&mut harness, center);
    button(&mut harness, screen, true);
    let revision = harness.state().document_revision();
    let mut frames = Vec::new();
    let mut last = screen;
    for step in 1..=7 {
        let (screen, took) = move_to(
            &mut harness,
            center + Vec3::new(0.0, 0.0, 60.0 * step as f64),
        );
        frames.push(took);
        last = screen;
    }
    // The button is still held: the app repaints with nothing changed.
    for _ in 0..4 {
        let started = Instant::now();
        harness.step();
        frames.push(started.elapsed());
    }
    let slowest = frames.iter().max().copied().unwrap_or_default();
    assert!(
        slowest < FRAME_BUDGET,
        "slowest Push/Pull frame {slowest:?}: {frames:?}"
    );

    button(&mut harness, last, false);
    let released = Instant::now();
    while harness.state().document_revision() == revision {
        assert!(
            released.elapsed() < COMMIT_BUDGET,
            "{}",
            harness.state().digest
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    let app = harness.state();
    let program = app
        .document
        .current_rule_program()
        .expect("the house program keeps owning the document");
    assert!(
        program.source.contains("place(extrude(\"shape 1\""),
        "the shape became no program part: {}",
        app.digest
    );
}

#[test]
fn pulling_a_rectangle_beside_the_house_stays_interactive() {
    pull_shape_beside_house(false);
}

#[test]
fn pulling_an_ellipse_beside_the_house_stays_interactive() {
    pull_shape_beside_house(true);
}
