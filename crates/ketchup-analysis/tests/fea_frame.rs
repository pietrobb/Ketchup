//! A 3D Euler–Bernoulli beam frame element against the hand formulas of
//! beam theory: its nodal values are exact for end loads and, with consistent
//! nodal loads, for a uniform load too.

use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaElement, FeaElementKind, FeaError, FeaFrameSection,
    FeaLoad, FeaMaterial, FeaModel, FeaNode, FeaSolveSettings,
};
use ketchup_geometry::linalg::{cross, dot, normalize, scale};
use std::f64::consts::PI;

const YOUNG_MPA: f64 = 210_000.0;
const POISSON: f64 = 0.3;

fn steel() -> FeaMaterial {
    FeaMaterial {
        id: 1,
        youngs_modulus_mpa: YOUNG_MPA,
        poisson_ratio: POISSON,
        yield_strength_mpa: Some(355.0),
        density_kg_per_m3: None,
    }
}

/// A `width_z` × `height_y` rectangle; its torsion constant is the thin-wall
/// free estimate `β·h·b³` with β = 1/3 · (1 − 0.63·b/h), only needed positive here.
fn rectangle(width_z: f64, height_y: f64) -> FeaFrameSection {
    let torsion = (1.0 - 0.63 * width_z / height_y) * height_y * width_z.powi(3) / 3.0;
    FeaFrameSection {
        area_mm2: width_z * height_y,
        second_moment_y_mm4: height_y * width_z.powi(3) / 12.0,
        second_moment_z_mm4: width_z * height_y.powi(3) / 12.0,
        torsion_constant_mm4: torsion,
        extreme_fibre_y_mm: height_y / 2.0,
        extreme_fibre_z_mm: width_z / 2.0,
        torsion_modulus_mm3: torsion / width_z,
    }
}

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

fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap())
}

/// `segments` beams from the origin along `direction` (unit) over `length`.
fn beam_line(
    segments: usize,
    length: f64,
    direction: [f64; 3],
    orientation: [f64; 3],
    section: FeaFrameSection,
) -> FeaModel {
    let nodes = (0..=segments)
        .map(|index| FeaNode {
            position_mm: scale(direction, length * count(index) / count(segments)),
        })
        .collect();
    let elements = (0..segments)
        .map(|index| FeaElement {
            id: u64::try_from(index).unwrap() + 1,
            kind: FeaElementKind::Frame2 {
                nodes: [index, index + 1],
                material_id: 1,
                section,
                orientation,
            },
        })
        .collect();
    FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "beam".into(),
        nodes,
        materials: vec![steel()],
        elements,
        constraints: vec![clamp(0)],
        loads: Vec::new(),
        couplings: Vec::new(),
    }
}

fn clamp(node: usize) -> FeaConstraint {
    FeaConstraint {
        node,
        displacement_mm: [Some(0.0); 3],
        rotation_rad: [Some(0.0); 3],
    }
}

fn close(found: f64, expected: f64) {
    assert!(
        (found - expected).abs() <= 1.0e-9 * expected.abs().max(1.0e-12),
        "{found} against {expected}"
    );
}

#[test]
fn a_cantilever_tip_load_deflects_p_l3_over_3_e_i_about_each_axis() {
    let length = 1_000.0;
    let section = rectangle(20.0, 40.0);
    let mut model = beam_line(8, length, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    let force = [0.0, -500.0, 300.0];
    model.loads = vec![FeaLoad::NodalForce {
        node: 8,
        force_n: force,
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let iz = section.second_moment_z_mm4;
    let iy = section.second_moment_y_mm4;
    let tip = solution.displacements_mm[8];
    close(tip[0], 0.0);
    close(tip[1], force[1] * length.powi(3) / (3.0 * YOUNG_MPA * iz));
    close(tip[2], force[2] * length.powi(3) / (3.0 * YOUNG_MPA * iy));
    let turn = solution.rotations_rad[8].unwrap();
    close(turn[2], force[1] * length.powi(2) / (2.0 * YOUNG_MPA * iz));
    close(turn[1], -force[2] * length.powi(2) / (2.0 * YOUNG_MPA * iy));

    // The clamp holds the force and its moment r × F about the clamp.
    close(solution.reaction_forces_n[0][1], -force[1]);
    close(solution.reaction_forces_n[0][2], -force[2]);
    let moment = solution.reaction_moments_n_mm[0].unwrap();
    close(moment[1], force[2] * length);
    close(moment[2], -force[1] * length);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() <= 1.0e-6)
    );

    // The work of the tip force is the strain energy.
    close(solution.total_strain_energy_n_mm, 0.5 * dot(force, tip));
    // Both bending moments peak at the clamp, on the same corner fibre.
    let stress = -force[1] * length * section.extreme_fibre_y_mm / iz
        + force[2] * length * section.extreme_fibre_z_mm / iy;
    close(solution.maximum_von_mises_stress_mpa, stress);
    assert!(solution.rotations_rad.iter().all(Option::is_some));
}

#[test]
fn a_cantilever_in_any_direction_bends_the_same() {
    let length = 750.0;
    let section = rectangle(15.0, 30.0);
    let direction = [1.0 / 3.0, 2.0 / 3.0, 2.0 / 3.0];
    let orientation = [0.0, 0.0, 1.0];
    let local_z = normalize(cross(direction, orientation)).unwrap();
    let local_y = cross(local_z, direction);
    let mut model = beam_line(5, length, direction, orientation, section);
    let force = 120.0;
    model.loads = vec![FeaLoad::NodalForce {
        node: 5,
        force_n: scale(local_y, force),
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let tip = solution.displacements_mm[5];
    close(
        dot(tip, local_y),
        force * length.powi(3) / (3.0 * YOUNG_MPA * section.second_moment_z_mm4),
    );
    assert!(dot(tip, local_z).abs() <= 1.0e-12);
    assert!(dot(tip, direction).abs() <= 1.0e-12);
}

#[test]
fn a_simply_supported_beam_under_uniform_load_sags_5_q_l4_over_384_e_i() {
    let length = 2_000.0;
    let section = rectangle(30.0, 60.0);
    let mut model = beam_line(4, length, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    // A pin at the start (it also stops the beam turning about its axis) and
    // a roller at the end.
    model.constraints = vec![
        FeaConstraint {
            node: 0,
            displacement_mm: [Some(0.0); 3],
            rotation_rad: [Some(0.0), None, None],
        },
        FeaConstraint {
            node: 4,
            displacement_mm: [None, Some(0.0), Some(0.0)],
            rotation_rad: [None; 3],
        },
    ];
    let load = 2.0;
    model.loads = (1..=4)
        .map(|element| FeaLoad::FrameDistributed {
            element,
            load_n_per_mm: [0.0, -load, 0.0],
        })
        .collect();
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let iz = section.second_moment_z_mm4;
    close(
        solution.displacements_mm[2][1],
        -5.0 * load * length.powi(4) / (384.0 * YOUNG_MPA * iz),
    );
    close(
        solution.rotations_rad[0].unwrap()[2],
        -load * length.powi(3) / (24.0 * YOUNG_MPA * iz),
    );
    close(solution.reaction_forces_n[0][1], load * length / 2.0);
    close(solution.reaction_forces_n[4][1], load * length / 2.0);
    close(
        solution.maximum_von_mises_stress_mpa,
        load * length.powi(2) / 8.0 * section.extreme_fibre_y_mm / iz,
    );
}

#[test]
fn a_single_element_finds_the_peak_moment_inside_it() {
    let length = 1_200.0;
    let section = rectangle(20.0, 50.0);
    let mut model = beam_line(1, length, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    model.constraints = vec![
        FeaConstraint {
            node: 0,
            displacement_mm: [Some(0.0); 3],
            rotation_rad: [Some(0.0), None, None],
        },
        FeaConstraint {
            node: 1,
            displacement_mm: [None, Some(0.0), Some(0.0)],
            rotation_rad: [None; 3],
        },
    ];
    let load = 1.5;
    model.loads = vec![FeaLoad::FrameDistributed {
        element: 1,
        load_n_per_mm: [0.0, -load, 0.0],
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();
    close(
        solution.maximum_von_mises_stress_mpa,
        load * length.powi(2) / 8.0 * section.extreme_fibre_y_mm / section.second_moment_z_mm4,
    );
}

/// Review 2026-11-19 og FEA-804: one simply supported element under a uniform load
/// moved nowhere at its nodes, so the largest move came out 0 instead of the
/// 5·q·L⁴/(384·E·I) of its middle. A load across both section axes checks the turn
/// about each.
#[test]
fn a_single_element_finds_the_largest_move_inside_it() {
    let length = 1_200.0;
    let section = rectangle(20.0, 50.0);
    let mut model = beam_line(1, length, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    model.constraints = vec![
        FeaConstraint {
            node: 0,
            displacement_mm: [Some(0.0); 3],
            rotation_rad: [Some(0.0), None, None],
        },
        FeaConstraint {
            node: 1,
            displacement_mm: [None, Some(0.0), Some(0.0)],
            rotation_rad: [None; 3],
        },
    ];
    let load = [0.0, -1.5, 0.4];
    model.loads = vec![FeaLoad::FrameDistributed {
        element: 1,
        load_n_per_mm: load,
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let sag = |load: f64, second_moment: f64| {
        5.0 * load * length.powi(4) / (384.0 * YOUNG_MPA * second_moment)
    };
    let sideways = sag(load[1], section.second_moment_z_mm4);
    let upwards = sag(load[2], section.second_moment_y_mm4);
    assert!(
        solution
            .displacements_mm
            .iter()
            .all(|moved| moved.iter().all(|value| value.abs() <= 1.0e-12))
    );
    close(solution.maximum_displacement_mm, sideways.hypot(upwards));
}

/// A cantilever under a uniform load moves most at its tip node, q·L⁴/(8·E·I), and
/// nothing inside an element moves further.
#[test]
fn a_cantilever_under_uniform_load_moves_most_at_its_tip() {
    let length = 900.0;
    let section = rectangle(20.0, 40.0);
    let mut model = beam_line(3, length, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    let load = 0.8;
    model.loads = (1..=3)
        .map(|element| FeaLoad::FrameDistributed {
            element,
            load_n_per_mm: [0.0, 0.0, -load],
        })
        .collect();
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let tip = load * length.powi(4) / (8.0 * YOUNG_MPA * section.second_moment_y_mm4);
    close(solution.displacements_mm[3][2], -tip);
    close(solution.maximum_displacement_mm, tip);
}

#[test]
fn a_twisted_shaft_turns_t_l_over_g_j() {
    let length = 500.0;
    let diameter = 20.0;
    let section = round_bar(diameter);
    let mut model = beam_line(5, length, [0.0, 0.0, 1.0], [1.0, 0.0, 0.0], section);
    let torque = 10_000.0;
    model.loads = vec![FeaLoad::NodalMoment {
        node: 5,
        moment_n_mm: [0.0, 0.0, torque],
    }];
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    let shear_modulus = YOUNG_MPA / (2.0 * (1.0 + POISSON));
    close(
        solution.rotations_rad[5].unwrap()[2],
        torque * length / (shear_modulus * section.torsion_constant_mm4),
    );
    close(solution.reaction_moments_n_mm[0].unwrap()[2], -torque);
    close(
        solution.maximum_von_mises_stress_mpa,
        3.0_f64.sqrt() * 16.0 * torque / (PI * diameter.powi(3)),
    );
}

#[test]
fn beam_inputs_that_cannot_be_solved_are_refused() {
    let section = rectangle(20.0, 40.0);
    let tip_load = FeaLoad::NodalForce {
        node: 2,
        force_n: [0.0, -10.0, 0.0],
    };
    let mut parallel = beam_line(2, 100.0, [1.0, 0.0, 0.0], [2.0, 0.0, 0.0], section);
    parallel.loads = vec![tip_load.clone()];
    assert_eq!(
        parallel.solve(FeaSolveSettings::default()),
        Err(FeaError::InvalidFrameOrientation { element: 1 })
    );

    let mut flat = beam_line(
        2,
        100.0,
        [1.0, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        FeaFrameSection {
            second_moment_z_mm4: 0.0,
            ..section
        },
    );
    flat.loads = vec![tip_load.clone()];
    assert_eq!(
        flat.solve(FeaSolveSettings::default()),
        Err(FeaError::InvalidFrameSection { element: 1 })
    );

    // Held only against moving, the clamp lets the beam swing about it.
    let mut pinned = beam_line(2, 100.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    pinned.constraints[0].rotation_rad = [None; 3];
    pinned.loads = vec![tip_load.clone()];
    assert_eq!(
        pinned.solve(FeaSolveSettings::default()),
        Err(FeaError::SingularSystem)
    );

    // A truss node has no rotation to hold or to load.
    let mut truss = beam_line(2, 100.0, [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], section);
    truss.nodes.push(FeaNode {
        position_mm: [100.0, 100.0, 0.0],
    });
    truss.elements.push(FeaElement {
        id: 9,
        kind: FeaElementKind::LinearTruss2 {
            nodes: [2, 3],
            material_id: 1,
            area_mm2: 50.0,
        },
    });
    truss.constraints.push(FeaConstraint {
        node: 3,
        displacement_mm: [Some(0.0); 3],
        rotation_rad: [None; 3],
    });
    truss.loads = vec![tip_load];
    let mut held_rotation = truss.clone();
    held_rotation.constraints[1].rotation_rad = [Some(0.0), None, None];
    assert_eq!(
        held_rotation.solve(FeaSolveSettings::default()),
        Err(FeaError::InvalidConstraint)
    );
    let mut moment = truss.clone();
    moment.loads.push(FeaLoad::NodalMoment {
        node: 3,
        moment_n_mm: [1.0, 0.0, 0.0],
    });
    assert_eq!(
        moment.solve(FeaSolveSettings::default()),
        Err(FeaError::InvalidLoad)
    );
    let mut spread = truss.clone();
    spread.loads.push(FeaLoad::FrameDistributed {
        element: 9,
        load_n_per_mm: [0.0, -1.0, 0.0],
    });
    assert_eq!(
        spread.solve(FeaSolveSettings::default()),
        Err(FeaError::InvalidLoad)
    );
    let solution = truss.solve(FeaSolveSettings::default()).unwrap();
    assert!(solution.rotations_rad[3].is_none());
    assert!(solution.rotations_rad[2].is_some());
}
