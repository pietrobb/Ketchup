//! Acceptance workflows replayed offscreen, without touching the real pointer.
//!
//! Every assertion reads document state — revision, canonical digest,
//! occurrence and definition counts — because that is the thing the workflow is
//! supposed to change. Painted text is deliberately never asserted on.

use crate::harness;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

use eframe::egui::{Key, Modifiers, Pos2, Rect, Vec2, accesskit::Role};
use harness::{Shell, ctrl, shift};
use ketchup_app::dialogs::ScriptedFileDialogs;
use ketchup_app::{
    AlignMode, AppCommand, DistributionMode, GeneralFinishKind, KetchupApp, RectangularPatternSpec,
    ViewFlag,
};
use ketchup_assistant::sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    AssistantStructureChange,
};
use ketchup_interaction::{
    Axis, ElementId, LocaleCatalog, Side, SnapKind, Vec3, exact_projection::TopologicalPickLocator,
    mesh_projection::MeshInteractionProjection,
};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, DerivedIdentity, Dimension, DocumentStore,
    EdgeFinishKind, EvaluationIdentity, FaceRef, FeatureId, FeatureKind, FeatureParameterBinding,
    FeatureParameterRecomputeScope, FeatureParameterTarget, InstancePath, NodeId, OccurrenceId,
    ParameterValueType, PortSpec, RuleOutput, SlotPath, SlotSegment, TagId, Transform,
};
use ketchup_model::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepGraph, ExactBRepOperation, ExactBRepPlanarGeometry,
    ExactBRepPlanarSegment,
};
use ketchup_model::exact_product::{
    EXACT_BREP_GRAPH_EVALUATOR_V1, ExactBRepGraphPackage, ExactBRepGraphWorkerEvidence,
    ExactBodyPackage,
};
use ketchup_model::graph::{EvaluationStatus, EvaluatorNodeKind};
use ketchup_model::import::{ImportFormat, StepImportMesh, StepMeshTriangle};
use ketchup_model::persistence;
use ketchup_model::topology::TopologicalElementKind;
use ketchup_scheduler::ExactWorkerSupervisor;

const PARAMETRIC_PROFILE: FeatureId = FeatureId(10);
const PARAMETRIC_RULE: NodeId = NodeId(302);
const PARAMETRIC_DEPENDENT: NodeId = NodeId(303);
const PARAMETRIC_UNRELATED_SOURCE: NodeId = NodeId(305);
const PARAMETRIC_UNRELATED: NodeId = NodeId(306);

/// Commits one typed CAD operation the way MCP `edit propose` + `commit` does.
fn apply_typed(app: &mut KetchupApp, operation: AssistantCadEditOperation) -> bool {
    app.headless_apply_cad_program(&AssistantCadEditProgram {
        operations: vec![operation],
    })
    .is_ok()
}

fn create_tag(app: &mut KetchupApp, id: TagId, name: String, visible: bool) -> bool {
    apply_typed(
        app,
        AssistantCadEditOperation::CreateTag {
            tag_id: id.0,
            name,
            visible,
        },
    )
}

fn set_occurrence_tag(app: &mut KetchupApp, occurrence: OccurrenceId, tag: Option<TagId>) -> bool {
    apply_typed(
        app,
        AssistantCadEditOperation::SetOccurrenceTag {
            selector: AssistantCadEntitySelector::Occurrences {
                occurrence_ids: vec![occurrence.0],
            },
            tag_id: tag.map(|tag| tag.0),
        },
    )
}
fn boolean_tool_profile(
    graph: &ExactBRepGraph,
    operation: ExactBRepBooleanOperation,
) -> Option<(FeatureId, &ExactBRepPlanarGeometry)> {
    let mut tool = graph.nodes.iter().find_map(|node| match &node.operation {
        ExactBRepOperation::Boolean {
            operation: candidate,
            tool,
            ..
        } if *candidate == operation => Some(*tool),
        _ => None,
    })?;
    loop {
        match &graph.nodes.get(tool.0 as usize)?.operation {
            ExactBRepOperation::RigidTransform { target, .. } => tool = *target,
            ExactBRepOperation::Extrude { profile, .. } => {
                let profile = graph.profiles.get(profile.0 as usize)?;
                return Some((FeatureId(profile.source_feature_id), &profile.geometry));
            }
            _ => return None,
        }
    }
}

fn is_line_arc_profile(geometry: &ExactBRepPlanarGeometry) -> bool {
    matches!(
        geometry,
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments,
        } if matches!(
            segments.as_slice(),
            [
                ExactBRepPlanarSegment::Line { .. },
                ExactBRepPlanarSegment::CircularArc { .. }
            ] | [
                ExactBRepPlanarSegment::CircularArc { .. },
                ExactBRepPlanarSegment::Line { .. }
            ]
        )
    )
}

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
                kind: FeatureKind::polygon(&[[0.0, 0.0], [20.0, 0.0], [20.0, 30.0], [0.0, 30.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(1),
                name: "Extrusion".to_owned(),
                kind: FeatureKind::extrusion(PARAMETRIC_PROFILE, dimension("10")),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(20),
                definition_id: DefinitionId(1),
                name: "Parametric box #1".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
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
                scope: FeatureParameterRecomputeScope::All,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    persistence::save_atomic(path, &document.current()).unwrap();
}

fn exact_worker_path() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_ketchup-performance-exact-worker"))
}

fn wait_for_exact_bodies(shell: &mut Shell, expected: usize) {
    shell.wait_until(|shell| {
        shell.settle();
        shell.app().exact_render_body_count() == expected
    });
    assert_eq!(shell.app().exact_render_body_count(), expected);
}

fn wait_for_one_exact_body(shell: &mut Shell) {
    wait_for_exact_bodies(shell, 1);
}

/// Types a Push/Pull depth for the selected drawn shape and waits until it is
/// applied. While a part's exact solid is still on its way the shape may be
/// drawn on it, so Enter waits ("Computing exact result…") and the frame the
/// solid arrives in applies the value; a next action taken before that drops
/// the waiting value, as it does for the user.
fn push_pull_typed_depth(shell: &mut Shell, depth: &str) {
    let revision = shell.app().document_revision();
    shell.type_text(depth);
    shell.press_key(Key::Enter);
    let applied = shell.wait_until(|shell| {
        shell.settle();
        shell.app().document_revision() > revision
    });
    assert!(
        applied,
        "Push/Pull {depth} was not applied: {}",
        shell.app().action_digest()
    );
}

/// Points the window at a file that is no exact worker, so no exact body is
/// published until the test connects the real worker: the test decides when
/// the exact result arrives instead of the speed of the machine.
fn hold_exact_bodies_back(shell: &mut Shell) {
    shell
        .app_mut()
        .connect_exact_worker(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
        .unwrap();
}

fn replace_parameter_expression(shell: &mut Shell, expression: &str) {
    let input = shell.catalog().text("parameters-expression");
    let apply = shell.catalog().text("parameters-apply");
    shell.focus_text_input(&input);
    shell.key(Key::A, ctrl());
    shell.type_text(expression);
    shell.click_row(&apply);
}

/// Types `value` into the dialog text field named by `key`.
fn set_dialog_field(shell: &mut Shell, key: &str, value: &str) {
    let label = shell.catalog().text(key);
    shell.focus_text_input(&label);
    shell.key(Key::A, ctrl());
    shell.type_text(value);
}

/// The dialog's refusal of `copies` new copies, naming the one copy limit.
fn copy_limit_exceeded(shell: &Shell, copies: usize) -> String {
    shell.catalog().format(
        "dialog-pattern-copy-limit-exceeded",
        &BTreeMap::from([("copies", copies.to_string()), ("limit", "512".to_owned())]),
    )
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
            mass: None,
            exact_input_digest: format!("headless-finish-input-{}", producer_feature_id.0),
            result_fingerprint: format!("headless-finish-result-{}", producer_feature_id.0),
            volume_mm3: 120_000.0,
            area_mm2: 0.0,
            topology_counts: [8, 12, 6, 1, 1],
            wire_count: None,
            bounds_mm: [[0.0, 0.0, 0.0], [100.0, 60.0, 20.0]],
            backend: "headless-finish-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
            faces: Vec::new(),
            edges: Vec::new(),
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

fn count_rotation_rings(shape: &eframe::egui::Shape) -> usize {
    match shape {
        eframe::egui::Shape::Path(path)
            if path.points.len() == 73
                && path.stroke.width == 1.6
                && matches!(
                    &path.stroke.color,
                    eframe::egui::epaint::ColorMode::Solid(colour) if colour.a() == 130
                )
                && path
                    .points
                    .first()
                    .zip(path.points.last())
                    .is_some_and(|(first, last)| first.distance(*last) < 0.01) =>
        {
            1
        }
        eframe::egui::Shape::Vec(shapes) => shapes.iter().map(count_rotation_rings).sum(),
        _ => 0,
    }
}

fn count_selection_outline_segments(shape: &eframe::egui::Shape) -> usize {
    match shape {
        eframe::egui::Shape::LineSegment { stroke, .. }
            if stroke.color == eframe::egui::Color32::from_rgb(240, 78, 35) =>
        {
            1
        }
        eframe::egui::Shape::Vec(shapes) => {
            shapes.iter().map(count_selection_outline_segments).sum()
        }
        _ => 0,
    }
}

/// Extents rounded to a micrometre, so a quarter turn can be compared exactly
/// without asserting on the last bit of a sine.
fn rounded_extents(size: Vec3) -> (f64, f64, f64) {
    let round = |value: f64| (value * 1_000.0).round() / 1_000.0;
    (round(size.x), round(size.y), round(size.z))
}

/// Revision, digest and both Undo stacks: what a refused Scale leaves alone.
fn scale_untouched_state(shell: &Shell) -> (u64, String, usize, usize) {
    (
        shell.app().document_revision(),
        shell.app().canonical_digest(),
        shell.app().undo_step_count(),
        shell.app().redo_step_count(),
    )
}

/// Scale refused the part naming `key`, showed no preview and changed
/// neither the document nor its Undo history.
fn assert_scale_refused(shell: &Shell, key: &str, before: &(u64, String, usize, usize)) {
    assert_eq!(shell.app().action_digest(), shell.catalog().text(key));
    assert_eq!(shell.app().scale_preview_factor(), None);
    assert_eq!(&scale_untouched_state(shell), before);
}

/// The box `origin` + `size` stretched by `factors` about `centre`.
fn stretched_about(origin: Vec3, size: Vec3, centre: Vec3, factors: [f64; 3]) -> [[f64; 3]; 2] {
    let corner = |point: Vec3| -> [f64; 3] {
        let (point, centre) = ([point.x, point.y, point.z], [centre.x, centre.y, centre.z]);
        std::array::from_fn(|axis| centre[axis] + (point[axis] - centre[axis]) * factors[axis])
    };
    [corner(origin), corner(origin + size)]
}

/// The box `origin` + `size` stretched by `factors` about its own centre.
fn stretched(origin: Vec3, size: Vec3, factors: [f64; 3]) -> [[f64; 3]; 2] {
    stretched_about(origin, size, origin + size * 0.5, factors)
}

/// Waits until the exact bodies on screen have exactly the `expected`
/// bounds, in any order.
fn assert_exact_bounds(shell: &mut Shell, expected: &[[[f64; 3]; 2]]) {
    let matches = |actual: &[[[f64; 3]; 2]]| {
        actual.len() == expected.len()
            && expected.iter().all(|bounds| {
                actual.iter().any(|candidate| {
                    (0..2).all(|corner| {
                        (0..3).all(|axis| {
                            (candidate[corner][axis] - bounds[corner][axis]).abs() < 1.0e-3
                        })
                    })
                })
            })
    };
    let arrived = shell.wait_until(|shell| {
        shell.step();
        matches(&shell.app().exact_render_bounds())
    });
    assert!(
        arrived,
        "exact bounds {:?}, expected {expected:?}: {}",
        shell.app().exact_render_bounds(),
        shell.app().action_digest()
    );
}

/// Types `text`, which replaces whatever the value box held, and presses
/// Enter.
fn enter_value(shell: &mut Shell, text: &str) {
    shell.type_text(text);
    assert_eq!(shell.app().value_input(), text);
    shell.press_key(Key::Enter);
}

/// The default shell with an exact worker, so a scaled part's exact body
/// arrives on screen.
fn scale_shell() -> Shell {
    let mut shell = Shell::new();
    shell
        .app_mut()
        .connect_exact_worker(exact_worker_path())
        .unwrap();
    shell
}

fn first_tag(shell: &Shell, id: OccurrenceId) -> Option<TagId> {
    shell.app().occurrence_tags(id).first().copied()
}

mod display_and_edit_commands;
mod move_scale_and_solid_tools;
mod profiles_selection_and_rotate;
mod tags_and_components;
mod visibility_and_patterns;
