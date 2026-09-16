mod harness;

use eframe::egui::Key;
use harness::Shell;
use ketchup_app::{AppCommand, dialogs::ScriptedFileDialogs};
use ketchup_core::document::{OccurrenceId, ProfileSegment, SpatialPathSegment};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    for _ in 0..3000 {
        shell.settle();
        if shell.app().exact_render_body_count() == expected {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(shell.app().exact_render_body_count(), expected);
}

fn oval_profile(half_width: f64, half_depth: f64) -> Vec<[f64; 2]> {
    (0..32)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / 32.0;
            [half_width * angle.cos(), half_depth * angle.sin()]
        })
        .collect()
}

fn circular_profile(radius: f64) -> Vec<ProfileSegment> {
    vec![
        ProfileSegment::CircularArc {
            start_mm: [-radius, 0.0],
            end_mm: [radius, 0.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
        ProfileSegment::CircularArc {
            start_mm: [radius, 0.0],
            end_mm: [-radius, 0.0],
            center_mm: [0.0, 0.0],
            clockwise: false,
        },
    ]
}

fn helical_path(
    centre_x: f64,
    radius: f64,
    start_z: f64,
    pitch: f64,
    turns: usize,
) -> Vec<SpatialPathSegment> {
    let quarter_turns = turns * 4;
    let delta = std::f64::consts::FRAC_PI_2;
    let rise_per_radian = pitch / std::f64::consts::TAU;
    let point = |angle: f64| {
        [
            centre_x + radius * angle.cos(),
            radius * angle.sin(),
            start_z + rise_per_radian * angle,
        ]
    };
    let derivative = |angle: f64| [-radius * angle.sin(), radius * angle.cos(), rise_per_radian];
    (0..quarter_turns)
        .map(|index| {
            let start_angle = index as f64 * delta;
            let end_angle = (index + 1) as f64 * delta;
            let start_mm = point(start_angle);
            let end_mm = point(end_angle);
            let start_tangent = derivative(start_angle);
            let end_tangent = derivative(end_angle);
            SpatialPathSegment::CubicBezier {
                start_mm,
                control_1_mm: std::array::from_fn(|axis| {
                    start_mm[axis] + start_tangent[axis] * delta / 3.0
                }),
                control_2_mm: std::array::from_fn(|axis| {
                    end_mm[axis] - end_tangent[axis] * delta / 3.0
                }),
                end_mm,
            }
        })
        .collect()
}

fn oval_profile_at(centre_x: f64, half_width: f64, half_depth: f64) -> Vec<[f64; 2]> {
    oval_profile(half_width, half_depth)
        .into_iter()
        .map(|[x, y]| [centre_x + x, y])
        .collect()
}

fn ribbed_cap_profile(centre_x: f64, radius: f64) -> Vec<[f64; 2]> {
    (0..48)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / 48.0;
            let ribbed_radius = radius * (1.0 + 0.025 * (1.0 + (16.0 * angle).cos()));
            [
                centre_x + ribbed_radius * angle.cos(),
                ribbed_radius * angle.sin(),
            ]
        })
        .collect()
}

fn front_panel_profile(half_width: f64, centre_y: f64, half_depth: f64) -> Vec<[f64; 2]> {
    (0..24)
        .map(|index| {
            let angle = std::f64::consts::TAU * f64::from(index) / 24.0;
            [
                half_width * angle.cos(),
                centre_y + half_depth * angle.sin(),
            ]
        })
        .collect()
}

fn commit_selected_loft(shell: &mut Shell) {
    shell.click_menu_command("menu-model", AppCommand::Loft);
    assert!(
        shell.app().loft_preview_parameters().is_some(),
        "{}",
        shell.app().action_digest()
    );
    shell.press_key(Key::Enter);
    assert!(shell.app().latest_loft_parameters().is_some());
}

fn latest_occurrence_id(shell: &Shell) -> OccurrenceId {
    shell
        .app()
        .document_snapshot()
        .occurrences()
        .map(|occurrence| occurrence.id())
        .max_by_key(|id| id.0)
        .expect("the modeled solid must have an occurrence")
}

fn combine_occurrences(
    shell: &mut Shell,
    command: AppCommand,
    target_id: OccurrenceId,
    target_point: [f64; 3],
    tool_id: OccurrenceId,
    tool_point: [f64; 3],
) {
    shell.click_menu_command("menu-model", command);
    assert!(
        shell
            .app_mut()
            .headless_select_solid_tool_operand(target_id, false),
        "target={target_point:?} {}",
        shell.app().action_digest()
    );
    assert!(
        shell
            .app_mut()
            .headless_select_solid_tool_operand(tool_id, false),
        "tool={tool_point:?} {}",
        shell.app().action_digest()
    );
    assert!(
        shell.app().has_occurrence_operation_preview(),
        "{}",
        shell.app().action_digest()
    );
    shell.press_key(Key::Enter);
    assert!(!shell.app().has_occurrence_operation_preview());
}

fn capture_headless_window(shell: &mut Shell, path: &Path) {
    shell.settle();
    let [width, height] = shell.save_render(path);
    assert!(width >= 1200 && height >= 800);
    println!(
        "capture={} revision={} digest={} size={}x{}",
        path.display(),
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        width,
        height,
    );
}

#[test]
fn headless_asymmetric_ketchup_bottle_is_documented_from_scratch() {
    let artifact_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../artifacts/manual-bottle")
        .join(format!("documented-{}", std::process::id()));
    std::fs::create_dir_all(&artifact_dir).unwrap();
    let document_path = artifact_dir.join("manual-ketchup-bottle.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.initialize_gpu();

    shell.click_at(shell.viewport_rect().center());
    shell.press_key(Key::Delete);
    assert_eq!(shell.app().occurrence_count(), 0);

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(24.0, 11.5), 0.0),
        (oval_profile(27.5, 13.2), 4.0),
        (oval_profile(30.0, 14.5), 16.0),
        (oval_profile(30.5, 14.7), 42.0),
        (oval_profile(30.0, 14.5), 78.0),
        (oval_profile(28.5, 13.8), 96.0),
        (oval_profile(25.0, 12.5), 106.0),
        (oval_profile(20.0, 11.0), 114.0),
        (oval_profile(14.0, 9.5), 120.0),
        (oval_profile(10.5, 8.5), 125.0),
    ]));
    assert_eq!(shell.app().occurrence_count(), 1);
    shell.click_menu_command("menu-view", AppCommand::HomeView);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("01-body-sections.png"));
    commit_selected_loft(&mut shell);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .expect("the real exact worker is required");
    wait_for_exact_bodies(&mut shell, 1);
    shell.click_menu_command("menu-view", AppCommand::ViewFront);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("02-slender-body.png"));

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(7.0, 3.2), -2.0),
        (oval_profile(18.0, 8.0), 0.0),
        (oval_profile(15.0, 6.5), 3.0),
        (oval_profile(5.0, 2.4), 6.5),
    ]));
    let punt_cutter_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);
    shell.click_menu_command("menu-view", AppCommand::ViewBottom);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidSubtract,
        OccurrenceId(1),
        [22.0, 0.0, 0.0],
        punt_cutter_id,
        [0.0, 0.0, -2.0],
    );
    wait_for_exact_bodies(&mut shell, 1);
    shell.click_menu_command("menu-view", AppCommand::ViewBottom);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("03-concave-punt.png"));

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(9.5, 9.5), 122.0),
        (oval_profile(9.5, 9.5), 149.0),
    ]));
    let neck_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 2);

    assert!(shell.app_mut().create_spatial_sweep(
        circular_profile(0.65),
        helical_path(0.0, 9.8, 125.0, 6.0, 3),
    ));
    let neck_thread_id = latest_occurrence_id(&shell);
    wait_for_exact_bodies(&mut shell, 3);
    shell.click_menu_command("menu-view", AppCommand::HomeView);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("04-smooth-thread.png"));
    combine_occurrences(
        &mut shell,
        AppCommand::SolidUnion,
        neck_id,
        [0.0, 9.5, 127.0],
        neck_thread_id,
        [10.45, 0.0, 125.0],
    );
    wait_for_exact_bodies(&mut shell, 2);

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(7.3, 7.3), 120.0),
        (oval_profile(7.3, 7.3), 149.0),
    ]));
    let neck_bore_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 3);
    shell.click_menu_command("menu-view", AppCommand::ViewFront);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidSubtract,
        neck_id,
        [0.0, 9.5, 127.0],
        neck_bore_id,
        [0.0, 7.3, 148.0],
    );
    wait_for_exact_bodies(&mut shell, 2);

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(9.5, 9.5), 146.5),
        (oval_profile(9.8, 9.8), 147.5),
        (oval_profile(10.1, 10.1), 148.4),
        (oval_profile(10.0, 10.0), 149.2),
        (oval_profile(9.6, 9.6), 150.0),
    ]));
    let pouring_lip_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 3);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidUnion,
        neck_id,
        [0.0, 9.5, 147.0],
        pouring_lip_id,
        [0.0, 10.1, 148.4],
    );
    wait_for_exact_bodies(&mut shell, 2);

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile(7.3, 7.3), 146.0),
        (oval_profile(7.35, 7.35), 147.4),
        (oval_profile(7.5, 7.5), 148.3),
        (oval_profile(7.7, 7.7), 149.2),
        (oval_profile(8.0, 8.0), 151.0),
    ]));
    let pouring_bore_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 3);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidSubtract,
        neck_id,
        [0.0, 9.6, 149.5],
        pouring_bore_id,
        [0.0, 8.0, 150.5],
    );
    wait_for_exact_bodies(&mut shell, 2);
    shell.click_menu_command("menu-view", AppCommand::ViewFront);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("05-rounded-opening.png"));

    assert!(shell.app_mut().create_loft_inputs(vec![
        (front_panel_profile(12.0, 10.0, 5.5), 18.0),
        (front_panel_profile(18.0, 10.0, 5.5), 22.0),
        (front_panel_profile(22.0, 10.0, 5.5), 32.0),
        (front_panel_profile(22.0, 10.0, 5.5), 72.0),
        (front_panel_profile(19.0, 9.5, 5.3), 88.0),
        (front_panel_profile(11.0, 9.0, 5.0), 98.0),
    ]));
    assert_eq!(shell.app().occurrence_count(), 3);
    let label_id = latest_occurrence_id(&shell);
    shell.click_menu_command("menu-view", AppCommand::HomeView);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(
        &mut shell,
        &artifact_dir.join("06-front-inlay-sections.png"),
    );
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 3);
    shell.click_menu_command("menu-view", AppCommand::ViewFront);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("07-complete-front.png"));

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile_at(48.0, 9.9, 9.9), 3.0),
        (oval_profile_at(48.0, 9.9, 9.9), 38.0),
    ]));
    let cap_core_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 4);

    assert!(shell.app_mut().create_spatial_sweep(
        circular_profile(0.8),
        helical_path(48.0, 10.25, 6.0, 6.0, 3),
    ));
    let cap_thread_groove_id = latest_occurrence_id(&shell);
    wait_for_exact_bodies(&mut shell, 5);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidUnion,
        cap_core_id,
        [48.0, 9.9, 20.0],
        cap_thread_groove_id,
        [59.05, 0.0, 6.0],
    );
    wait_for_exact_bodies(&mut shell, 4);

    assert!(shell.app_mut().create_loft_inputs(vec![
        (oval_profile_at(48.0, 13.8, 13.8), 0.0),
        (ribbed_cap_profile(48.0, 14.5), 4.0),
        (ribbed_cap_profile(48.0, 14.5), 34.0),
        (oval_profile_at(48.0, 13.8, 13.8), 37.0),
    ]));
    let cap_id = latest_occurrence_id(&shell);
    commit_selected_loft(&mut shell);
    wait_for_exact_bodies(&mut shell, 5);
    shell.click_menu_command("menu-view", AppCommand::ViewFront);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    combine_occurrences(
        &mut shell,
        AppCommand::SolidSubtract,
        cap_id,
        [48.0, 14.5, 20.0],
        cap_core_id,
        [48.0, 9.9, 38.0],
    );
    wait_for_exact_bodies(&mut shell, 4);
    shell.click_menu_command("menu-view", AppCommand::ViewBottom);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("08-threaded-cap.png"));

    for (occurrence_id, color) in [
        (OccurrenceId(1), [181, 28, 25]),
        (neck_id, [181, 28, 25]),
        (label_id, [246, 220, 122]),
        (cap_id, [238, 232, 218]),
    ] {
        assert!(shell.app_mut().headless_select_occurrence(occurrence_id));
        assert!(shell.app_mut().set_selected_occurrence_color(Some(color)));
    }
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(OccurrenceId(1))
            .and_then(|occurrence| occurrence.color()),
        Some([181, 28, 25])
    );
    assert_eq!(
        shell
            .app()
            .document_snapshot()
            .occurrence(label_id)
            .and_then(|occurrence| occurrence.color()),
        Some([246, 220, 122])
    );
    shell.click_menu_command("menu-view", AppCommand::HomeView);
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);
    capture_headless_window(&mut shell, &artifact_dir.join("09-colored-bottle.png"));

    let final_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(document_path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell.settle();
    wait_for_exact_bodies(&mut shell, 4);
    assert_eq!(shell.app().canonical_digest(), final_digest);
    assert_eq!(shell.app().exact_render_body_count(), 4);
    println!("document={}", document_path.display());
}
