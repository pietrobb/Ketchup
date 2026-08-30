use ketchup_core::document::{
    BodyId, BooleanOperation, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId,
    Dimension, DocumentStore, EdgeFinishKind, FeatureId, FeatureKind, LoftSection, StableFaceRole,
};
use ketchup_core::exact_brep_graph::{
    ExactBRepBooleanOperation, ExactBRepEdgeFinishKind, ExactBRepGraph, ExactBRepGraphError,
    ExactBRepOperation, ExactBRepPlanarGeometry, ExactBRepTopologyKind, MAX_EXACT_BREP_GRAPH_BYTES,
    MAX_EXACT_BREP_GRAPH_SEGMENTS,
};
use ketchup_core::exact_product::{
    ExactFaceRole, ExactFeatureChainRequest, ExactProductError, build_box_render_package,
    canonical_reference_lineage_digest,
};
use ketchup_core::persistence;
use ketchup_core::sketch::{
    FeatureDirection, FeatureExtent, FeatureExtentEnd, PadSpec, PocketSpec, PrincipalPlane,
    SketchEntity, SketchEntityId, SketchSpec, WorkplaneFrame, WorkplaneSpec, WorkplaneSupport,
    WorkplaneSupportHealth,
};
use ketchup_core::topology::{
    TopologicalElementKind, TopologicalElementRef, TopologicalReferenceStability,
};

const DEFINITION: DefinitionId = DefinitionId(1);
const BASE_PROFILE: FeatureId = FeatureId(10);
const BASE_EXTRUSION: FeatureId = FeatureId(11);
const TOOL_PROFILE: FeatureId = FeatureId(20);
const TOOL_EXTRUSION: FeatureId = FeatureId(21);
const BOOLEAN: FeatureId = FeatureId(30);

fn dimension(value: f64) -> Dimension {
    Dimension::new(value.to_string(), value).unwrap()
}

fn topology_reference(
    snapshot: &ketchup_core::document::Snapshot,
    producer: FeatureId,
    kind: TopologicalElementKind,
    ordinal: u32,
) -> TopologicalElementRef {
    TopologicalElementRef::new(
        snapshot.document_id(),
        DEFINITION,
        producer,
        producer,
        kind,
        format!("generated-source/{}/{ordinal}", kind.token()),
        format!("generated-result/{}/{ordinal}", kind.token()),
        TopologicalReferenceStability::Guaranteed,
        "ketchup.exact-brep-graph-evaluator.v1",
        "occt.v1",
        "1e-7-mm",
        format!("result-{}", producer.0),
        format!("geometry-{}-{ordinal}", kind.token()),
    )
    .unwrap()
}

fn arbitrary_boolean_document() -> DocumentStore {
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Arbitrary graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: BASE_PROFILE,
                definition_id: DEFINITION,
                name: "Base pentagon".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![
                        [-12.0, -8.0],
                        [18.0, -6.0],
                        [24.0, 9.0],
                        [3.0, 20.0],
                        [-17.0, 7.0],
                    ],
                },
            },
            CanonicalCommand::CreateFeature {
                id: BASE_EXTRUSION,
                definition_id: DEFINITION,
                name: "Unequal base".into(),
                kind: FeatureKind::Extrusion {
                    profile: BASE_PROFILE,
                    height: dimension(13.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_PROFILE,
                definition_id: DEFINITION,
                name: "Slanted tool".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-3.0, -15.0], [27.0, 4.0], [5.0, 24.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: TOOL_EXTRUSION,
                definition_id: DEFINITION,
                name: "Unequal tool".into(),
                kind: FeatureKind::Extrusion {
                    profile: TOOL_PROFILE,
                    height: dimension(19.0),
                },
            },
            CanonicalCommand::CreateFeature {
                id: BOOLEAN,
                definition_id: DEFINITION,
                name: "Generic intersection".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Intersect,
                    target: BASE_EXTRUSION,
                    tool: TOOL_EXTRUSION,
                },
            },
        ]))
        .unwrap();
    document
}

#[test]
fn compiler_preserves_arbitrary_unequal_extrusions_as_a_topological_graph() {
    let mut document = arbitrary_boolean_document();
    let before = document.current();
    assert_eq!(
        ExactFeatureChainRequest::from_snapshot(&before, DEFINITION),
        Err(ExactProductError::UnsupportedBoolean(
            BooleanOperation::Intersect
        ))
    );

    let graph = ExactBRepGraph::from_snapshot(&before, DEFINITION, BOOLEAN).unwrap();
    assert_eq!(graph.profiles.len(), 2);
    assert_eq!(graph.nodes.len(), 3);
    assert_eq!(graph.nodes[0].source_feature_id, BASE_EXTRUSION.0);
    assert_eq!(graph.nodes[1].source_feature_id, TOOL_EXTRUSION.0);
    assert_eq!(graph.nodes[2].source_feature_id, BOOLEAN.0);
    assert!(matches!(
        graph.nodes[2].operation,
        ExactBRepOperation::Boolean {
            operation: ExactBRepBooleanOperation::Intersect,
            target,
            tool,
        } if target.0 == 0 && tool.0 == 1
    ));
    assert!(graph.profiles.iter().all(|profile| matches!(
        &profile.geometry,
        ExactBRepPlanarGeometry::Boundary { closed: true, segments }
            if segments.len() >= 3
    )));

    let bytes = graph.to_bytes().unwrap();
    assert_eq!(ExactBRepGraph::from_bytes(&bytes).unwrap(), graph);
    let reopened = persistence::load(&persistence::save(&before)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, BOOLEAN).unwrap(),
        graph
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: FeatureId(40),
                definition_id: DEFINITION,
                name: "Unrelated branch profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[100.0, 100.0], [110.0, 100.0], [105.0, 109.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(41),
                definition_id: DEFINITION,
                name: "Unrelated branch body".into(),
                kind: FeatureKind::Extrusion {
                    profile: FeatureId(40),
                    height: dimension(7.0),
                },
            },
        ]))
        .unwrap();
    let with_unrelated_branch =
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN).unwrap();
    assert_eq!(with_unrelated_branch.graph_digest, graph.graph_digest);
    assert_ne!(
        with_unrelated_branch.canonical_input_digest,
        graph.canonical_input_digest
    );
}

#[test]
fn topology_shell_and_edge_finish_compile_to_typed_target_bound_nodes() {
    let mut document = arbitrary_boolean_document();
    let face = topology_reference(
        &document.current(),
        BOOLEAN,
        TopologicalElementKind::Face,
        3,
    );
    let shell = FeatureId(40);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: shell,
            definition_id: DEFINITION,
            name: "Topology shell".into(),
            kind: FeatureKind::TopologyShell {
                target: BOOLEAN,
                removed_faces: vec![face.clone()],
                thickness: dimension(2.5),
            },
        }]))
        .unwrap();
    let edge = topology_reference(&document.current(), shell, TopologicalElementKind::Edge, 7);
    let finish = FeatureId(41);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: finish,
            definition_id: DEFINITION,
            name: "Topology chamfer".into(),
            kind: FeatureKind::TopologyEdgeFinish {
                target: shell,
                edges: vec![edge.clone()],
                kind: EdgeFinishKind::Chamfer,
                amount: dimension(1.25),
            },
        }]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, finish).unwrap();
    assert_eq!(graph.nodes.len(), 5);
    let ExactBRepOperation::Shell {
        target,
        removed_faces,
        thickness_bits,
    } = &graph.nodes[3].operation
    else {
        panic!("fourth node must be a topology shell");
    };
    assert_eq!(target.0, 2);
    assert_eq!(removed_faces.len(), 1);
    assert_eq!(removed_faces[0].kind, ExactBRepTopologyKind::Face);
    assert_eq!(removed_faces[0].reference().unwrap(), face);
    assert_eq!(f64::from_bits(*thickness_bits), 2.5);

    let ExactBRepOperation::EdgeFinish {
        target,
        edges,
        kind,
        amount_bits,
    } = &graph.nodes[4].operation
    else {
        panic!("last node must be a topology edge finish");
    };
    assert_eq!(target.0, 3);
    assert_eq!(*kind, ExactBRepEdgeFinishKind::Chamfer);
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].kind, ExactBRepTopologyKind::Edge);
    assert_eq!(edges[0].reference().unwrap(), edge);
    assert_eq!(f64::from_bits(*amount_bits), 1.25);

    let bytes = graph.to_bytes().unwrap();
    assert_eq!(ExactBRepGraph::from_bytes(&bytes).unwrap(), graph);
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, finish).unwrap(),
        graph
    );

    let mut tampered = graph;
    let ExactBRepOperation::Shell { removed_faces, .. } = &mut tampered.nodes[3].operation else {
        unreachable!();
    };
    removed_faces[0].reference_bytes[0] ^= 1;
    assert_eq!(tampered.validate(), Err(ExactBRepGraphError::InvalidGraph));
}

#[test]
fn topology_face_offset_compiles_and_round_trips_with_signed_distance() {
    let mut document = arbitrary_boolean_document();
    let face = topology_reference(
        &document.current(),
        BOOLEAN,
        TopologicalElementKind::Face,
        2,
    );
    let offset = FeatureId(40);
    document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id: offset,
            definition_id: DEFINITION,
            name: "Topology face offset".into(),
            kind: FeatureKind::TopologyFaceOffset {
                target: BOOLEAN,
                face: face.clone(),
                distance: dimension(-3.5),
            },
        }]))
        .unwrap();

    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, offset).unwrap();
    assert_eq!(graph.nodes.len(), 4);
    let ExactBRepOperation::FaceOffset {
        target,
        face: selector,
        distance_bits,
    } = &graph.nodes[3].operation
    else {
        panic!("last node must be a topology face offset");
    };
    assert_eq!(target.0, 2);
    assert_eq!(selector.kind, ExactBRepTopologyKind::Face);
    assert_eq!(selector.reference().unwrap(), face);
    assert_eq!(f64::from_bits(*distance_bits), -3.5);
    assert_eq!(
        ExactBRepGraph::from_bytes(&graph.to_bytes().unwrap()).unwrap(),
        graph
    );

    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, offset).unwrap(),
        graph
    );
}

#[test]
fn graph_digest_and_payload_tampering_fail_closed() {
    let graph =
        ExactBRepGraph::from_snapshot(&arbitrary_boolean_document().current(), DEFINITION, BOOLEAN)
            .unwrap();

    let mut tampered = graph.clone();
    let ExactBRepOperation::Extrude {
        distance_bits,
        interval,
        ..
    } = &mut tampered.nodes[0].operation
    else {
        panic!("first node must be an extrusion");
    };
    *distance_bits = 14.0_f64.to_bits();
    interval.end_bits = 14.0_f64.to_bits();
    assert_eq!(
        tampered.validate(),
        Err(ExactBRepGraphError::DigestMismatch)
    );
    assert_eq!(
        tampered.to_bytes(),
        Err(ExactBRepGraphError::DigestMismatch)
    );

    let mut value: serde_json::Value = serde_json::from_slice(&graph.to_bytes().unwrap()).unwrap();
    value["graph_digest"] = serde_json::Value::String("00".repeat(32));
    let bytes = serde_json::to_vec(&value).unwrap();
    assert_eq!(
        ExactBRepGraph::from_bytes(&bytes),
        Err(ExactBRepGraphError::DigestMismatch)
    );
}

#[test]
fn compiler_uses_one_contract_for_pad_revolve_sweep_and_loft() {
    let mut document = DocumentStore::new();
    let pad_definition = DefinitionId(2);
    let workplane = FeatureId(100);
    let sketch = FeatureId(101);
    let pad = FeatureId(102);
    let line_arc_sketch = SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [0.0, 0.0],
                end_mm: [20.0, 0.0],
            },
            SketchEntity::Arc {
                id: SketchEntityId(2),
                start_mm: [0.0, 0.0],
                end_mm: [20.0, 0.0],
                center_mm: [10.0, 0.0],
                clockwise: true,
            },
        ],
        constraints: Vec::new(),
    };
    let region = line_arc_sketch.solved_regions().unwrap()[0].id;
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: pad_definition,
                name: "Pad graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: workplane,
                definition_id: pad_definition,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: sketch,
                definition_id: pad_definition,
                name: "Line arc".into(),
                kind: FeatureKind::Sketch(line_arc_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: pad_definition,
                name: "Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch,
                    region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(8.0)),
                }),
            },
        ]))
        .unwrap();

    let revolve_definition = DefinitionId(3);
    let revolve_profile = FeatureId(200);
    let revolve = FeatureId(201);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: revolve_definition,
                name: "Revolve graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: revolve_profile,
                definition_id: revolve_definition,
                name: "Revolve profile".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[2.0, 0.0], [7.0, 0.0], [8.0, 12.0], [2.0, 10.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: revolve,
                definition_id: revolve_definition,
                name: "Revolve".into(),
                kind: FeatureKind::Revolve {
                    profile: revolve_profile,
                    axis_start_mm: [0.0, 0.0],
                    axis_end_mm: [0.0, 1.0],
                    angle_degrees: 275.0,
                },
            },
        ]))
        .unwrap();

    let sweep_definition = DefinitionId(4);
    let sweep_profile = FeatureId(300);
    let sweep_path = FeatureId(301);
    let sweep = FeatureId(302);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: sweep_definition,
                name: "Sweep graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: sweep_profile,
                definition_id: sweep_definition,
                name: "Sweep rectangle".into(),
                kind: FeatureKind::Profile {
                    points_mm: vec![[-2.0, -1.0], [3.0, -1.0], [3.0, 4.0], [-2.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep_path,
                definition_id: sweep_definition,
                name: "Straight path".into(),
                kind: FeatureKind::SegmentProfile {
                    segments: vec![ketchup_core::document::ProfileSegment::Line {
                        start_mm: [0.0, 0.0],
                        end_mm: [15.0, 8.0],
                    }],
                    closed: false,
                },
            },
            CanonicalCommand::CreateFeature {
                id: sweep,
                definition_id: sweep_definition,
                name: "Sweep".into(),
                kind: FeatureKind::Sweep {
                    profile: sweep_profile,
                    path: sweep_path,
                },
            },
        ]))
        .unwrap();

    let loft_definition = DefinitionId(5);
    let lower = FeatureId(400);
    let upper = FeatureId(401);
    let loft = FeatureId(402);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: loft_definition,
                name: "Loft graph".into(),
            },
            CanonicalCommand::CreateFeature {
                id: lower,
                definition_id: loft_definition,
                name: "Lower spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-8.0, -4.0], [9.0, -3.0], [7.0, 6.0], [-6.0, 5.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: upper,
                definition_id: loft_definition,
                name: "Upper spline".into(),
                kind: FeatureKind::SplineProfile {
                    control_points_mm: vec![[-4.0, -2.0], [5.0, -2.0], [4.0, 3.0], [-3.0, 4.0]],
                },
            },
            CanonicalCommand::CreateFeature {
                id: loft,
                definition_id: loft_definition,
                name: "Loft".into(),
                kind: FeatureKind::Loft {
                    sections: vec![
                        LoftSection {
                            profile: lower,
                            elevation_mm: 0.0,
                        },
                        LoftSection {
                            profile: upper,
                            elevation_mm: 35.0,
                        },
                    ],
                },
            },
        ]))
        .unwrap();

    let snapshot = document.current();
    let pad_graph = ExactBRepGraph::from_snapshot(&snapshot, pad_definition, pad).unwrap();
    let revolve_graph =
        ExactBRepGraph::from_snapshot(&snapshot, revolve_definition, revolve).unwrap();
    let sweep_graph = ExactBRepGraph::from_snapshot(&snapshot, sweep_definition, sweep).unwrap();
    let loft_graph = ExactBRepGraph::from_snapshot(&snapshot, loft_definition, loft).unwrap();

    assert!(matches!(
        pad_graph.nodes[0].operation,
        ExactBRepOperation::Extrude { .. }
    ));
    assert!(matches!(
        revolve_graph.nodes[0].operation,
        ExactBRepOperation::Revolve { .. }
    ));
    assert!(matches!(
        sweep_graph.nodes[0].operation,
        ExactBRepOperation::Sweep { .. }
    ));
    assert!(matches!(
        loft_graph.nodes[0].operation,
        ExactBRepOperation::Loft { .. }
    ));
    assert!(matches!(
        &pad_graph.profiles[0].geometry,
        ExactBRepPlanarGeometry::Boundary {
            closed: true,
            segments
        } if segments.len() == 2
    ));
    assert!(matches!(
        &sweep_graph.profiles[1].geometry,
        ExactBRepPlanarGeometry::Boundary {
            closed: false,
            segments
        } if segments.len() == 1
    ));
    for graph in [pad_graph, revolve_graph, sweep_graph, loft_graph] {
        let bytes = graph.to_bytes().unwrap();
        assert_eq!(ExactBRepGraph::from_bytes(&bytes).unwrap(), graph);
    }
}

#[test]
fn graph_is_deterministic_across_recompile_save_open_and_undo_redo() {
    let mut document = arbitrary_boolean_document();
    let initial = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN).unwrap(),
        initial
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN)
            .unwrap()
            .to_bytes()
            .unwrap(),
        initial.to_bytes().unwrap()
    );

    let reopened = persistence::load(&persistence::save(&document.current())).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, BOOLEAN).unwrap(),
        initial
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: BASE_EXTRUSION,
                dimension: dimension(17.0),
            },
        ]))
        .unwrap();
    let changed = ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN).unwrap();
    assert_ne!(changed.graph_digest, initial.graph_digest);
    assert_ne!(
        changed.canonical_input_digest,
        initial.canonical_input_digest
    );

    let undone = document.undo().unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&undone, DEFINITION, BOOLEAN).unwrap(),
        initial
    );
    let redone = document.redo().unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&redone, DEFINITION, BOOLEAN).unwrap(),
        changed
    );
}

#[test]
fn missing_unsupported_and_suppressed_inputs_fail_closed() {
    let mut document = arbitrary_boolean_document();
    let before = document.current();
    let before_digest = before.canonical_digest();

    assert_eq!(
        ExactBRepGraph::from_snapshot(&before, DEFINITION, FeatureId(999)),
        Err(ExactBRepGraphError::FeatureNotFound(FeatureId(999)))
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&before, DEFINITION, BASE_PROFILE),
        Err(ExactBRepGraphError::UnsupportedFeature(BASE_PROFILE))
    );
    assert_eq!(document.current().canonical_digest(), before_digest);

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetBodyFeatureSuppression {
                definition_id: DEFINITION,
                body_id: BodyId(1),
                suppressed_feature_ids: vec![BOOLEAN],
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN),
        Err(ExactBRepGraphError::SuppressedFeature(BOOLEAN))
    );
}

#[test]
fn canonical_cycle_is_rejected_without_changing_the_valid_graph() {
    let mut document = arbitrary_boolean_document();
    let before = document.current();
    let graph = ExactBRepGraph::from_snapshot(&before, DEFINITION, BOOLEAN).unwrap();
    let revision_count = document.revision_count();
    let undo_steps = document.visible_undo_steps();
    let first = FeatureId(50);
    let second = FeatureId(51);
    let shell = |target| FeatureKind::Shell {
        target,
        removed_faces: vec![StableFaceRole::new("graph.cycle.face").unwrap()],
        thickness: dimension(1.0),
    };

    let error = match document.apply_batch(&CommandBatch::new(vec![
        CanonicalCommand::CreateFeature {
            id: first,
            definition_id: DEFINITION,
            name: "Cycle first".into(),
            kind: shell(second),
        },
        CanonicalCommand::CreateFeature {
            id: second,
            definition_id: DEFINITION,
            name: "Cycle second".into(),
            kind: shell(first),
        },
    ])) {
        Ok(_) => panic!("dependency cycle was accepted"),
        Err(error) => error,
    };
    assert_eq!(error, CanonicalError::FeatureDependencyCycle(first));
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(document.revision_count(), revision_count);
    assert_eq!(document.visible_undo_steps(), undo_steps);
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, BOOLEAN).unwrap(),
        graph
    );
}

#[test]
fn byte_and_segment_resource_limits_fail_without_mutating_the_document() {
    assert_eq!(
        ExactBRepGraph::from_bytes(&vec![0; MAX_EXACT_BREP_GRAPH_BYTES + 1]),
        Err(ExactBRepGraphError::ResourceLimit)
    );

    let mut document = DocumentStore::new();
    let segment_count_per_profile = 1_024;
    let profile_count = MAX_EXACT_BREP_GRAPH_SEGMENTS / segment_count_per_profile + 1;
    let mut commands = vec![CanonicalCommand::CreateDefinition {
        id: DEFINITION,
        name: "Resource bounded graph".into(),
    }];
    let mut producer = FeatureId(0);
    for profile_index in 0..profile_count {
        let profile = FeatureId(600 + profile_index as u64 * 2);
        let extrusion = FeatureId(profile.0 + 1);
        let center_x = profile_index as f64 * 30.0;
        let points_mm = (0..segment_count_per_profile)
            .map(|point_index| {
                let angle =
                    std::f64::consts::TAU * point_index as f64 / segment_count_per_profile as f64;
                [center_x + 10.0 * angle.cos(), 10.0 * angle.sin()]
            })
            .collect();
        commands.push(CanonicalCommand::CreateFeature {
            id: profile,
            definition_id: DEFINITION,
            name: format!("Bounded profile {profile_index}"),
            kind: FeatureKind::Profile { points_mm },
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: extrusion,
            definition_id: DEFINITION,
            name: format!("Bounded extrusion {profile_index}"),
            kind: FeatureKind::Extrusion {
                profile,
                height: dimension(2.0),
            },
        });
        producer = if profile_index == 0 {
            extrusion
        } else {
            let boolean = FeatureId(2_000 + profile_index as u64);
            commands.push(CanonicalCommand::CreateFeature {
                id: boolean,
                definition_id: DEFINITION,
                name: format!("Aggregate union {profile_index}"),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Union,
                    target: producer,
                    tool: extrusion,
                },
            });
            boolean
        };
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    let before = document.current();
    let revision_count = document.revision_count();
    let undo_steps = document.visible_undo_steps();

    assert_eq!(
        ExactBRepGraph::from_snapshot(&before, DEFINITION, producer),
        Err(ExactBRepGraphError::ResourceLimit)
    );
    assert_eq!(
        document.current().canonical_digest(),
        before.canonical_digest()
    );
    assert_eq!(document.revision_count(), revision_count);
    assert_eq!(document.visible_undo_steps(), undo_steps);
}

#[test]
fn generalized_extents_compile_to_bounded_signed_intervals_and_fail_closed() {
    let target_sketch_id = FeatureId(700);
    let target = FeatureId(701);
    let base_plane = FeatureId(702);
    let pad_sketch_id = FeatureId(703);
    let top_plane = FeatureId(704);
    let pocket_sketch_id = FeatureId(705);
    let pad = FeatureId(706);
    let pocket = FeatureId(707);
    let rectangle = |workplane, min, max| SketchSpec {
        workplane,
        entities: vec![
            SketchEntity::Line {
                id: SketchEntityId(1),
                start_mm: [min, min],
                end_mm: [max, min],
            },
            SketchEntity::Line {
                id: SketchEntityId(2),
                start_mm: [max, min],
                end_mm: [max, max],
            },
            SketchEntity::Line {
                id: SketchEntityId(3),
                start_mm: [max, max],
                end_mm: [min, max],
            },
            SketchEntity::Line {
                id: SketchEntityId(4),
                start_mm: [min, max],
                end_mm: [min, min],
            },
        ],
        constraints: Vec::new(),
    };
    let target_sketch = rectangle(base_plane, 0.0, 20.0);
    let pad_sketch = rectangle(base_plane, 2.0, 8.0);
    let pocket_sketch = rectangle(top_plane, 2.0, 8.0);
    let target_region = target_sketch.solved_regions().unwrap()[0].id;
    let region = pad_sketch.solved_regions().unwrap()[0].id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Generalized extents".into(),
            },
            CanonicalCommand::CreateFeature {
                id: base_plane,
                definition_id: DEFINITION,
                name: "Base plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: target_sketch_id,
                definition_id: DEFINITION,
                name: "Target sketch".into(),
                kind: FeatureKind::Sketch(target_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad_sketch_id,
                definition_id: DEFINITION,
                name: "Pad sketch".into(),
                kind: FeatureKind::Sketch(pad_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: target,
                definition_id: DEFINITION,
                name: "Exact target".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: target_sketch_id,
                    region: target_region,
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(10.0)),
                }),
            },
        ]))
        .unwrap();

    let target_snapshot = document.current();
    let request =
        ExactFeatureChainRequest::from_snapshot_for_producer(&target_snapshot, DEFINITION, target)
            .unwrap();
    let evidence = [
        ExactFaceRole::Top,
        ExactFaceRole::Bottom,
        ExactFaceRole::East,
    ]
    .map(|role| {
        (
            role,
            canonical_reference_lineage_digest(
                request.document_id,
                request.producer_feature_id(),
                role.semantic_role(),
                role.source_element_id(),
                role.expected_type(),
            ),
            format!("extent-resolution.{role:?}"),
        )
    });
    let package = build_box_render_package(
        &request,
        "extent-resolution-input".into(),
        "extent-resolution-result".into(),
        "test-backend".into(),
        "test-tolerance".into(),
        request.expected_bounds_mm(),
        evidence,
    )
    .unwrap();
    let top = package.reference(ExactFaceRole::Top).unwrap().clone();
    document
        .register_exact_reference_evidence(top.clone())
        .unwrap();

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateFeature {
                id: top_plane,
                definition_id: DEFINITION,
                name: "Top face plane".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace {
                        reference: Box::new(top.clone()),
                        health: WorkplaneSupportHealth::Resolved,
                    },
                    frame: WorkplaneFrame::principal(PrincipalPlane::Xy).offset(10.0),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket_sketch_id,
                definition_id: DEFINITION,
                name: "Pocket sketch".into(),
                kind: FeatureKind::Sketch(pocket_sketch),
            },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: DEFINITION,
                name: "Oblique bounded Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: pad_sketch_id,
                    region,
                    direction: FeatureDirection::Vector([1.0, 0.0, 1.0]),
                    extent: FeatureExtent::Bidirectional {
                        along: FeatureExtentEnd::UpToFace(Box::new(top.clone())),
                        opposite: FeatureExtentEnd::Blind(dimension(3.0)),
                    },
                }),
            },
            CanonicalCommand::CreateFeature {
                id: pocket,
                definition_id: DEFINITION,
                name: "Bounded Through All Pocket".into(),
                kind: FeatureKind::SketchPocket(PocketSpec {
                    target,
                    sketch: pocket_sketch_id,
                    region,
                    support: Box::new(top.clone()),
                    direction: FeatureDirection::OppositeNormal,
                    extent: FeatureExtent::ThroughAll,
                }),
            },
        ]))
        .unwrap();
    let snapshot = document.current();

    let pad_graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, pad).unwrap();
    let ExactBRepOperation::Extrude { interval, .. } = pad_graph.nodes[0].operation else {
        panic!("Pad must compile to one interval extrusion");
    };
    let inverse_sqrt_two = 1.0 / 2.0_f64.sqrt();
    assert_eq!(
        interval.direction(),
        [inverse_sqrt_two, 0.0, inverse_sqrt_two]
    );
    assert_eq!(interval.start_mm(), -3.0);
    assert!((interval.end_mm() - 10.0 * 2.0_f64.sqrt()).abs() < 1.0e-12);

    let pocket_graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, pocket).unwrap();
    let ExactBRepOperation::ProfileCut { interval, .. } = pocket_graph.nodes[1].operation else {
        panic!("Pocket must compile after its target");
    };
    assert_eq!(interval.direction(), [0.0, 0.0, -1.0]);
    assert_eq!(interval.start_mm(), -1.0);
    assert_eq!(interval.end_mm(), 11.0);
    assert_eq!(
        ExactBRepGraph::from_bytes(&pocket_graph.to_bytes().unwrap()).unwrap(),
        pocket_graph
    );
    let reopened = persistence::load(&persistence::save(&snapshot)).unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, pad).unwrap(),
        pad_graph
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&reopened.snapshot(), DEFINITION, pocket).unwrap(),
        pocket_graph
    );

    let mut off_face = persistence::load(&persistence::save(&snapshot))
        .unwrap()
        .into_editable()
        .unwrap_or_else(|_| panic!("current schema must remain editable"));
    off_face
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::DeleteFeature { id: pad },
            CanonicalCommand::CreateFeature {
                id: pad,
                definition_id: DEFINITION,
                name: "Off-face Pad".into(),
                kind: FeatureKind::Pad(PadSpec {
                    sketch: pad_sketch_id,
                    region,
                    direction: FeatureDirection::Vector([10.0, 0.0, 1.0]),
                    extent: FeatureExtent::UpToFace(Box::new(top.clone())),
                }),
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&off_face.current(), DEFINITION, pad),
        Err(ExactBRepGraphError::UnresolvedExtent)
    );

    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetFeatureDimension {
                id: target,
                dimension: dimension(12.0),
            },
        ]))
        .unwrap();
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, pad),
        Err(ExactBRepGraphError::UnresolvedExtent)
    );
    assert_eq!(
        ExactBRepGraph::from_snapshot(&document.current(), DEFINITION, pocket),
        Err(ExactBRepGraphError::UnresolvedExtent)
    );
}
