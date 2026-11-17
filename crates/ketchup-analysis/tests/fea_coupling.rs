//! Joints between the nodes of different parts: a rigid joint makes two parts
//! act as one, a spring joint adds its compliance in series, and joints that
//! cannot be built are refused with the reason.

use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaCoupling, FeaCouplingKind, FeaCouplingProblem,
    FeaElement, FeaElementKind, FeaError, FeaFrameSection, FeaLoad, FeaMaterial, FeaModel, FeaNode,
    FeaSolveSettings,
};
use std::f64::consts::PI;

const YOUNG_MPA: f64 = 210_000.0;
const LENGTH_MM: f64 = 1_200.0;
const JOINT_MM: f64 = 500.0;

fn round_bar(diameter: f64) -> FeaFrameSection {
    FeaFrameSection {
        area_mm2: PI * diameter.powi(2) / 4.0,
        second_moment_y_mm4: PI * diameter.powi(4) / 64.0,
        second_moment_z_mm4: PI * diameter.powi(4) / 64.0,
        torsion_constant_mm4: PI * diameter.powi(4) / 32.0,
        extreme_fibre_y_mm: diameter / 2.0,
        extreme_fibre_z_mm: diameter / 2.0,
        torsion_modulus_mm3: PI * diameter.powi(3) / 16.0,
    }
}

fn frame(id: u64, nodes: [usize; 2]) -> FeaElement {
    FeaElement {
        id,
        kind: FeaElementKind::Frame2 {
            nodes,
            material_id: 1,
            section: round_bar(30.0),
            orientation: [0.0, 1.0, 0.0],
        },
    }
}

fn clamp(node: usize) -> FeaConstraint {
    FeaConstraint {
        node,
        displacement_mm: [Some(0.0); 3],
        rotation_rad: [Some(0.0); 3],
    }
}

fn node(x: f64, z: f64) -> FeaNode {
    FeaNode {
        position_mm: [x, 0.0, z],
    }
}

/// Two parts along x, clamped at the origin: a bar to the joint (nodes 0–1)
/// and a bar from the joint to the tip (nodes 2–3), with nodes 1 and 2 at the
/// same place but not shared.
fn two_parts(joint: FeaCouplingKind) -> FeaModel {
    FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "two parts".into(),
        nodes: vec![
            node(0.0, 0.0),
            node(JOINT_MM, 0.0),
            node(JOINT_MM, 0.0),
            node(LENGTH_MM, 0.0),
        ],
        materials: vec![FeaMaterial {
            id: 1,
            youngs_modulus_mpa: YOUNG_MPA,
            poisson_ratio: 0.3,
            yield_strength_mpa: Some(355.0),
            density_kg_per_m3: None,
        }],
        elements: vec![frame(1, [0, 1]), frame(2, [2, 3])],
        constraints: vec![clamp(0)],
        loads: vec![FeaLoad::NodalForce {
            node: 3,
            force_n: [0.0, 0.0, -400.0],
        }],
        couplings: vec![FeaCoupling {
            nodes: [1, 2],
            kind: joint,
        }],
    }
}

fn close(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() <= 1.0e-8 * expected.abs().max(1.0e-12),
        "{found} against {expected}"
    );
}

fn bending_stiffness() -> f64 {
    YOUNG_MPA * round_bar(30.0).second_moment_y_mm4
}

#[test]
fn two_parts_joined_rigidly_bend_as_one_cantilever() {
    let solution = two_parts(FeaCouplingKind::Rigid)
        .solve(FeaSolveSettings::default())
        .unwrap();
    let force = -400.0;
    close(
        solution.displacements_mm[3][2],
        force * LENGTH_MM.powi(3) / (3.0 * bending_stiffness()),
    );
    // The second part's joint node moves and turns with the first part's.
    assert_eq!(solution.displacements_mm[1], solution.displacements_mm[2]);
    assert_eq!(solution.rotations_rad[1], solution.rotations_rad[2]);
    close(solution.reaction_forces_n[0][2], -force);
    close(
        solution.reaction_moments_n_mm[0].unwrap()[1],
        force * LENGTH_MM,
    );
    assert!(solution.maximum_free_dof_residual_n <= 1.0e-6);
    close(
        solution.total_strain_energy_n_mm,
        0.5 * force * solution.displacements_mm[3][2],
    );
}

#[test]
fn a_spring_joint_adds_its_compliance_in_series() {
    let translation = 2_000.0;
    let rotation = 4.0e7;
    let solution = two_parts(FeaCouplingKind::Spring {
        translation_n_per_mm: [translation; 3],
        rotation_n_mm_per_rad: [rotation; 3],
    })
    .solve(FeaSolveSettings::default())
    .unwrap();
    let force = -400.0;
    let outer = LENGTH_MM - JOINT_MM;
    // The joint carries the force and its moment about the joint; the springs
    // give way by force / stiffness and the turn swings the outer part.
    let expected = force * LENGTH_MM.powi(3) / (3.0 * bending_stiffness())
        + force / translation
        + force * outer * outer / rotation;
    close(solution.displacements_mm[3][2], expected);
    close(
        solution.displacements_mm[2][2] - solution.displacements_mm[1][2],
        force / translation,
    );
    // The energy of the springs is part of the strain energy.
    close(
        solution.total_strain_energy_n_mm,
        0.5 * force * solution.displacements_mm[3][2],
    );
}

#[test]
fn a_node_held_off_a_part_by_a_rigid_joint_carries_the_lever_arm() {
    // A bar along x clamped at the origin; node 2 sits `offset` above its
    // free end and follows it rigidly, through node 3 halfway there, so one
    // joint follows another and node 3 turns only because of the joints.
    let offset = 80.0;
    let mut model = two_parts(FeaCouplingKind::Rigid);
    model.nodes = vec![
        node(0.0, 0.0),
        node(LENGTH_MM, 0.0),
        node(LENGTH_MM, offset),
        node(LENGTH_MM, offset / 2.0),
    ];
    model.elements = vec![frame(1, [0, 1])];
    model.couplings = vec![
        FeaCoupling {
            nodes: [3, 2],
            kind: FeaCouplingKind::Rigid,
        },
        FeaCoupling {
            nodes: [1, 3],
            kind: FeaCouplingKind::Rigid,
        },
    ];
    let force = 900.0;
    model.loads = vec![FeaLoad::NodalForce {
        node: 2,
        force_n: [force, 0.0, 0.0],
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    // At the bar's end the force acts with the moment offset·force about y.
    let moment = offset * force;
    let axial = YOUNG_MPA * round_bar(30.0).area_mm2;
    let end_turn = moment * LENGTH_MM / bending_stiffness();
    close(solution.rotations_rad[1].unwrap()[1], end_turn);
    close(
        solution.displacements_mm[1][2],
        -moment * LENGTH_MM.powi(2) / (2.0 * bending_stiffness()),
    );
    close(
        solution.displacements_mm[2][0],
        force * LENGTH_MM / axial + end_turn * offset,
    );
    close(solution.rotations_rad[3].unwrap()[1], end_turn);
    assert_eq!(solution.rotations_rad[2], None);
    close(solution.reaction_moments_n_mm[0].unwrap()[1], -moment);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() <= 1.0e-6)
    );
    close(
        solution.total_strain_energy_n_mm,
        0.5 * force * solution.displacements_mm[2][0],
    );
}

#[test]
fn a_coupling_changes_the_model_digest() {
    let rigid = two_parts(FeaCouplingKind::Rigid);
    let mut spring = rigid.clone();
    spring.couplings[0].kind = FeaCouplingKind::Spring {
        translation_n_per_mm: [1.0; 3],
        rotation_n_mm_per_rad: [0.0; 3],
    };
    let mut none = rigid.clone();
    none.couplings.clear();
    assert_ne!(rigid.digest(), spring.digest());
    assert_ne!(rigid.digest(), none.digest());
}

#[test]
fn joints_that_cannot_be_built_are_refused_with_the_reason() {
    let refused = |model: FeaModel, coupling: usize, problem: FeaCouplingProblem| {
        assert_eq!(
            model.solve(FeaSolveSettings::default()),
            Err(FeaError::InvalidCoupling { coupling, problem })
        );
    };
    let rigid = || two_parts(FeaCouplingKind::Rigid);
    let spring = |translation: [f64; 3], rotation: [f64; 3]| {
        two_parts(FeaCouplingKind::Spring {
            translation_n_per_mm: translation,
            rotation_n_mm_per_rad: rotation,
        })
    };

    let mut same = rigid();
    same.couplings[0].nodes = [1, 1];
    refused(same, 0, FeaCouplingProblem::InvalidNode);
    let mut outside = rigid();
    outside.couplings[0].nodes = [1, 9];
    refused(outside, 0, FeaCouplingProblem::InvalidNode);

    refused(
        spring([0.0; 3], [0.0; 3]),
        0,
        FeaCouplingProblem::InvalidStiffness,
    );
    refused(
        spring([1.0, -1.0, 1.0], [0.0; 3]),
        0,
        FeaCouplingProblem::InvalidStiffness,
    );
    refused(
        spring([f64::NAN, 1.0, 1.0], [0.0; 3]),
        0,
        FeaCouplingProblem::InvalidStiffness,
    );
    let mut apart = spring([1.0; 3], [0.0; 3]);
    apart.couplings[0].nodes = [1, 3];
    refused(apart, 0, FeaCouplingProblem::SpringNodesApart);

    // Two nodes no frame element ends at cannot carry a turn spring.
    let mut loose = spring([1.0; 3], [0.0; 3]);
    loose
        .nodes
        .extend([node(LENGTH_MM, 0.0), node(LENGTH_MM, 0.0)]);
    loose.couplings.push(FeaCoupling {
        nodes: [4, 5],
        kind: FeaCouplingKind::Spring {
            translation_n_per_mm: [1.0; 3],
            rotation_n_mm_per_rad: [1.0, 0.0, 0.0],
        },
    });
    refused(loose, 1, FeaCouplingProblem::TurnSpringNeedsTurningNodes);

    let mut twice = rigid();
    twice.couplings.push(FeaCoupling {
        nodes: [3, 2],
        kind: FeaCouplingKind::Rigid,
    });
    refused(twice, 1, FeaCouplingProblem::FollowsTwice);

    let mut circle = rigid();
    circle.couplings.push(FeaCoupling {
        nodes: [2, 1],
        kind: FeaCouplingKind::Rigid,
    });
    refused(circle, 0, FeaCouplingProblem::Cycle);

    let mut held = rigid();
    held.constraints.push(clamp(2));
    refused(held, 0, FeaCouplingProblem::ConstrainedFollower);
}
