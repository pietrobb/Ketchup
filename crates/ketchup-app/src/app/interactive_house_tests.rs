//! Hovering, selecting, moving, rotating, scaling and filleting beside the
//! 750-part timber-frame house stays interactive: no pointer frame and no
//! release re-scans the whole house.
use super::push_pull_house_tests::*;
use crate::*;
use egui_kittest::Harness;
use std::time::{Duration, Instant};

const TOP: f64 = PULL_STEP_MM * PULL_STEPS as f64;
const DRAG_STEPS: usize = 6;

fn centre() -> Vec3 {
    (START + END) * 0.5
}

/// The house with a box drawn and pulled up beside it.
fn house_with_box() -> Harness<'static, KetchupApp> {
    let mut harness = harness(house());
    draw(&mut harness, false);
    pull(&mut harness, centre());
    harness
}

fn tool(harness: &mut Harness<'_, KetchupApp>, command: AppCommand) {
    harness.state_mut().dispatch_command(command);
    harness.step();
}

fn assert_frames(what: &str, frames: &[Duration]) {
    let slowest = frames.iter().max().copied().unwrap_or_default();
    assert!(
        slowest < FRAME_BUDGET,
        "slowest {what} frame {slowest:?}: {frames:?}"
    );
}

fn assert_release(what: &str, release: Duration) {
    assert!(
        release < RELEASE_BUDGET,
        "the {what} release froze the window for {release:?}"
    );
}

fn wait_for_commit(harness: &mut Harness<'_, KetchupApp>, revision: u64, what: &str) {
    let released = Instant::now();
    while harness.state().document_revision() == revision {
        assert!(
            released.elapsed() < COMMIT_BUDGET,
            "{what} never committed: {}",
            harness.state().digest
        );
        harness.step();
        std::thread::sleep(Duration::from_millis(5));
    }
    settle(harness);
}

/// Moves the pointer from `from` to `to` in `DRAG_STEPS` timed frames.
fn sweep(harness: &mut Harness<'_, KetchupApp>, from: Vec3, to: Vec3) -> (Pos2, Vec<Duration>) {
    let mut frames = Vec::new();
    let mut last = Pos2::ZERO;
    for step in 1..=DRAG_STEPS {
        let (screen, took) = move_to(
            harness,
            from + (to - from) * (step as f64 / DRAG_STEPS as f64),
        );
        frames.push(took);
        last = screen;
    }
    (last, frames)
}

/// Presses at `from`, drags to `to` and releases; every frame is timed.
fn drag(harness: &mut Harness<'_, KetchupApp>, from: Vec3, to: Vec3, what: &str) {
    let revision = harness.state().document_revision();
    move_to(harness, from + Vec3::new(5.0, 5.0, 0.0));
    let (screen, _) = move_to(harness, from);
    let press = button(harness, screen, true);
    let (last, mut frames) = sweep(harness, from, to);
    frames.push(press);
    assert_frames(what, &frames);
    assert_release(what, button(harness, last, false));
    wait_for_commit(harness, revision, what);
}

/// A click whose press and release are both timed frames.
fn timed_click(harness: &mut Harness<'_, KetchupApp>, at: Vec3, what: &str) {
    let (screen, hover) = move_to(harness, at);
    let press = button(harness, screen, true);
    assert_frames(what, &[hover, press]);
    assert_release(what, button(harness, screen, false));
}

#[test]
fn hovering_and_selecting_across_the_house_stays_interactive() {
    let _turn = one_at_a_time();
    let mut harness = harness(house());
    tool(&mut harness, AppCommand::Select);
    let rect = harness.state().camera.viewport_rect.unwrap();
    let mut frames = Vec::new();
    for step in 0..=20 {
        let t = step as f32 / 20.0;
        let position = rect.lerp_inside(Vec2::new(0.25 + 0.5 * t, 0.3 + 0.4 * t));
        frames.push(event(&mut harness, egui::Event::PointerMoved(position)));
    }
    assert_frames("hover", &frames);
    let centre = rect.center();
    event(&mut harness, egui::Event::PointerMoved(centre));
    let press = button(&mut harness, centre, true);
    assert_frames("select", &[press]);
    assert_release("select", button(&mut harness, centre, false));
    assert!(
        !harness.state().selection.occurrences.is_empty(),
        "the click selected no part of the house"
    );
}

#[test]
fn moving_a_box_beside_the_house_stays_interactive() {
    let _turn = one_at_a_time();
    let mut harness = house_with_box();
    tool(&mut harness, AppCommand::Move);
    let top = centre() + Vec3::new(0.0, 0.0, TOP);
    drag(&mut harness, top, top + Vec3::new(400.0, 0.0, 0.0), "Move");
}

#[test]
fn scaling_a_box_beside_the_house_stays_interactive() {
    let _turn = one_at_a_time();
    let mut harness = house_with_box();
    let corner = Vec3::new(END.x, END.y, TOP);
    click(&mut harness, centre() + Vec3::new(0.0, 0.0, TOP));
    tool(&mut harness, AppCommand::Scale);
    drag(
        &mut harness,
        corner,
        corner + (corner - centre()) * 0.5,
        "Scale",
    );
}

#[test]
fn rotating_a_box_beside_the_house_stays_interactive() {
    let _turn = one_at_a_time();
    let mut harness = house_with_box();
    tool(&mut harness, AppCommand::Rotate);
    let pivot = centre() + Vec3::new(0.0, 0.0, TOP);
    let revision = harness.state().document_revision();
    timed_click(&mut harness, pivot, "Rotate pivot");
    let reference = pivot + Vec3::new(300.0, 0.0, 0.0);
    timed_click(&mut harness, reference, "Rotate reference");
    let (last, frames) = sweep(&mut harness, reference, pivot + Vec3::new(0.0, 300.0, 0.0));
    assert_frames("Rotate", &frames);
    let press = button(&mut harness, last, true);
    assert_frames("Rotate", &[press]);
    assert_release("Rotate", button(&mut harness, last, false));
    wait_for_commit(&mut harness, revision, "Rotate");
}

#[test]
fn filleting_a_box_edge_beside_the_house_stays_interactive() {
    let _turn = one_at_a_time();
    let mut harness = house_with_box();
    tool(&mut harness, AppCommand::Select);
    let revision = harness.state().document_revision();
    let edge = Vec3::new(START.x, centre().y, TOP);
    timed_click(&mut harness, edge, "edge select");
    let started = Instant::now();
    tool(&mut harness, AppCommand::Fillet);
    assert_release("Fillet start", started.elapsed());
    assert!(
        harness
            .state()
            .tool_preview
            .get::<GeneralFinishPreview>()
            .is_some(),
        "the clicked edge gave no Fillet preview: {}",
        harness.state().digest
    );
    harness.state_mut().value_box.input = "20".to_owned();
    // What Enter in the focused value box does.
    let started = Instant::now();
    assert!(
        harness.state_mut().apply_value_input(),
        "{}",
        harness.state().digest
    );
    harness.step();
    assert_release("Fillet confirm", started.elapsed());
    wait_for_commit(&mut harness, revision, "Fillet");
}
