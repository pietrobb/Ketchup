//! Two occurrences of one meshed block, the second stacked on the first,
//! joined top face to bottom face: bonded they deform together where the faces
//! touch; by a rigid connector the joined faces move as one plane, and a
//! spring connector adds exactly the force over its stiffness.

use super::*;
use ketchup_analysis::fea::{FeaCouplingKind, FeaModel, FeaSolution};
use ketchup_model::assembly::{
    AssemblyMate, AssemblyMateEndpoint, AssemblyMateId, AssemblyMateKind,
};
use ketchup_model::exact_product::ExactBRepGraphFaceEvidence;
use ketchup_scheduler::{
    ExactBRepVolumeMeshPackage, ExactFeaAssemblyPart, ExactFeaAssemblySetup, ExactFeaJoint,
    ExactFeaJointFace, ExactFeaJointKind, ExactFeaMateJoint, assembly_bound_model,
};

const HEIGHT_MM: f64 = 5.0;

fn steel() -> FeaMaterial {
    FeaMaterial {
        id: 1,
        youngs_modulus_mpa: 200_000.0,
        poisson_ratio: 0.3,
        yield_strength_mpa: Some(250.0),
        density_kg_per_m3: None,
    }
}

/// The block's bottom and top face ordinals.
fn bottom_and_top(package: &ExactBRepVolumeMeshPackage) -> (u32, u32) {
    let by_height = |left: &&ExactBRepGraphFaceEvidence, right: &&ExactBRepGraphFaceEvidence| {
        left.centroid_mm[2].total_cmp(&right.centroid_mm[2])
    };
    let faces = &package.source.face_evidence;
    (
        faces.iter().min_by(by_height).unwrap().face_ordinal,
        faces.iter().max_by(by_height).unwrap().face_ordinal,
    )
}

fn stack_setup(
    lower: OccurrenceId,
    upper: OccurrenceId,
    (bottom, top): (u32, u32),
    kind: ExactFeaJointKind,
) -> ExactFeaAssemblySetup {
    ExactFeaAssemblySetup {
        case_id: "stacked-blocks".into(),
        parts: vec![
            ExactFeaAssemblyPart {
                instance_path: InstancePath::root(lower),
                material: steel(),
                constrained_face_ordinals: vec![bottom],
                face_tractions: Vec::new(),
            },
            ExactFeaAssemblyPart {
                instance_path: InstancePath::root(upper),
                material: steel(),
                constrained_face_ordinals: Vec::new(),
                face_tractions: vec![ExactFeaFaceTraction {
                    face_ordinal: top,
                    traction_local_n_per_mm2: [0.0, 0.0, -1.0],
                }],
            },
        ],
        joints: vec![ExactFeaJoint {
            faces: [
                ExactFeaJointFace {
                    part: 0,
                    face_ordinal: top,
                },
                ExactFeaJointFace {
                    part: 1,
                    face_ordinal: bottom,
                },
            ],
            kind,
        }],
        mate_joints: Vec::new(),
    }
}

fn near(found: f64, expected: f64, scale: f64) {
    assert!(
        (found - expected).abs() <= 1.0e-9 * scale,
        "{found} against {expected}"
    );
}

fn balanced(solution: &FeaSolution) {
    assert!(solution.maximum_free_dof_residual_n <= 1.0e-7);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() <= 1.0e-8)
    );
}

#[test]
fn stacked_occurrences_joined_by_a_rigid_or_a_spring_connector_solve_as_one_assembly() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, extrusion, lower) = simple_extrusion_document();
    let upper = OccurrenceId(901);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: upper,
                definition_id: definition,
                name: "Stacked occurrence".into(),
                transform: Transform::from_translation(0.0, 0.0, HEIGHT_MM).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, extrusion).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor
        .evaluate_exact_brep_graph_volume_mesh(
            &graph,
            ExactVolumeMeshWireOptions {
                surface_deflection_mm: 0.1,
                angular_deflection_rad: 0.2,
                max_tetrahedra: 256,
                max_relative_volume_error: 1.0e-10,
                min_tetrahedron_quality: 1.0e-5,
            },
        )
        .unwrap();
    let faces = bottom_and_top(&package);
    let packages = [&package, &package];

    let rigid = assembly_bound_model(
        &snapshot,
        &packages,
        &stack_setup(lower, upper, faces, ExactFeaJointKind::RigidConnector),
    )
    .unwrap();
    let mesh_nodes = package.mesh.vertices_mm.len();
    assert_eq!(rigid.parts[1].first_node, mesh_nodes);
    assert_eq!(rigid.model.nodes.len(), 2 * mesh_nodes + 1);
    assert_eq!(
        rigid.model.elements.len(),
        2 * package.mesh.tetrahedra.len()
    );
    // The upper block stands where the assembly puts it.
    assert!(
        rigid.model.nodes[mesh_nodes..2 * mesh_nodes]
            .iter()
            .all(|node| node.position_mm[2] >= HEIGHT_MM - 1.0e-12)
    );
    let reference = rigid.joint_nodes[0].unwrap();
    near(
        rigid.model.nodes[reference].position_mm[2],
        HEIGHT_MM,
        HEIGHT_MM,
    );
    let rigid_solution = rigid.model.solve(FeaSolveSettings::default()).unwrap();
    balanced(&rigid_solution);
    let scale = rigid_solution.maximum_displacement_mm;
    assert!(scale > 0.0);
    // Every joined node moves with the reference as one rigid plane.
    let turn = rigid_solution.rotations_rad[reference].unwrap();
    let moved = rigid_solution.displacements_mm[reference];
    for coupling in &rigid.model.couplings {
        let [leader, node] = coupling.nodes;
        assert_eq!(leader, reference);
        let lever = ketchup_geometry::linalg::sub(
            rigid.model.nodes[node].position_mm,
            rigid.model.nodes[reference].position_mm,
        );
        let expected =
            ketchup_geometry::linalg::add(moved, ketchup_geometry::linalg::cross(turn, lever));
        for (found, expected) in rigid_solution.displacements_mm[node].iter().zip(expected) {
            near(*found, expected, scale);
        }
    }

    let stiffness = 1_000.0;
    let spring = assembly_bound_model(
        &snapshot,
        &packages,
        &stack_setup(
            lower,
            upper,
            faces,
            ExactFeaJointKind::SpringConnector {
                translation_n_per_mm: [stiffness; 3],
                rotation_n_mm_per_rad: [1.0e6; 3],
            },
        ),
    )
    .unwrap();
    let spring_solution = spring.model.solve(FeaSolveSettings::default()).unwrap();
    balanced(&spring_solution);
    // The 8 x 6 mm top carries 48 N down through the joint's centre: the lower
    // block deforms as before and the upper one sinks a further 48 N / k.
    let sink = -48.0 / stiffness;
    for node in 0..2 * mesh_nodes {
        let extra = if node < mesh_nodes { 0.0 } else { sink };
        for axis in 0..3 {
            let expected =
                rigid_solution.displacements_mm[node][axis] + if axis == 2 { extra } else { 0.0 };
            near(
                spring_solution.displacements_mm[node][axis],
                expected,
                scale + sink.abs(),
            );
        }
    }
}

#[test]
fn assembly_joints_that_cannot_be_built_are_refused() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, extrusion, lower) = simple_extrusion_document();
    let upper = OccurrenceId(902);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateOccurrence {
                id: upper,
                definition_id: definition,
                name: "Stacked occurrence".into(),
                transform: Transform::from_translation(0.0, 0.0, HEIGHT_MM).unwrap(),
                parent: None,
                tags: Default::default(),
                visible: true,
            },
        ]))
        .unwrap();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, definition, extrusion).unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = supervisor
        .evaluate_exact_brep_graph_volume_mesh(
            &graph,
            ExactVolumeMeshWireOptions {
                surface_deflection_mm: 0.1,
                angular_deflection_rad: 0.2,
                max_tetrahedra: 64,
                max_relative_volume_error: 1.0e-10,
                min_tetrahedron_quality: 1.0e-5,
            },
        )
        .unwrap();
    let faces = bottom_and_top(&package);
    let packages = [&package, &package];
    let setup = stack_setup(lower, upper, faces, ExactFeaJointKind::Bonded);
    let refused = |setup: &ExactFeaAssemblySetup| {
        assembly_bound_model(&snapshot, &packages, setup).unwrap_err()
    };

    let mut one_part = setup.clone();
    one_part.joints[0].faces[1].part = 0;
    assert_eq!(
        refused(&one_part),
        ExactFeaSetupError::InvalidJoint { joint: 0 }
    );
    let mut unknown_face = setup.clone();
    unknown_face.joints[0].faces[1].face_ordinal = u32::MAX;
    assert_eq!(
        refused(&unknown_face),
        ExactFeaSetupError::InvalidJoint { joint: 0 }
    );
    let mut repeated = setup.clone();
    repeated.joints.push(repeated.joints[0].clone());
    assert_eq!(
        refused(&repeated),
        ExactFeaSetupError::InvalidJoint { joint: 1 }
    );
    let mut other_steel = setup.clone();
    other_steel.parts[1].material.youngs_modulus_mpa = 70_000.0;
    assert_eq!(
        refused(&other_steel),
        ExactFeaSetupError::ConflictingMaterial
    );
    assert_eq!(
        assembly_bound_model(&snapshot, &[&package], &setup).unwrap_err(),
        ExactFeaSetupError::IncompleteSetup
    );
}

fn block_mesh(
    supervisor: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
    definition: DefinitionId,
    extrusion: FeatureId,
) -> ExactBRepVolumeMeshPackage {
    let graph = ExactBRepGraph::from_snapshot(snapshot, definition, extrusion).unwrap();
    supervisor
        .evaluate_exact_brep_graph_volume_mesh(
            &graph,
            ExactVolumeMeshWireOptions {
                surface_deflection_mm: 0.1,
                angular_deflection_rad: 0.2,
                max_tetrahedra: 64,
                max_relative_volume_error: 1.0e-10,
                min_tetrahedron_quality: 1.0e-5,
            },
        )
        .unwrap()
}

#[test]
fn a_face_to_face_mate_joins_the_faces_it_names() {
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, extrusion, lower) = simple_extrusion_document();
    let upper = OccurrenceId(903);
    let beside = OccurrenceId(904);
    let placed = |id, name: &str, transform| CanonicalCommand::CreateOccurrence {
        id,
        definition_id: definition,
        name: name.into(),
        transform,
        parent: None,
        tags: Default::default(),
        visible: true,
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            placed(
                upper,
                "Stacked occurrence",
                Transform::from_translation(0.0, 0.0, HEIGHT_MM).unwrap(),
            ),
            placed(
                beside,
                "Beside occurrence",
                Transform::from_translation(20.0, 0.0, 0.0).unwrap(),
            ),
        ]))
        .unwrap();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let evidence = block_mesh(&mut supervisor, &document.current(), definition, extrusion);
    let face = |role| {
        evidence
            .source
            .planar_face_attachments
            .iter()
            .find(|attachment| attachment.reference().role() == Some(role))
            .unwrap()
            .clone()
    };
    let mate = AssemblyMateId(77);
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateAssemblyMate(AssemblyMate::new(
                mate,
                AssemblyMateEndpoint::resolved_planar_face(lower, face(ExactFaceRole::Top)),
                AssemblyMateEndpoint::resolved_planar_face(upper, face(ExactFaceRole::Bottom)),
                AssemblyMateKind::CoincidentPlanar {
                    offset_mm: 0.0,
                    reversed: true,
                },
            )),
        ]))
        .unwrap();
    let snapshot = document.current();
    let package = block_mesh(&mut supervisor, &snapshot, definition, extrusion);
    let packages = [&package, &package];
    let explicit = stack_setup(
        lower,
        upper,
        bottom_and_top(&package),
        ExactFeaJointKind::Bonded,
    );
    let mut by_mate = explicit.clone();
    by_mate.joints.clear();
    by_mate.mate_joints = vec![ExactFeaMateJoint {
        mate_id: mate,
        kind: ExactFeaJointKind::Bonded,
    }];
    // The mate's faces are the top of the lower block and the bottom of the upper.
    assert_eq!(
        assembly_bound_model(&snapshot, &packages, &by_mate)
            .unwrap()
            .model,
        assembly_bound_model(&snapshot, &packages, &explicit)
            .unwrap()
            .model
    );

    let refused = |setup: &ExactFeaAssemblySetup| {
        assembly_bound_model(&snapshot, &packages, setup).unwrap_err()
    };
    let mut unknown = by_mate.clone();
    unknown.mate_joints[0].mate_id = AssemblyMateId(78);
    assert_eq!(
        refused(&unknown),
        ExactFeaSetupError::InvalidMate {
            mate_id: AssemblyMateId(78)
        }
    );
    // The mate's upper end is not one of the studied parts.
    let mut elsewhere = by_mate.clone();
    elsewhere.parts[1].instance_path = InstancePath::root(beside);
    assert_eq!(
        refused(&elsewhere),
        ExactFeaSetupError::InvalidMate { mate_id: mate }
    );
    // A mate joins its faces once, explicitly or by the mate.
    let mut twice = by_mate.clone();
    twice.joints = explicit.joints.clone();
    assert_eq!(
        refused(&twice),
        ExactFeaSetupError::InvalidJoint { joint: 1 }
    );
}

/// The model with each tied node merged into the node it lies on, and where
/// every node of the tied model went.
fn merged(model: &FeaModel) -> (FeaModel, Vec<usize>) {
    use ketchup_analysis::fea::{FeaElementKind, FeaLoad};
    let mut onto = BTreeMap::new();
    for coupling in &model.couplings {
        let FeaCouplingKind::Tie { others, weights } = coupling.kind else {
            panic!("only ties expected");
        };
        let triangle = [coupling.nodes[0], others[0], others[1]];
        let (leader, _) = triangle
            .into_iter()
            .zip(weights)
            .find(|(_, weight)| *weight >= 1.0 - 1.0e-9)
            .expect("each tied node lies on a node");
        onto.insert(coupling.nodes[1], leader);
    }
    let mut kept = vec![usize::MAX; model.nodes.len()];
    let mut nodes = Vec::new();
    for (node, slot) in kept.iter_mut().enumerate() {
        if !onto.contains_key(&node) {
            *slot = nodes.len();
            nodes.push(model.nodes[node]);
        }
    }
    let at: Vec<usize> = (0..model.nodes.len())
        .map(|node| kept[*onto.get(&node).unwrap_or(&node)])
        .collect();
    let mut merged = model.clone();
    merged.nodes = nodes;
    merged.couplings.clear();
    for element in &mut merged.elements {
        let FeaElementKind::LinearTetrahedron4 { nodes, .. } = &mut element.kind else {
            panic!("only tetrahedra expected");
        };
        *nodes = nodes.map(|node| at[node]);
    }
    for constraint in &mut merged.constraints {
        constraint.node = at[constraint.node];
    }
    for load in &mut merged.loads {
        match load {
            FeaLoad::NodalForce { node, .. } => *node = at[*node],
            FeaLoad::SurfaceTraction { nodes, .. } => *nodes = nodes.map(|node| at[node]),
            _ => panic!("only forces and tractions expected"),
        }
    }
    (merged, at)
}

#[test]
fn stacked_occurrences_bonded_over_a_face_deform_as_one_body_and_apart_ones_are_refused() {
    // R1119og-FEA-800: a bonded joint ties the faces node by node where they
    // touch, so the stack is one body; a connector would hold the faces plane.
    let _turn = crate::integration_support::file_turn();
    let (mut document, definition, extrusion, lower) = simple_extrusion_document();
    let upper = OccurrenceId(905);
    let beside = OccurrenceId(906);
    let placed = |id, transform| CanonicalCommand::CreateOccurrence {
        id,
        definition_id: definition,
        name: "Bonded occurrence".into(),
        transform,
        parent: None,
        tags: Default::default(),
        visible: true,
    };
    document
        .apply_batch(&CommandBatch::new(vec![
            placed(
                upper,
                Transform::from_translation(0.0, 0.0, HEIGHT_MM).unwrap(),
            ),
            placed(beside, Transform::from_translation(20.0, 0.0, 0.0).unwrap()),
        ]))
        .unwrap();
    let snapshot = document.current();
    let mut supervisor =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = block_mesh(&mut supervisor, &snapshot, definition, extrusion);
    let packages = [&package, &package];
    let faces = bottom_and_top(&package);
    let setup = stack_setup(lower, upper, faces, ExactFeaJointKind::Bonded);

    let bonded = assembly_bound_model(&snapshot, &packages, &setup).unwrap();
    assert_eq!(bonded.joint_nodes, vec![None]);
    let mesh_nodes = package.mesh.vertices_mm.len();
    assert_eq!(bonded.model.nodes.len(), 2 * mesh_nodes);
    assert!(!bonded.model.couplings.is_empty());
    let solution = bonded.model.solve(FeaSolveSettings::default()).unwrap();
    balanced(&solution);
    let (one_body, at) = merged(&bonded.model);
    let one_solution = one_body.solve(FeaSolveSettings::default()).unwrap();
    let scale = one_solution.maximum_displacement_mm;
    assert!(scale > 0.0);
    for (node, merged_node) in at.iter().enumerate() {
        for axis in 0..3 {
            near(
                solution.displacements_mm[node][axis],
                one_solution.displacements_mm[*merged_node][axis],
                scale,
            );
        }
    }

    // The block beside does not touch the lower block's top anywhere.
    let mut apart = setup.clone();
    apart.parts[1].instance_path = InstancePath::root(beside);
    assert_eq!(
        assembly_bound_model(&snapshot, &packages, &apart).unwrap_err(),
        ExactFeaSetupError::InvalidJoint { joint: 0 }
    );
}
