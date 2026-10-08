use super::*;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use ketchup_assistant::sidecar::AssistantCadLoftSection;
use ketchup_geometry::sketch::{FeatureExtent, PadOperation, PadProfile, PadSpec};
use ketchup_model::document::{
    EdgeRef, FaceRef, InstancePathStep, ProposalGoal, SpatialPathSegment,
};
use ketchup_model::exact_brep_graph::{EXACT_BREP_GRAPH_SCHEMA_V12, ExactBRepOperation};
use ketchup_model::graph::{EvaluatorNodeKind, PortSpec};

#[path = "planning_topology_tests.rs"]
mod planning_topology;

#[path = "selection_path_tests.rs"]
mod selection_path;

fn apply_reviewed_model_intent(app: &mut KetchupApp, intent: AssistantModelIntent) -> bool {
    app.prepare_assistant_model_intent(intent) && app.confirm_assistant_proposal()
}

fn select_initial_top_face(app: &mut KetchupApp) {
    app.selection.select_exact(
        SelectionId {
            definition_id: INITIAL_BOX_DEFINITION,
            instance_path: InstancePath::root(OccurrenceId(1)),
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
        },
        false,
    );
}

fn install_graph_result(
    app: &mut KetchupApp,
    definition_id: DefinitionId,
    producer_feature_id: FeatureId,
    fallback_bounds_mm: Option<[[f64; 3]; 2]>,
) {
    let snapshot = app.document.current();
    let graph =
        ExactBRepGraph::from_snapshot(&snapshot, definition_id, producer_feature_id).unwrap();
    let [minimum_mm, maximum_mm] = graph
        .producer_bounds_mm()
        .unwrap()
        .or(fallback_bounds_mm)
        .unwrap();
    let minimum = Vec3::new(minimum_mm[0], minimum_mm[1], minimum_mm[2]);
    let maximum = Vec3::new(maximum_mm[0], maximum_mm[1], maximum_mm[2]);
    let size = maximum - minimum;
    let vertices_mm = box_corners(size.x, size.y, size.z)
        .map(|point| {
            let point = point + minimum;
            [point.x, point.y, point.z]
        })
        .to_vec();
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
            exact_input_digest: "headless-topology-input".into(),
            result_fingerprint: "headless-topology-result".into(),
            volume_mm3: size.x * size.y * size.z,
            area_mm2: 0.0,
            topology_counts: [8, 12, 6, 1, 1],
            wire_count: None,
            bounds_mm: [
                [minimum.x, minimum.y, minimum.z],
                [maximum.x, maximum.y, maximum.z],
            ],
            backend: "headless-topology-backend.v1".into(),
            tolerance: "1e-7-mm".into(),
            faces: Vec::new(),
            edges: vec![
                ketchup_model::exact_product::ExactBRepGraphEdgeEvidence {
                    edge_ordinal: 0,
                    curve_kind: "line".into(),
                    length_mm: size.x,
                    centroid_mm: [minimum.x + size.x * 0.5, minimum.y, minimum.z],
                    bounds_mm: [
                        [minimum.x, minimum.y, minimum.z],
                        [maximum.x, minimum.y, minimum.z],
                    ],
                    closed: false,
                    circle_radius_mm: None,
                    axis_origin_mm: Some([minimum.x, minimum.y, minimum.z]),
                    unit_axis_direction: Some([1.0, 0.0, 0.0]),
                    adjacent_face_ordinals: vec![0, 2],
                },
                ketchup_model::exact_product::ExactBRepGraphEdgeEvidence {
                    edge_ordinal: 1,
                    curve_kind: "line".into(),
                    length_mm: size.y,
                    centroid_mm: [maximum.x, minimum.y + size.y * 0.5, minimum.z],
                    bounds_mm: [
                        [maximum.x, minimum.y, minimum.z],
                        [maximum.x, maximum.y, minimum.z],
                    ],
                    closed: false,
                    circle_radius_mm: None,
                    axis_origin_mm: Some([maximum.x, minimum.y, minimum.z]),
                    unit_axis_direction: Some([0.0, 1.0, 0.0]),
                    adjacent_face_ordinals: vec![0, 5],
                },
            ],
        },
        &StepImportMesh {
            vertices_mm,
            triangles,
        },
    )
    .unwrap();
    assert!(app.headless_install_exact_package(ExactBodyPackage::Graph(package)));
}

pub(crate) fn install_initial_graph_result(app: &mut KetchupApp) {
    install_graph_result(app, INITIAL_BOX_DEFINITION, FeatureId(2), None);
}

fn select_initial_topological(app: &mut KetchupApp, kind: TopologicalElementKind, ordinal: u32) {
    assert!(app.select_topological_locator(TopologicalPickLocator {
        instance_path: InstancePath::root(OccurrenceId(1)),
        producer_feature_id: FeatureId(2),
        kind,
        ordinal,
    }));
}

pub(super) fn lossy_legacy_document() -> Vec<u8> {
    let mut bytes = b"KETCHUPDOC".to_vec();
    bytes.extend_from_slice(&0_u16.to_le_bytes());
    bytes.extend_from_slice(&7_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.extend_from_slice(&42_u64.to_le_bytes());
    bytes.extend_from_slice(&1_u32.to_le_bytes());
    bytes.push(b'x');
    bytes.extend_from_slice(&3.5_f64.to_bits().to_le_bytes());
    bytes.extend_from_slice(&0_u32.to_le_bytes());
    bytes
}

pub(super) fn through_cut_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DefinitionId(10),
                name: "Exact cut body".to_owned(),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(11),
                definition_id: DefinitionId(10),
                name: "Outer profile".to_owned(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(12),
                definition_id: DefinitionId(10),
                name: "Base extrusion".to_owned(),
                kind: FeatureKind::extrusion(FeatureId(11), Dimension::from_decimal("10").unwrap()),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(13),
                definition_id: DefinitionId(10),
                name: "Cut profile".to_owned(),
                kind: FeatureKind::polygon(&[[4.0, 4.0], [6.0, 4.0], [6.0, 6.0], [4.0, 6.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(14),
                definition_id: DefinitionId(10),
                name: "Through cut".to_owned(),
                kind: FeatureKind::through_cut(FeatureId(12), FeatureId(13)),
            },
            CanonicalCommand::CreateOccurrence {
                id: OccurrenceId(10),
                definition_id: DefinitionId(10),
                name: "Cut body occurrence".to_owned(),
                transform: Transform::identity(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    document.discard_history_before_current();
    document
}

fn exact_worker_executable() -> PathBuf {
    let executable_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(executable_name)
}

fn current_box_package(app: &KetchupApp) -> ExactBodyPackage {
    ketchup_model::testing::box_package(
        &app.document.current(),
        INITIAL_BOX_DEFINITION,
        FeatureId(2),
        "result",
        &[
            ExactFaceRole::Top,
            ExactFaceRole::Bottom,
            ExactFaceRole::LinearSide,
        ],
    )
    .expect("the default box compiles to an exact graph")
}

/// A closed jagged sphere: every edge separates differently oriented
/// triangles, so the whole tessellation counts as feature edges.
fn jagged_sphere_binary_stl(stacks: usize, slices: usize) -> Vec<u8> {
    let point = |stack: usize, slice: usize| {
        if stack == 0 {
            return [0.0, 0.0, 10.0];
        }
        if stack == stacks {
            return [0.0, 0.0, -10.0];
        }
        let radius = 10.0 + f64::from(u8::from((stack + slice).is_multiple_of(2))) * 1.5;
        let polar = std::f64::consts::PI * stack as f64 / stacks as f64;
        let azimuth = std::f64::consts::TAU * (slice % slices) as f64 / slices as f64;
        [
            radius * polar.sin() * azimuth.cos(),
            radius * polar.sin() * azimuth.sin(),
            radius * polar.cos(),
        ]
    };
    let mut facets = Vec::new();
    for stack in 0..stacks {
        for slice in 0..slices {
            let corners = [
                point(stack, slice),
                point(stack, slice + 1),
                point(stack + 1, slice + 1),
                point(stack + 1, slice),
            ];
            if stack > 0 {
                facets.push([corners[0], corners[2], corners[1]]);
            }
            if stack + 1 < stacks {
                facets.push([corners[0], corners[3], corners[2]]);
            }
        }
    }
    let mut source = vec![0_u8; 80];
    source.extend_from_slice(&(facets.len() as u32).to_le_bytes());
    for facet in &facets {
        let normal = triangle_normal(facet.map(|point| Vec3::new(point[0], point[1], point[2])));
        for value in [normal.x, normal.y, normal.z] {
            source.extend_from_slice(&(value as f32).to_le_bytes());
        }
        for corner in facet {
            for value in corner {
                source.extend_from_slice(&(*value as f32).to_le_bytes());
            }
        }
        source.extend_from_slice(&0_u16.to_le_bytes());
    }
    source
}

/// The faint ring-and-tick colour the guide uses, which no other viewport
/// layer paints — the solid axis lines of the ground plane are opaque.
fn guide_tick_colour(axis: Axis) -> Color32 {
    let solid = axis_color(axis);
    Color32::from_rgba_unmultiplied(solid.r(), solid.g(), solid.b(), 130)
}

fn painted_segments(context: &egui::Context, app: &mut KetchupApp, colour: Color32) -> usize {
    let output = context.run(egui::RawInput::default(), |context| app.ui(context));
    let mut count = 0;
    for clipped in output.shapes {
        count_segments_coloured(&clipped.shape, colour, &mut count);
    }
    count
}

fn count_segments_coloured(shape: &egui::Shape, colour: Color32, count: &mut usize) {
    match shape {
        egui::Shape::LineSegment { stroke, .. } if stroke.color == colour => *count += 1,
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                count_segments_coloured(shape, colour, count);
            }
        }
        _ => {}
    }
}

fn selection_stroke_segments(context: &egui::Context, app: &mut KetchupApp) -> usize {
    let output = context.run(egui::RawInput::default(), |context| app.ui(context));
    let mut count = 0;
    for clipped in output.shapes {
        count_selection_segments(&clipped.shape, &mut count);
    }
    count
}

fn count_selection_segments(shape: &egui::Shape, count: &mut usize) {
    match shape {
        egui::Shape::LineSegment { stroke, .. } => {
            if stroke.color == Color32::from_rgb(240, 78, 35) {
                *count += 1;
            }
        }
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                count_selection_segments(shape, count);
            }
        }
        _ => {}
    }
}

fn press_in_a_frame(events: Vec<egui::Event>, typing: bool) -> Option<AppCommand> {
    let context = egui::Context::default();
    let mut pressed = None;
    let _ = context.run(
        egui::RawInput {
            events,
            ..Default::default()
        },
        |context| pressed = context.input_mut(|input| keymap::pressed(input, typing)),
    );
    pressed
}

fn key_event(chord: &egui::KeyboardShortcut) -> egui::Event {
    egui::Event::Key {
        key: chord.logical_key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: chord.modifiers,
    }
}

mod assistant;
mod assistant_reviews;
mod cad_edit;
mod drawing;
mod exact_modeling;
mod interchange;
mod organization;
pub(crate) mod shell;
mod transform;
mod viewport;
