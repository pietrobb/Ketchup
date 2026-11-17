//! Two occurrences of one meshed block, the second stacked on the first,
//! joined top face to bottom face: rigidly the joined faces move as one
//! plane, and a spring joint adds exactly the force over its stiffness.

use super::*;
use ketchup_analysis::fea::{FeaCouplingKind, FeaSolution};
use ketchup_model::exact_product::ExactBRepGraphFaceEvidence;
use ketchup_scheduler::{
    ExactBRepVolumeMeshPackage, ExactFeaAssemblyPart, ExactFeaAssemblySetup, ExactFeaJoint,
    ExactFeaJointFace, assembly_bound_model,
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
    kind: FeaCouplingKind,
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
fn stacked_occurrences_joined_rigidly_or_by_a_spring_solve_as_one_assembly() {
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
        &stack_setup(lower, upper, faces, FeaCouplingKind::Rigid),
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
    let reference = rigid.joint_nodes[0];
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
            FeaCouplingKind::Spring {
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
    let setup = stack_setup(lower, upper, faces, FeaCouplingKind::Rigid);
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
