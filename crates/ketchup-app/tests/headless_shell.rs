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
use harness::{Shell, ctrl, shift};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{
    AlignMode, AppCommand, AssistantWorkspaceMode, DistributionMode, GeneralFinishKind,
    RectangularPatternSpec,
};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DerivedIdentity, Dimension, DocumentStore,
    EdgeFinishKind, EvaluationIdentity, FeatureId, FeatureKind, FeatureParameterBinding,
    FeatureParameterTarget, InstancePath, NodeId, OccurrenceId, ParameterValueType, PortSpec,
    RuleOutput, SlotPath, SlotSegment, TagId, Transform,
};
use ketchup_core::exact_brep_graph::ExactBRepGraph;
use ketchup_core::exact_product::{
    EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1, EXACT_BOOLEAN_SPLIT_EVALUATOR_V1,
    EXACT_BOOLEAN_UNION_EVALUATOR_V1, EXACT_CIRCLE_EVALUATOR_V1, EXACT_CIRCULAR_CUT_EVALUATOR_V1,
    EXACT_LOFT_EVALUATOR_V1, EXACT_PLANAR_OFFSET_EVALUATOR_V1, EXACT_SWEEP_EVALUATOR_V1,
    ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence, ExactBodyPackage, ExactFaceRole,
    ExactFeatureChainRequest,
};
use ketchup_core::graph::{EvaluationStatus, EvaluatorNodeKind};
use ketchup_core::import::{ImportFormat, StepImportMesh, StepMeshTriangle};
use ketchup_core::intent::WorkflowIntent;
use ketchup_core::persistence;
use ketchup_core::topology::TopologicalElementKind;
use ketchup_interaction::{
    Axis, ElementId, LocaleCatalog, Side, SnapKind, Vec3, exact_projection::TopologicalPickLocator,
};
use ketchup_scheduler::ExactWorkerSupervisor;

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
                target: FeatureParameterTarget::new(
                    PARAMETRIC_PROFILE,
                    "bounds.width",
                    ParameterValueType::Length,
                )
                .unwrap(),
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
fn window_menu_toggles_outliner_and_tags_without_mutating_the_document() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell
            .app_mut()
            .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Tab);
        shell.settle();
        let outliner = shell.catalog().text("dock-outliner");
        let tags = shell.catalog().text("dock-tags");
        assert!(shell.app().outliner_visible());
        assert!(shell.app().tags_visible());
        assert!(shell.has_visible_label(&outliner));
        assert!(shell.has_visible_label(&tags));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &outliner);
        shell.press_key(Key::Escape);
        assert!(!shell.app().outliner_visible());
        assert!(!shell.has_visible_label(&outliner));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &tags);
        shell.press_key(Key::Escape);
        assert!(!shell.app().tags_visible());
        assert!(!shell.has_visible_label(&tags));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-file", AppCommand::New);
        assert!(!shell.app().outliner_visible());
        assert!(!shell.app().tags_visible());

        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &outliner);
        shell.press_key(Key::Escape);
        shell.open_menu("menu-window");
        shell.click_role_and_label(Role::CheckBox, &tags);
        shell.press_key(Key::Escape);
        assert!(shell.app().outliner_visible());
        assert!(shell.app().tags_visible());
        assert!(shell.has_visible_label(&outliner));
        assert!(shell.has_visible_label(&tags));
    }
}

#[test]
fn home_view_is_localized_accessible_framed_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::HomeView),
            shell.catalog().text("view-home")
        );

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let projection = shell.app().projection_mode();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::HomeView));
        shell.click_command(AppCommand::HomeView);
        assert_eq!(shell.app().camera_orientation(), (-2.25, 0.52));
        let home_zoom = shell.app().camera_zoom();
        let rect = shell.viewport_rect();
        for occurrence in [1, 2] {
            let (origin, size) = shell.app().occurrence_box_geometry(occurrence).unwrap();
            let center = shell.app().project_to_screen(
                origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
                rect,
            );
            assert!(rect.contains(center));
        }

        shell.click_menu_command("menu-view", AppCommand::ViewTop);
        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        shell.click_menu_command("menu-view", AppCommand::HomeView);
        assert_eq!(shell.app().camera_orientation(), (-2.25, 0.52));
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);

        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        shell.press_key(Key::Home);
        assert_eq!(shell.app().camera_orientation(), (-2.25, 0.52));
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-home-view",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn previous_view_is_localized_swappable_gesture_aware_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::PreviousView),
            shell.catalog().text("view-previous")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ViewProjection);
        shell.click_menu_command("menu-view", AppCommand::ZoomSelection);

        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let probe = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        let remembered_orientation = shell.app().camera_orientation();
        let remembered_projection = shell.app().projection_mode();
        let remembered_zoom = shell.app().camera_zoom();
        let remembered_probe = shell.app().project_to_screen(probe, rect);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::HomeView);
        let home_orientation = shell.app().camera_orientation();
        let home_projection = shell.app().projection_mode();
        let home_zoom = shell.app().camera_zoom();
        let home_probe = shell.app().project_to_screen(probe, rect);

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::PreviousView));
        shell.click_command(AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), remembered_orientation);
        assert_eq!(shell.app().projection_mode(), remembered_projection);
        assert!((shell.app().camera_zoom() - remembered_zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(remembered_probe)
                < 0.01
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-previous-view")
        );

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), home_orientation);
        assert_eq!(shell.app().projection_mode(), home_projection);
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(home_probe)
                < 0.01
        );

        shell.click_command(AppCommand::Orbit);
        shell.drag(rect.center(), rect.center() + Vec2::new(80.0, 40.0));
        assert_ne!(shell.app().camera_orientation(), home_orientation);
        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert_eq!(shell.app().camera_orientation(), home_orientation);

        shell.scroll_at(rect.center(), 120.0);
        assert_ne!(shell.app().camera_zoom(), home_zoom);
        shell.app_mut().previous_view();
        assert!((shell.app().camera_zoom() - home_zoom).abs() < 1.0e-5);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn grid_axes_toggle_is_localized_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewGridAxes),
            shell.catalog().text("view-grid-axes")
        );
        assert!(shell.app().grid_axes_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::ViewGridAxes));
        shell.click_command(AppCommand::ViewGridAxes);
        assert!(!shell.app().grid_axes_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grid-axes-hidden")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().grid_axes_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-previous-view")
        );

        shell.click_menu_command("menu-view", AppCommand::ViewGridAxes);
        assert!(!shell.app().grid_axes_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewGridAxes);
        assert!(shell.app().grid_axes_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grid-axes-shown")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn xray_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewXray),
            shell.catalog().text("view-xray")
        );
        assert!(!shell.app().xray_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewXray));
        shell.click_command(AppCommand::ViewXray);
        assert!(shell.app().xray_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-xray-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().xray_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewXray);
        assert!(shell.app().xray_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewXray);
        assert!(!shell.app().xray_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-xray-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn white_background_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewWhiteBackground),
            shell.catalog().text("view-white-background")
        );
        assert!(!shell.app().white_background_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewWhiteBackground));
        shell.click_command(AppCommand::ViewWhiteBackground);
        assert!(shell.app().white_background_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-white-background-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().white_background_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewWhiteBackground);
        assert!(shell.app().white_background_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewWhiteBackground);
        assert!(!shell.app().white_background_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-white-background-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn shadows_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewShadows),
            shell.catalog().text("view-shadows")
        );
        assert!(!shell.app().shadows_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewShadows));
        shell.click_command(AppCommand::ViewShadows);
        assert!(shell.app().shadows_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shadows-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().shadows_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewShadows);
        assert!(shell.app().shadows_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewShadows);
        assert!(!shell.app().shadows_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shadows-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn fog_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewFog),
            shell.catalog().text("view-fog")
        );
        assert!(!shell.app().fog_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewFog));
        shell.click_command(AppCommand::ViewFog);
        assert!(shell.app().fog_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fog-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().fog_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewFog);
        assert!(shell.app().fog_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewFog);
        assert!(!shell.app().fog_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fog-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn shaded_command_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewShaded),
            shell.catalog().text("view-shaded")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ViewShaded));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::ViewXray);
        shell.click_menu_command("menu-view", AppCommand::ViewEdges);
        shell.click_menu_command("menu-view", AppCommand::ViewWireframe);
        shell.click_menu_command("menu-view", AppCommand::ViewMonochrome);
        shell.click_menu_command("menu-view", AppCommand::ViewHiddenLine);
        assert!(shell.app().xray_visible());
        assert!(!shell.app().edges_visible());
        assert!(shell.app().wireframe_visible());
        assert!(shell.app().monochrome_visible());
        assert!(shell.app().hidden_line_visible());

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewShaded));
        shell.click_command(AppCommand::ViewShaded);
        assert!(!shell.app().wireframe_visible());
        assert!(!shell.app().monochrome_visible());
        assert!(!shell.app().hidden_line_visible());
        assert!(shell.app().xray_visible());
        assert!(!shell.app().edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-shaded-restored")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().wireframe_visible());
        assert!(shell.app().monochrome_visible());
        assert!(shell.app().hidden_line_visible());
        assert!(shell.app().xray_visible());
        assert!(!shell.app().edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewShaded);
        assert!(!shell.app().wireframe_visible());
        assert!(!shell.app().monochrome_visible());
        assert!(!shell.app().hidden_line_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn wireframe_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewWireframe),
            shell.catalog().text("view-wireframe")
        );
        assert!(!shell.app().wireframe_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewWireframe));
        shell.click_command(AppCommand::ViewWireframe);
        assert!(shell.app().wireframe_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-wireframe-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().wireframe_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewWireframe);
        assert!(shell.app().wireframe_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewWireframe);
        assert!(!shell.app().wireframe_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-wireframe-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn monochrome_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewMonochrome),
            shell.catalog().text("view-monochrome")
        );
        assert!(!shell.app().monochrome_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewMonochrome));
        shell.click_command(AppCommand::ViewMonochrome);
        assert!(shell.app().monochrome_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-monochrome-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().monochrome_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewMonochrome);
        assert!(shell.app().monochrome_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewMonochrome);
        assert!(!shell.app().monochrome_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-monochrome-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn hidden_line_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewHiddenLine),
            shell.catalog().text("view-hidden-line")
        );
        assert!(!shell.app().hidden_line_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewHiddenLine));
        shell.click_command(AppCommand::ViewHiddenLine);
        assert!(shell.app().hidden_line_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-line-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().hidden_line_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHiddenLine);
        assert!(shell.app().hidden_line_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHiddenLine);
        assert!(!shell.app().hidden_line_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-line-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewEdges),
            shell.catalog().text("view-edges")
        );
        assert!(shell.app().edges_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewEdges));
        shell.click_command(AppCommand::ViewEdges);
        assert!(!shell.app().edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-edges-hidden")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(shell.app().edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewEdges);
        assert!(!shell.app().edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewEdges);
        assert!(shell.app().edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-edges-shown")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn profiles_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewProfiles),
            shell.catalog().text("view-profiles")
        );
        assert!(!shell.app().profiles_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewProfiles));
        shell.click_command(AppCommand::ViewProfiles);
        assert!(shell.app().profiles_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-profiles-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().profiles_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewProfiles);
        assert!(shell.app().profiles_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewProfiles);
        assert!(!shell.app().profiles_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-profiles-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn halos_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewHalos),
            shell.catalog().text("view-halos")
        );
        assert!(!shell.app().halos_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewHalos));
        shell.click_command(AppCommand::ViewHalos);
        assert!(shell.app().halos_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-halos-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().halos_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHalos);
        assert!(shell.app().halos_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHalos);
        assert!(!shell.app().halos_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-halos-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn depth_cue_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewDepthCue),
            shell.catalog().text("view-depth-cue")
        );
        assert!(!shell.app().depth_cue_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewDepthCue));
        shell.click_command(AppCommand::ViewDepthCue);
        assert!(shell.app().depth_cue_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-depth-cue-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().depth_cue_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewDepthCue);
        assert!(shell.app().depth_cue_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewDepthCue);
        assert!(!shell.app().depth_cue_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-depth-cue-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn fade_distant_edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewFadeDistantEdges),
            shell.catalog().text("view-fade-distant-edges")
        );
        assert!(!shell.app().fade_distant_edges_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewFadeDistantEdges));
        shell.click_command(AppCommand::ViewFadeDistantEdges);
        assert!(shell.app().fade_distant_edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fade-distant-edges-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().fade_distant_edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewFadeDistantEdges);
        assert!(shell.app().fade_distant_edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewFadeDistantEdges);
        assert!(!shell.app().fade_distant_edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-fade-distant-edges-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn high_contrast_edges_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewHighContrastEdges),
            shell.catalog().text("view-high-contrast-edges")
        );
        assert!(!shell.app().high_contrast_edges_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewHighContrastEdges));
        shell.click_command(AppCommand::ViewHighContrastEdges);
        assert!(shell.app().high_contrast_edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-high-contrast-edges-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().high_contrast_edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHighContrastEdges);
        assert!(shell.app().high_contrast_edges_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHighContrastEdges);
        assert!(!shell.app().high_contrast_edges_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-high-contrast-edges-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn selection_halo_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewSelectionHalo),
            shell.catalog().text("view-selection-halo")
        );
        assert!(!shell.app().selection_halo_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewSelectionHalo));
        shell.click_command(AppCommand::ViewSelectionHalo);
        assert!(shell.app().selection_halo_visible());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-halo-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().selection_halo_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewSelectionHalo);
        assert!(shell.app().selection_halo_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewSelectionHalo);
        assert!(!shell.app().selection_halo_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-halo-hidden")
        );
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn endpoints_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewEndpoints),
            shell.catalog().text("view-endpoints")
        );
        assert!(!shell.app().endpoints_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewEndpoints));
        shell.click_command(AppCommand::ViewEndpoints);
        assert!(shell.app().endpoints_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-endpoints-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().endpoints_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewEndpoints);
        assert!(shell.app().endpoints_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewEndpoints);
        assert!(!shell.app().endpoints_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-endpoints-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn midpoints_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewMidpoints),
            shell.catalog().text("view-midpoints")
        );
        assert!(!shell.app().midpoints_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewMidpoints));
        shell.click_command(AppCommand::ViewMidpoints);
        assert!(shell.app().midpoints_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-midpoints-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().midpoints_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewMidpoints);
        assert!(shell.app().midpoints_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewMidpoints);
        assert!(!shell.app().midpoints_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-midpoints-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn extensions_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewExtensions),
            shell.catalog().text("view-extensions")
        );
        assert!(!shell.app().extensions_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewExtensions));
        shell.click_command(AppCommand::ViewExtensions);
        assert!(shell.app().extensions_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-extensions-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().extensions_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewExtensions);
        assert!(shell.app().extensions_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewExtensions);
        assert!(!shell.app().extensions_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-extensions-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn jitter_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewJitter),
            shell.catalog().text("view-jitter")
        );
        assert!(!shell.app().jitter_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewJitter));
        shell.click_command(AppCommand::ViewJitter);
        assert!(shell.app().jitter_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-jitter-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().jitter_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewJitter);
        assert!(shell.app().jitter_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewJitter);
        assert!(!shell.app().jitter_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-jitter-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn dashes_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewDashes),
            shell.catalog().text("view-dashes")
        );
        assert!(!shell.app().dashes_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewDashes));
        shell.click_command(AppCommand::ViewDashes);
        assert!(shell.app().dashes_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-dashes-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().dashes_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewDashes);
        assert!(shell.app().dashes_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewDashes);
        assert!(!shell.app().dashes_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-dashes-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn color_by_axis_toggle_is_localized_pickable_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewColorByAxis),
            shell.catalog().text("view-color-by-axis")
        );
        assert!(!shell.app().color_by_axis_visible());
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        let pick_point = shell.top_face_centre(1);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(pick_point);
        assert!(shell.offers(AppCommand::ViewColorByAxis));
        shell.click_command(AppCommand::ViewColorByAxis);
        assert!(shell.app().color_by_axis_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-color-by-axis-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(pick_point);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().color_by_axis_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewColorByAxis);
        assert!(shell.app().color_by_axis_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewColorByAxis);
        assert!(!shell.app().color_by_axis_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-color-by-axis-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn hidden_objects_toggle_is_localized_non_interactive_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ViewHiddenObjects),
            shell.catalog().text("view-hidden-objects")
        );
        assert!(!shell.app().hidden_objects_visible());
        assert_eq!(shell.app().hidden_ghost_count(), 0);
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::ViewHiddenObjects)
        );

        let hidden_point = shell.top_face_centre(1);
        shell.click_at(hidden_point);
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        shell.click_menu_command("menu-view", AppCommand::Hide);
        assert_eq!(shell.app().hidden_occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::ViewHiddenObjects)
        );

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(hidden_point);
        assert!(shell.offers(AppCommand::ViewHiddenObjects));
        shell.click_command(AppCommand::ViewHiddenObjects);
        assert!(shell.app().hidden_objects_visible());
        assert_eq!(shell.app().hidden_ghost_count(), 2);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-objects-shown")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.click_at(hidden_point);
        assert_eq!(shell.app().selected_occurrence_count(), 0);

        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!(!shell.app().hidden_objects_visible());
        assert_eq!(shell.app().hidden_ghost_count(), 0);

        shell.click_menu_command("menu-view", AppCommand::ViewHiddenObjects);
        assert!(shell.app().hidden_objects_visible());
        shell.click_menu_command("menu-view", AppCommand::ViewHiddenObjects);
        assert!(!shell.app().hidden_objects_visible());
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-hidden-objects-hidden")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn center_selection_is_localized_accessible_centered_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::CenterSelection),
            shell.catalog().text("view-center-selection")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::CenterSelection));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ViewFront);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CenterSelection));

        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::CenterSelection));
        shell.press_key(Key::Escape);

        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let selected_center = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        assert!(
            shell
                .app()
                .project_to_screen(selected_center, rect)
                .distance(rect.center())
                > 1.0
        );
        let zoom = shell.app().camera_zoom();
        let orientation = shell.app().camera_orientation();
        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::CenterSelection));
        shell.click_command(AppCommand::CenterSelection);
        assert!(
            shell
                .app()
                .project_to_screen(selected_center, rect)
                .distance(rect.center())
                < 0.01
        );
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert_eq!(shell.app().camera_orientation(), orientation);
        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-center-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn zoom_window_is_localized_bounded_reversible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomWindow),
            shell.catalog().text("view-zoom-window")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));

        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::ZoomWindow));
        shell.press_key(Key::Escape);

        let rect = shell.viewport_rect();
        let zoom = shell.app().camera_zoom();
        let orientation = shell.app().camera_orientation();
        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::ZoomWindow);
        shell.drag(rect.center(), rect.center() + Vec2::new(3.0, 3.0));
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert!(!shell.app().command_is_enabled(AppCommand::PreviousView));
        shell.press_key(Key::Escape);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-cancelled")
        );

        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let probe = origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5);
        let probe_before = shell.app().project_to_screen(probe, rect);
        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::ZoomWindow));
        shell.click_command(AppCommand::ZoomWindow);
        shell.drag(
            probe_before - Vec2::new(80.0, 60.0),
            probe_before + Vec2::new(80.0, 60.0),
        );

        assert!(shell.app().camera_zoom() > zoom);
        assert_eq!(shell.app().camera_orientation(), orientation);
        assert_eq!(shell.app().projection_mode(), projection);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(rect.center())
                < 0.01
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-window")
        );
        assert!(shell.app().command_is_enabled(AppCommand::PreviousView));
        shell.click_menu_command("menu-view", AppCommand::PreviousView);
        assert!((shell.app().camera_zoom() - zoom).abs() < 1.0e-5);
        assert!(
            shell
                .app()
                .project_to_screen(probe, rect)
                .distance(probe_before)
                < 0.01
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn standard_orthographic_views_are_localized_accessible_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        for (command, key) in [
            (AppCommand::ViewTop, "view-top"),
            (AppCommand::ViewBottom, "view-bottom"),
            (AppCommand::ViewFront, "view-front"),
            (AppCommand::ViewBack, "view-back"),
            (AppCommand::ViewRight, "view-right"),
            (AppCommand::ViewLeft, "view-left"),
        ] {
            assert_eq!(
                shell.app().command_label(command),
                shell.catalog().text(key)
            );
            assert!(shell.offers(command));
            shell.click_command(command);
            assert_eq!(
                shell.app().action_digest(),
                shell.catalog().format(
                    "digest-view-changed",
                    &BTreeMap::from([("view", shell.catalog().text(key))]),
                )
            );
            shell.click_menu_command("menu-view", command);
            assert_eq!(
                shell.app().action_digest(),
                shell.catalog().format(
                    "digest-view-changed",
                    &BTreeMap::from([("view", shell.catalog().text(key))]),
                )
            );
            assert_eq!(shell.app().document_revision(), revision);
            assert_eq!(shell.app().canonical_digest(), digest);
            assert_eq!(shell.app().undo_step_count(), undo_steps);
        }
    }
}

#[test]
fn zoom_selection_is_localized_selection_bound_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomSelection),
            shell.catalog().text("view-zoom-selection")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ZoomSelection));

        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(1_000.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let all_zoom = shell.app().camera_zoom();
        shell.click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::ZoomSelection));

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-view", AppCommand::ZoomSelection);

        assert!(shell.app().camera_zoom() > all_zoom * 2.0);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-zoom-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );
        let rect = shell.viewport_rect();
        let (origin, size) = shell.app().occurrence_box_geometry(1).unwrap();
        let selected_center = shell.app().project_to_screen(
            origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
            rect,
        );
        assert!(selected_center.distance(rect.center()) < rect.width() * 0.1);
        let (origin, size) = shell.app().occurrence_box_geometry(2).unwrap();
        let unselected_center = shell.app().project_to_screen(
            origin + Vec3::new(size.x * 0.5, size.y * 0.5, size.z * 0.5),
            rect,
        );
        assert!(!rect.expand(5.0).contains(unselected_center));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert!(!shell.app().command_is_enabled(AppCommand::ZoomSelection));
    }
}

#[test]
fn zoom_steps_are_localized_keyboard_accessible_bounded_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomIn),
            shell.catalog().text("view-zoom-in")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::ZoomOut),
            shell.catalog().text("view-zoom-out")
        );
        shell.open_menu("menu-view");
        assert!(shell.offers(AppCommand::ZoomIn));
        assert!(shell.offers(AppCommand::ZoomOut));
        shell.press_key(Key::Escape);

        let projection = shell.app().projection_mode();
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let initial_zoom = shell.app().camera_zoom();

        shell.click_menu_command("menu-view", AppCommand::ZoomIn);
        assert!((shell.app().camera_zoom() - initial_zoom * 1.25).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-in")
        );
        shell.click_menu_command("menu-view", AppCommand::ZoomOut);
        assert!((shell.app().camera_zoom() - initial_zoom).abs() < 1.0e-5);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-zoom-out")
        );

        shell.key(Key::Plus, ctrl());
        assert!((shell.app().camera_zoom() - initial_zoom * 1.25).abs() < 1.0e-5);
        shell.key(Key::Minus, ctrl());
        assert!((shell.app().camera_zoom() - initial_zoom).abs() < 1.0e-5);

        assert_eq!(shell.app().projection_mode(), projection);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
}

#[test]
fn help_about_is_localized_informative_and_presentation_only() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let version = shell.catalog().format(
            "about-version",
            &BTreeMap::from([("version", env!("CARGO_PKG_VERSION").to_owned())]),
        );
        let license = shell.catalog().format(
            "about-license",
            &BTreeMap::from([("license", env!("CARGO_PKG_LICENSE").to_owned())]),
        );
        let close = shell.catalog().text("about-close");

        assert_eq!(
            shell.app().command_label(AppCommand::About),
            shell.catalog().text("help-about")
        );
        shell.click_menu_command("menu-help", AppCommand::About);
        assert!(shell.app().about_visible());
        assert!(shell.has_visible_label(&version));
        assert!(shell.has_visible_label(&license));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_role_and_label(Role::Button, &close);
        assert!(!shell.app().about_visible());
        shell.click_menu_command("menu-help", AppCommand::About);
        assert!(shell.app().about_visible());
        shell.key(Key::N, ctrl());
        assert!(shell.app().about_visible());
        assert!(shell.has_visible_label(&version));
        assert!(shell.has_visible_label(&license));
    }
}

#[test]
fn rename_occurrence_is_localized_selection_bound_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let occurrence_id = OccurrenceId(1);
        let original_name = shell
            .app()
            .occurrence_name(occurrence_id)
            .expect("the initial occurrence exists");
        assert_eq!(
            shell.app().command_label(AppCommand::RenameOccurrence),
            shell.catalog().text("model-rename-occurrence")
        );

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        assert!(!shell.app().rename_occurrence_visible());

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let input_label = shell.catalog().text("dialog-rename-occurrence-name");
        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(
            shell.app().rename_occurrence_input(),
            Some(original_name.as_str())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );
        assert!(!shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        let renamed = "Housing instance";
        shell.focus_text_input(&input_label);
        shell.type_text(renamed);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-confirm"),
        );
        assert!(!shell.app().rename_occurrence_visible());
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(renamed.to_owned())
        );
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-occurrence",
                &BTreeMap::from([("name", renamed.to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name.clone())
        );

        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale rename");
        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name.clone())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-occurrence-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RenameOccurrence);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Missing occurrence");
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_occurrence_rename());
        assert!(shell.app().rename_occurrence_visible());
        assert_eq!(shell.app().occurrence_name(occurrence_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn replace_component_is_localized_state_preserving_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::ReplaceComponent),
            shell.catalog().text("model-replace-component")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::ReplaceComponent));
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert!(!shell.app().component_replacement_visible());

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().create_box());
        let occurrence_id = OccurrenceId(2);
        let original_definition = DefinitionId(2);
        let replacement_definition = DefinitionId(1);
        let original_name = shell.app().occurrence_name(occurrence_id).unwrap();
        let original_geometry = shell
            .app()
            .occurrence_box_geometry(occurrence_id.0)
            .unwrap();
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(original_definition)
        );
        assert!(shell.app().command_is_enabled(AppCommand::ReplaceComponent));
        assert!(shell.app_mut().set_selected_occurrence_grounded(true));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert_eq!(
            shell.app().component_replacement_input(),
            Some((occurrence_id, replacement_definition))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-cancel"),
        );
        assert!(!shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-confirm"),
        );
        assert!(!shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(occurrence_id));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().occurrence_name(occurrence_id),
            Some(original_name)
        );
        assert_eq!(
            shell.app().occurrence_box_geometry(occurrence_id.0),
            Some(original_geometry)
        );
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(replacement_definition)
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-replaced-component",
                &BTreeMap::from([(
                    "name",
                    shell.app().definition_name(replacement_definition).unwrap(),
                )]),
            )
        );
        let replaced_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(original_definition)
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), replaced_digest);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_component_replacement());
        assert!(shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().occurrence_definition_id(occurrence_id),
            Some(replacement_definition)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        let (missing_occurrence, _) = shell.app().component_replacement_input().unwrap();
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_component_replacement());
        assert!(shell.app().component_replacement_visible());
        assert_eq!(
            shell.app().occurrence_definition_id(missing_occurrence),
            None
        );
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn replace_component_rejects_stale_confirmation_without_mutation() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-model", AppCommand::ReplaceComponent);
        assert!(shell.app().component_replacement_visible());
        assert!(shell.app_mut().rotate_selected(30.0));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-replace-component-confirm"),
        );
        assert!(shell.app().component_replacement_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(2))
        );
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
    }
}

#[test]
fn deselect_is_localized_exact_shortcut_parity_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::Deselect),
            shell.catalog().text("action-deselect")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::Deselect));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.press_key(Key::Escape);

        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-cleared")
        );
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));
        let action_digest = shell.app().action_digest().to_owned();

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().action_digest(), action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        let action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().action_digest(), action_digest);

        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Deselect));
        let group_revision = shell.app().document_revision();
        let group_digest = shell.app().canonical_digest();
        let group_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Deselect);

        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().document_revision(), group_revision);
        assert_eq!(shell.app().canonical_digest(), group_digest);
        assert_eq!(shell.app().undo_step_count(), group_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-selection-cleared")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Deselect));
    }
}

#[test]
fn select_all_is_localized_exact_shortcut_parity_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::SelectAll),
            shell.catalog().text("action-select-all")
        );
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().create_box());
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.app().command_is_enabled(AppCommand::SelectAll));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.key(Key::A, ctrl());

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let expected_action_digest = shell.catalog().format(
            "digest-selected-all",
            &BTreeMap::from([("count", "2".to_owned())]),
        );
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert!(!shell.app().command_is_enabled(AppCommand::SelectAll));
        shell.key(Key::A, ctrl());
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), expected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 3);
    }
}

#[test]
fn invert_selection_is_localized_scope_exact_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::InvertSelection),
            shell.catalog().text("action-invert-selection")
        );
        assert!(shell.app().command_is_enabled(AppCommand::InvertSelection));
        shell.click_at(shell.viewport_rect().center());
        assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::InvertSelection));
        shell.click_command(AppCommand::InvertSelection);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-inverted-selection",
                &BTreeMap::from([("count", "1".to_owned())]),
            )
        );

        shell.key(Key::I, ctrl());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(2)));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_menu_command("menu-edit", AppCommand::InvertSelection);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.key(Key::I, ctrl());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.click_menu_command("menu-edit", AppCommand::InvertSelection);
        assert_eq!(context_shell.app().selected_occurrence_count(), 1);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.key(Key::I, ctrl());
        assert_eq!(context_shell.app().selected_occurrence_count(), 0);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn cut_is_localized_atomic_pasteable_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Cut),
            shell.catalog().text("action-cut")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Cut));
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Cut));
        let before_cut = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Cut);

        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-cut-to-clipboard",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Cut));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let cut_digest = shell.app().canonical_digest();
        let cut_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(shell.app().canonical_digest(), cut_digest);
        assert_eq!(shell.app().action_digest(), cut_action_digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), before_cut);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), cut_digest);

        let paste_revision = shell.app().document_revision();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), cut_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 2);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        assert!(group_shell.app_mut().create_box());
        group_shell.settle();
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Cut));
        let group_revision = group_shell.app().document_revision();
        let group_digest = group_shell.app().canonical_digest();
        group_shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(group_shell.app().document_revision(), group_revision);
        assert_eq!(group_shell.app().canonical_digest(), group_digest);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_at(context_shell.top_face_centre(2));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Cut));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        context_shell.click_menu_command("menu-edit", AppCommand::Cut);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
    }
}

#[test]
fn duplicate_is_localized_atomic_clipboard_preserving_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Duplicate),
            shell.catalog().text("action-duplicate")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Duplicate));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app_mut().create_box());
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Duplicate));
        let before_duplicate = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.key(Key::D, ctrl());

        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-duplicated-selection",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        let duplicated = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_duplicate);
        assert_eq!(shell.app().occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        shell.click_menu_command("menu-edit", AppCommand::Duplicate);
        assert_eq!(shell.app().canonical_digest(), duplicated);
        assert_eq!(shell.app().occurrence_count(), 4);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_duplicate);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), duplicated);
        assert_eq!(shell.app().occurrence_count(), 4);

        let paste_revision = shell.app().document_revision();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

        let mut group_shell = Shell::with_catalog(catalog.clone());
        assert!(group_shell.app_mut().create_box());
        group_shell.settle();
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Duplicate));
        let group_revision = group_shell.app().document_revision();
        let group_digest = group_shell.app().canonical_digest();
        let group_undo_steps = group_shell.app().undo_step_count();
        group_shell.click_menu_command("menu-edit", AppCommand::Duplicate);
        assert_eq!(group_shell.app().document_revision(), group_revision);
        assert_eq!(group_shell.app().canonical_digest(), group_digest);
        assert_eq!(group_shell.app().undo_step_count(), group_undo_steps);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_at(context_shell.top_face_centre(2));
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::Duplicate)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.key(Key::D, ctrl());
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn copy_paste_is_localized_atomic_stale_safe_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Copy),
            shell.catalog().text("action-copy")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Paste),
            shell.catalog().text("action-paste")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Copy));
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::Copy));
        let copy_revision = shell.app().document_revision();
        let copy_digest = shell.app().canonical_digest();
        let copy_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-copied-to-clipboard",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        assert_eq!(shell.app().document_revision(), copy_revision);
        assert_eq!(shell.app().canonical_digest(), copy_digest);
        assert_eq!(shell.app().undo_step_count(), copy_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert!(shell.app_mut().create_box());
        shell.settle();

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-copied-to-clipboard",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );
        let before_paste = shell.app().canonical_digest();
        let paste_revision = shell.app().document_revision();
        let paste_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-edit", AppCommand::Paste);

        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().document_revision(), paste_revision + 1);
        assert_eq!(shell.app().undo_step_count(), paste_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-pasted-from-clipboard",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );
        let pasted = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 3);
        assert_eq!(shell.app().canonical_digest(), before_paste);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().canonical_digest(), pasted);

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 5);
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        group_shell.click_at(group_shell.top_face_centre(1));
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        group_shell.click_menu_command("menu-edit", AppCommand::Paste);
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(group_shell.app().group_count(), 1);
        assert!(!group_shell.app().command_is_enabled(AppCommand::Copy));
        let group_digest = group_shell.app().action_digest().to_owned();
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert_eq!(group_shell.app().action_digest(), group_digest);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.click_at(context_shell.top_face_centre(1));
        context_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(!context_shell.app().command_is_enabled(AppCommand::Copy));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Paste));
        let revision = context_shell.app().document_revision();
        let digest = context_shell.app().canonical_digest();
        let undo_steps = context_shell.app().undo_step_count();
        let action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(context_shell.app().document_revision(), revision);
        assert_eq!(context_shell.app().canonical_digest(), digest);
        assert_eq!(context_shell.app().undo_step_count(), undo_steps);
        assert_eq!(context_shell.app().action_digest(), action_digest);
    }
}

#[test]
fn delete_is_localized_atomic_group_aware_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Delete),
            shell.catalog().text("action-delete")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Delete));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(shell.app().command_is_enabled(AppCommand::Delete));
        let before_delete = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.press_key(Key::Delete);

        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Delete));
        let deleted = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), before_delete);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), deleted);

        let mut group_shell = Shell::with_catalog(catalog.clone());
        group_shell.click_at(group_shell.top_face_centre(1));
        group_shell.click_menu_command("menu-edit", AppCommand::Copy);
        group_shell.click_menu_command("menu-edit", AppCommand::Paste);
        group_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        group_shell.click_menu_command("menu-model", AppCommand::Group);
        assert_eq!(group_shell.app().group_count(), 1);
        assert!(group_shell.app().command_is_enabled(AppCommand::Delete));
        let grouped = group_shell.app().canonical_digest();
        let grouped_revision = group_shell.app().document_revision();
        let grouped_undo_steps = group_shell.app().undo_step_count();

        group_shell.click_menu_command("menu-edit", AppCommand::Delete);

        assert_eq!(group_shell.app().occurrence_count(), 0);
        assert_eq!(group_shell.app().group_count(), 0);
        assert_eq!(group_shell.app().document_revision(), grouped_revision + 1);
        assert_eq!(group_shell.app().undo_step_count(), grouped_undo_steps + 1);
        assert_eq!(
            group_shell.app().action_digest(),
            group_shell.catalog().format(
                "digest-deleted",
                &BTreeMap::from([("count", "2".to_owned())]),
            )
        );
        let group_deleted = group_shell.app().canonical_digest();
        group_shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(group_shell.app().occurrence_count(), 2);
        assert_eq!(group_shell.app().group_count(), 1);
        assert_eq!(group_shell.app().canonical_digest(), grouped);
        group_shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(group_shell.app().occurrence_count(), 0);
        assert_eq!(group_shell.app().group_count(), 0);
        assert_eq!(group_shell.app().canonical_digest(), group_deleted);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(!context_shell.app().command_is_enabled(AppCommand::Delete));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn group_ungroup_is_localized_atomic_and_context_bound() {
    for (locale_index, catalog) in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Group),
            shell.catalog().text("model-group")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Ungroup),
            shell.catalog().text("model-ungroup")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Group));
        assert!(!shell.app().command_is_enabled(AppCommand::Ungroup));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(shell.app().command_is_enabled(AppCommand::Group));
        let ungrouped = shell.app().canonical_digest();
        let group_revision = shell.app().document_revision();
        let group_undo_steps = shell.app().undo_step_count();

        if locale_index == 0 {
            shell.key(Key::G, ctrl());
        } else {
            shell.click_menu_command("menu-model", AppCommand::Group);
        }

        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().document_revision(), group_revision + 1);
        assert_eq!(shell.app().undo_step_count(), group_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-grouped",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Group));
        assert!(shell.app().command_is_enabled(AppCommand::Ungroup));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let grouped = shell.app().canonical_digest();
        let ungroup_revision = shell.app().document_revision();
        let ungroup_undo_steps = shell.app().undo_step_count();

        if locale_index == 0 {
            shell.click_menu_command("menu-model", AppCommand::Ungroup);
        } else {
            shell.key(Key::G, harness::ctrl_shift());
        }

        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), ungroup_revision + 1);
        assert_eq!(shell.app().undo_step_count(), ungroup_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-ungrouped",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(shell.app().command_is_enabled(AppCommand::Group));
        assert!(!shell.app().command_is_enabled(AppCommand::Ungroup));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let regrouped = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grouped);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().canonical_digest(), ungrouped);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().canonical_digest(), regrouped);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(!context_shell.app().command_is_enabled(AppCommand::Group));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Ungroup));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::Group);
        context_shell.click_menu_command("menu-model", AppCommand::Ungroup);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn make_component_is_localized_atomic_and_context_bound() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::MakeComponent),
            shell.catalog().text("model-make-component")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeComponent));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::Group);
        assert!(shell.app().command_is_enabled(AppCommand::MakeComponent));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        let grouped = shell.app().canonical_digest();
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        let component_name = shell.catalog().format(
            "model-component-name",
            &BTreeMap::from([("number", "1".to_owned())]),
        );

        shell.click_menu_command("menu-model", AppCommand::MakeComponent);

        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-made-component",
                &BTreeMap::from([("name", component_name), ("count", "2".to_owned()),])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeComponent));
        let converted = shell.app().canonical_digest();

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().group_count(), 1);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().canonical_digest(), grouped);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().group_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), converted);
        assert!(!shell.app().command_is_enabled(AppCommand::Paste));

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        context_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::MakeComponent)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::MakeComponent);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn select_all_instances_is_localized_context_bound_and_document_preserving() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::SelectAllInstances),
            shell.catalog().text("model-select-all-instances")
        );

        shell.click_at(shell.top_face_centre(1));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let single_revision = shell.app().document_revision();
        let single_digest = shell.app().canonical_digest();
        let single_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), single_revision);
        assert_eq!(shell.app().canonical_digest(), single_digest);
        assert_eq!(shell.app().undo_step_count(), single_undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert_ne!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            shell.app().occurrence_definition_id(OccurrenceId(3))
        );
        shell
            .app_mut()
            .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Tab);
        shell.settle();
        let definition_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let component_heading = shell.catalog().format(
            "outliner-component",
            &BTreeMap::from([
                ("name", definition_name),
                ("count", "2".to_owned()),
                ("dimensions", "100 × 60 × 20".to_owned()),
            ]),
        );
        shell.click_row(&component_heading);
        let peer_name = shell.app().occurrence_name(OccurrenceId(2)).unwrap();
        let peer_row = shell.catalog().format(
            "outliner-instance",
            &BTreeMap::from([("visibility", "◉".to_owned()), ("name", peer_name)]),
        );
        shell.click_row(&peer_row);
        shell
            .app_mut()
            .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Dock);
        shell.settle();
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-definition",
                &BTreeMap::from([
                    (
                        "name",
                        shell.app().definition_name(DefinitionId(1)).unwrap(),
                    ),
                    ("count", "2".to_owned()),
                ])
            )
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::SelectAllInstances)
        );
        let action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), action_digest);
    }
}

#[test]
fn assign_tag_is_localized_atomic_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(700);
        let temporary_tag = TagId(701);
        for (target, name) in [(tag, "Hardware"), (temporary_tag, "Temporary")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.settle();
        assert_eq!(
            shell.app().command_label(AppCommand::AssignTag),
            shell.catalog().text("model-assign-tag")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::AssignTag));

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::AssignTag));
        let cancel_revision = shell.app().document_revision();
        let cancel_digest = shell.app().canonical_digest();
        let cancel_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        assert!(shell.app().tag_assignment_visible());
        assert_eq!(shell.app().tag_assignment_input(), Some(None));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-cancel"),
        );
        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), cancel_revision);
        assert_eq!(shell.app().canonical_digest(), cancel_digest);
        assert_eq!(shell.app().undo_step_count(), cancel_undo_steps);

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        shell.click_command(AppCommand::AssignTag);
        assert!(shell.app().tag_assignment_visible());
        shell.click_row("Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );

        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));
        let after = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = after.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-assigned-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("tag", "Hardware".to_owned()),])
            )
        );

        let tagged_revision = shell.app().document_revision();
        let tagged_digest = shell.app().canonical_digest();
        let tagged_undo_steps = shell.app().undo_step_count();
        let tagged_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        assert!(shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), tagged_revision);
        assert_eq!(shell.app().canonical_digest(), tagged_digest);
        assert_eq!(shell.app().undo_step_count(), tagged_undo_steps);
        assert_eq!(shell.app().action_digest(), tagged_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell.click_row(&shell.catalog().text("dialog-assign-tag-untagged"));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), None);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), None);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row(&shell.catalog().text("dialog-assign-tag-untagged"));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_assignment());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row("Temporary");
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::DeleteTag {
                    target: temporary_tag,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_assignment());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());

        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        let stale_revision = shell.app().document_revision();
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let intervening_revision = shell.app().document_revision();
        assert!(intervening_revision > stale_revision);
        shell.settle();
        assert!(!shell.app().tag_assignment_visible());
        assert_eq!(shell.app().document_revision(), intervening_revision);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));
    }
}

#[test]
fn tags_panel_create_is_localized_canonical_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        let create_label = shell.catalog().text("tags-create");
        let confirm_label = shell.catalog().text("dialog-create-tag-confirm");
        let cancel_label = shell.catalog().text("dialog-create-tag-cancel");
        let name_label = shell.catalog().text("dialog-create-tag-name");
        assert!(shell.has_role_and_label(Role::Button, &create_label));

        let initial = shell.app().document_snapshot();
        let occurrence = initial.occurrence(OccurrenceId(1)).unwrap();
        let preserved = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tag(),
            occurrence.visible(),
        );
        let initial_revision = shell.app().document_revision();
        let initial_digest = shell.app().canonical_digest();
        let initial_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &create_label);
        assert!(shell.app().tag_creation_visible());
        shell.click_role_and_label(Role::Button, &cancel_label);
        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps);

        shell.click_role_and_label(Role::Button, &create_label);
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), initial_revision);
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        assert_eq!(shell.app().tag_creation_input(), Some("Hardware"));
        shell.click_role_and_label(Role::Button, &confirm_label);

        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), initial_revision + 1);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
        let created = shell
            .app()
            .document_snapshot()
            .tags()
            .find(|tag| tag.name() == "Hardware")
            .map(|tag| (tag.id(), tag.visible()));
        assert_eq!(created, Some((TagId(1), true)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-created-tag",
                &BTreeMap::from([("name", "Hardware".to_owned())]),
            )
        );
        let after = shell.app().document_snapshot();
        let occurrence = after.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            ),
            preserved
        );

        let created_revision = shell.app().document_revision();
        let created_digest = shell.app().canonical_digest();
        let created_undo_steps = shell.app().undo_step_count();
        let created_action_digest = shell.app().action_digest().to_owned();
        let created_selection = shell.app().selected_occurrence_count();
        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), created_revision);
        assert_eq!(shell.app().canonical_digest(), created_digest);
        assert_eq!(shell.app().undo_step_count(), created_undo_steps);
        assert_eq!(shell.app().action_digest(), created_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), created_selection);
        shell.click_role_and_label(Role::Button, &cancel_label);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Namespace drift");
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: TagId(850),
                    name: "Intervening tag".to_owned(),
                    visible: false,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Namespace drift")
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(TagId(850)), None);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Stale tag");
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Stale tag")
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert!(shell.app().document_snapshot().tags().next().is_none());
        let restored = shell.app().document_snapshot();
        let occurrence = restored.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            ),
            preserved
        );
    }
}

#[test]
fn tags_panel_create_from_selection_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let create_label = shell.catalog().text("tags-create-from-selection");
        let confirm_label = shell
            .catalog()
            .text("dialog-create-tag-from-selection-confirm");
        let cancel_label = shell.catalog().text("dialog-create-tag-cancel");
        let name_label = shell.catalog().text("dialog-create-tag-name");
        let other_tag = TagId(1401);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: other_tag,
                    name: "Other".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: Some(other_tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &create_label));
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_role_and_label(Role::Button, &create_label);
        assert!(!shell.app().tag_creation_visible());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &create_label);
        assert!(shell.app().tag_creation_visible());
        shell.focus_text_input(&name_label);
        shell.type_text("Hardware");
        shell.click_role_and_label(Role::Button, &confirm_label);

        let created_tag = TagId(1402);
        assert!(!shell.app().tag_creation_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().tag_visibility(created_tag), Some(true));
        let assigned = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = assigned.occurrence(id).unwrap();
            assert_eq!(occurrence.tag(), Some(created_tag));
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                (
                    preserved[index].0.clone(),
                    preserved[index].1,
                    preserved[index].2,
                    preserved[index].3,
                    preserved[index].5,
                )
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-created-tag-from-selection",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "3".to_owned()),]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(created_tag), None);
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            assert_eq!(restored.occurrence(id).unwrap().tag(), preserved[index].4);
        }

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Other");
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        shell.click_role_and_label(Role::Button, &cancel_label);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_creation());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Selection drift")
        );
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        shell.click_role_and_label(Role::Button, &create_label);
        shell.focus_text_input(&name_label);
        shell.type_text("Stale tag");
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_creation_visible());
        assert!(
            shell
                .app()
                .document_snapshot()
                .tags()
                .all(|tag| tag.name() != "Stale tag")
        );
    }
}

#[test]
fn tags_panel_delete_unused_is_localized_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let unused_tag = TagId(801);
        let used_tag = TagId(802);
        for (target, name) in [(unused_tag, "Unused"), (used_tag, "Used")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row("Used");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let unused_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Unused".to_owned())]),
        );
        let used_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Used".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &unused_delete));
        assert!(shell.has_role_and_label(Role::Button, &used_delete));

        let baseline = shell.app().document_snapshot();
        let occurrence = baseline.occurrence(OccurrenceId(1)).unwrap();
        let preserved_occurrence = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tag(),
            occurrence.visible(),
        );
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &unused_delete);
        assert!(shell.app().tag_deletion_visible());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-cancel"),
        );
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);

        shell.click_role_and_label(Role::Button, &unused_delete);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
        assert!(shell.app().document_snapshot().tag(unused_tag).is_some());
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &unused_delete);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-confirm"),
        );

        assert!(!shell.app().tag_deletion_visible());
        assert!(shell.app().document_revision() > revision);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let deleted = shell.app().document_snapshot();
        assert!(deleted.tag(unused_tag).is_none());
        assert_eq!(deleted.tag(used_tag).unwrap().name(), "Used");
        assert!(deleted.tag(used_tag).unwrap().visible());
        let occurrence = deleted.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted-tag",
                &BTreeMap::from([("name", "Unused".to_owned())]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(unused_tag).unwrap().name(), "Unused");
        assert!(restored.tag(unused_tag).unwrap().visible());
        assert_eq!(restored.tag(used_tag).unwrap().name(), "Used");
        let occurrence = restored.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
    }
}

#[test]
fn tags_panel_delete_used_exact_plan_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(825);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: tag,
                    name: "Hardware".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(shell.app_mut().create_box());
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);

        let delete_label = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let baseline = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = baseline.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &delete_label);
        assert!(shell.app().tag_deletion_visible());
        assert!(shell.has_visible_label(&shell.catalog().format(
            "dialog-delete-used-tag-message",
            &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),]),
        )));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-cancel"),
        );
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);

        shell.click_role_and_label(Role::Button, &delete_label);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-delete-tag-confirm"),
        );

        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let deleted = shell.app().document_snapshot();
        assert!(deleted.tag(tag).is_none());
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = deleted.occurrence(id).unwrap();
            assert_eq!(occurrence.tag(), None);
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-deleted-used-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(tag).unwrap().name(), "Hardware");
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(restored.occurrence(id).unwrap().tag(), Some(tag));
        }
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        let redone = shell.app().document_snapshot();
        assert!(redone.tag(tag).is_none());
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(redone.occurrence(id).unwrap().tag(), None);
        }
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert!(shell.app().document_snapshot().tag(tag).is_some());

        shell.click_role_and_label(Role::Button, &delete_label);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
        let stale = shell.app().document_snapshot();
        assert!(stale.tag(tag).is_some());
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert_eq!(stale.occurrence(id).unwrap().tag(), Some(tag));
        }
    }
}

#[test]
fn tags_panel_delete_local_only_is_localized_present_disabled_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let local_tag = TagId(826);
        let missing_tag = TagId(827);
        for (target, name) in [(local_tag, "Local"), (missing_tag, "Missing")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: Some(local_tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(shell.app_mut().group_selected());
        assert!(shell.app_mut().make_component());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let local_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Local".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &local_delete));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let action_digest = shell.app().action_digest().to_owned();
        let selected_occurrences = shell.app().selected_occurrence_count();
        shell.click_role_and_label(Role::Button, &local_delete);
        assert!(!shell.app().tag_deletion_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), action_digest);
        assert_eq!(
            shell.app().selected_occurrence_count(),
            selected_occurrences
        );

        let missing_delete = shell.catalog().format(
            "tags-delete",
            &BTreeMap::from([("name", "Missing".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &missing_delete);
        assert!(shell.app().tag_deletion_visible());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::DeleteTag {
                    target: missing_tag,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let action_digest = shell.app().action_digest().to_owned();
        let selected_occurrences = shell.app().selected_occurrence_count();
        assert!(!shell.app_mut().confirm_tag_deletion());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), action_digest);
        assert_eq!(
            shell.app().selected_occurrence_count(),
            selected_occurrences
        );
        shell.settle();
        assert!(!shell.app().tag_deletion_visible());
    }
}

#[test]
fn tags_panel_clear_assignments_is_localized_atomic_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let used_tag = TagId(850);
        let unused_tag = TagId(851);
        for (target, name) in [(used_tag, "Hardware"), (unused_tag, "Unused")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(used_tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().prepare_assistant_intent(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(2),
                visible: false,
            }
        ));
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let clear_used = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let clear_unused = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Unused".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &clear_used));
        assert!(shell.has_role_and_label(Role::Button, &clear_unused));

        let unused_revision = shell.app().document_revision();
        let unused_digest = shell.app().canonical_digest();
        let unused_undo_steps = shell.app().undo_step_count();
        let unused_action_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(Role::Button, &clear_unused);
        assert!(!shell.app().tag_clear_visible());
        assert_eq!(shell.app().document_revision(), unused_revision);
        assert_eq!(shell.app().canonical_digest(), unused_digest);
        assert_eq!(shell.app().undo_step_count(), unused_undo_steps);
        assert_eq!(shell.app().action_digest(), unused_action_digest);

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(shell.app().tag_clear_visible());
        let cancel_revision = shell.app().document_revision();
        let cancel_digest = shell.app().canonical_digest();
        let cancel_undo_steps = shell.app().undo_step_count();
        let cancel_action_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-clear-tag-cancel"),
        );
        assert!(!shell.app().tag_clear_visible());
        assert_eq!(shell.app().document_revision(), cancel_revision);
        assert_eq!(shell.app().canonical_digest(), cancel_digest);
        assert_eq!(shell.app().undo_step_count(), cancel_undo_steps);
        assert_eq!(shell.app().action_digest(), cancel_action_digest);

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: TagId(854),
                    name: "Namespace drift".to_owned(),
                    visible: false,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let namespace_revision = shell.app().document_revision();
        let namespace_digest = shell.app().canonical_digest();
        let namespace_undo_steps = shell.app().undo_step_count();
        let namespace_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), namespace_revision);
        assert_eq!(shell.app().canonical_digest(), namespace_digest);
        assert_eq!(shell.app().undo_step_count(), namespace_undo_steps);
        assert_eq!(shell.app().action_digest(), namespace_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.settle();

        shell.click_role_and_label(Role::Button, &clear_used);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();
        shell.click_role_and_label(Role::Button, &clear_used);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-clear-tag-confirm"),
        );

        assert!(!shell.app().tag_clear_visible());
        assert!(shell.app().document_revision() > revision);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let cleared = shell.app().document_snapshot();
        let tag = cleared.tag(used_tag).unwrap();
        assert_eq!(tag.name(), "Hardware");
        assert!(tag.visible());
        assert!(cleared.tag(unused_tag).is_some());
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = cleared.occurrence(id).unwrap();
            assert_eq!(occurrence.tag(), None);
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-cleared-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),])
            )
        );
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &clear_used));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(used_tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(used_tag));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .tag(used_tag)
                .unwrap()
                .name(),
            "Hardware"
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), None);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), None);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(used_tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(used_tag));

        let missing_tag = TagId(852);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: missing_tag,
                    name: "Missing".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: Some(missing_tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        let clear_missing = shell.catalog().format(
            "tags-clear",
            &BTreeMap::from([("name", "Missing".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &clear_missing);
        assert!(shell.app().tag_clear_visible());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: None,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::DeleteTag {
                    target: missing_tag,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_tag_clear());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        shell.settle();
        assert!(!shell.app().tag_clear_visible());
    }
}

#[test]
fn tags_panel_rename_is_localized_canonical_stale_safe_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(901);
        let other_tag = TagId(902);
        for (target, name) in [(tag, "Hardware"), (other_tag, "Other")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row("Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let rename_label = shell.catalog().format(
            "tags-rename",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let name_label = shell.catalog().text("dialog-rename-tag-name");
        let confirm_label = shell.catalog().text("dialog-rename-tag-confirm");
        let cancel_label = shell.catalog().text("dialog-rename-tag-cancel");
        assert!(shell.has_role_and_label(Role::Button, &rename_label));

        let baseline = shell.app().document_snapshot();
        let occurrence = baseline.occurrence(OccurrenceId(1)).unwrap();
        let preserved_occurrence = (
            occurrence.name().to_owned(),
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tag(),
            occurrence.visible(),
        );
        let baseline_revision = shell.app().document_revision();
        let baseline_digest = shell.app().canonical_digest();
        let baseline_undo_steps = shell.app().undo_step_count();
        let baseline_selection = shell.app().selected_occurrence_count();

        shell.click_role_and_label(Role::Button, &rename_label);
        assert!(shell.app().tag_rename_visible());
        assert_eq!(shell.app().tag_rename_input(), Some("Hardware"));
        shell.click_role_and_label(Role::Button, &cancel_label);
        assert!(!shell.app().tag_rename_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
        let invalid_action_digest = shell.app().action_digest().to_owned();

        shell.click_role_and_label(Role::Button, &rename_label);
        assert!(!shell.app_mut().confirm_tag_rename());
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Other");
        assert!(!shell.app_mut().confirm_tag_rename());
        assert_eq!(shell.app().document_revision(), baseline_revision);
        assert_eq!(shell.app().canonical_digest(), baseline_digest);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps);
        assert_eq!(shell.app().action_digest(), invalid_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), baseline_selection);
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("  Mechanical  ");
        shell.click_role_and_label(Role::Button, &confirm_label);

        assert!(!shell.app().tag_rename_visible());
        assert_eq!(shell.app().document_revision(), baseline_revision + 1);
        assert_eq!(shell.app().undo_step_count(), baseline_undo_steps + 1);
        let renamed = shell.app().document_snapshot();
        let renamed_tag = renamed.tag(tag).unwrap();
        assert_eq!(renamed_tag.name(), "Mechanical");
        assert!(renamed_tag.visible());
        assert_eq!(renamed.tag(other_tag).unwrap().name(), "Other");
        let occurrence = renamed.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            ),
            preserved_occurrence
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-tag",
                &BTreeMap::from([
                    ("old_name", "Hardware".to_owned()),
                    ("name", "Mechanical".to_owned()),
                ]),
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        let restored = shell.app().document_snapshot();
        assert_eq!(restored.tag(tag).unwrap().name(), "Hardware");
        assert!(restored.tag(tag).unwrap().visible());
        assert_eq!(
            restored.occurrence(OccurrenceId(1)).unwrap().tag(),
            Some(tag)
        );
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        let redone = shell.app().document_snapshot();
        assert_eq!(redone.tag(tag).unwrap().name(), "Mechanical");
        assert!(redone.tag(tag).unwrap().visible());
        assert_eq!(redone.occurrence(OccurrenceId(1)).unwrap().tag(), Some(tag));
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().document_snapshot().tag(tag).unwrap().name(),
            "Hardware"
        );

        let other_rename_label = shell.catalog().format(
            "tags-rename",
            &BTreeMap::from([("name", "Other".to_owned())]),
        );
        shell.click_role_and_label(Role::Button, &other_rename_label);
        assert!(shell.app().tag_rename_visible());
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::DeleteTag { target: other_tag })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        let missing_selection = shell.app().selected_occurrence_count();
        assert!(!shell.app_mut().confirm_tag_rename());
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
        assert_eq!(shell.app().selected_occurrence_count(), missing_selection);
        assert!(shell.app().document_snapshot().tag(other_tag).is_none());
        shell.settle();
        assert!(!shell.app().tag_rename_visible());
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .tag(other_tag)
                .unwrap()
                .name(),
            "Other"
        );

        shell.click_role_and_label(Role::Button, &rename_label);
        shell.focus_text_input(&name_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale name");
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::RenameDefinition {
                    target: DefinitionId(1),
                    name: "Intervening definition".to_owned(),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();
        assert!(!shell.app().tag_rename_visible());
        assert_eq!(
            shell.app().document_snapshot().tag(tag).unwrap().name(),
            "Hardware"
        );
    }
}

#[test]
fn tags_panel_visibility_toggle_is_localized_canonical_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(701);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: tag,
                    name: "Hardware".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.settle();

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::AssignTag);
        shell.click_row("Hardware");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-assign-tag-confirm"),
        );
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        let row = shell.catalog().format(
            "tags-row",
            &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "1".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::CheckBox, &row));
        assert_eq!(shell.app().tag_visibility(tag), Some(true));

        let before = shell.app().document_snapshot();
        let occurrence = before.occurrence(OccurrenceId(1)).unwrap();
        let preserved = (
            occurrence.definition_id(),
            occurrence.transform(),
            occurrence.parent(),
            occurrence.tag(),
            occurrence.visible(),
        );
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::CheckBox, &row);

        assert_eq!(shell.app().tag_visibility(tag), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        let hidden = shell.app().document_snapshot();
        let hidden_occurrence = hidden.occurrence(OccurrenceId(1)).unwrap();
        assert_eq!(
            (
                hidden_occurrence.definition_id(),
                hidden_occurrence.transform(),
                hidden_occurrence.parent(),
                hidden_occurrence.tag(),
                hidden_occurrence.visible(),
            ),
            preserved
        );
        assert_eq!(hidden.tag(tag).unwrap().name(), "Hardware");
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-tag-visibility",
                &BTreeMap::from([
                    ("name", "Hardware".to_owned()),
                    ("visibility", shell.catalog().text("visibility-hidden"),),
                ])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(tag), Some(true));
        assert_eq!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .tag(),
            Some(tag)
        );

        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app_mut().set_tag_visibility(tag, true));
        assert!(!shell.app_mut().set_tag_visibility(TagId(999), false));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
    }
}

#[test]
fn tags_panel_bulk_visibility_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        let show_all_label = shell.catalog().text("tags-show-all");
        let hide_all_label = shell.catalog().text("tags-hide-all");
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
        shell.settle();
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(true));
        assert!(!shell.app_mut().set_all_tag_visibility(false));
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        let visible_tag = TagId(801);
        let hidden_tag = TagId(802);
        for (target, name, visible) in [
            (visible_tag, "Hardware", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), visible_tag),
            (OccurrenceId(2), hidden_tag),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &show_all_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let shown = shell.app().document_snapshot();
        assert_eq!(shown.tag(visible_tag).unwrap().name(), "Hardware");
        assert_eq!(shown.tag(hidden_tag).unwrap().name(), "Hidden");
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = shown.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-all-tags-visibility",
                &BTreeMap::from([
                    ("count", "1".to_owned()),
                    ("visibility", shell.catalog().text("visibility-shown")),
                ])
            )
        );
        let shown_revision = shell.app().document_revision();
        let shown_digest = shell.app().canonical_digest();
        let shown_undo_steps = shell.app().undo_step_count();
        let shown_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(true));
        assert_eq!(shell.app().document_revision(), shown_revision);
        assert_eq!(shell.app().canonical_digest(), shown_digest);
        assert_eq!(shell.app().undo_step_count(), shown_undo_steps);
        assert_eq!(shell.app().action_digest(), shown_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        shell.click_role_and_label(Role::Button, &hide_all_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-all-tags-visibility",
                &BTreeMap::from([
                    ("count", "1".to_owned()),
                    ("visibility", shell.catalog().text("visibility-hidden")),
                ])
            )
        );
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().set_all_tag_visibility(false));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_all_label));
        assert!(shell.has_role_and_label(Role::Button, &hide_all_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_invert_visibility_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_tag_visibility());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        let empty_invert_label = shell.catalog().text("tags-invert-visibility");
        assert!(shell.has_role_and_label(Role::Button, &empty_invert_label));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Dock);
        shell.settle();

        let visible_tag = TagId(851);
        let hidden_tag = TagId(852);
        for (target, name, visible) in [
            (visible_tag, "Hardware", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), visible_tag),
            (OccurrenceId(2), hidden_tag),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let invert_label = shell.catalog().text("tags-invert-visibility");
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &invert_label);

        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let inverted = shell.app().document_snapshot();
        assert_eq!(inverted.tag(visible_tag).unwrap().name(), "Hardware");
        assert_eq!(inverted.tag(hidden_tag).unwrap().name(), "Hidden");
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = inverted.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-inverted-tags-visibility",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2)].into_iter().enumerate() {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_isolate_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let target_tag = TagId(901);
        let visible_tag = TagId(902);
        let hidden_tag = TagId(903);
        for (target, name, visible) in [
            (target_tag, "Hardware", false),
            (visible_tag, "Visible", true),
            (hidden_tag, "Hidden", false),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), target_tag),
            (OccurrenceId(2), visible_tag),
            (OccurrenceId(3), hidden_tag),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(2));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let isolate_label = shell.catalog().format(
            "tags-isolate",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &isolate_label);

        assert_eq!(shell.app().tag_visibility(target_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let isolated = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = isolated.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-isolated-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("name", "Hardware".to_owned()),])
            )
        );

        let isolated_revision = shell.app().document_revision();
        let isolated_digest = shell.app().canonical_digest();
        let isolated_undo_steps = shell.app().undo_step_count();
        let isolated_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_tag(target_tag));
        assert!(!shell.app_mut().isolate_tag(TagId(9999)));
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), isolated_revision);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().undo_step_count(), isolated_undo_steps);
        assert_eq!(shell.app().action_digest(), isolated_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(target_tag), Some(false));
        assert_eq!(shell.app().tag_visibility(visible_tag), Some(true));
        assert_eq!(shell.app().tag_visibility(hidden_tag), Some(false));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
    }
}

#[test]
fn tags_panel_isolate_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(951);
        let selected_b = TagId(952);
        let other = TagId(953);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app_mut().set_tag_visibility(selected_b, false));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let isolate_label = shell.catalog().text("tags-isolate-selection");
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &isolate_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(false));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let isolated = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = isolated.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-isolated-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let isolated_revision = shell.app().document_revision();
        let isolated_digest = shell.app().canonical_digest();
        let isolated_undo_steps = shell.app().undo_step_count();
        let isolated_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), isolated_revision);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().undo_step_count(), isolated_undo_steps);
        assert_eq!(shell.app().action_digest(), isolated_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        let restored = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = restored.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().isolate_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &isolate_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_hide_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(961);
        let selected_b = TagId(962);
        let other = TagId(963);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let hide_label = shell.catalog().text("tags-hide-selection");
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &hide_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let hidden = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = hidden.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &hide_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_show_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(964);
        let selected_b = TagId(965);
        let other = TagId(966);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let hide_label = shell.catalog().text("tags-hide-selection");
        shell.click_role_and_label(Role::Button, &hide_label);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));

        let show_label = shell.catalog().text("tags-show-selection");
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &show_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let shown = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = shown.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-shown-selected-tags",
                &BTreeMap::from([("count", "2".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let shown_revision = shell.app().document_revision();
        let shown_digest = shell.app().canonical_digest();
        let shown_undo_steps = shell.app().undo_step_count();
        let shown_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        assert_eq!(shell.app().document_revision(), shown_revision);
        assert_eq!(shell.app().canonical_digest(), shown_digest);
        assert_eq!(shell.app().undo_step_count(), shown_undo_steps);
        assert_eq!(shell.app().action_digest(), shown_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().show_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &show_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_invert_selection_is_localized_atomic_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let selected_a = TagId(967);
        let selected_b = TagId(968);
        let other = TagId(969);
        for (target, name) in [
            (selected_a, "Hardware"),
            (selected_b, "Fasteners"),
            (other, "Other"),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().create_box());
        assert!(shell.app_mut().create_box());
        for (target, tag) in [
            (OccurrenceId(1), selected_a),
            (OccurrenceId(2), selected_b),
            (OccurrenceId(3), other),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app_mut().set_tag_visibility(selected_b, false));
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let invert_label = shell.catalog().text("tags-invert-selection");
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.name().to_owned(),
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.tag(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &invert_label);

        assert_eq!(shell.app().tag_visibility(selected_a), Some(false));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(true));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let inverted = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = inverted.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.name().to_owned(),
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.tag(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-inverted-selected-tags",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(selected_a), Some(true));
        assert_eq!(shell.app().tag_visibility(selected_b), Some(false));
        assert_eq!(shell.app().tag_visibility(other), Some(true));
        assert_eq!(shell.app().selected_occurrence_count(), 2);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_selected_tags());
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);

        assert!(shell.app_mut().create_box());
        let untagged_revision = shell.app().document_revision();
        let untagged_digest = shell.app().canonical_digest();
        let untagged_undo_steps = shell.app().undo_step_count();
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().invert_selected_tags());
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &invert_label));
        assert_eq!(shell.app().document_revision(), untagged_revision);
        assert_eq!(shell.app().canonical_digest(), untagged_digest);
        assert_eq!(shell.app().undo_step_count(), untagged_undo_steps);
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
    }
}

#[test]
fn tags_panel_select_matching_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let hardware = TagId(971);
        let fasteners = TagId(972);
        for (target, name) in [(hardware, "Hardware"), (fasteners, "Fasteners")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        for _ in 0..5 {
            assert!(shell.app_mut().create_box());
        }
        assert_eq!(shell.app().occurrence_count(), 6);
        for (target, tag) in [
            (OccurrenceId(1), hardware),
            (OccurrenceId(2), hardware),
            (OccurrenceId(3), fasteners),
            (OccurrenceId(4), fasteners),
            (OccurrenceId(5), fasteners),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().prepare_assistant_intent(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(4),
                visible: false,
            }
        ));
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(shell.app_mut().select_tag_occurrences(hardware));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(2),
                    tag: Some(fasteners),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let select_label = shell.catalog().text("tags-select-matching");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 4);
        for id in [
            OccurrenceId(1),
            OccurrenceId(2),
            OccurrenceId(3),
            OccurrenceId(5),
        ] {
            assert!(shell.app().occurrence_is_selected(id));
        }
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(4)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(6)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-matching-tags",
                &BTreeMap::from([("count", "4".to_owned()), ("tags", "2".to_owned()),])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 4);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().set_all_tag_visibility(false));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 4);
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().tag_visibility(hardware), Some(true));
        assert_eq!(shell.app().tag_visibility(fasteners), Some(true));

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().select_untagged_occurrences());
        assert!(shell.app().occurrence_is_selected(OccurrenceId(6)));
        let untagged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_matching_tags());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(6)));
        assert_eq!(shell.app().action_digest(), untagged_action_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
    }
}

#[test]
fn tags_panel_select_all_tagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let hardware = TagId(981);
        let fasteners = TagId(982);
        let hidden = TagId(983);
        for (target, name, visible) in [
            (hardware, "Hardware", true),
            (fasteners, "Fasteners", true),
            (hidden, "Hidden", false),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        for _ in 0..5 {
            assert!(shell.app_mut().create_box());
        }
        assert_eq!(shell.app().occurrence_count(), 6);
        for (target, tag) in [
            (OccurrenceId(1), hardware),
            (OccurrenceId(2), hardware),
            (OccurrenceId(3), fasteners),
            (OccurrenceId(4), fasteners),
            (OccurrenceId(5), hidden),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        assert!(shell.app_mut().prepare_assistant_intent(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(4),
                visible: false,
            }
        ));
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let select_label = shell.catalog().text("tags-select-all-tagged");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 3);
        for id in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)] {
            assert!(shell.app().occurrence_is_selected(id));
        }
        for id in [OccurrenceId(4), OccurrenceId(5), OccurrenceId(6)] {
            assert!(!shell.app().occurrence_is_selected(id));
        }
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-all-tagged",
                &BTreeMap::from([("count", "3".to_owned())])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_all_tagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(shell.app_mut().set_tag_visibility(hardware, false));
        assert!(shell.app_mut().set_tag_visibility(fasteners, false));
        let empty_revision = shell.app().document_revision();
        let empty_digest = shell.app().canonical_digest();
        let empty_undo_steps = shell.app().undo_step_count();
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_all_tagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        assert_eq!(shell.app().document_revision(), empty_revision);
        assert_eq!(shell.app().canonical_digest(), empty_digest);
        assert_eq!(shell.app().undo_step_count(), empty_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));
    }
}

#[test]
fn tags_panel_select_tagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1001);
        let empty_tag = TagId(1002);
        for (target, name) in [(tag, "Hardware"), (empty_tag, "Empty")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.settle();
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        for target in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let select_label = shell.catalog().format(
            "tags-select",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_at(shell.top_face_centre(3));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(3)));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-tag",
                &BTreeMap::from([("name", "Hardware".to_owned()), ("count", "2".to_owned()),])
            )
        );

        let selection_digest = shell.app().action_digest().to_owned();
        shell.click_role_and_label(Role::Button, &select_label);
        assert!(!shell.app_mut().select_tag_occurrences(tag));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), selection_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        assert!(!shell.app_mut().select_tag_occurrences(empty_tag));
        assert!(!shell.app_mut().select_tag_occurrences(TagId(9999)));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert_eq!(shell.app().action_digest(), selection_digest);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        assert!(shell.app_mut().set_tag_visibility(tag, false));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();
        let hidden_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_tag_occurrences(tag));
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), hidden_revision);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps);
        assert_eq!(shell.app().action_digest(), hidden_action_digest);
    }
}

#[test]
fn tags_panel_select_untagged_is_localized_ephemeral_and_fail_closed() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1101);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: tag,
                    name: "Hardware".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: Some(tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(shell.app_mut().prepare_assistant_intent(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(3),
                visible: false,
            }
        ));
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();

        let select_label = shell.catalog().text("tags-select-untagged");
        assert!(shell.has_role_and_label(Role::Button, &select_label));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(!shell.app().occurrence_is_selected(OccurrenceId(3)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-selected-untagged",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

        let selected_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().select_untagged_occurrences());
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), selected_action_digest);
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(2),
                    tag: Some(tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        let tagged_revision = shell.app().document_revision();
        let tagged_digest = shell.app().canonical_digest();
        let tagged_undo_steps = shell.app().undo_step_count();
        let tagged_action_digest = shell.app().action_digest().to_owned();
        shell.settle();
        assert!(shell.has_role_and_label(Role::Button, &select_label));

        shell.click_role_and_label(Role::Button, &select_label);

        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert_eq!(shell.app().document_revision(), tagged_revision);
        assert_eq!(shell.app().canonical_digest(), tagged_digest);
        assert_eq!(shell.app().undo_step_count(), tagged_undo_steps);
        assert_eq!(shell.app().action_digest(), tagged_action_digest);
    }
}

#[test]
fn tags_panel_assign_selection_is_localized_canonical_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1201);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::CreateTag {
                    target: tag,
                    name: "Hardware".to_owned(),
                    visible: true,
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().occurrence_count(), 3);
        assert!(
            shell
                .app_mut()
                .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                    target: OccurrenceId(1),
                    tag: Some(tag),
                })
        );
        assert!(shell.app_mut().confirm_assistant_proposal());
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let assign_label = shell.catalog().format(
            "tags-assign-selection",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &assign_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &assign_label);

        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        let assigned = shell.app().document_snapshot();
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = assigned.occurrence(id).unwrap();
            assert_eq!(occurrence.tag(), Some(tag));
            assert_eq!(
                (
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-assigned-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("tag", "Hardware".to_owned()),])
            )
        );
        assert!(shell.has_role_and_label(Role::Button, &assign_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), None);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(3)), None);
        shell.click_role_and_label(Role::Button, &assign_label);
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        let unchanged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().assign_selection_to_tag(tag));
        assert!(!shell.app_mut().assign_selection_to_tag(TagId(9999)));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), unchanged_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.has_role_and_label(Role::Button, &assign_label));
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().assign_selection_to_tag(tag));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
    }
}

#[test]
fn tags_panel_remove_selection_is_localized_canonical_context_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let tag = TagId(1301);
        let other_tag = TagId(1302);
        for (target, name) in [(tag, "Hardware"), (other_tag, "Other")] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::CreateTag {
                        target,
                        name: name.to_owned(),
                        visible: true,
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert!(shell.app_mut().create_box());
        for (target, assigned_tag) in [
            (OccurrenceId(1), tag),
            (OccurrenceId(2), tag),
            (OccurrenceId(3), other_tag),
        ] {
            assert!(
                shell
                    .app_mut()
                    .prepare_assistant_intent(WorkflowIntent::SetOccurrenceTag {
                        target,
                        tag: Some(assigned_tag),
                    })
            );
            assert!(shell.app_mut().confirm_assistant_proposal());
        }
        shell
            .app_mut()
            .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
        shell.settle();
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert_eq!(shell.app().selected_occurrence_count(), 3);

        let remove_label = shell.catalog().format(
            "tags-remove-selection",
            &BTreeMap::from([("name", "Hardware".to_owned())]),
        );
        let remove_other_label = shell.catalog().format(
            "tags-remove-selection",
            &BTreeMap::from([("name", "Other".to_owned())]),
        );
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        assert!(shell.has_role_and_label(Role::Button, &remove_other_label));
        let before = shell.app().document_snapshot();
        let preserved = [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)].map(|id| {
            let occurrence = before.occurrence(id).unwrap();
            (
                occurrence.definition_id(),
                occurrence.transform(),
                occurrence.parent(),
                occurrence.visible(),
            )
        });
        let revision = shell.app().document_revision();
        let undo_steps = shell.app().undo_step_count();

        shell.click_role_and_label(Role::Button, &remove_label);

        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 3);
        let removed = shell.app().document_snapshot();
        assert_eq!(removed.occurrence(OccurrenceId(1)).unwrap().tag(), None);
        assert_eq!(removed.occurrence(OccurrenceId(2)).unwrap().tag(), None);
        assert_eq!(
            removed.occurrence(OccurrenceId(3)).unwrap().tag(),
            Some(other_tag)
        );
        for (index, id) in [OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]
            .into_iter()
            .enumerate()
        {
            let occurrence = removed.occurrence(id).unwrap();
            assert_eq!(
                (
                    occurrence.definition_id(),
                    occurrence.transform(),
                    occurrence.parent(),
                    occurrence.visible(),
                ),
                preserved[index]
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-removed-selected-tag",
                &BTreeMap::from([("count", "2".to_owned()), ("tag", "Hardware".to_owned()),])
            )
        );
        assert!(shell.has_role_and_label(Role::Button, &remove_label));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(1)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(2)), Some(tag));
        assert_eq!(shell.app().occurrence_tag(OccurrenceId(3)), Some(other_tag));
        shell.click_role_and_label(Role::Button, &remove_label);
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        let unchanged_revision = shell.app().document_revision();
        let unchanged_digest = shell.app().canonical_digest();
        let unchanged_undo_steps = shell.app().undo_step_count();
        let unchanged_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().remove_selection_from_tag(tag));
        assert!(!shell.app_mut().remove_selection_from_tag(TagId(9999)));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), unchanged_action_digest);

        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert!(shell.has_role_and_label(Role::Button, &remove_label));
        assert!(shell.has_role_and_label(Role::Button, &remove_other_label));
        let empty_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().remove_selection_from_tag(other_tag));
        assert_eq!(shell.app().document_revision(), unchanged_revision);
        assert_eq!(shell.app().canonical_digest(), unchanged_digest);
        assert_eq!(shell.app().undo_step_count(), unchanged_undo_steps);
        assert_eq!(shell.app().action_digest(), empty_action_digest);
    }
}

#[test]
fn hide_unhide_selection_is_localized_exact_state_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::Hide),
            shell.catalog().text("model-hide")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::Unhide),
            shell.catalog().text("model-unhide")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!shell.app().command_is_enabled(AppCommand::Unhide));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!shell.app().command_is_enabled(AppCommand::Unhide));
        let visible_digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::Hide);
        let hidden_digest = shell.app().canonical_digest();
        assert_ne!(hidden_digest, visible_digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                !shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );
        assert!(!shell.app().command_is_enabled(AppCommand::Hide));
        assert!(shell.app().command_is_enabled(AppCommand::Unhide));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        shell.click_menu_command("menu-view", AppCommand::Unhide);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 2);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app_mut().prepare_assistant_intent(
            WorkflowIntent::SetOccurrenceVisibility {
                target: OccurrenceId(1),
                visible: false,
            }
        ));
        assert!(shell.app_mut().confirm_assistant_proposal());
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(shell.app().command_is_enabled(AppCommand::Hide));
        assert!(shell.app().command_is_enabled(AppCommand::Unhide));
        let mixed_digest = shell.app().canonical_digest();
        let mixed_undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-view", AppCommand::Hide);
        assert_eq!(shell.app().undo_step_count(), mixed_undo_steps + 1);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                !shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), mixed_digest);
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        shell.click_menu_command("menu-view", AppCommand::Unhide);
        for id in [OccurrenceId(1), OccurrenceId(2)] {
            assert!(
                shell
                    .app()
                    .document_snapshot()
                    .occurrence(id)
                    .unwrap()
                    .visible()
            );
        }
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(!context_shell.app().command_is_enabled(AppCommand::Hide));
        assert!(!context_shell.app().command_is_enabled(AppCommand::Unhide));
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        assert!(!context_shell.app_mut().set_selection_visibility(false));
        assert!(!context_shell.app_mut().set_selection_visibility(true));
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn hide_others_is_localized_root_scope_exact_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::HideOthers),
            shell.catalog().text("model-hide-others")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        shell.click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(shell.app().command_is_enabled(AppCommand::HideOthers));
        let visible_revision = shell.app().document_revision();
        let visible_digest = shell.app().canonical_digest();
        let visible_undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::HideOthers));
        shell.click_command(AppCommand::HideOthers);

        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), visible_revision + 1);
        assert_eq!(shell.app().undo_step_count(), visible_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert!(
            shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(1))
                .unwrap()
                .visible()
        );
        assert!(
            !shell
                .app()
                .document_snapshot()
                .occurrence(OccurrenceId(2))
                .unwrap()
                .visible()
        );
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-hidden-others",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        let isolated_digest = shell.app().canonical_digest();
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-model", AppCommand::HideOthers);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), isolated_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.double_click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().edit_context_depth(), 1);
        assert!(!shell.app().command_is_enabled(AppCommand::HideOthers));
        let context_revision = shell.app().document_revision();
        let context_digest = shell.app().canonical_digest();
        let context_undo_steps = shell.app().undo_step_count();
        let context_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().hide_others());
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), context_revision);
        assert_eq!(shell.app().canonical_digest(), context_digest);
        assert_eq!(shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn unhide_all_is_localized_root_scope_exact_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::UnhideAll),
            shell.catalog().text("model-unhide-all")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::UnhideAll));

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        shell.click_menu_command("menu-view", AppCommand::Hide);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::UnhideAll));
        let hidden_revision = shell.app().document_revision();
        let hidden_digest = shell.app().canonical_digest();
        let hidden_undo_steps = shell.app().undo_step_count();

        shell.secondary_click_at(shell.top_face_centre(1));
        assert!(shell.offers(AppCommand::UnhideAll));
        shell.click_command(AppCommand::UnhideAll);

        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), hidden_revision + 1);
        assert_eq!(shell.app().undo_step_count(), hidden_undo_steps + 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(1)));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-unhidden-all",
                &BTreeMap::from([("count", "1".to_owned())])
            )
        );
        let visible_digest = shell.app().canonical_digest();

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), hidden_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-model", AppCommand::UnhideAll);
        assert_eq!(shell.app().hidden_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), visible_digest);
        assert_eq!(shell.app().selected_occurrence_count(), 1);

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.double_click_at(shell.top_face_centre(1));
        assert_eq!(shell.app().edit_context_depth(), 1);
        assert!(!shell.app().command_is_enabled(AppCommand::UnhideAll));
        let context_revision = shell.app().document_revision();
        let context_digest = shell.app().canonical_digest();
        let context_undo_steps = shell.app().undo_step_count();
        let context_action_digest = shell.app().action_digest().to_owned();
        shell.click_menu_command("menu-model", AppCommand::UnhideAll);
        assert_eq!(shell.app().hidden_occurrence_count(), 1);
        assert_eq!(shell.app().document_revision(), context_revision);
        assert_eq!(shell.app().canonical_digest(), context_digest);
        assert_eq!(shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(shell.app().action_digest(), context_action_digest);
    }
}

#[test]
fn ground_occurrence_is_localized_state_bound_and_undoable() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::GroundOccurrence),
            shell.catalog().text("model-ground-occurrence")
        );
        assert_eq!(
            shell.app().command_label(AppCommand::UngroundOccurrence),
            shell.catalog().text("model-unground-occurrence")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        let initial_revision = shell.app().document_revision();
        let initial_digest = shell.app().canonical_digest();
        let initial_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        assert_eq!(shell.app().document_revision(), initial_revision);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps);

        shell.click_at(shell.top_face_centre(1));
        let selected_reference = shell
            .app()
            .selected_reference()
            .expect("the exact root occurrence is primary");
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(selected_reference.instance_path.is_root());
        assert!(shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        let grounded_digest = shell.app().canonical_digest();
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(
            shell.app().selected_reference(),
            Some(selected_reference.clone())
        );
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-grounded-occurrence")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        let ungrounded_digest = shell.app().canonical_digest();
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert_eq!(shell.app().selected_reference(), Some(selected_reference));
        assert_eq!(shell.app().undo_step_count(), initial_undo_steps + 2);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-ungrounded-occurrence")
        );
        assert!(shell.app().command_is_enabled(AppCommand::GroundOccurrence));
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grounded_digest);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), initial_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().grounded_occurrence_count(), 1);
        assert_eq!(shell.app().canonical_digest(), grounded_digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().grounded_occurrence_count(), 0);
        assert_eq!(shell.app().canonical_digest(), ungrounded_digest);

        let mut multi_shell = Shell::with_catalog(catalog.clone());
        multi_shell.click_at(multi_shell.viewport_rect().center());
        assert!(
            multi_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        multi_shell.settle();
        multi_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        multi_shell.click_at_with(multi_shell.top_face_centre(1), shift());
        assert_eq!(multi_shell.app().selected_occurrence_count(), 2);
        assert!(
            !multi_shell
                .app()
                .command_is_enabled(AppCommand::GroundOccurrence)
        );
        assert!(
            !multi_shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        let multi_revision = multi_shell.app().document_revision();
        let multi_digest = multi_shell.app().canonical_digest();
        let multi_undo_steps = multi_shell.app().undo_step_count();
        multi_shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        multi_shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        assert_eq!(multi_shell.app().grounded_occurrence_count(), 0);
        assert_eq!(multi_shell.app().document_revision(), multi_revision);
        assert_eq!(multi_shell.app().canonical_digest(), multi_digest);
        assert_eq!(multi_shell.app().undo_step_count(), multi_undo_steps);

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        context_shell.double_click_at(context_shell.top_face_centre(2));
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::GroundOccurrence)
        );
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::UngroundOccurrence)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        context_shell.click_menu_command("menu-model", AppCommand::GroundOccurrence);
        context_shell.click_menu_command("menu-model", AppCommand::UngroundOccurrence);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
    }
}

#[test]
fn occurrence_align_is_localized_previewed_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::AlignOccurrences),
            shell.catalog().text("model-align-occurrences")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::AlignOccurrences));

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app_mut()
                .copy_selected(Vec3::new(300.0, 300.0, 100.0))
        );
        shell
            .app_mut()
            .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Tab);
        shell.settle();
        let definition_name = shell.catalog().format(
            "model-default-box",
            &BTreeMap::from([("number", "1".to_owned())]),
        );
        let component_heading = shell.catalog().format(
            "outliner-component",
            &BTreeMap::from([
                ("name", definition_name),
                ("count", "2".to_owned()),
                ("dimensions", "100 × 60 × 20".to_owned()),
            ]),
        );
        shell.click_row(&component_heading);
        let reference_name = shell
            .app()
            .occurrence_name(OccurrenceId(1))
            .expect("the reference occurrence exists");
        let reference_row = shell.catalog().format(
            "outliner-instance",
            &BTreeMap::from([("visibility", "◉".to_owned()), ("name", reference_name)]),
        );
        shell.click_row(&reference_row);
        shell
            .app_mut()
            .set_assistant_workspace_mode(ketchup_app::AssistantWorkspaceMode::Dock);
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert_eq!(
            shell.app().occurrence_align_inputs(),
            Some((OccurrenceId(2), OccurrenceId(1), Axis::X, AlignMode::Center))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );
        assert!(!shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-axis-y"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-minimum"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 0.0, 100.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-axis-z"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-preview"),
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 300.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-align-committed")
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            Some(DefinitionId(1))
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(1))
        );
        let aligned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), aligned_digest);

        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert!(!shell.app().occurrence_align_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert!(shell.app_mut().preview_pending_occurrence_align());
        assert!(shell.app().occurrence_align_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        assert!(!shell.app().occurrence_align_preview_is_current());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-align-occurrences-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_at_with(shell.top_face_centre(2), shift());
        assert!(shell.app().command_is_enabled(AppCommand::AlignOccurrences));
        shell.click_menu_command("menu-model", AppCommand::AlignOccurrences);
        let (moving_id, reference_id, _, _) = shell.app().occurrence_align_inputs().unwrap();
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_align());
        assert!(!shell.app_mut().confirm_occurrence_align());
        assert!(shell.app().occurrence_align_visible());
        assert_eq!(shell.app().occurrence_definition_id(moving_id), None);
        assert_eq!(shell.app().occurrence_definition_id(reference_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn distribute_occurrences_is_localized_previewed_even_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::DistributeOccurrences),
            shell.catalog().text("model-distribute-occurrences")
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(!shell.app().occurrence_distribution_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(100.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(400.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(400.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert_eq!(shell.app().occurrence_distribution_axis(), Some(Axis::X));
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-axis-y"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-axis-x"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(300.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(3)),
            Some((Vec3::new(600.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        assert!(!shell.app_mut().confirm_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision + 2);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-distribute-committed")
        );
        let origins = (1..=4)
            .map(|id| shell.app().occurrence_box_geometry(id).unwrap().0.x)
            .collect::<Vec<_>>();
        assert_eq!(origins, vec![0.0, 300.0, 600.0, 900.0]);
        let distributed_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), distributed_digest);

        shell.click_at(shell.top_face_centre(2));
        assert!(shell.app_mut().move_selected(Vec3::new(100.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::DistributeOccurrences)
        );
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert!(!shell.app().occurrence_distribution_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(shell.app_mut().preview_pending_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().occurrence_distribution_preview_is_current());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-distribute-occurrences-cancel"),
        );

        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_occurrence_distribution());
        assert!(!shell.app_mut().confirm_occurrence_distribution());
        assert!(shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn distribute_equal_gaps_is_localized_previewed_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(200.0, 0.0, 0.0)));
        assert!(shell.app_mut().rotate_selected(90.0));
        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app_mut().copy_selected(Vec3::new(500.0, 0.0, 0.0)));
        assert!(shell.app_mut().copy_selected(Vec3::new(300.0, 0.0, 0.0)));
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        let first = shell.app().occurrence_box_geometry(1).unwrap();
        let last = shell.app().occurrence_box_geometry(4).unwrap();

        shell.click_menu_command("menu-model", AppCommand::DistributeOccurrences);
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::Centers)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-equal-gaps"),
        );
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::EqualGaps)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        let second = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .unwrap();
        let third = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(3))
            .unwrap();
        let gaps = [
            second.0.x - (first.0.x + first.1.x),
            third.0.x - (second.0.x + second.1.x),
            last.0.x - (third.0.x + third.1.x),
        ];
        assert!(gaps.iter().all(|gap| (*gap - gaps[0]).abs() < 1.0e-9));

        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-centers"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-equal-gaps"),
        );
        assert_eq!(
            shell.app().occurrence_distribution_mode(),
            Some(DistributionMode::EqualGaps)
        );
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert!(shell.app().occurrence_distribution_preview_is_current());
        shell.settle();
        shell.click_role_and_label(
            Role::Button,
            &shell
                .catalog()
                .text("dialog-distribute-occurrences-confirm"),
        );
        assert!(!shell.app().occurrence_distribution_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-distribute-committed")
        );
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(DefinitionId(1))
        );
        let distributed_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), distributed_digest);
    }
}

#[test]
fn distribute_equal_gaps_rejects_an_overlapping_span_without_mutation() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    assert!(shell.app_mut().copy_selected(Vec3::new(10.0, 0.0, 0.0)));
    assert!(shell.app_mut().copy_selected(Vec3::new(10.0, 0.0, 0.0)));
    shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().preview_distribute_occurrences(
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]),
        Axis::X,
        DistributionMode::EqualGaps,
    ));
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    let mut stale_shell = Shell::new();
    stale_shell.click_at(stale_shell.top_face_centre(1));
    assert!(
        stale_shell
            .app_mut()
            .copy_selected(Vec3::new(150.0, 0.0, 0.0))
    );
    assert!(
        stale_shell
            .app_mut()
            .copy_selected(Vec3::new(250.0, 0.0, 0.0))
    );
    stale_shell.click_menu_command("menu-edit", AppCommand::SelectAll);
    assert!(stale_shell.app_mut().preview_distribute_occurrences(
        &BTreeSet::from([OccurrenceId(1), OccurrenceId(2), OccurrenceId(3)]),
        Axis::X,
        DistributionMode::EqualGaps,
    ));
    assert!(stale_shell.app_mut().create_box());
    let stale_revision = stale_shell.app().document_revision();
    let stale_digest = stale_shell.app().canonical_digest();
    assert!(!stale_shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(stale_shell.app().document_revision(), stale_revision);
    assert_eq!(stale_shell.app().canonical_digest(), stale_digest);
}

#[test]
fn linear_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::LinearPattern),
            shell.catalog().text("model-linear-pattern")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::LinearPattern));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(!shell.app().linear_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::LinearPattern));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert_eq!(
            shell.app().linear_pattern_inputs(),
            Some((Axis::X, "100", "2"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );
        assert!(!shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        let spacing_label = shell.catalog().text("dialog-linear-pattern-spacing");
        shell.focus_text_input(&spacing_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.focus_text_input(&spacing_label);
        shell.type_text("125");
        let count_label = shell.catalog().text("dialog-linear-pattern-count");
        shell.focus_text_input(&count_label);
        shell.key(Key::A, ctrl());
        shell.type_text("4");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-preview"),
        );
        assert!(shell.app().linear_pattern_visible());
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(4)),
            Some((Vec3::new(375.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-confirm"),
        );
        assert!(!shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-linear-pattern-committed")
        );
        for occurrence_id in 1..=4 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        let definition_name = shell.app().definition_name(DefinitionId(1)).unwrap();
        for occurrence_id in 2..=4 {
            assert_eq!(
                shell.app().occurrence_name(OccurrenceId(occurrence_id)),
                Some(shell.catalog().format(
                    "model-copy-occurrence",
                    &BTreeMap::from([
                        ("name", definition_name.clone()),
                        ("number", occurrence_id.to_string()),
                    ]),
                ))
            );
            assert_eq!(
                shell.app().occurrence_box_geometry(occurrence_id),
                Some((
                    Vec3::new(125.0 * (occurrence_id - 1) as f64, 0.0, 0.0),
                    Vec3::new(100.0, 60.0, 20.0),
                ))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 4);

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::LinearPattern));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert!(!shell.app().linear_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().preview_pending_linear_pattern());
        assert!(shell.app().linear_pattern_preview_is_current());
        assert!(shell.app_mut().preview_rectangular_pattern(
            OccurrenceId(1),
            RectangularPatternSpec {
                primary_axis: Axis::X,
                primary_spacing_mm: 200.0,
                primary_count: 2,
                secondary_axis: Axis::Y,
                secondary_spacing_mm: 200.0,
                secondary_count: 2,
            },
        ));
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().linear_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-linear-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::LinearPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_linear_pattern());
        assert!(!shell.app_mut().confirm_linear_pattern());
        assert!(shell.app().linear_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn rectangular_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::RectangularPattern),
            shell.catalog().text("model-rectangular-pattern")
        );
        assert!(
            !shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(!shell.app().rectangular_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert_eq!(
            shell.app().rectangular_pattern_inputs(),
            Some((Axis::X, "100", "2", Axis::Y, "100", "2"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );
        assert!(!shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        for (key, value) in [
            ("dialog-rectangular-pattern-primary-spacing", "125"),
            ("dialog-rectangular-pattern-primary-count", "3"),
            ("dialog-rectangular-pattern-secondary-spacing", "75"),
        ] {
            let label = shell.catalog().text(key);
            shell.focus_text_input(&label);
            shell.key(Key::A, ctrl());
            shell.type_text(value);
        }
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(2)),
            Some((Vec3::new(0.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(6)),
            Some((Vec3::new(250.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );

        let primary_spacing = shell
            .catalog()
            .text("dialog-rectangular-pattern-primary-spacing");
        shell.focus_text_input(&primary_spacing);
        shell.key(Key::A, ctrl());
        shell.type_text("150");
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-preview"),
        );
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(
            shell
                .app()
                .occurrence_operation_preview_geometry(OccurrenceId(6)),
            Some((Vec3::new(300.0, 75.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-confirm"),
        );
        assert!(!shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 6);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-rectangular-pattern-committed")
        );
        for occurrence_id in 1..=6 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 6);

        shell.click_at(shell.top_face_centre(1));
        assert!(
            shell
                .app()
                .command_is_enabled(AppCommand::RectangularPattern)
        );
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert!(!shell.app().rectangular_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().preview_pending_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().rectangular_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rectangular-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::RectangularPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_rectangular_pattern());
        assert!(!shell.app_mut().confirm_rectangular_pattern());
        assert!(shell.app().rectangular_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn rectangular_pattern_rejects_invalid_and_stale_inputs_without_mutation() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();
    let undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 2,
            secondary_axis: Axis::X,
            secondary_spacing_mm: 100.0,
            secondary_count: 2,
        },
    ));
    assert!(!shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 101,
            secondary_axis: Axis::Y,
            secondary_spacing_mm: 100.0,
            secondary_count: 100,
        },
    ));
    assert!(!shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().undo_step_count(), undo_steps);

    assert!(shell.app_mut().preview_rectangular_pattern(
        OccurrenceId(1),
        RectangularPatternSpec {
            primary_axis: Axis::X,
            primary_spacing_mm: 100.0,
            primary_count: 2,
            secondary_axis: Axis::Y,
            secondary_spacing_mm: 100.0,
            secondary_count: 2,
        },
    ));
    assert!(shell.app_mut().create_box());
    let stale_revision = shell.app().document_revision();
    let stale_digest = shell.app().canonical_digest();
    let stale_undo_steps = shell.app().undo_step_count();
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), stale_revision);
    assert_eq!(shell.app().canonical_digest(), stale_digest);
    assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
}

#[test]
fn circular_pattern_is_localized_previewed_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(
            shell.app().command_label(AppCommand::CircularPattern),
            shell.catalog().text("model-circular-pattern")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::CircularPattern));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(!shell.app().circular_pattern_visible());

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CircularPattern));
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert_eq!(
            shell.app().circular_pattern_inputs(),
            Some((Axis::Z, "0", "0", "0", "90", "4"))
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );
        assert!(!shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        let angle_label = shell.catalog().text("dialog-circular-pattern-angle");
        shell.focus_text_input(&angle_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().canonical_digest(), digest);

        shell.focus_text_input(&angle_label);
        shell.type_text("90");
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        assert!(shell.app().circular_pattern_visible());
        assert!(shell.app().has_occurrence_operation_preview());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the first rotated occurrence is previewed");
        assert!((origin.x + 60.0).abs() < 1.0e-9);
        assert!(origin.y.abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 60.0).abs() < 1.0e-9);
        assert!((size.y - 100.0).abs() < 1.0e-9);
        assert!((size.z - 20.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-axis-x"),
        );
        assert!(!shell.app().has_occurrence_operation_preview());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the X-axis occurrence is previewed");
        assert!(origin.x.abs() < 1.0e-9);
        assert!((origin.y + 20.0).abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 100.0).abs() < 1.0e-9);
        assert!((size.y - 20.0).abs() < 1.0e-9);
        assert!((size.z - 60.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-axis-z"),
        );
        let centre_x_label = shell.catalog().text("dialog-circular-pattern-centre-x");
        shell.focus_text_input(&centre_x_label);
        shell.key(Key::A, ctrl());
        shell.type_text("100");
        assert!(!shell.app().has_occurrence_operation_preview());
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-preview"),
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2))
            .expect("the offset-center occurrence is previewed");
        assert!((origin.x - 40.0).abs() < 1.0e-9);
        assert!((origin.y + 100.0).abs() < 1.0e-9);
        assert!(origin.z.abs() < 1.0e-9);
        assert!((size.x - 60.0).abs() < 1.0e-9);
        assert!((size.y - 100.0).abs() < 1.0e-9);
        assert!((size.z - 20.0).abs() < 1.0e-9);

        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-confirm"),
        );
        assert!(!shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), revision + 1);
        assert_eq!(shell.app().occurrence_count(), 4);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-circular-pattern-committed")
        );
        for occurrence_id in 1..=4 {
            assert_eq!(
                shell
                    .app()
                    .occurrence_definition_id(OccurrenceId(occurrence_id)),
                Some(DefinitionId(1))
            );
        }
        let patterned_digest = shell.app().canonical_digest();
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().occurrence_count(), 1);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), patterned_digest);
        assert_eq!(shell.app().occurrence_count(), 4);

        shell.click_at(shell.top_face_centre(1));
        assert!(shell.app().command_is_enabled(AppCommand::CircularPattern));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_revision = shell.app().document_revision();
        let drift_digest = shell.app().canonical_digest();
        let drift_undo_steps = shell.app().undo_step_count();
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert!(!shell.app().circular_pattern_preview_is_current());
        assert_eq!(shell.app().document_revision(), drift_revision);
        assert_eq!(shell.app().canonical_digest(), drift_digest);
        assert_eq!(shell.app().undo_step_count(), drift_undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().preview_pending_circular_pattern());
        assert!(shell.app().circular_pattern_preview_is_current());
        assert!(
            shell
                .app_mut()
                .preview_linear_pattern(OccurrenceId(1), Axis::X, 100.0, 2,)
        );
        let foreign_revision = shell.app().document_revision();
        let foreign_digest = shell.app().canonical_digest();
        let foreign_undo_steps = shell.app().undo_step_count();
        let foreign_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app().circular_pattern_preview_is_current());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), foreign_revision);
        assert_eq!(shell.app().canonical_digest(), foreign_digest);
        assert_eq!(shell.app().undo_step_count(), foreign_undo_steps);
        assert_eq!(shell.app().action_digest(), foreign_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().rotate_selected(15.0));
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-circular-pattern-cancel"),
        );

        shell.click_menu_command("menu-model", AppCommand::CircularPattern);
        assert!(shell.app_mut().delete_selected());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().preview_pending_circular_pattern());
        assert!(!shell.app_mut().confirm_circular_pattern());
        assert!(shell.app().circular_pattern_visible());
        assert_eq!(shell.app().occurrence_definition_id(OccurrenceId(1)), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }

    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let digest = shell.app().canonical_digest();
    assert!(shell.app_mut().preview_circular_pattern(
        OccurrenceId(1),
        Axis::Z,
        Vec3::new(0.0, 0.0, 0.0),
        45.0,
        3,
    ));
    assert!(shell.app_mut().copy_selected(Vec3::new(200.0, 0.0, 0.0)));
    let changed_digest = shell.app().canonical_digest();
    assert_ne!(changed_digest, digest);
    assert!(!shell.app_mut().confirm_occurrence_operation_preview());
    assert_eq!(shell.app().canonical_digest(), changed_digest);
    assert_eq!(shell.app().occurrence_count(), 2);
}

#[test]
fn rename_definition_is_localized_validated_shared_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        let definition_id = DefinitionId(1);
        let original_name = shell
            .app()
            .definition_name(definition_id)
            .expect("the initial definition exists");
        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-edit", AppCommand::Copy);
        shell.click_menu_command("menu-edit", AppCommand::Paste);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            shell.app().occurrence_definition_id(OccurrenceId(2))
        );
        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();

        assert_eq!(
            shell.app().command_label(AppCommand::RenameDefinition),
            shell.catalog().text("model-rename-definition")
        );
        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        assert!(shell.app().rename_definition_visible());
        assert_eq!(
            shell.app().rename_definition_input(),
            Some(original_name.as_str())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        assert!(!shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&shell.catalog().text("dialog-rename-definition-name"));
        shell.key(Key::A, ctrl());
        shell.type_text("Selection drift");
        shell.click_menu_command("menu-edit", AppCommand::Deselect);
        let drift_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().selected_occurrence_count(), 0);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
        assert_eq!(shell.app().action_digest(), drift_action_digest);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        let input_label = shell.catalog().text("dialog-rename-definition-name");
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.press_key(Key::Backspace);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-confirm"),
        );
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);

        let renamed = "Shared housing";
        shell.focus_text_input(&input_label);
        shell.type_text(renamed);
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-confirm"),
        );
        assert!(!shell.app().rename_definition_visible());
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(renamed.to_owned())
        );
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().undo_step_count(), undo_steps + 1);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-renamed-definition",
                &BTreeMap::from([("name", renamed.to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(original_name.clone())
        );
        assert_eq!(shell.app().occurrence_count(), 2);

        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Stale definition");
        assert!(shell.app_mut().create_box());
        let stale_revision = shell.app().document_revision();
        let stale_digest = shell.app().canonical_digest();
        let stale_undo_steps = shell.app().undo_step_count();
        let stale_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().document_revision(), stale_revision);
        assert_eq!(shell.app().canonical_digest(), stale_digest);
        assert_eq!(shell.app().undo_step_count(), stale_undo_steps);
        assert_eq!(shell.app().action_digest(), stale_action_digest);
        assert_eq!(
            shell.app().definition_name(definition_id),
            Some(original_name.clone())
        );
        shell.click_role_and_label(
            Role::Button,
            &shell.catalog().text("dialog-rename-definition-cancel"),
        );
        shell.click_menu_command("menu-edit", AppCommand::Undo);

        shell.click_at(shell.top_face_centre(1));
        shell.click_menu_command("menu-model", AppCommand::SelectAllInstances);
        shell.click_menu_command("menu-model", AppCommand::RenameDefinition);
        shell.focus_text_input(&input_label);
        shell.key(Key::A, ctrl());
        shell.type_text("Missing definition");
        assert!(shell.app_mut().delete_selected());
        assert!(shell.app_mut().purge_unused_definitions());
        let missing_revision = shell.app().document_revision();
        let missing_digest = shell.app().canonical_digest();
        let missing_undo_steps = shell.app().undo_step_count();
        let missing_action_digest = shell.app().action_digest().to_owned();
        assert!(!shell.app_mut().confirm_definition_rename());
        assert!(shell.app().rename_definition_visible());
        assert_eq!(shell.app().definition_name(definition_id), None);
        assert_eq!(shell.app().document_revision(), missing_revision);
        assert_eq!(shell.app().canonical_digest(), missing_digest);
        assert_eq!(shell.app().undo_step_count(), missing_undo_steps);
        assert_eq!(shell.app().action_digest(), missing_action_digest);
    }
}

#[test]
fn purge_unused_definitions_is_localized_safe_and_one_undo_step() {
    for catalog in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ] {
        let mut shell = Shell::with_catalog(catalog);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().occurrence_count(), 1);
        assert_eq!(shell.app().purgeable_definition_count(), 0);
        assert_eq!(
            shell.app().command_label(AppCommand::PurgeUnused),
            shell.catalog().text("model-purge-unused")
        );

        assert!(shell.app_mut().create_box());
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::SelectAll);
        shell.click_menu_command("menu-edit", AppCommand::Delete);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 2);
        let after_delete_steps = shell.app().undo_step_count();

        shell.click_menu_command("menu-model", AppCommand::PurgeUnused);
        assert_eq!(shell.app().definition_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 0);
        assert_eq!(shell.app().undo_step_count(), after_delete_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().format(
                "digest-purged-unused",
                &BTreeMap::from([("count", "2".to_owned())])
            )
        );

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 0);
        assert_eq!(shell.app().purgeable_definition_count(), 2);
        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().definition_count(), 0);
        assert_eq!(shell.app().occurrence_count(), 0);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().occurrence_count(), 2);
        assert_eq!(shell.app().purgeable_definition_count(), 0);

        let revision = shell.app().document_revision();
        let digest = shell.app().canonical_digest();
        let undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::PurgeUnused);
        assert_eq!(shell.app().document_revision(), revision);
        assert_eq!(shell.app().canonical_digest(), digest);
        assert_eq!(shell.app().undo_step_count(), undo_steps);
    }
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
fn push_pull_without_a_selected_face_never_targets_the_initial_box() {
    let mut shell = Shell::new();
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("15");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
    assert_eq!(shell.app().document_height_mm(), 20.0);
    assert!(!shell.app().can_undo());
    assert!(!shell.app().has_smart_push_pull_chooser());
    assert_eq!(
        shell.app().action_digest(),
        shell.catalog().text("error-push-pull-selection-required")
    );
}

#[test]
fn localized_smart_push_pull_chooser_cancels_without_mutation_through_accesskit() {
    let mut shell = Shell::with_catalog(LocaleCatalog::slovak());
    let hole_center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(hole_center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(hole_center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let revision = shell.app().document_revision();
    let digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("-20");
    shell.press_key(Key::Enter);

    let box_name = shell.catalog().format(
        "model-default-box",
        &BTreeMap::from([("number", "1".to_owned())]),
    );
    let occurrence = shell.catalog().format(
        "model-default-occurrence",
        &BTreeMap::from([("name", box_name)]),
    );
    let target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", shell.catalog().text("model-default-extrusion")),
            ("feature_id", "2".to_owned()),
            ("occurrence", occurrence),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    let cancel_label = shell.catalog().text("choice-smart-push-pull-cancel");
    assert!(shell.app().has_smart_push_pull_chooser());
    assert!(shell.has_role_and_label(
        Role::RadioButton,
        &shell.catalog().text("choice-smart-push-pull-new-feature")
    ));
    assert!(shell.has_role_and_label(Role::RadioButton, &target_label));
    assert!(shell.has_role_and_label(Role::Button, &cancel_label));
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);

    shell.click_role_and_label(Role::Button, &cancel_label);
    assert!(!shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), revision);
    assert_eq!(shell.app().canonical_digest(), digest);
}

#[test]
fn circle_push_pull_correction_chooser_and_preview_are_atomic_through_accesskit() {
    let mut shell = Shell::new();
    let center = Vec3::new(35.0, 25.0, 20.0);
    shell.click_command(AppCommand::Circle);
    shell.click_at(shell.app().viewport_position(center).unwrap());
    shell.click_at(
        shell
            .app()
            .viewport_position(center + Vec3::new(10.0, 0.0, 0.0))
            .unwrap(),
    );
    let profile_revision = shell.app().document_revision();
    let profile_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("10");
    shell.press_key(Key::Enter);
    let original_revision = shell.app().document_revision();
    let original_digest = shell.app().canonical_digest();
    assert_eq!(original_revision, profile_revision + 1);

    let cut_target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", "Extrusion".to_owned()),
            ("feature_id", "2".to_owned()),
            ("occurrence", "Box-1 #1".to_owned()),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    let continue_label = shell.catalog().text("choice-smart-push-pull-continue");
    let cancel_label = shell.catalog().text("choice-smart-push-pull-cancel");

    shell.type_text("-20");
    shell.press_key(Key::Enter);
    assert!(shell.app().has_smart_push_pull_chooser());
    assert_eq!(shell.app().document_revision(), original_revision);
    assert_eq!(shell.app().canonical_digest(), original_digest);
    shell.click_role_and_label(Role::Button, &cancel_label);
    assert_eq!(shell.app().document_revision(), original_revision);
    assert_eq!(shell.app().canonical_digest(), original_digest);

    shell.type_text("-20");
    shell.press_key(Key::Enter);
    shell.click_role_and_label(Role::RadioButton, &cut_target_label);
    shell.click_role_and_label(Role::Button, &continue_label);
    assert!(shell.app().has_occurrence_operation_preview());
    assert_eq!(shell.app().document_revision(), original_revision);
    assert_eq!(shell.app().canonical_digest(), original_digest);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), original_revision);
    assert_eq!(shell.app().canonical_digest(), original_digest);

    shell.type_text("-20");
    shell.press_key(Key::Enter);
    shell.click_role_and_label(Role::RadioButton, &cut_target_label);
    shell.click_role_and_label(Role::Button, &continue_label);
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().document_revision(), original_revision + 1);
    assert_ne!(shell.app().canonical_digest(), original_digest);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().document_revision(), profile_revision);
    assert_eq!(shell.app().canonical_digest(), profile_digest);
    assert_eq!(shell.app().circle_profile_count(), 1);
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

    let cut_target_label = shell.catalog().format(
        "choice-smart-push-pull-cut-target",
        &BTreeMap::from([
            ("feature", "Extrusion".to_owned()),
            ("feature_id", "2".to_owned()),
            ("occurrence", "Box-1 #1".to_owned()),
            ("occurrence_id", "1".to_owned()),
        ]),
    );
    let continue_label = shell.catalog().text("choice-smart-push-pull-continue");
    shell.click_command(AppCommand::PushPull);
    shell.app_mut().set_push_pull_distance_input("-20");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    assert!(shell.app().has_smart_push_pull_chooser());
    assert!(shell.has_role_and_label(
        Role::RadioButton,
        &shell.catalog().text("choice-smart-push-pull-new-feature")
    ));
    assert!(shell.has_role_and_label(Role::RadioButton, &cut_target_label));
    assert_eq!(shell.app().document_revision(), hole_profile_revision);
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    shell.click_role_and_label(Role::Button, &continue_label);
    assert!(!shell.app().has_smart_push_pull_chooser());
    assert!(!shell.app().has_occurrence_operation_preview());
    assert!(shell.app().preview_action_digest().is_some());
    assert_eq!(shell.app().document_revision(), hole_profile_revision);
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    shell.press_key(Key::Enter);
    let independent_feature_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), hole_profile_revision + 1);
    assert_ne!(independent_feature_digest, hole_profile_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), independent_feature_digest);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), hole_profile_digest);

    shell.app_mut().set_push_pull_distance_input("-20");
    assert!(shell.app_mut().start_preview());
    shell.settle();
    shell.click_role_and_label(Role::RadioButton, &cut_target_label);
    shell.click_role_and_label(Role::Button, &continue_label);
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
    assert!(shell.app().has_smart_push_pull_chooser());
    shell.click_role_and_label(Role::RadioButton, &cut_target_label);
    shell.click_role_and_label(Role::Button, &continue_label);
    assert!(shell.app().has_occurrence_operation_preview());
    shell.press_key(Key::Enter);
    let hole_digest = shell.app().canonical_digest();
    assert!(shell.app().document_revision() > hole_profile_revision);
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
fn exact_worker_preserves_assembly_references_while_publishing_general_finish_topology() {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let locator = TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind: TopologicalElementKind::Face,
        ordinal: 3,
    };
    let mut prepared = false;
    for _ in 0..150 {
        shell.settle();
        if shell.app().exact_stable_reference_count() >= 2
            && shell.app_mut().prepare_assistant_general_finish(
                locator.clone(),
                GeneralFinishKind::Shell,
                2.0,
            )
        {
            prepared = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(prepared, "{}", shell.app().action_digest());
    assert!(shell.app().exact_stable_reference_count() >= 2);
    let preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(preview.0, FeatureId(2));
    assert_eq!(preview.1.kind, TopologicalElementKind::Face);
    assert_eq!(preview.1.producer_element_id, "generated-result/face/3");
    assert_eq!(preview.2, GeneralFinishKind::Shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
}

fn install_general_finish_graph_result(shell: &mut Shell, producer_feature_id: FeatureId) {
    let snapshot = shell.app().document_snapshot();
    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, DefinitionId(1), producer_feature_id).unwrap();
    let vertices_mm = vec![
        [0.0, 0.0, 0.0],
        [100.0, 0.0, 0.0],
        [0.0, 60.0, 0.0],
        [100.0, 60.0, 0.0],
        [0.0, 0.0, 20.0],
        [100.0, 0.0, 20.0],
        [0.0, 60.0, 20.0],
        [100.0, 60.0, 20.0],
    ];
    let triangles = [
        ([0, 2, 1], 0),
        ([1, 2, 3], 0),
        ([4, 5, 6], 1),
        ([5, 7, 6], 1),
        ([0, 1, 4], 2),
        ([1, 5, 4], 2),
        ([2, 6, 3], 3),
        ([3, 6, 7], 3),
        ([0, 4, 2], 4),
        ([2, 4, 6], 4),
        ([1, 3, 5], 5),
        ([3, 7, 5], 5),
    ]
    .into_iter()
    .map(|(vertex_indices, face_ordinal)| StepMeshTriangle {
        vertex_indices,
        face_ordinal,
    })
    .collect();
    let package = ExactBRepGraphPackage::from_worker_evidence(
        &graph,
        ExactBRepGraphWorkerEvidence {
            exact_input_digest: format!("headless-finish-input-{}", producer_feature_id.0),
            result_fingerprint: format!("headless-finish-result-{}", producer_feature_id.0),
            volume_mm3: 120_000.0,
            topology_counts: [8, 12, 6, 1, 1],
            bounds_mm: [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            backend: "headless-finish-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
        },
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .unwrap();
    assert!(
        shell
            .app_mut()
            .headless_install_exact_package(ExactBodyPackage::Graph(package))
    );
}

fn select_general_finish_topology(
    shell: &mut Shell,
    producer_feature_id: FeatureId,
    kind: TopologicalElementKind,
    ordinal: u32,
) {
    assert!(
        shell
            .app_mut()
            .select_topological_locator(TopologicalPickLocator {
                instance_path: InstancePath::root(OccurrenceId(1)),
                producer_feature_id,
                kind,
                ordinal,
            })
    );
}

#[test]
fn topology_bound_push_pull_edits_a_non_top_planar_face_through_headless_ui() {
    let mut shell = Shell::new();
    install_general_finish_graph_result(&mut shell, FeatureId(2));
    select_general_finish_topology(&mut shell, FeatureId(2), TopologicalElementKind::Face, 3);
    assert_eq!(
        shell.app().selected_reference().unwrap().element,
        ElementId::Face {
            axis: Axis::Y,
            side: Side::Maximum,
        }
    );
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::PushPull);
    shell.type_text("5");
    shell.press_key(Key::Enter);

    assert_eq!(shell.app().document_revision(), before_revision + 1);
    let committed_digest = shell.app().canonical_digest();
    assert_ne!(committed_digest, before_digest);
    assert_eq!(
        shell.app().occurrence_box_geometry(1),
        Some((Vec3::new(0.0, 0.0, 0.0), Vec3::new(100.0, 65.0, 20.0)))
    );
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
}

#[test]
fn imported_exact_finishes_and_face_push_pull_recompute_through_headless_ui() {
    let source_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpora/r0/step/self-authored-box.step");
    let directory = tempfile::tempdir().unwrap();
    let document_path = directory.path().join("imported-face-push-pull.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_import(ImportFormat::Step, &source_path)
        .queue_save(&document_path)
        .queue_open(&document_path)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().delete_selected());
    shell.settle();

    shell.click_menu_command("menu-file", AppCommand::ImportExactStep);
    shell.click_button_label(&shell.catalog().text("dialog-import-step-confirm"));
    wait_for_one_exact_body(&mut shell);
    let snapshot = shell.app().document_snapshot();
    let imported = snapshot
        .features()
        .find(|feature| matches!(feature.kind(), FeatureKind::ImportedExactBody(_)))
        .unwrap();
    let imported_feature_id = imported.id();
    let imported_definition_id = imported.definition_id();
    let imported_occurrence_id = snapshot
        .occurrences()
        .find(|occurrence| occurrence.definition_id() == imported_definition_id)
        .unwrap()
        .id();
    assert_eq!(
        shell.app().exact_current_producer_ids(),
        [imported_feature_id]
    );
    let imported_digest = shell.app().canonical_digest();
    for (kind, element_kind, ordinal, amount_mm) in [
        (
            GeneralFinishKind::Shell,
            TopologicalElementKind::Face,
            1,
            1.0,
        ),
        (
            GeneralFinishKind::Fillet,
            TopologicalElementKind::Edge,
            0,
            0.75,
        ),
        (
            GeneralFinishKind::Chamfer,
            TopologicalElementKind::Edge,
            5,
            0.75,
        ),
    ] {
        let before_revision = shell.app().document_revision();
        let before_undo_steps = shell.app().undo_step_count();
        assert!(shell.app_mut().prepare_assistant_general_finish(
            TopologicalPickLocator {
                instance_path: InstancePath::root(imported_occurrence_id),
                producer_feature_id: imported_feature_id,
                kind: element_kind,
                ordinal,
            },
            kind,
            amount_mm,
        ));
        let preview = shell.app().general_finish_preview_parameters().unwrap();
        assert_eq!(preview.0, imported_feature_id);
        assert_eq!(preview.1.kind, element_kind);
        assert_eq!(preview.1.producer_feature_id, imported_feature_id);
        assert_eq!(preview.2, kind);
        assert_eq!(preview.3, amount_mm);
        assert_eq!(shell.app().document_revision(), before_revision);
        assert_eq!(shell.app().canonical_digest(), imported_digest);
        assert!(shell.app_mut().confirm_assistant_general_finish());
        assert!(shell.app().document_revision() > before_revision);
        assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
        let committed_digest = shell.app().canonical_digest();
        let generated_feature_id = match kind {
            GeneralFinishKind::Shell => shell.app().latest_topology_shell_parameters().unwrap().0,
            GeneralFinishKind::Fillet | GeneralFinishKind::Chamfer => {
                shell
                    .app()
                    .latest_topology_edge_finish_parameters()
                    .unwrap()
                    .0
            }
        };
        for _ in 0..300 {
            shell.settle();
            if shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id),
            "{kind:?}: {}",
            shell.app().action_digest()
        );
        shell.key(Key::Z, ctrl());
        assert_eq!(shell.app().canonical_digest(), imported_digest);
        shell.key(Key::Y, ctrl());
        assert_eq!(shell.app().canonical_digest(), committed_digest);
        for _ in 0..300 {
            shell.settle();
            if shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            shell
                .app()
                .exact_current_producer_ids()
                .contains(&generated_feature_id),
            "{kind:?} did not recompute after Redo"
        );
        shell.key(Key::Z, ctrl());
        assert_eq!(shell.app().canonical_digest(), imported_digest);
    }
    assert!(!shell.app_mut().prepare_assistant_general_finish(
        TopologicalPickLocator {
            instance_path: InstancePath::root(imported_occurrence_id),
            producer_feature_id: imported_feature_id,
            kind: TopologicalElementKind::Face,
            ordinal: u32::MAX,
        },
        GeneralFinishKind::Shell,
        1.0,
    ));
    assert_eq!(shell.app().canonical_digest(), imported_digest);
    assert!(
        shell
            .app_mut()
            .select_topological_locator(TopologicalPickLocator {
                instance_path: InstancePath::root(imported_occurrence_id),
                producer_feature_id: imported_feature_id,
                kind: TopologicalElementKind::Face,
                ordinal: 0,
            })
    );
    let before_revision = shell.app().document_revision();
    let before_undo_steps = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();
    shell.app_mut().set_push_pull_distance_input("5");
    assert!(shell.app_mut().start_preview());
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert!(shell.app_mut().confirm_preview());

    assert!(shell.app().document_revision() > before_revision);
    assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
    let committed_digest = shell.app().canonical_digest();
    let snapshot = shell.app().document_snapshot();
    let (offset_feature_id, offset_target, offset_face, offset_distance) = snapshot
        .features()
        .find_map(|feature| match feature.kind() {
            FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => Some((feature.id(), *target, face, distance.millimetres())),
            _ => None,
        })
        .unwrap();
    assert_eq!(offset_target, imported_feature_id);
    assert_eq!(offset_face.producer_feature_id, imported_feature_id);
    assert_eq!(offset_face.producer_element_id, "imported-result/face/0");
    assert_eq!(offset_distance, 5.0);
    let graph = ExactBRepGraph::from_snapshot(&snapshot, imported_definition_id, offset_feature_id)
        .unwrap();
    let source = std::fs::read(&source_path).unwrap();
    let mut tampered_source = source.clone();
    tampered_source[0] ^= 1;
    let mut direct_worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    assert!(
        direct_worker
            .evaluate_exact_brep_graph_with_imported_source(&graph, &tampered_source)
            .is_err()
    );
    direct_worker
        .evaluate_exact_brep_graph_with_imported_source(&graph, &source)
        .unwrap();
    for _ in 0..300 {
        shell.settle();
        if shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    shell.click_menu_command("menu-file", AppCommand::Save);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), committed_digest);
    for _ in 0..300 {
        shell.settle();
        if shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        shell
            .app()
            .exact_current_producer_ids()
            .contains(&offset_feature_id)
    );
}

#[test]
fn general_shell_fillet_and_chamfer_preview_exact_stable_selections_and_persist() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("general-shell-finish.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&path)
        .queue_open(&path);
    let mut shell = Shell::with_dialogs(dialogs);
    install_general_finish_graph_result(&mut shell, FeatureId(2));
    select_general_finish_topology(&mut shell, FeatureId(2), TopologicalElementKind::Face, 3);

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    shell.open_menu("menu-model");
    assert!(shell.offers(AppCommand::Shell));
    shell.click_command(AppCommand::Shell);
    let shell_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(shell_preview.0, FeatureId(2));
    assert_eq!(shell_preview.1.kind, TopologicalElementKind::Face);
    assert_eq!(shell_preview.1.producer_feature_id, FeatureId(2));
    assert_eq!(
        shell_preview.1.producer_element_id,
        "generated-result/face/3"
    );
    assert_eq!(shell_preview.2, GeneralFinishKind::Shell);
    assert_eq!(shell_preview.3, 2.0);
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
    let shell_parameters = shell.app().latest_topology_shell_parameters().unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell_parameters.1, shell_preview.1);
    assert_eq!(shell_parameters.2, 2.5);

    install_general_finish_graph_result(&mut shell, shell_parameters.0);
    let edge_locator = TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: shell_parameters.0,
        kind: TopologicalElementKind::Edge,
        ordinal: 2,
    };
    let assistant_revision = shell.app().document_revision();
    let assistant_digest = shell.app().canonical_digest();
    assert!(shell.app_mut().prepare_assistant_general_finish(
        edge_locator,
        GeneralFinishKind::Fillet,
        1.25,
    ));
    let fillet_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(fillet_preview.0, shell_parameters.0);
    assert_eq!(fillet_preview.1.kind, TopologicalElementKind::Edge);
    assert_eq!(
        fillet_preview.1.producer_element_id,
        "generated-result/edge/2"
    );
    assert_eq!(fillet_preview.2, GeneralFinishKind::Fillet);
    assert_eq!(fillet_preview.3, 1.25);
    assert_eq!(shell.app().document_revision(), assistant_revision);
    assert_eq!(shell.app().canonical_digest(), assistant_digest);
    assert!(shell.app_mut().confirm_assistant_general_finish());

    let fillet_digest = shell.app().canonical_digest();
    let fillet = shell
        .app()
        .latest_topology_edge_finish_parameters()
        .unwrap();
    assert_eq!(shell.app().document_revision(), before_revision + 2);
    assert_eq!(fillet.1, fillet_preview.1);
    assert_eq!(fillet.2, EdgeFinishKind::Fillet);
    assert_eq!(fillet.3, 1.25);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    assert!(
        shell
            .app()
            .latest_topology_edge_finish_parameters()
            .is_none()
    );

    install_general_finish_graph_result(&mut shell, shell_parameters.0);
    select_general_finish_topology(
        &mut shell,
        shell_parameters.0,
        TopologicalElementKind::Edge,
        9,
    );
    shell.click_menu_command("menu-model", AppCommand::Chamfer);
    let chamfer_preview = shell.app().general_finish_preview_parameters().unwrap();
    assert_eq!(
        chamfer_preview.1.producer_element_id,
        "generated-result/edge/9"
    );
    assert_eq!(chamfer_preview.2, GeneralFinishKind::Chamfer);
    shell.type_text("0.75");
    shell.press_key(Key::Enter);
    let chamfer_digest = shell.app().canonical_digest();
    let chamfer = shell
        .app()
        .latest_topology_edge_finish_parameters()
        .unwrap();
    assert_ne!(chamfer_digest, fillet_digest);
    assert_eq!(chamfer.1.producer_element_id, "generated-result/edge/9");
    assert_eq!(chamfer.2, EdgeFinishKind::Chamfer);
    assert_eq!(chamfer.3, 0.75);
    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), shell_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    assert!(path.is_file());
    shell.click_menu_command("menu-file", AppCommand::New);
    assert!(shell.app().latest_topology_shell_parameters().is_none());
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), chamfer_digest);
    assert_eq!(
        shell.app().latest_topology_shell_parameters(),
        Some(shell_parameters)
    );
    assert_eq!(
        shell.app().latest_topology_edge_finish_parameters(),
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
fn ctrl_copy_drag_snaps_source_endpoint_exactly_to_target_endpoint() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    let target_origin = Vec3::new(137.3, -42.7, 0.0);
    assert!(shell.app_mut().copy_selected(target_origin));
    shell.click_menu_command("menu-view", AppCommand::ZoomFit);

    shell.click_command(AppCommand::Move);
    let (source_point, source_anchor, target_point, target_anchor) = [
        Vec3::new(0.0, 0.0, 0.0),
        Vec3::new(100.0, 0.0, 0.0),
        Vec3::new(0.0, 60.0, 0.0),
        Vec3::new(100.0, 60.0, 0.0),
        Vec3::new(0.0, 0.0, 20.0),
        Vec3::new(100.0, 0.0, 20.0),
        Vec3::new(0.0, 60.0, 20.0),
        Vec3::new(100.0, 60.0, 20.0),
    ]
    .into_iter()
    .find_map(|source_point| {
        let source_anchor = shell.app().viewport_position(source_point)?;
        shell.move_pointer(source_anchor);
        if shell.app().hovered_snap_position() != Some(source_point) {
            return None;
        }
        let target_point = target_origin + source_point;
        let target_anchor = shell.app().viewport_position(target_point)?;
        shell.move_pointer(target_anchor);
        (shell.app().hovered_snap_position() == Some(target_point)).then_some((
            source_point,
            source_anchor,
            target_point,
            target_anchor,
        ))
    })
    .expect("one corresponding source/target endpoint pair must be visible");
    shell.move_pointer(source_anchor);
    assert_eq!(shell.app().hovered_snap_position(), Some(source_point));
    shell.move_pointer(target_anchor);
    assert_eq!(shell.app().hovered_snap_position(), Some(target_point));
    shell.move_pointer(source_anchor);

    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    let source_geometry = shell.app().occurrence_box_geometry(1).unwrap();
    let target_geometry = shell.app().occurrence_box_geometry(2).unwrap();
    shell.drag_with(source_anchor, target_anchor, ctrl());

    let snapped_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 3);
    assert_eq!(shell.app().definition_count(), 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1),
        Some(source_geometry)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some(target_geometry)
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(3),
        Some(target_geometry)
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), snapped_digest);
    assert_eq!(
        shell.app().occurrence_box_geometry(3),
        Some(target_geometry)
    );
}

#[test]
fn exact_align_previews_then_commits_one_transform_batch_with_undo_redo() {
    let mut shell = Shell::new();
    shell.click_at(shell.viewport_rect().center());
    assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 30.0)));
    shell.settle();
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();

    for axis in [Axis::X, Axis::Y, Axis::Z] {
        for mode in [AlignMode::Minimum, AlignMode::Center, AlignMode::Maximum] {
            assert!(shell.app_mut().preview_align_occurrences(
                OccurrenceId(2),
                OccurrenceId(1),
                axis,
                mode,
            ));
            assert_eq!(shell.app().document_revision(), before_revision);
            assert_eq!(shell.app().canonical_digest(), before_digest);
        }
    }

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
        Some((Vec3::new(0.0, 25.0, 30.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let aligned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(2),
        Some((Vec3::new(0.0, 25.0, 30.0), Vec3::new(100.0, 60.0, 20.0)))
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
            .preview_linear_pattern(OccurrenceId(1), Axis::X, 200.0, 5,)
    );
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(2)),
        Some((Vec3::new(200.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert_eq!(
        shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(5)),
        Some((Vec3::new(800.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );

    assert!(shell.app_mut().confirm_occurrence_operation_preview());
    let patterned_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().active_box_count(), 5);
    assert_eq!(shell.app().definition_count(), 1);
    for occurrence_id in 1..=5 {
        assert_eq!(
            shell
                .app()
                .occurrence_definition_id(OccurrenceId(occurrence_id)),
            shell.app().occurrence_definition_id(OccurrenceId(1)),
        );
    }
    assert_eq!(
        shell.app().occurrence_box_geometry(5),
        Some((Vec3::new(800.0, 0.0, 0.0), Vec3::new(100.0, 60.0, 20.0)))
    );
    assert!(shell.app_mut().undo());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 1);
    assert!(shell.app_mut().redo());
    assert_eq!(shell.app().canonical_digest(), patterned_digest);
    assert_eq!(shell.app().active_box_count(), 5);
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
fn a_viewport_drag_in_rotate_turns_the_occurrence_and_the_arrow_keys_pick_the_axis() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box");
    // The default box is 100 x 60, so a quarter turn about the blue axis has to
    // swap those two extents. Reading the extents proves the body really turned
    // instead of merely reporting that it did.
    assert_eq!((size.x, size.y), (100.0, 60.0));
    let centre = Vec3::new(
        origin.x + size.x * 0.5,
        origin.y + size.y * 0.5,
        origin.z + size.z,
    );

    shell.click_command(AppCommand::Rotate);
    let before_revision = shell.app().document_revision();
    let before_digest = shell.app().canonical_digest();
    // Grab an arm on the top face and swing it a quarter turn.
    let from = shell.app().project_to_screen(
        Vec3::new(centre.x + size.x * 0.25, centre.y, centre.z),
        rect,
    );
    let to = shell.app().project_to_screen(
        Vec3::new(centre.x, centre.y + size.x * 0.25, centre.z),
        rect,
    );
    shell.drag(from, to);

    assert_eq!(
        shell.app().document_revision(),
        before_revision + 1,
        "one completed Rotate drag must produce exactly one canonical revision: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Rotate must not create geometry"
    );
    let turned = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (turned.x, turned.y),
        (60.0, 100.0),
        "a quarter turn about the blue axis must swap the width and the depth"
    );

    shell.key(Key::Z, ctrl());
    assert_eq!(
        shell.app().canonical_digest(),
        before_digest,
        "one Rotate drag must be exactly one undo step"
    );

    // The arrow keys pin the protractor to a coloured axis, the way SketchUp
    // does, and pressing the same arrow again releases the pin.
    shell.click_command(AppCommand::Rotate);
    shell.press_key(Key::ArrowRight);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a typed angle must commit exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    let about_x = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        rounded_extents(about_x),
        (100.0, 20.0, 60.0),
        "a locked red axis must turn the depth into the height"
    );

    // Pressing the same arrow again releases the pin, so the protractor falls
    // back to blue Z and a quarter turn swaps the width and the depth instead.
    shell.press_key(Key::ArrowRight);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("90");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    let released = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        rounded_extents(released),
        (20.0, 100.0, 60.0),
        "pressing the locked arrow again must release the lock back to blue Z"
    );
}

/// SketchUp treats a value typed straight after a rotation as a correction of
/// that rotation, not as a second one. Choosing 45 and then thinking better of
/// it and typing 40 has to leave the body at 40, not at 85.
#[test]
fn a_typed_angle_corrects_the_last_rotate_instead_of_stacking_onto_it() {
    let mut shell = Shell::new();
    shell.click_at(shell.top_face_centre(1));
    let size = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box")
        .1;
    assert_eq!((size.x, size.y), (100.0, 60.0));
    // The axis-aligned extents of a box turned about the blue axis, which is
    // what reading the geometry back can prove.
    let turned_extents = |degrees: f64| {
        let (sin, cos) = degrees.to_radians().sin_cos();
        let round = |value: f64| (value * 1_000.0).round() / 1_000.0;
        (
            round(size.x.mul_add(cos.abs(), size.y * sin.abs())),
            round(size.x.mul_add(sin.abs(), size.y * cos.abs())),
        )
    };
    let base_digest = shell.app().canonical_digest();

    shell.click_command(AppCommand::Rotate);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("45");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    let first = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (
            (first.x * 1_000.0).round() / 1_000.0,
            (first.y * 1_000.0).round() / 1_000.0
        ),
        turned_extents(45.0)
    );

    // Thinking better of it: 40 replaces the 45, so the extents are those of a
    // single 40° turn and the history still holds exactly one step.
    shell.type_text("40");
    shell.press_key(Key::Enter);
    let corrected = shell.app().occurrence_box_geometry(1).unwrap().1;
    assert_eq!(
        (
            (corrected.x * 1_000.0).round() / 1_000.0,
            (corrected.y * 1_000.0).round() / 1_000.0
        ),
        turned_extents(40.0),
        "a typed angle must replace the last rotation, not add to it: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "corrections must stay one undo step"
    );

    // Zero is a legitimate correction back to where the body started.
    shell.type_text("0");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().canonical_digest(),
        base_digest,
        "typing zero must undo the turn rather than refuse the value"
    );
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), base_digest);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps,
        "the whole correction chain must undo in a single step"
    );
}

#[test]
fn a_pinned_move_travels_along_the_blue_axis_and_the_arrow_releases_the_pin() {
    let mut shell = Shell::new();
    let rect = shell.viewport_rect();
    shell.click_at(shell.top_face_centre(1));
    let (origin, size) = shell
        .app()
        .occurrence_box_geometry(1)
        .expect("the default scene has one box");
    assert_eq!(origin.z, 0.0);
    let top_centre = Vec3::new(
        origin.x + size.x * 0.5,
        origin.y + size.y * 0.5,
        origin.z + size.z,
    );

    // Without a pin the pointer stays on the plane it grabbed, which is exactly
    // why a part could never be set down on top of another one.
    shell.click_command(AppCommand::Move);
    let before_digest = shell.app().canonical_digest();
    let from = shell.app().project_to_screen(top_centre, rect);
    let lifted = Vec3::new(top_centre.x, top_centre.y, top_centre.z + 40.0);
    shell.drag(from, shell.app().project_to_screen(lifted, rect));
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0.z,
        0.0,
        "an unpinned Move must keep its own plane: {:?}",
        shell.app().action_digest()
    );
    if shell.app().canonical_digest() != before_digest {
        shell.key(Key::Z, ctrl());
    }
    assert_eq!(shell.app().canonical_digest(), before_digest);

    // The up arrow pins the travel to blue Z, and then the very same drag lifts
    // the body by the full distance it was dragged.
    shell.press_key(Key::ArrowUp);
    let before_steps = shell.app().undo_step_count();
    let from = shell.app().project_to_screen(top_centre, rect);
    shell.drag(from, shell.app().project_to_screen(lifted, rect));
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "one pinned drag must be exactly one undo step: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().active_box_count(),
        1,
        "Move must not create geometry"
    );
    let raised = shell.app().occurrence_box_geometry(1).unwrap().0;
    assert_eq!(
        (raised.x, raised.y, raised.z),
        (origin.x, origin.y, 40.0),
        "a Move pinned to blue Z must travel only along it"
    );

    // The pin outlives the gesture, so a typed distance is read along the same
    // axis instead of needing the whole vector spelled out.
    shell.click_command(AppCommand::Move);
    let anchor = Vec3::new(top_centre.x, top_centre.y, raised.z + size.z);
    shell.click_at(shell.app().project_to_screen(anchor, rect));
    let before_steps = shell.app().undo_step_count();
    shell.type_text("25");
    shell.press_key(Key::Enter);
    assert_eq!(
        shell.app().undo_step_count(),
        before_steps + 1,
        "a typed distance along the pinned axis must commit: {:?}",
        shell.app().action_digest()
    );
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0.z,
        65.0,
        "a typed distance must be read along the pinned axis"
    );

    // Pressing the same arrow again releases the pin, and an exact vector can
    // still reach the blue axis on its own.
    shell.press_key(Key::ArrowUp);
    let before_steps = shell.app().undo_step_count();
    shell.type_text("0,0,-15");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().undo_step_count(), before_steps + 1);
    assert_eq!(
        shell.app().occurrence_box_geometry(1).unwrap().0.z,
        50.0,
        "an exact x,y,z vector must move in Z with no pin at all"
    );
}

/// Extents rounded to a micrometre, so a quarter turn can be compared exactly
/// without asserting on the last bit of a sine.
fn rounded_extents(size: Vec3) -> (f64, f64, f64) {
    let round = |value: f64| (value * 1_000.0).round() / 1_000.0;
    (round(size.x), round(size.y), round(size.z))
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
fn d_profile_solid_intersect_preserves_review_lifecycle_exact_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-intersect.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(120.0, 20.0, 0.0);
    let end = Vec3::new(140.0, 20.0, 0.0);
    let bulge = Vec3::new(130.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    shell.type_text("20");
    shell.press_key(Key::Enter);
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.settle();
    assert_eq!(shell.app().active_box_count(), 2);

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();

    let preview_intersection = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidIntersect);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 10.0, 20.0))
            .unwrap();
        let tool = shell
            .app()
            .viewport_position(Vec3::new(30.0, 25.0, 20.0))
            .unwrap();
        shell.click_at(target);
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
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
            "D-profile Intersect preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.x > 0.0
                && origin.y > 0.0
                && origin.z.abs() < 2.0e-5
                && size.x > 19.9
                && size.y > 9.9
                && (size.z - 20.0).abs() < 2.0e-5
                && origin.x + size.x < 100.0
                && origin.y + size.y < 60.0,
            "D-profile preview must remain strictly contained: origin={origin:?}, size={size:?}"
        );
    };

    preview_intersection(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);

    preview_intersection(&mut shell);
    shell.press_key(Key::Enter);
    let intersect_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    let tool_profile = request.boolean.as_ref().unwrap().profile_feature_id;
    assert_ne!(tool_profile, request.profile_feature_id);
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_rectangle(&request).unwrap();
    assert!(direct_package.vertices.len() > 8);
    assert_eq!(
        direct_package
            .reference(ExactFaceRole::ArcSide)
            .unwrap()
            .profile_feature_id,
        tool_profile
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let wait_for_intersect = |shell: &mut Shell| {
        for _ in 0..100 {
            shell.settle();
            if shell.app().exact_render_body_count() == 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            shell.app().exact_render_body_count(),
            1,
            "exact Intersect result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_intersect(&mut shell);
    assert!(
        shell.app().exact_render_triangle_count() > 12,
        "the accepted Intersect mesh must preserve the curved arc side"
    );
    let arc_side = shell
        .app()
        .exact_reference_for_occurrence(
            &InstancePath::root(OccurrenceId(1)),
            ExactFaceRole::ArcSide,
        )
        .unwrap();
    assert_eq!(arc_side.body.profile_feature_id, tool_profile);
    assert_eq!(arc_side.body.role(), Some(ExactFaceRole::ArcSide));

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    assert_eq!(shell.app().active_box_count(), 1);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), intersect_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_BOOLEAN_INTERSECT_EVALUATOR_V1);
    assert!(
        reopened
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_intersect(&mut shell);
    let reopened_arc_side = shell
        .app()
        .exact_reference_for_occurrence(
            &InstancePath::root(OccurrenceId(1)),
            ExactFaceRole::ArcSide,
        )
        .unwrap();
    assert_eq!(reopened_arc_side.body.profile_feature_id, tool_profile);
    assert_eq!(reopened_arc_side.body.role(), Some(ExactFaceRole::ArcSide));
}

#[test]
fn d_profile_solid_union_preserves_review_lifecycle_exact_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-union.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(-10.0, -20.0, 0.0);
    let end = Vec3::new(-10.0, 80.0, 0.0);
    let bulge = Vec3::new(120.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    shell.type_text("20");
    shell.press_key(Key::Enter);
    assert_eq!(shell.app().active_box_count(), 2);

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();

    let preview_union = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidUnion);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 30.0, 20.0))
            .unwrap();
        shell.move_pointer(target);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(1))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        assert_eq!(
            shell
                .app()
                .hovered_selection()
                .map(|selection| selection.instance_path.clone()),
            Some(InstancePath::root(OccurrenceId(1)))
        );
        shell.click_at(target);

        let tool = shell
            .app()
            .viewport_position(Vec3::new(115.0, 30.0, 20.0))
            .unwrap();
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
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
            "D-profile Union preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BOOLEAN_UNION_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.x < -9.9
                && origin.y < -40.0
                && origin.z.abs() < 2.0e-5
                && size.x > 129.0
                && size.y > 145.0
                && (size.z - 20.0).abs() < 2.0e-5,
            "D-profile preview must strictly contain the host: origin={origin:?}, size={size:?}"
        );
    };

    preview_union(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);

    preview_union(&mut shell);
    shell.press_key(Key::Enter);
    let union_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 1);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    let tool_profile = request.boolean.as_ref().unwrap().profile_feature_id;
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_rectangle(&request).unwrap();
    assert!(direct_package.vertices.len() > 8);
    assert_eq!(
        direct_package
            .reference(ExactFaceRole::ArcSide)
            .unwrap()
            .profile_feature_id,
        tool_profile
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();

    let wait_for_union = |shell: &mut Shell| {
        for _ in 0..100 {
            shell.settle();
            if shell.app().exact_render_body_count() == 1 {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(
            shell.app().exact_render_body_count(),
            1,
            "exact Union result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_union(&mut shell);
    assert!(shell.app().exact_render_triangle_count() > 12);
    let arc_side = shell
        .app()
        .exact_reference_for_occurrence(
            &InstancePath::root(OccurrenceId(1)),
            ExactFaceRole::ArcSide,
        )
        .unwrap();
    assert_eq!(arc_side.body.profile_feature_id, tool_profile);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().active_box_count(), 2);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), union_digest);
    assert_eq!(shell.app().active_box_count(), 1);

    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), union_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_BOOLEAN_UNION_EVALUATOR_V1);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_union(&mut shell);
    let reopened_arc_side = shell
        .app()
        .exact_reference_for_occurrence(
            &InstancePath::root(OccurrenceId(1)),
            ExactFaceRole::ArcSide,
        )
        .unwrap();
    assert_eq!(reopened_arc_side.body.profile_feature_id, tool_profile);
}

#[test]
fn d_profile_solid_split_preserves_review_lifecycle_partition_mesh_and_arc_lineage() {
    let directory = tempfile::tempdir().unwrap();
    let saved = directory.path().join("d-profile-split.ketchup");
    let dialogs = ScriptedFileDialogs::new()
        .queue_save(&saved)
        .queue_open(&saved)
        .always_discard();
    let mut shell = Shell::with_dialogs(dialogs);

    let start = Vec3::new(120.0, 20.0, 0.0);
    let end = Vec3::new(140.0, 20.0, 0.0);
    let bulge = Vec3::new(130.0, 30.0, 0.0);
    shell.click_command(AppCommand::Arc);
    shell.click_at(shell.app().viewport_position(start).unwrap());
    shell.click_at(shell.app().viewport_position(end).unwrap());
    shell.click_at(shell.app().viewport_position(bulge).unwrap());
    shell.click_command(AppCommand::PushPull);
    shell.type_text("20");
    shell.press_key(Key::Enter);
    assert!(shell.app_mut().move_selected(Vec3::new(-100.0, 0.0, 0.0)));
    shell.settle();

    let before_revision = shell.app().document_revision();
    let before_undo = shell.app().undo_step_count();
    let before_digest = shell.app().canonical_digest();
    let preview_split = |shell: &mut Shell| {
        shell.click_menu_command("menu-model", AppCommand::SolidSplit);
        let target = shell
            .app()
            .viewport_position(Vec3::new(10.0, 10.0, 20.0))
            .unwrap();
        let tool = shell
            .app()
            .viewport_position(Vec3::new(30.0, 25.0, 20.0))
            .unwrap();
        shell.click_at(target);
        shell.move_pointer(tool);
        for _ in 0..4 {
            if shell.app().hovered_selection().is_some_and(|selection| {
                selection.instance_path == InstancePath::root(OccurrenceId(2))
            }) {
                break;
            }
            shell.press_key(Key::Tab);
        }
        shell.click_at(tool);
        assert!(
            shell.app().has_occurrence_operation_preview(),
            "D-profile Split preview failed: {}",
            shell.app().action_digest()
        );
        assert_eq!(
            shell.app().push_pull_preview_exact_evaluator(),
            Some(EXACT_BOOLEAN_SPLIT_EVALUATOR_V1)
        );
        let (origin, size) = shell
            .app()
            .occurrence_operation_preview_geometry(OccurrenceId(1))
            .unwrap();
        assert!(
            origin.length() < 2.0e-5
                && (size.x - 100.0).abs() < 2.0e-5
                && (size.y - 60.0).abs() < 2.0e-5
                && (size.z - 20.0).abs() < 2.0e-5
        );
    };

    preview_split(&mut shell);
    assert_eq!(shell.app().document_revision(), before_revision);
    assert_eq!(shell.app().canonical_digest(), before_digest);
    assert_eq!(shell.app().undo_step_count(), before_undo);
    shell.press_key(Key::Escape);
    assert_eq!(shell.app().canonical_digest(), before_digest);

    preview_split(&mut shell);
    shell.press_key(Key::Enter);
    let split_digest = shell.app().canonical_digest();
    assert_eq!(shell.app().document_revision(), before_revision + 1);
    assert_eq!(shell.app().undo_step_count(), before_undo + 1);
    assert_eq!(shell.app().active_box_count(), 2);
    let result_definition = shell.app().selected_reference().unwrap().definition_id;
    let request = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(request.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    let tool_profile = request.boolean.as_ref().unwrap().profile_feature_id;
    assert!(
        request
            .boolean
            .as_ref()
            .and_then(|boolean| boolean.profile.as_ref())
            .is_some_and(|profile| profile.is_line_arc_d_profile())
    );
    let mut worker = ExactWorkerSupervisor::spawn(exact_worker_path()).unwrap();
    let direct_package = worker.evaluate_rectangle(&request).unwrap();
    assert!(direct_package.vertices.len() > 16);
    assert_eq!(
        direct_package
            .reference(ExactFaceRole::ArcSide)
            .unwrap()
            .profile_feature_id,
        tool_profile
    );
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    let wait_for_arc_side = |shell: &mut Shell| {
        for _ in 0..100 {
            shell.settle();
            if shell
                .app()
                .exact_reference_for_occurrence(
                    &InstancePath::root(OccurrenceId(1)),
                    ExactFaceRole::ArcSide,
                )
                .is_some()
            {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "exact D-profile Split result failed: {}",
            shell.app().action_digest()
        );
    };
    wait_for_arc_side(&mut shell);
    assert!(shell.app().exact_render_triangle_count() > 24);

    shell.key(Key::Z, ctrl());
    assert_eq!(shell.app().canonical_digest(), before_digest);
    shell.key(Key::Y, ctrl());
    assert_eq!(shell.app().canonical_digest(), split_digest);
    shell.click_menu_command("menu-file", AppCommand::SaveAs);
    shell.click_menu_command("menu-file", AppCommand::New);
    shell.click_menu_command("menu-file", AppCommand::Open);
    assert_eq!(shell.app().canonical_digest(), split_digest);
    let reopened = ExactFeatureChainRequest::from_snapshot(
        &shell.app().document_snapshot(),
        result_definition,
    )
    .unwrap();
    assert_eq!(reopened.evaluator(), EXACT_BOOLEAN_SPLIT_EVALUATOR_V1);
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    wait_for_arc_side(&mut shell);
    let reopened_arc_side = shell
        .app()
        .exact_reference_for_occurrence(
            &InstancePath::root(OccurrenceId(1)),
            ExactFaceRole::ArcSide,
        )
        .unwrap();
    assert_eq!(reopened_arc_side.body.profile_feature_id, tool_profile);
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
fn make_unique_is_localized_exact_selection_bound_and_one_undo_step() {
    for (locale_index, catalog) in [
        LocaleCatalog::english(),
        LocaleCatalog::slovak(),
        LocaleCatalog::pseudo(),
    ]
    .into_iter()
    .enumerate()
    {
        let mut shell = Shell::with_catalog(catalog.clone());
        assert_eq!(
            shell.app().command_label(AppCommand::MakeUnique),
            shell.catalog().text("model-make-unique")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_at(shell.top_face_centre(1));
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));
        let single_revision = shell.app().document_revision();
        let single_digest = shell.app().canonical_digest();
        let single_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(shell.app().document_revision(), single_revision);
        assert_eq!(shell.app().canonical_digest(), single_digest);
        assert_eq!(shell.app().undo_step_count(), single_undo_steps);
        shell.press_key(Key::Escape);
        shell.click_at(shell.top_face_centre(1));

        shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert!(shell.app_mut().copy_selected(Vec3::new(150.0, 25.0, 0.0)));
        shell.settle();
        shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        assert_eq!(shell.app().definition_count(), 1);
        assert!(shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_at_with(shell.top_face_centre(1), shift());
        assert_eq!(shell.app().selected_occurrence_count(), 2);
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));
        let multiple_revision = shell.app().document_revision();
        let multiple_digest = shell.app().canonical_digest();
        let multiple_undo_steps = shell.app().undo_step_count();
        shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(shell.app().document_revision(), multiple_revision);
        assert_eq!(shell.app().canonical_digest(), multiple_digest);
        assert_eq!(shell.app().undo_step_count(), multiple_undo_steps);

        shell.click_at(shell.top_face_centre(2));
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::MakeUnique));
        let source_definition_id = shell
            .app()
            .occurrence_definition_id(OccurrenceId(1))
            .expect("the source occurrence exists");
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(source_definition_id)
        );
        let before_revision = shell.app().document_revision();
        let before_digest = shell.app().canonical_digest();
        let before_undo_steps = shell.app().undo_step_count();
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        if locale_index == 0 {
            shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        } else {
            shell
                .app_mut()
                .set_assistant_workspace_mode(AssistantWorkspaceMode::Tab);
            shell.settle();
            let make_unique = shell.catalog().text("model-make-unique");
            shell.click_role_and_label(Role::Button, &make_unique);
        }

        let unique_digest = shell.app().canonical_digest();
        let unique_definition_id = shell
            .app()
            .occurrence_definition_id(OccurrenceId(2))
            .expect("the unique occurrence exists");
        assert_ne!(unique_definition_id, source_definition_id);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(1)),
            Some(source_definition_id)
        );
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(shell.app().active_box_count(), 2);
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));
        assert_eq!(shell.app().document_revision(), before_revision + 1);
        assert_eq!(shell.app().undo_step_count(), before_undo_steps + 1);
        assert_eq!(
            shell.app().action_digest(),
            shell.catalog().text("digest-made-unique")
        );
        assert!(!shell.app().command_is_enabled(AppCommand::MakeUnique));

        shell.click_menu_command("menu-edit", AppCommand::Undo);
        assert_eq!(shell.app().canonical_digest(), before_digest);
        assert_eq!(shell.app().definition_count(), 1);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(source_definition_id)
        );
        assert_eq!(shell.app().undo_step_count(), before_undo_steps);
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        shell.click_menu_command("menu-edit", AppCommand::Redo);
        assert_eq!(shell.app().canonical_digest(), unique_digest);
        assert_eq!(shell.app().definition_count(), 2);
        assert_eq!(
            shell.app().occurrence_definition_id(OccurrenceId(2)),
            Some(unique_definition_id)
        );
        assert_eq!(shell.app().selected_occurrence_count(), 1);
        assert!(shell.app().occurrence_is_selected(OccurrenceId(2)));
        assert!(shell.app().command_is_enabled(AppCommand::Paste));

        let mut context_shell = Shell::with_catalog(catalog);
        context_shell.click_at(context_shell.viewport_rect().center());
        context_shell.click_menu_command("menu-edit", AppCommand::Copy);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
        assert!(
            context_shell
                .app_mut()
                .copy_selected(Vec3::new(150.0, 25.0, 0.0))
        );
        context_shell.settle();
        context_shell.click_menu_command("menu-view", AppCommand::ZoomFit);
        let selected = context_shell.top_face_centre(2);
        context_shell.double_click_at(selected);
        assert_eq!(context_shell.app().edit_context_depth(), 1);
        assert!(
            !context_shell
                .app()
                .command_is_enabled(AppCommand::MakeUnique)
        );
        let context_revision = context_shell.app().document_revision();
        let context_digest = context_shell.app().canonical_digest();
        let context_undo_steps = context_shell.app().undo_step_count();
        let context_action_digest = context_shell.app().action_digest().to_owned();
        context_shell.click_menu_command("menu-model", AppCommand::MakeUnique);
        assert_eq!(context_shell.app().document_revision(), context_revision);
        assert_eq!(context_shell.app().canonical_digest(), context_digest);
        assert_eq!(context_shell.app().undo_step_count(), context_undo_steps);
        assert_eq!(context_shell.app().action_digest(), context_action_digest);
        context_shell.press_key(Key::Escape);
        assert_eq!(context_shell.app().edit_context_depth(), 0);
        assert!(context_shell.app().command_is_enabled(AppCommand::Paste));
    }
}
