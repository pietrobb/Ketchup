//! Drawing a rectangle or an ellipse beside the timber-frame house and pulling
//! it up stays interactive: no frame of the drag re-plans the whole house, and
//! the release turns the shape into a part of the house program. Once the house
//! no longer owns the document, pulling the top of the pulled shape again
//! lengthens its extrusion, also without a slow frame.
use crate::*;
use egui_kittest::Harness;
use std::time::{Duration, Instant};

const HOUSE: &str = include_str!("../../../../examples/programs/tiny-house.star");

/// One frame of the drag or of the held button. In release a frame takes about
/// 20-60 ms; before the fixes every drag step re-planned the house, or looked
/// up the picked face among all faces of the house, for 0.6-1 s.
const FRAME_BUDGET: Duration =
    Duration::from_millis(if cfg!(debug_assertions) { 3_000 } else { 400 });

/// The frame of the release, which publishes the edited house program. It took
/// about 1.7 s while the program report (issues, relations, loads) was listed
/// twice and thrown away; without it about 0.4 s.
const RELEASE_BUDGET: Duration = Duration::from_millis(if cfg!(debug_assertions) {
    10_000
} else {
    1_000
});

/// From release to the committed part.
const COMMIT_BUDGET: Duration = Duration::from_millis(if cfg!(debug_assertions) {
    20_000
} else {
    5_000
});

const START: Vec3 = Vec3::new(-2500.0, -2500.0, 0.0);
const END: Vec3 = Vec3::new(-1500.0, -1800.0, 0.0);
const PULL_STEP_MM: f64 = 60.0;
const PULL_STEPS: usize = 7;

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

fn house() -> KetchupApp {
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
    app
}

/// Hiding one part by hand is an edit outside the program, so the house parts
/// stay but no program owns the document any more.
fn detach(app: &mut KetchupApp) {
    let first = app.document.current().occurrences().next().unwrap().id();
    app.apply_batch_with_work_recovery(&CommandBatch::new(vec![
        CanonicalCommand::SetOccurrenceVisibility {
            id: first,
            visible: false,
        },
    ]))
    .unwrap();
    assert!(app.document.current_rule_program().is_none());
}

fn harness(mut app: KetchupApp) -> Harness<'static, KetchupApp> {
    let worker = ketchup_application::evaluation::exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker alongside the app tests");
    app.headless_force_exact_worker_path(&worker);
    app.enable_headless_instanced_scene();
    let mut harness = Harness::builder()
        .with_size(Vec2::new(1600.0, 1000.0))
        .with_step_dt(1.0 / 60.0)
        .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
    settle(&mut harness);
    harness
}

fn draw(harness: &mut Harness<'_, KetchupApp>, ellipse: bool) {
    harness.state_mut().dispatch_command(if ellipse {
        AppCommand::Ellipse
    } else {
        AppCommand::Rectangle
    });
    harness.step();
    let revision = harness.state().document_revision();
    click(harness, START);
    for step in 1..=3 {
        move_to(harness, START + (END - START) * (step as f64 / 4.0));
    }
    click(harness, END);
    if ellipse {
        // The second radius.
        click(harness, Vec3::new(END.x, START.y - 300.0, 0.0));
    }
    assert_ne!(
        harness.state().document_revision(),
        revision,
        "no shape was drawn"
    );
    settle(harness);
}

/// Drags the face under `at` up by `PULL_STEPS * PULL_STEP_MM` and checks that
/// no frame of the drag, the held button or the release is slow.
fn pull(harness: &mut Harness<'_, KetchupApp>, at: Vec3) {
    harness.state_mut().dispatch_command(AppCommand::PushPull);
    harness.step();
    move_to(harness, at + Vec3::new(5.0, 5.0, 0.0));
    let (screen, _) = move_to(harness, at);
    button(harness, screen, true);
    let revision = harness.state().document_revision();
    let mut frames = Vec::new();
    let mut last = screen;
    for step in 1..=PULL_STEPS {
        let (screen, took) = move_to(
            harness,
            at + Vec3::new(0.0, 0.0, PULL_STEP_MM * step as f64),
        );
        frames.push(took);
        last = screen;
    }
    let slowest = frames.iter().max().copied().unwrap_or_default();
    assert!(
        slowest < FRAME_BUDGET,
        "slowest Push/Pull frame {slowest:?}: {frames:?}"
    );
    // The button is still held. A loaded machine may finish the exact
    // evaluation now, and that frame publishes it like the release does.
    for _ in 0..4 {
        let started = Instant::now();
        harness.step();
        let took = started.elapsed();
        assert!(took < RELEASE_BUDGET, "a held frame took {took:?}");
    }

    let release = button(harness, last, false);
    assert!(
        release < RELEASE_BUDGET,
        "the release froze the window for {release:?}"
    );
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
    settle(harness);
}

fn pull_shape_beside_house(ellipse: bool) {
    let mut harness = harness(house());
    draw(&mut harness, ellipse);
    pull(
        &mut harness,
        if ellipse { START } else { (START + END) * 0.5 },
    );
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

/// The second pull grabs the top of the first one. The extrusion grows to
/// twice the pulled height; no face offset with a seam is fused on top.
fn pull_top_of_pulled_ellipse(detach_before_drawing: bool) {
    let mut app = house();
    if detach_before_drawing {
        detach(&mut app);
    }
    let mut harness = harness(app);
    draw(&mut harness, true);
    pull(&mut harness, START);
    if !detach_before_drawing {
        detach(harness.state_mut());
        settle(&mut harness);
    }
    let height = PULL_STEP_MM * PULL_STEPS as f64;
    pull(&mut harness, START + Vec3::new(0.0, 0.0, height));

    let snapshot = harness.state().document.current();
    let shape = snapshot
        .definitions()
        .find(|definition| {
            definition.name() == "shape 1" || definition.name().starts_with("Ellipse")
        })
        .expect("the pulled ellipse");
    let kinds = shape
        .feature_ids()
        .iter()
        .map(|id| snapshot.feature(*id).unwrap().kind())
        .collect::<Vec<_>>();
    assert!(
        !kinds
            .iter()
            .any(|kind| matches!(kind, FeatureKind::FaceOffset { .. })),
        "{kinds:?}"
    );
    let Some(FeatureKind::Pad(PadSpec {
        extent: FeatureExtent::Blind(extent),
        ..
    })) = kinds.last()
    else {
        panic!("{kinds:?}");
    };
    assert!(
        (extent.millimetres() - 2.0 * height).abs() < 1.0,
        "{kinds:?}"
    );
}

#[test]
fn pulling_the_top_of_an_ellipse_drawn_after_the_house_was_detached_lengthens_it() {
    pull_top_of_pulled_ellipse(true);
}

#[test]
fn pulling_the_top_of_a_house_ellipse_after_detaching_lengthens_it() {
    pull_top_of_pulled_ellipse(false);
}
