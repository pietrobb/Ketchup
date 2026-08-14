//! Acceptance workflows replayed offscreen, without touching the real pointer.
//!
//! Every assertion reads document state — revision, canonical digest,
//! occurrence and definition counts — because that is the thing the workflow is
//! supposed to change. Painted text is deliberately never asserted on.

mod harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, Pos2, Rect, Vec2, accesskit::Role};
use harness::{Shell, ctrl};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{AlignMode, AppCommand, GeneralFinishKind};
use ketchup_core::document::{
    BottleEdgeFinishKind, CanonicalCommand, CommandBatch, DefinitionId, DerivedIdentity, Dimension,
    DocumentStore, EvaluationIdentity, FeatureId, FeatureKind, FeatureParameterBinding,
    FeatureParameterSlot, FeatureParameterTarget, InstancePath, NodeId, OccurrenceId, PortSpec,
    RuleOutput, SlotPath, SlotSegment, Transform,
};
use ketchup_core::exact_product::{
    EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1, EXACT_BOOLEAN_SPLIT_EVALUATOR_V1,
    EXACT_CIRCLE_EVALUATOR_V1, EXACT_CIRCULAR_CUT_EVALUATOR_V1, EXACT_LOFT_EVALUATOR_V1,
    EXACT_PLANAR_OFFSET_EVALUATOR_V1, EXACT_SWEEP_EVALUATOR_V1,
};
use ketchup_core::graph::{EvaluationStatus, EvaluatorNodeKind};
use ketchup_core::persistence;
use ketchup_interaction::{Axis, ElementId, LocaleCatalog, SnapKind, Vec3};

const PARAMETRIC_PROFILE: FeatureId = FeatureId(10);
const PARAMETRIC_RULE: NodeId = NodeId(302);
const PARAMETRIC_DEPENDENT: NodeId = NodeId(303);
const PARAMETRIC_UNRELATED_SOURCE: NodeId = NodeId(305);
const PARAMETRIC_UNRELATED: NodeId = NodeId(306);

fn dimension(value: &str) -> Dimension {
    Dimension::from_decimal(value).unwrap()
}

fn write_parametric_fixture(path: &Path) {
    let width_output = SlotSegment::new(PARAMETRIC_RULE, "dimensions", "profile_width").unwrap();
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(1),
                name: "Parametric box".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: PARAMETRIC_PROFILE,
                definition_id: DefinitionId(1),
                name: "Rectangle".to_owned(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[0.0, 0.0], [20.0, 0.0], [20.0, 30.0], [0.0, 30.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(1),
                name: "Extrusion".to_owned(),
                kind: FeatureKind::Extrusion {
                    profile: PARAMETRIC_PROFILE,
                    height: dimension("10"),
                },
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(20),
                definition_id: DefinitionId(1),
                name: "Parametric box #1".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tag: None,
                visible: true,
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: NodeId(301),
                name: "Width source".to_owned(),
                dimension: dimension("20"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateRuleNode {
                id: PARAMETRIC_RULE,
                name: "Driven width".to_owned(),
                expression: "$301".to_owned(),
                input_ports: vec![PortSpec::number("width").unwrap()],
                output_ports: vec![PortSpec::number("dimensions").unwrap()],
                outputs: vec![RuleOutput::new(width_output.clone(), vec![]).unwrap()],
                override_parameters: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: PARAMETRIC_DEPENDENT,
                name: "Width audit".to_owned(),
                expression: "$302 + 1".to_owned(),
            },
            CanonicalCommand::CreateEvaluatorNode {
                id: PARAMETRIC_UNRELATED_SOURCE,
                name: "Unrelated source".to_owned(),
                dimension: dimension("7"),
                dependencies: vec![],
            },
            CanonicalCommand::CreateExpressionNode {
                id: PARAMETRIC_UNRELATED,
                name: "Unrelated result".to_owned(),
                expression: "$305 * 3".to_owned(),
            },
            CanonicalCommand::UpsertFeatureParameterBinding(FeatureParameterBinding {
                target: FeatureParameterTarget {
                    feature_id: PARAMETRIC_PROFILE,
                    slot: FeatureParameterSlot::ProfileWidth,
                },
                derived_from: DerivedIdentity::new(
                    PARAMETRIC_RULE,
                    SlotPath::new(vec![width_output]).unwrap(),
                )
                .unwrap(),
            }),
        ]))
        .unwrap();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::RecomputeFeatureParameters {
                identity: EvaluationIdentity::default(),
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn exact_worker_path() -> PathBuf {
    let name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(name);
    if colocated.is_file() {
        colocated
    } else {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/debug")
            .join(name)
    }
}

fn wait_for_one_exact_body(shell: &mut Shell) {
    for _ in 0..100 {
        shell.settle();
        if shell.app().exact_render_body_count() == 1 {
            return;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(shell.app().exact_render_body_count(), 1);
}

fn replace_parameter_expression(shell: &mut Shell, expression: &str) {
    let input = shell.catalog().text("parameters-expression");
    let apply = shell.catalog().text("parameters-apply");
    shell.focus_text_input(&input);
    shell.key(Key::A, ctrl());
    shell.type_text(expression);
    shell.click_row(&apply);
}

#[test]
fn the_designed_shell_lays_itself_out_without_a_window() {
    let shell = Shell::new();

    assert!(
        shell.viewport_rect().area() > 0.0,
        "the viewport must be laid out"
    );
    assert!(
        shell.offers(AppCommand::Select),
        "the tool rail must offer Select under an accessible name, not a glyph"
    );
    assert!(
        shell.offers(AppCommand::Move),
        "the tool rail must offer Move under an accessible name, not a glyph"
    );
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn localized_shells_fit_and_support_screen_reader_keyboard_focus() {
    let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0));
    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert!(shell.viewport_rect().area() > 0.0, "{locale} viewport");

        for key in [
            "menu-file",
            "menu-edit",
            "menu-view",
            "menu-draw",
            "menu-tools",
            "menu-model",
            "menu-window",
            "menu-help",
        ] {
            let rect = shell.menu_rect(key);
            assert!(rect.area() > 0.0, "{locale} {key} has no layout box");
            assert!(
                screen.contains(rect.min) && screen.contains(rect.max),
                "{locale} {key} overflows the 1600 x 1000 acceptance viewport: {rect:?}"
            );
        }

        let visible_rects = shell.visible_accesskit_rects();
        assert!(visible_rects.len() > 20, "{locale} accessibility tree");
        for (node, rect) in visible_rects {
            let frame_tolerance = 1.5;
            assert!(
                rect.min.x >= screen.min.x - frame_tolerance
                    && rect.min.y >= screen.min.y - frame_tolerance
                    && rect.max.x <= screen.max.x + frame_tolerance
                    && rect.max.y <= screen.max.y + frame_tolerance,
                "{locale} publishes {node} outside the acceptance viewport: {rect:?}"
            );
        }

        for command in [
            AppCommand::Select,
            AppCommand::Rectangle,
            AppCommand::Circle,
            AppCommand::Arc,
            AppCommand::PushPull,
            AppCommand::Move,
            AppCommand::Measure,
            AppCommand::Orbit,
            AppCommand::Pan,
        ] {
            assert!(
                shell.offers(command),
                "{locale} must expose {command:?} by its localized AccessKit name"
            );
        }

        shell.focus_command(AppCommand::Rectangle);
        assert!(
            shell.command_is_focused(AppCommand::Rectangle),
            "{locale} AccessKit focus action must reach Rectangle"
        );
        let expected_digest = shell.catalog().format(
            "digest-tool-active",
            &BTreeMap::from([("tool", shell.catalog().text("tool-rectangle"))]),
        );
        shell.press_key(Key::Enter);
        assert_eq!(shell.app().action_digest(), expected_digest, "{locale}");
    }
}

#[test]
fn parameter_expression_recomputes_dependents_atomically_and_round_trips_through_the_shell() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = directory.path().join("parametric-fixture.ketchup");
    let saved = directory.path().join("parametric-saved.ketchup");
    write_parametric_fixture(&fixture);
    let dialogs = ScriptedFileDialogs::new()
        .queue_open(&fixture)
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell.click_menu_command("menu-file", AppCommand::Open);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [20.0, 30.0, 10.0]]]
    );

    let initial_revision = shell.app().document_revision();
    let initial_digest = shell.app().canonical_digest();
    assert!(!shell.app().can_undo());
    replace_parameter_expression(&mut shell, "$301 * 2");

    let changed_revision = shell.app().document_revision();
    let changed_digest = shell.app().canonical_digest();
    assert_eq!(changed_revision, initial_revision + 1);
    assert_ne!(changed_digest, initial_digest);
    assert_eq!(
        shell.app().parameter_last_recomputed_nodes(),
        &BTreeSet::from([PARAMETRIC_RULE, PARAMETRIC_DEPENDENT])
    );
    assert!(
        !shell
            .app()
            .parameter_last_recomputed_nodes()
            .contains(&PARAMETRIC_UNRELATED_SOURCE)
    );
    assert!(
        !shell
            .app()
            .parameter_last_recomputed_nodes()
            .contains(&PARAMETRIC_UNRELATED)
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .feature(PARAMETRIC_PROFILE)
            .unwrap()
            .kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
    ));
    let report = shell
        .app()
        .document_snapshot()
        .evaluate(&EvaluationIdentity::default())
        .unwrap();
    assert!(matches!(
        report.node(PARAMETRIC_DEPENDENT).unwrap().status,
        EvaluationStatus::Evaluated(value) if (value - 41.0).abs() < f64::EPSILON
    ));
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    replace_parameter_expression(&mut shell, "(");
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    replace_parameter_expression(&mut shell, "$303");
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert_eq!(shell.app().canonical_digest(), changed_digest);

    let input = shell.catalog().text("parameters-expression");
    let apply = shell.catalog().text("parameters-apply");
    shell.focus_text_input(&input);
    shell.key(Key::A, ctrl());
    shell.type_text("$301 * 3");
    shell.click_command(AppCommand::Circle);
    shell.click_at(
        shell
            .app()
            .viewport_position(Vec3::new(10.0, 15.0, 10.0))
            .unwrap(),
    );
    shell.click_at(
        shell
            .app()
            .viewport_position(Vec3::new(15.0, 15.0, 10.0))
            .unwrap(),
    );
    let intervening_revision = shell.app().document_revision();
    let intervening_digest = shell.app().canonical_digest();
    assert_eq!(intervening_revision, changed_revision + 1);
    shell.click_row(&apply);
    assert_eq!(shell.app().document_revision(), intervening_revision);
    assert_eq!(shell.app().canonical_digest(), intervening_digest);
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-parameter-stale")
    );
    assert!(matches!(
        shell
            .app()
            .document_snapshot()
            .evaluator_node(PARAMETRIC_RULE)
            .unwrap()
            .kind(),
        EvaluatorNodeKind::Rule { source, .. } if source == "$301 * 2"
    ));
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), changed_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), initial_digest);
    assert!(
        !shell.app().can_undo(),
        "the valid edit must be one undo step"
    );
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [20.0, 30.0, 10.0]]]
    );
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(saved.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().document_revision(), changed_revision);
    assert!(!shell.app().can_undo());
    let reopened = shell.app().document_snapshot();
    assert!(matches!(
        reopened.evaluator_node(PARAMETRIC_RULE).unwrap().kind(),
        EvaluatorNodeKind::Rule { source, .. } if source == "$301 * 2"
    ));
    assert!(matches!(
        reopened.feature(PARAMETRIC_PROFILE).unwrap().kind(),
        FeatureKind::Profile { points_mm }
            if points_mm == &vec![[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]
    ));
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_one_exact_body(&mut shell);
    assert_eq!(
        shell.app().exact_render_bounds(),
        vec![[[0.0, 0.0, 0.0], [40.0, 30.0, 10.0]]]
    );

    for (locale, catalog) in [
        ("en-US", LocaleCatalog::english()),
        ("sk-SK", LocaleCatalog::slovak()),
        ("pseudo", LocaleCatalog::pseudo()),
    ] {
        let dialogs = ScriptedFileDialogs::new()
            .queue_open(&fixture)
            .always_discard();
        let mut localized = Shell::with_catalog_and_dialogs(catalog, dialogs);
        localized.click_menu_command("menu-file", AppCommand::Open);
        let selector = localized.catalog().text("parameters-node");
        let input = localized.catalog().text("parameters-expression");
        let apply = localized.catalog().text("parameters-apply");
        assert!(
            localized.has_role_and_label(Role::ComboBox, &selector),
            "{locale} selector"
        );
        assert!(
            localized.has_role_and_label(Role::TextInput, &input),
            "{locale} input"
        );
        assert!(
            localized.has_role_and_label(Role::Button, &apply),
            "{locale} apply"
        );
        localized.focus_combo_box(&selector);
        localized.focus_text_input(&input);
    }
}

#[test]
fn every_tool_in_the_rail_is_reachable_by_its_accessible_name() {
    let shell = Shell::new();

    for command in [
        AppCommand::Select,
        AppCommand::Line,
        AppCommand::Rectangle,
        AppCommand::Circle,
        AppCommand::Arc,
        AppCommand::PushPull,
        AppCommand::Move,
        AppCommand::Measure,
        AppCommand::Orbit,
        AppCommand::Pan,
    ] {
        assert!(
            shell.offers(command),
            "{command:?} paints an icon and must still expose an accessible name"
        );
    }
}

#[test]
fn circle_center_radius_is_exact_snapped_undoable_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("circles.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let center = Vec3::new(35.0, 25.0, 20.0);
    let center_screen = shell.app().viewport_position(center).unwrap();
    let radial_screen = shell
        .app()
        .viewport_position(Vec3::new(55.0, 25.0, 20.0))
        .unwrap();

    shell.click_command(AppCommand::Circle);
    shell.click_at(center_screen);
    shell.move_pointer(radial_screen);

    let (preview_center, preview_radius) = shell.app().circle_preview_geometry().unwrap();
    assert!(
        preview_center.distance(center) < 1.0e-5,
        "preview center {preview_center:?}, expected {center:?}"
    );
    let center = preview_center;
    assert!((preview_radius - 20.0).abs() < 1.0e-5);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("12.5");
    shell.press_key(Key::Enter);

    let first_digest = shell.app().canonical_digest();
    let (committed_center, committed_radius) = shell.app().latest_circle_geometry().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert!(committed_center.distance(center) < 1.0e-6);
    assert!((committed_radius - 12.5).abs() < 1.0e-12);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().circle_profile_count(), 0);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);

    shell.click_command(AppCommand::Circle);
    let center_screen = shell.app().viewport_position(center).unwrap();
    shell.click_at(center_screen + Vec2::new(3.0, 0.0));
    let (snapped_center, _) = shell.app().circle_preview_geometry().unwrap();
    assert!(
        snapped_center.distance(center) < 1.0e-6,
        "existing circle centre snap"
    );

    let endpoint = Vec3::new(100.0, 60.0, 20.0);
    let endpoint_screen = shell.app().viewport_position(endpoint).unwrap();
    let viewport_center = shell.viewport_rect().center();
    let endpoint_screen = endpoint_screen + (viewport_center - endpoint_screen).normalized() * 2.0;
    shell.move_pointer(endpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.click_at(endpoint_screen);

    let expected_radius = (endpoint.x - center.x).hypot(endpoint.y - center.y);
    let (second_center, second_radius) = shell.app().latest_circle_geometry().unwrap();
    assert_eq!(shell.app().circle_profile_count(), 2);
    assert!(second_center.distance(center) < 1.0e-6);
    assert!((second_radius - expected_radius).abs() < 1.0e-12);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().circle_profile_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().circle_profile_count(), 2);
    assert_eq!(
        shell.app().latest_circle_geometry(),
        Some((second_center, second_radius))
    );
}

#[test]
fn arc_endpoint_bulge_is_exact_snapped_undoable_and_persistent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("arcs.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let circle_center = Vec3::new(35.0, 25.0, 20.0);
    let circle_radius = 12.5;
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(circle_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(circle_center + Vec3::new(circle_radius, 0.0, 0.0))
            .unwrap(),
    );
    assert_eq!(shell.app().circle_profile_count(), 1);
    let (circle_center, circle_radius) = shell.app().latest_circle_geometry().unwrap();

    let before_arc_revision = shell.app().document_revision();
    let before_arc_digest = shell.app().canonical_digest();
    shell.click_command(AppCommand::Arc);
    let center_screen = shell.app().viewport_position(circle_center).unwrap();
    shell.move_pointer(center_screen + Vec2::new(3.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Center));
    shell.click_at(center_screen + Vec2::new(3.0, 0.0));

    let midpoint = Vec3::new(50.0, 0.0, 20.0);
    let midpoint_screen = shell.app().viewport_position(midpoint).unwrap();
    shell.move_pointer(midpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Midpoint));
    shell.click_at(midpoint_screen);

    let chord = midpoint - circle_center;
    let chord_length = chord.x.hypot(chord.y);
    let chord_midpoint = (circle_center + midpoint) * 0.5;
    let normal = Vec3::new(-chord.y / chord_length, chord.x / chord_length, 0.0);
    let preview_bulge = chord_midpoint + normal * 8.0;
    shell.move_pointer(shell.app().viewport_position(preview_bulge).unwrap());
    let (preview_start, preview_end, _, _) = shell.app().arc_preview_geometry().unwrap();
    assert!(preview_start.distance(circle_center) < 1.0e-6);
    assert!(preview_end.distance(midpoint) < 1.0e-6);
    assert_eq!(shell.app().document_revision(), before_arc_revision);
    assert_eq!(shell.app().canonical_digest(), before_arc_digest);

    shell.type_text("12.5");
    shell.press_key(Key::Enter);
    let first_arc_digest = shell.app().canonical_digest();
    let (start, end, center, _) = shell.app().latest_arc_geometry().unwrap();
    let exact_bulge = chord_midpoint + normal * 12.5;
    assert_eq!(shell.app().document_revision(), before_arc_revision + 1);
    assert_eq!(shell.app().arc_profile_count(), 1);
    assert!(start.distance(circle_center) < 1.0e-6);
    assert!(end.distance(midpoint) < 1.0e-6);
    assert!((center.distance(start) - center.distance(exact_bulge)).abs() < 1.0e-8);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_arc_digest);
    assert_eq!(shell.app().arc_profile_count(), 0);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), first_arc_digest);
    assert_eq!(shell.app().arc_profile_count(), 1);

    shell.click_command(AppCommand::Arc);
    let endpoint = Vec3::new(0.0, 0.0, 20.0);
    let endpoint_screen = shell.app().viewport_position(endpoint).unwrap();
    let endpoint_screen =
        endpoint_screen + (shell.viewport_rect().center() - endpoint_screen).normalized() * 2.0;
    shell.move_pointer(endpoint_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.click_at(endpoint_screen);

    let delta = endpoint - circle_center;
    let distance_squared = delta.x * delta.x + delta.y * delta.y;
    let base_scale = circle_radius * circle_radius / distance_squared;
    let tangent_scale = circle_radius * (distance_squared - circle_radius * circle_radius).sqrt()
        / distance_squared;
    let tangent = Vec3::new(
        circle_center.x + delta.x * base_scale - delta.y * tangent_scale,
        circle_center.y + delta.y * base_scale + delta.x * tangent_scale,
        20.0,
    );
    let tangent_screen = shell.app().viewport_position(tangent).unwrap();
    shell.move_pointer(tangent_screen);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Tangent));
    shell.click_at(tangent_screen);

    let tangent_chord = tangent - endpoint;
    let tangent_length = tangent_chord.x.hypot(tangent_chord.y);
    let tangent_midpoint = (endpoint + tangent) * 0.5;
    let tangent_normal = Vec3::new(
        -tangent_chord.y / tangent_length,
        tangent_chord.x / tangent_length,
        0.0,
    );
    shell.click_at(
        shell
            .app()
            .viewport_position(tangent_midpoint + tangent_normal * 6.0)
            .unwrap(),
    );
    assert_eq!(shell.app().arc_profile_count(), 2);
    let (_, snapped_tangent, _, _) = shell.app().latest_arc_geometry().unwrap();
    assert!(snapped_tangent.distance(tangent) < 1.0e-6);

    let persisted_digest = shell.app().canonical_digest();
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert_eq!(shell.app().arc_profile_count(), 0);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), persisted_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert_eq!(shell.app().arc_profile_count(), 2);
}

#[test]
fn circle_push_pull_creates_an_exact_cylinder_and_circular_hole_with_one_step_history() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("circle-push-pull.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let cylinder_center = Vec3::new(75.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(cylinder_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(cylinder_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    assert_eq!(shell.app().circle_profile_count(), 1);
    assert_eq!(shell.app().active_box_count(), 2);
    let cylinder_selection = shell.app().selected_reference().unwrap();
    assert_eq!(
        cylinder_selection.instance_path.root_occurrence(),
        OccurrenceId(2)
    );
    assert_eq!(
        cylinder_selection.element,
        ketchup_interaction::ElementId::Face {
            axis: Axis::Z,
            side: ketchup_interaction::Side::Maximum,
        }
    );
    assert!(shell.app().occurrence_box_geometry(2).is_some());
    let cylinder_profile_digest = shell.app().canonical_digest();
    let cylinder_profile_revision = shell.app().document_revision();

    shell.click_command(AppCommand::PushPull);
    assert_eq!(
        shell
            .app()
            .selected_reference()
            .unwrap()
            .instance_path
            .root_occurrence(),
        OccurrenceId(2)
    );
    shell.app_mut().set_push_pull_distance_input("30");
    assert!(shell.app_mut().start_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_CIRCLE_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), cylinder_profile_revision);
    assert_eq!(shell.app().canonical_digest(), cylinder_profile_digest);
    shell.app_mut().cancel_preview();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("30");
    shell.press_key(Key::Enter);
    let cylinder_digest = shell.app().canonical_digest();
    assert_eq!(
        shell.app().document_revision(),
        cylinder_profile_revision + 1
    );
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap().1.z, 30.0);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), cylinder_profile_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), cylinder_digest);

    let hole_center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(hole_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(hole_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let hole_profile_digest = shell.app().canonical_digest();
    let hole_profile_revision = shell.app().document_revision();
    assert_eq!(shell.app().active_box_count(), 3);

    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    assert!(shell.app_mut().start_preview());
    assert!(shell.app().has_occurrence_operation_preview());
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_CIRCULAR_CUT_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), hole_profile_revision);
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    shell.app_mut().cancel_preview();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("-20");
    shell.press_key(Key::Enter);
    let hole_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), hole_profile_revision + 1);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().circle_profile_count(), 2);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    assert_eq!(shell.app().active_box_count(), 3);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_digest);
    assert_eq!(shell.app().active_box_count(), 2);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), hole_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    assert_eq!(shell.app().circle_profile_count(), 2);
}

#[test]
fn general_revolve_selects_an_axis_previews_exact_angle_and_commits_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("general-revolve.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);
    assert!(shell.app_mut().create_closed_polyline(vec![
        [120.0, 0.0],
        [140.0, 0.0],
        [140.0, 30.0],
        [120.0, 30.0],
    ]));
    shell.settle();

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Revolve),
        "the selected closed profile must expose Revolve through AccessKit"
    );
    shell.click_command(AppCommand::Revolve);

    let axis_start = shell
        .app()
        .viewport_position(Vec3::new(110.0, 0.0, 0.0))
        .unwrap();
    let axis_end = shell
        .app()
        .viewport_position(Vec3::new(110.0, 30.0, 0.0))
        .unwrap();
    shell.click_at(axis_start);
    assert!(!shell.app().has_revolve_preview());
    shell.click_at(axis_end);
    let (axis_start_mm, axis_end_mm, angle_degrees) =
        shell.app().revolve_preview_parameters().unwrap();
    assert!((axis_start_mm[0] - 110.0).abs() < 1.0e-4);
    assert!(axis_start_mm[1].abs() < 1.0e-4);
    assert!((axis_end_mm[0] - 110.0).abs() < 1.0e-4);
    assert!((axis_end_mm[1] - 30.0).abs() < 1.0e-4);
    assert_eq!(angle_degrees, 360.0);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("225");
    assert_eq!(
        shell.app().revolve_preview_parameters(),
        Some((axis_start_mm, axis_end_mm, 225.0))
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_revolve_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        (committed.1, committed.2, committed.3),
        (axis_start_mm, axis_end_mm, 225.0)
    );
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_revolve_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_revolve_parameters(), Some(committed));

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().latest_revolve_parameters().is_none());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_revolve_parameters(), Some(committed));
}

#[test]
fn planar_offset_previews_signed_exact_bounds_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_closed_polyline(vec![
        [120.0, 0.0],
        [220.0, 0.0],
        [220.0, 60.0],
        [120.0, 60.0],
    ]));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::PlanarOffset),
        "the selected rectangular profile must expose localized Planar Offset through AccessKit"
    );
    shell.click_command(AppCommand::PlanarOffset);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("-31");
    assert!(shell.app().planar_offset_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.key(Key::A, ctrl());
    shell.type_text("-7.5");
    let preview = shell.app().planar_offset_preview_parameters().unwrap();
    assert_eq!(preview.1, -7.5);
    assert_eq!(preview.2, [[127.5, 7.5, 0.0], [212.5, 52.5, 0.0]]);
    assert_eq!(
        shell.app().planar_offset_preview_exact_evaluator(),
        Some(EXACT_PLANAR_OFFSET_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Enter);
    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_planar_offset_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(committed.1, preview.0);
    assert_eq!(committed.2, -7.5);
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_planar_offset_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(
        shell.app().latest_planar_offset_parameters(),
        Some(committed)
    );
}

#[test]
fn sweep_previews_exact_profile_path_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_sweep_inputs(
        vec![[-5.0, -10.0], [5.0, -10.0], [5.0, 10.0], [-5.0, 10.0]],
        [0.0, 0.0],
        [0.0, 125.0],
    ));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Sweep),
        "the selected profile/path definition must expose localized Sweep through AccessKit"
    );
    shell.click_command(AppCommand::Sweep);
    let preview = shell.app().sweep_preview_parameters().unwrap();
    assert_eq!(preview.2, [[-5.0, 0.0, -10.0], [5.0, 125.0, 10.0]]);
    assert_eq!(preview.3, 25_000.0);
    assert_eq!(
        shell.app().sweep_preview_exact_evaluator(),
        Some(EXACT_SWEEP_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Escape);
    assert!(shell.app().sweep_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.click_menu_command("menu-model", AppCommand::Sweep);
    assert_eq!(shell.app().sweep_preview_parameters(), Some(preview));
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_sweep_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!((committed.1, committed.2), (preview.0, preview.1));
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_sweep_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_sweep_parameters(), Some(committed));
}

#[test]
fn spline_profiles_preview_exact_loft_and_commit_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_loft_inputs(vec![
        (
            vec![[-20.0, -10.0], [20.0, -10.0], [20.0, 10.0], [-20.0, 10.0]],
            0.0,
        ),
        (
            vec![[-10.0, -5.0], [10.0, -5.0], [10.0, 5.0], [-10.0, 5.0]],
            80.0,
        ),
    ]));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.open_menu("menu-model");
    assert!(
        shell.offers(AppCommand::Loft),
        "the selected ordered spline profiles must expose localized Loft through AccessKit"
    );
    shell.click_command(AppCommand::Loft);
    let preview = shell.app().loft_preview_parameters().unwrap();
    assert_eq!(
        preview
            .0
            .iter()
            .map(|section| section.1)
            .collect::<Vec<_>>(),
        [0.0, 80.0]
    );
    assert_eq!(preview.1, [[-20.0, -10.0, 0.0], [20.0, 10.0, 80.0]]);
    assert_eq!(preview.2, 8);
    assert_eq!(
        shell.app().loft_preview_exact_evaluator(),
        Some(EXACT_LOFT_EVALUATOR_V1)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.press_key(Key::Escape);
    assert!(shell.app().loft_preview_parameters().is_none());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.click_menu_command("menu-model", AppCommand::Loft);
    assert_eq!(shell.app().loft_preview_parameters(), Some(preview.clone()));
    shell.press_key(Key::Enter);

    let committed_digest = shell.app().canonical_digest();
    let committed = shell.app().latest_loft_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(committed.1, preview.0);
    assert_ne!(committed_digest, before_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app().latest_loft_parameters().is_none());
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    assert_eq!(shell.app().latest_loft_parameters(), Some(committed));
}

#[test]
fn general_shell_fillet_and_chamfer_preview_exact_stable_selections_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("general-shell-finish.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);

    shell.click_at(shell.top_face_centre(1));
    assert!(matches!(
        shell.app().selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::Z,
            side: ketchup_interaction::Side::Maximum,
        }
    ));
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    shell.open_menu("menu-model");
    assert!(shell.offers(AppCommand::Shell));
    shell.click_command(AppCommand::Shell);
    assert_eq!(
        shell.app().general_finish_preview_parameters(),
        Some((
            ketchup_core::document::FeatureId(2),
            "extrusion.top".to_owned(),
            GeneralFinishKind::Shell,
            2.0,
        ))
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    shell.type_text("2.5");
    assert_eq!(
        shell.app().general_finish_preview_parameters().unwrap().3,
        2.5
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let shell_digest = shell.app().canonical_digest();
    let shell_parameters = shell.app().latest_general_shell_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell_parameters.1, "extrusion.top");
    assert_eq!(shell_parameters.2, 2.5);

    assert!(matches!(
        shell.app().selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::Z,
            side: ketchup_interaction::Side::Maximum,
        }
    ));

    shell.open_menu("menu-model");
    assert!(shell.offers(AppCommand::Fillet));
    assert!(shell.offers(AppCommand::Chamfer));
    shell.click_command(AppCommand::Fillet);
    assert_eq!(
        shell.app().general_finish_preview_parameters(),
        Some((
            shell_parameters.0,
            "shell.edge.top-east".to_owned(),
            GeneralFinishKind::Fillet,
            1.0,
        ))
    );
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    shell.type_text("1.25");
    shell.press_key(Key::Enter);

    let fillet_digest = shell.app().canonical_digest();
    let fillet = shell.app().latest_general_edge_finish_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    assert_eq!(fillet.1, "shell.edge.top-east");
    assert_eq!(fillet.2, BottleEdgeFinishKind::Fillet);
    assert_eq!(fillet.3, 1.25);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    assert!(
        shell
            .app()
            .latest_general_edge_finish_parameters()
            .is_none()
    );

    shell.click_menu_command("menu-model", AppCommand::Chamfer);
    assert_eq!(
        shell.app().general_finish_preview_parameters().unwrap().2,
        GeneralFinishKind::Chamfer
    );
    shell.type_text("0.75");
    shell.press_key(Key::Enter);
    let chamfer_digest = shell.app().canonical_digest();
    let chamfer = shell.app().latest_general_edge_finish_parameters().unwrap();
    assert_ne!(chamfer_digest, fillet_digest);
    assert_eq!(chamfer.2, BottleEdgeFinishKind::Chamfer);
    assert_eq!(chamfer.3, 0.75);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().latest_general_shell_parameters().is_none());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);
    assert_eq!(
        shell.app().latest_general_shell_parameters(),
        Some(shell_parameters)
    );
    assert_eq!(
        shell.app().latest_general_edge_finish_parameters(),
        Some(chamfer)
    );
}

#[test]
fn clicking_the_viewport_selects_the_occurrence() {
    let mut shell = Shell::new();
    let centre = shell.viewport_rect().center();

    shell.click_at(centre);

    assert_eq!(
        shell.app().selected_occurrence_count(),
        1,
        "clicking geometry must select exactly the occurrence under the pointer"
    );
}

#[test]
fn viewport_snap_hysteresis_acquires_retains_and_releases_an_endpoint() {
    let mut shell = Shell::new();
    let endpoint = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();

    shell.move_pointer(endpoint + eframe::egui::Vec2::new(7.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(10.0, 0.0));
    assert_eq!(
        shell.app().hovered_snap_kind(),
        Some(SnapKind::Endpoint),
        "the release radius must prevent flicker outside the acquire radius"
    );
    shell.move_pointer(endpoint + eframe::egui::Vec2::new(14.0, 0.0));
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Face));
}

#[test]
fn tab_cycles_overlapping_occurrences_and_click_selects_the_visible_choice() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Dock);
    shell.settle();
    let centre = shell.viewport_rect().center();
    shell.click_at(centre);
    assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.move_pointer(centre);

    assert_eq!(shell.app().hovered_overlap_choice(), Some((0, 2)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(1))
    );
    shell.press_key(Key::Tab);
    assert_eq!(shell.app().hovered_overlap_choice(), Some((1, 2)));
    assert_eq!(
        shell.app().hovered_selection().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
    shell.click_at(centre);
    assert_eq!(
        shell.app().selected_reference().unwrap().instance_path,
        InstancePath::root(OccurrenceId(2))
    );
}

#[test]
fn snapped_measurement_updates_the_value_box_without_mutating_the_document() {
    let mut shell = Shell::new();
    shell.click_command(AppCommand::Measure);
    let left = shell
        .app()
        .viewport_position(Vec3::new(0.0, 0.0, 20.0))
        .unwrap();
    let right = shell
        .app()
        .viewport_position(Vec3::new(100.0, 0.0, 20.0))
        .unwrap();
    let centre = shell.viewport_rect().center();
    let left = left + (centre - left).normalized() * 2.0;
    let right = right + (centre - right).normalized() * 2.0;
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_at(left);
    shell.move_pointer(right);
    assert_eq!(shell.app().hovered_snap_kind(), Some(SnapKind::Endpoint));
    assert_eq!(
        shell.app().hovered_snap_position(),
        Some(Vec3::new(100.0, 0.0, 20.0))
    );
    shell.click_at(right);

    assert_eq!(
        shell.app().measured_points(),
        Some((Vec3::new(0.0, 0.0, 20.0), Vec3::new(100.0, 0.0, 20.0),))
    );
    assert_eq!(shell.app().value_input(), "100");
    assert_eq!(shell.app().measured_distance_mm(), Some(100.0));
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn a_click_outside_geometry_clears_the_selection() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    assert_eq!(shell.app().selected_occurrence_count(), 1);

    shell.click_at(rect.left_top() + eframe::egui::Vec2::new(12.0, 12.0));

    assert_eq!(shell.app().selected_occurrence_count(), 0);
}

#[test]
fn copying_an_occurrence_shares_one_definition() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    let before = shell.app().document_revision();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();

    assert_eq!(shell.app().active_box_count(), 2, "a second occurrence");
    assert_eq!(
        shell.app().definition_count(),
        1,
        "Copy must reuse the definition instead of cloning it"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one gesture must produce exactly one canonical revision"
    );
}

#[test]
fn exact_align_previews_then_commits_one_transform_batch_with_undo_redo() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::X,
        AlignMode::Maximum,
    ));
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(0.0, 25.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let aligned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some((Vec3::new(0.0, 25.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(shell.app().definition_count(), 1);
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), aligned_digest);
}

#[test]
fn exact_align_preview_fails_closed_after_the_document_changes() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    assert!(shell.app_mut().preview_align_occurrences(
        OccurrenceId(2),
        OccurrenceId(1),
        Axis::Y,
        AlignMode::Center,
    ));
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();

    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
}

#[test]
fn exact_linear_pattern_previews_then_commits_one_shared_definition_batch() {
    let mut shell = Shell::new();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::X, 125.0, 4,)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(125.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(4)),
        Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let patterned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 4);
    assert_eq!(shell.app().definition_count(), 1);
    for occurrence_id in 1..=4 {
        assert_eq!(
            shell
                .app()
                .occurrence_definition_id(OccurrenceId(occurrence_id)),
            shell.app().occurrence_definition_id(OccurrenceId(1)),
        );
    }
    assert_eq!(
        shell.app().occurrence_box_geometry(4),
        Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), patterned_digest);
    assert_eq!(shell.app().active_box_count(), 4);
}

#[test]
fn exact_linear_pattern_rejects_invalid_input_and_stale_confirmation() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let before_digest = shell.app().canonical_digest();
    assert!(
        !shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, f64::NAN, 3,)
    );
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), before_digest);

    assert!(
        shell
            .app_mut()
            .preview_linear_pattern(OccurrenceId(1), Axis::Y, -80.0, 3,)
    );
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, 10.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn undo_and_redo_return_the_document_to_identical_canonical_states() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let composed = shell.app().canonical_digest();
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    let copied = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_once = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_once = shell.app().canonical_digest();

    shell.key(Key::Z, ctrl());
    let undone_twice = shell.app().canonical_digest();
    shell.key(Key::Y, ctrl());
    let redone_twice = shell.app().canonical_digest();

    assert_eq!(
        undone_once, composed,
        "Undo must restore the previous state"
    );
    assert_eq!(redone_once, copied, "Redo must restore the copied state");
    assert_eq!(undone_once, undone_twice, "Undo must be reproducible");
    assert_eq!(redone_once, redone_twice, "Redo must be reproducible");
    assert_ne!(undone_once, redone_once);
}

#[test]
fn a_viewport_drag_in_move_commits_exactly_one_canonical_batch() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(rect.center());
    shell.click_command(AppCommand::Move);

    let before = shell.app().document_revision();
    let from = rect.center();
    shell.drag(from, from + eframe::egui::Vec2::new(120.0, 0.0));

    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "one completed drag must produce exactly one canonical revision"
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Move must not create geometry"
    );
}

#[test]
fn cut_through_commits_one_canonical_undo_step_from_the_headless_shell() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::CutThrough);
    let start = shell
        .app()
        .viewport_position(Vec3::new(20.0, 15.0, 20.0))
        .unwrap();
    let end = shell
        .app()
        .viewport_position(Vec3::new(50.0, 35.0, 20.0))
        .unwrap();
    shell.click_at(start);
    shell.click_at(end);

    let cut_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_ne!(cut_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), cut_digest);
}

#[test]
fn pocket_previews_exact_depth_and_commits_from_the_headless_shell() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::Pocket);
    let start = shell
        .app()
        .viewport_position(Vec3::new(20.0, 15.0, 20.0))
        .unwrap();
    let end = shell
        .app()
        .viewport_position(Vec3::new(50.0, 35.0, 20.0))
        .unwrap();
    shell.click_at(start);
    shell.click_at(end);

    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::A, ctrl());
    shell.type_text("8");
    shell.press_key(Key::Enter);

    let pocket_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_ne!(pocket_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), pocket_digest);
}

#[test]
fn solid_union_previews_and_ctrl_keeps_the_tool_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    assert!(shell.app_mut().move_selected(Vec3::new(0.0, -35.0, 0.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidUnion);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 30.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(125.0, 30.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at_with(tool, ctrl());

    assert!(shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap()
            .1,
        Vec3::new(135.0, 60.0, 20.0)
    );

    shell.press_key(Key::Enter);
    let union_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 2, "Ctrl must keep the tool");
    assert_ne!(union_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), union_digest);
}

#[test]
fn solid_intersect_previews_exact_overlap_and_commits_in_one_undo_step() {
    let mut shell = Shell::new();
    assert!(shell.app_mut().create_box());
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 10.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(120.0, 50.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at(tool);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Intersect preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some((Vec3::new(35.0, 35.0, 0.0), Vec3::new(65.0, 25.0, 20.0)))
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1)
    );

    shell.press_key(Key::Enter);
    let intersect_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_ne!(intersect_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn solid_split_previews_target_partition_and_commits_from_localized_headless_shell() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    assert!(shell.app_mut().create_box());
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let target_geometry = shell.app().occurrence_box_geometry(1).unwrap();

    shell.click_menu_command("menu-model", AppCommand::SolidSplit);
    let target = shell
        .app()
        .viewport_position(Vec3::new(10.0, 10.0, 20.0))
        .unwrap();
    let splitter = shell
        .app()
        .viewport_position(Vec3::new(120.0, 50.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.click_at(splitter);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Split preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1)),
        Some(target_geometry)
    );
    assert_eq!(
        shell.app().push_pull_preview_exact_evaluator(),
        Some(EXACT_BOOLEAN_SPLIT_EVALUATOR_V1)
    );

    shell.press_key(Key::Enter);
    let split_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "Split must preserve the splitter"
    );
    assert_ne!(split_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), split_digest);
    assert_eq!(shell.app().active_box_count(), 2);
}

#[test]
fn solid_subtract_previews_consumes_the_tool_and_undo_restores_both_solids() {
    let mut shell = Shell::new();
    let rectangle_start = shell
        .app()
        .viewport_position(Vec3::new(115.0, 20.0, 0.0))
        .unwrap();
    shell.click_command(AppCommand::Rectangle);
    shell.click_at(rectangle_start);
    shell.type_text("30,20");
    shell.press_key(Key::Enter);
    shell.click_command(AppCommand::PushPull);
    shell.type_text("20");
    shell.press_key(Key::Enter);
    assert!(shell.app_mut().move_selected(Vec3::new(-95.0, -5.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().occurrence_box_geometry(2).unwrap().1.z, 20.0);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_menu_command("menu-model", AppCommand::SolidSubtract);
    let target = shell
        .app()
        .viewport_position(Vec3::new(80.0, 45.0, 20.0))
        .unwrap();
    let tool = shell
        .app()
        .viewport_position(Vec3::new(35.0, 25.0, 20.0))
        .unwrap();
    shell.click_at(target);
    shell.move_pointer(tool);
    if shell
        .app()
        .hovered_selection()
        .is_none_or(|selection| selection.instance_path != InstancePath::root(OccurrenceId(2)))
    {
        shell.press_key(Key::Tab);
    }
    assert_eq!(
        shell
            .app()
            .hovered_selection()
            .map(|selection| selection.instance_path.clone()),
        Some(InstancePath::root(OccurrenceId(2)))
    );
    shell.click_at(tool);

    assert!(
        shell.app().has_occurrence_operation_preview(),
        "Subtract preview failed: {}",
        shell.app().action_digest()
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.press_key(Key::Enter);

    let subtract_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_ne!(subtract_digest, before_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), subtract_digest);
    assert_eq!(shell.app().active_box_count(), 1);
}

#[test]
fn the_model_menu_offers_make_unique() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());

    shell.open_menu("menu-model");

    assert!(
        shell.offers(AppCommand::MakeUnique),
        "the Model menu must expose Make Unique"
    );
}

#[test]
fn make_unique_clones_the_definition_in_one_undo_step() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().definition_count(), 1);

    let before = shell.app().document_revision();
    shell.click_menu_command("menu-model", AppCommand::MakeUnique);

    assert_eq!(
        shell.app().definition_count(),
        2,
        "Make Unique must clone the definition"
    );
    assert_eq!(
        shell.app().active_box_count(),
        2,
        "and keep both occurrences"
    );
    assert_eq!(
        shell.app().document_revision(),
        before + 1,
        "Make Unique must be one undo step"
    );
}
