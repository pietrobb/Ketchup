use ketchup_core::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaConvergenceError, FeaElement, FeaElementKind, FeaError,
    FeaLoad, FeaMaterial, FeaModel, FeaNode, FeaSolveSettings, MAX_FEA_CONSTRAINTS,
    MAX_FEA_ELEMENTS, MAX_FEA_LOADS, MAX_FEA_MATERIALS, solve_convergence_study,
};

const YOUNG_MPA: f64 = 200_000.0;
const POISSON: f64 = 0.3;

fn near(actual: f64, expected: f64, tolerance: f64) {
    assert!(
        (actual - expected).abs() <= tolerance,
        "expected {expected} +/- {tolerance}, got {actual}"
    );
}

fn steel() -> FeaMaterial {
    FeaMaterial {
        id: 1,
        youngs_modulus_mpa: YOUNG_MPA,
        poisson_ratio: POISSON,
        yield_strength_mpa: Some(250.0),
    }
}

fn axial_bar(element_count: usize, force_n: f64) -> FeaModel {
    let length_mm = 1_000.0;
    let nodes = (0..=element_count)
        .map(|index| FeaNode {
            position_mm: [length_mm * index as f64 / element_count as f64, 0.0, 0.0],
        })
        .collect::<Vec<_>>();
    let elements = (0..element_count)
        .map(|index| FeaElement {
            id: index as u64 + 1,
            kind: FeaElementKind::LinearTruss2 {
                nodes: [index, index + 1],
                material_id: 1,
                area_mm2: 100.0,
            },
        })
        .collect();
    let constraints = (0..=element_count)
        .map(|node| FeaConstraint {
            node,
            displacement_mm: [(node == 0).then_some(0.0), Some(0.0), Some(0.0)],
        })
        .collect();
    FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "analytical-axial-bar".into(),
        nodes,
        materials: vec![steel()],
        elements,
        constraints,
        loads: vec![FeaLoad::NodalForce {
            node: element_count,
            force_n: [force_n, 0.0, 0.0],
        }],
    }
}

#[test]
fn axial_bar_matches_closed_form_and_reports_equilibrium_and_limits() {
    let model = axial_bar(4, 10_000.0);
    let solution = model.solve(FeaSolveSettings::default()).unwrap();

    // u = F L / E A = 0.5 mm; sigma = F / A = 100 MPa.
    near(solution.maximum_displacement_mm, 0.5, 1.0e-10);
    near(solution.maximum_von_mises_stress_mpa, 100.0, 1.0e-10);
    near(solution.total_strain_energy_n_mm, 2_500.0, 1.0e-8);
    near(solution.reaction_forces_n[0][0], -10_000.0, 1.0e-8);
    near(solution.force_balance_n[0], 0.0, 1.0e-8);
    near(solution.force_balance_n[1], 0.0, 1.0e-12);
    near(solution.force_balance_n[2], 0.0, 1.0e-12);
    assert!(solution.maximum_free_dof_residual_n < 1.0e-8);
    assert_eq!(solution.model_digest, model.digest());
    assert_eq!(solution.validity.maximum_yield_utilization, Some(0.4));
    assert!(solution.is_within_declared_limits());
}

#[test]
fn reversed_connectivity_duplicate_element_is_rejected() {
    let mut model = axial_bar(1, 10_000.0);
    model.elements.push(FeaElement {
        id: 2,
        kind: FeaElementKind::LinearTruss2 {
            nodes: [1, 0],
            material_id: 1,
            area_mm2: 100.0,
        },
    });

    assert_eq!(
        model.solve(FeaSolveSettings::default()),
        Err(FeaError::DuplicateElement)
    );
}

#[test]
fn refined_axial_bar_produces_explicit_convergence_evidence() {
    let levels = [
        axial_bar(1, 10_000.0),
        axial_bar(2, 10_000.0),
        axial_bar(8, 10_000.0),
    ];
    let evidence = solve_convergence_study(&levels, FeaSolveSettings::default()).unwrap();

    assert_eq!(evidence.levels.len(), 3);
    assert!(evidence.converged);
    assert!(evidence.final_displacement_relative_change < 1.0e-12);
    assert!(evidence.final_energy_relative_change < 1.0e-12);
    assert_eq!(evidence.levels[2].element_count, 8);
}

#[test]
fn tetrahedron_patch_matches_constant_uniaxial_stress() {
    let nodes = vec![
        FeaNode {
            position_mm: [0.0, 0.0, 0.0],
        },
        FeaNode {
            position_mm: [100.0, 0.0, 0.0],
        },
        FeaNode {
            position_mm: [0.0, 10.0, 0.0],
        },
        FeaNode {
            position_mm: [0.0, 0.0, 10.0],
        },
    ];
    let first = [100.0_f64, 0.0, 0.0];
    let second = [0.0_f64, 10.0, 0.0];
    let third = [0.0_f64, 0.0, 10.0];
    let edge_a = [
        second[0] - first[0],
        second[1] - first[1],
        second[2] - first[2],
    ];
    let edge_b = [
        third[0] - first[0],
        third[1] - first[1],
        third[2] - first[2],
    ];
    let cross = [
        edge_a[1] * edge_b[2] - edge_a[2] * edge_b[1],
        edge_a[2] * edge_b[0] - edge_a[0] * edge_b[2],
        edge_a[0] * edge_b[1] - edge_a[1] * edge_b[0],
    ];
    let face_normal_x = cross[0] / (cross[0].powi(2) + cross[1].powi(2) + cross[2].powi(2)).sqrt();
    let model = FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "tetrahedron-constant-strain-patch".into(),
        nodes,
        materials: vec![steel()],
        elements: vec![FeaElement {
            id: 1,
            kind: FeaElementKind::LinearTetrahedron4 {
                nodes: [0, 1, 2, 3],
                material_id: 1,
            },
        }],
        constraints: vec![
            FeaConstraint {
                node: 0,
                displacement_mm: [Some(0.0), Some(0.0), Some(0.0)],
            },
            FeaConstraint {
                node: 1,
                displacement_mm: [None, Some(0.0), Some(0.0)],
            },
            FeaConstraint {
                node: 2,
                displacement_mm: [Some(0.0), None, Some(0.0)],
            },
            FeaConstraint {
                node: 3,
                displacement_mm: [Some(0.0), Some(0.0), None],
            },
        ],
        loads: vec![FeaLoad::SurfaceTraction {
            nodes: [1, 2, 3],
            traction_n_per_mm2: [200.0 * face_normal_x, 0.0, 0.0],
        }],
    };

    let solution = model.solve(FeaSolveSettings::default()).unwrap();
    near(solution.displacements_mm[1][0], 0.1, 1.0e-10);
    near(solution.displacements_mm[2][1], -0.003, 1.0e-10);
    near(solution.displacements_mm[3][2], -0.003, 1.0e-10);
    near(solution.maximum_von_mises_stress_mpa, 200.0, 1.0e-8);
    assert!(solution.maximum_free_dof_residual_n < 1.0e-8);
    assert!(
        solution
            .force_balance_n
            .iter()
            .all(|value| value.abs() < 1.0e-8)
    );
}

#[test]
fn aggregate_model_collections_are_bounded_before_validation_work() {
    let baseline = axial_bar(1, 10_000.0);

    let mut materials = baseline.clone();
    materials.materials.resize(MAX_FEA_MATERIALS + 1, steel());
    assert_eq!(
        materials.solve(FeaSolveSettings::default()),
        Err(FeaError::ModelTooLarge)
    );

    let mut elements = baseline.clone();
    elements
        .elements
        .resize(MAX_FEA_ELEMENTS + 1, baseline.elements[0].clone());
    assert_eq!(
        elements.solve(FeaSolveSettings::default()),
        Err(FeaError::ModelTooLarge)
    );

    let mut constraints = baseline.clone();
    constraints
        .constraints
        .resize(MAX_FEA_CONSTRAINTS + 1, baseline.constraints[0].clone());
    assert_eq!(
        constraints.solve(FeaSolveSettings::default()),
        Err(FeaError::ModelTooLarge)
    );

    let mut loads = baseline;
    loads.loads.resize(
        MAX_FEA_LOADS + 1,
        FeaLoad::NodalForce {
            node: 1,
            force_n: [10_000.0, 0.0, 0.0],
        },
    );
    assert_eq!(
        loads.solve(FeaSolveSettings::default()),
        Err(FeaError::ModelTooLarge)
    );
}

#[test]
fn incomplete_singular_and_out_of_scope_models_fail_closed() {
    let mut singular = axial_bar(1, 10_000.0);
    singular.constraints = vec![FeaConstraint {
        node: 0,
        displacement_mm: [Some(0.0), None, None],
    }];
    assert_eq!(
        singular.solve(FeaSolveSettings::default()),
        Err(FeaError::SingularSystem)
    );

    let mut missing_material = axial_bar(1, 10_000.0);
    missing_material.materials.clear();
    assert_eq!(
        missing_material.solve(FeaSolveSettings::default()),
        Err(FeaError::MissingMaterial)
    );

    let mut unsupported = axial_bar(1, 10_000.0);
    unsupported.schema = "ketchup.fea-model.v2".into();
    assert_eq!(
        unsupported.solve(FeaSolveSettings::default()),
        Err(FeaError::UnsupportedSchema)
    );

    let excessive = axial_bar(1, 2_000_000.0)
        .solve(FeaSolveSettings::default())
        .unwrap();
    assert!(!excessive.validity.within_small_deformation_limit);
    assert_eq!(
        excessive.validity.within_linear_elastic_stress_limit,
        Some(false)
    );
    assert!(!excessive.is_within_declared_limits());
}

#[test]
fn nonzero_dirichlet_displacement_produces_analytical_stress_and_reactions() {
    let mut model = axial_bar(1, 0.0);
    model.constraints.push(FeaConstraint {
        node: 1,
        displacement_mm: [Some(1.0), None, None],
    });

    let solution = model.solve(FeaSolveSettings::default()).unwrap();
    near(solution.displacements_mm[1][0], 1.0, 1.0e-12);
    near(solution.maximum_von_mises_stress_mpa, 200.0, 1.0e-10);
    near(solution.total_strain_energy_n_mm, 10_000.0, 1.0e-8);
    near(solution.reaction_forces_n[0][0], -20_000.0, 1.0e-8);
    near(solution.reaction_forces_n[1][0], 20_000.0, 1.0e-8);
    near(solution.force_balance_n[0], 0.0, 1.0e-8);
}

#[test]
fn traction_on_an_internal_tetrahedron_face_is_rejected() {
    let model = FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "internal-face".into(),
        nodes: vec![
            FeaNode {
                position_mm: [0.0, 0.0, 0.0],
            },
            FeaNode {
                position_mm: [1.0, 0.0, 0.0],
            },
            FeaNode {
                position_mm: [0.0, 1.0, 0.0],
            },
            FeaNode {
                position_mm: [0.0, 0.0, 1.0],
            },
            FeaNode {
                position_mm: [0.0, 0.0, -1.0],
            },
        ],
        materials: vec![steel()],
        elements: vec![
            FeaElement {
                id: 1,
                kind: FeaElementKind::LinearTetrahedron4 {
                    nodes: [0, 1, 2, 3],
                    material_id: 1,
                },
            },
            FeaElement {
                id: 2,
                kind: FeaElementKind::LinearTetrahedron4 {
                    nodes: [0, 2, 1, 4],
                    material_id: 1,
                },
            },
        ],
        constraints: vec![FeaConstraint {
            node: 0,
            displacement_mm: [Some(0.0), Some(0.0), Some(0.0)],
        }],
        loads: vec![FeaLoad::SurfaceTraction {
            nodes: [0, 1, 2],
            traction_n_per_mm2: [1.0, 0.0, 0.0],
        }],
    };

    assert_eq!(
        model.solve(FeaSolveSettings::default()),
        Err(FeaError::NonBoundaryTraction)
    );
}

#[test]
fn convergence_refuses_unrelated_case_domain_or_load() {
    let first = axial_bar(1, 10_000.0);
    let mut wrong_case = axial_bar(2, 10_000.0);
    wrong_case.case_id = "different".into();
    assert_eq!(
        solve_convergence_study(&[first.clone(), wrong_case], FeaSolveSettings::default()),
        Err(FeaConvergenceError::CaseMismatch)
    );

    let mut wrong_domain = axial_bar(2, 10_000.0);
    wrong_domain.nodes.last_mut().unwrap().position_mm[0] = 999.0;
    assert_eq!(
        solve_convergence_study(&[first.clone(), wrong_domain], FeaSolveSettings::default()),
        Err(FeaConvergenceError::DomainMismatch)
    );

    let mut wrong_material = axial_bar(2, 10_000.0);
    wrong_material.materials[0].youngs_modulus_mpa *= 0.5;
    assert_eq!(
        solve_convergence_study(
            &[first.clone(), wrong_material],
            FeaSolveSettings::default()
        ),
        Err(FeaConvergenceError::MaterialMismatch)
    );

    assert_eq!(
        solve_convergence_study(&[first, axial_bar(2, 9_000.0)], FeaSolveSettings::default()),
        Err(FeaConvergenceError::LoadMismatch)
    );
}

#[test]
fn convergence_refuses_changed_element_material_assignments() {
    let mut coarse = axial_bar(1, 10_000.0);
    let mut fine = axial_bar(2, 10_000.0);
    let alternate = FeaMaterial {
        id: 2,
        youngs_modulus_mpa: 198_000.0,
        poisson_ratio: POISSON,
        yield_strength_mpa: Some(250.0),
    };
    coarse.materials.push(alternate.clone());
    fine.materials.push(alternate);
    for element in &mut fine.elements {
        let FeaElementKind::LinearTruss2 { material_id, .. } = &mut element.kind else {
            unreachable!("axial bar fixture only contains truss elements");
        };
        *material_id = 2;
    }

    assert_eq!(
        solve_convergence_study(&[coarse, fine], FeaSolveSettings::default()),
        Err(FeaConvergenceError::MaterialMismatch)
    );
}

#[test]
fn convergence_refuses_spatially_swapped_equal_volume_materials() {
    let mut coarse = axial_bar(2, 10_000.0);
    let mut fine = axial_bar(4, 10_000.0);
    let alternate = FeaMaterial {
        id: 2,
        youngs_modulus_mpa: 100_000.0,
        poisson_ratio: POISSON,
        yield_strength_mpa: Some(250.0),
    };
    coarse.materials.push(alternate.clone());
    fine.materials.push(alternate);

    let FeaElementKind::LinearTruss2 { material_id, .. } = &mut coarse.elements[1].kind else {
        unreachable!("axial bar fixture only contains truss elements");
    };
    *material_id = 2;
    for (index, element) in fine.elements.iter_mut().enumerate() {
        let FeaElementKind::LinearTruss2 { material_id, .. } = &mut element.kind else {
            unreachable!("axial bar fixture only contains truss elements");
        };
        *material_id = if index < 2 { 2 } else { 1 };
    }

    assert_eq!(
        solve_convergence_study(&[coarse, fine], FeaSolveSettings::default()),
        Err(FeaConvergenceError::MaterialMismatch)
    );
}

#[test]
fn convergence_refuses_redistributed_equal_resultant_load() {
    let coarse = axial_bar(1, 10_000.0);
    let mut fine = axial_bar(2, 10_000.0);
    fine.loads = vec![
        FeaLoad::NodalForce {
            node: 1,
            force_n: [100.0, 0.0, 0.0],
        },
        FeaLoad::NodalForce {
            node: 2,
            force_n: [9_900.0, 0.0, 0.0],
        },
    ];

    assert_eq!(
        solve_convergence_study(&[coarse, fine], FeaSolveSettings::default()),
        Err(FeaConvergenceError::LoadMismatch)
    );
}

#[test]
fn convergence_refuses_changed_boundary_condition() {
    let coarse = axial_bar(1, 10_000.0);
    let mut fine = axial_bar(2, 10_000.0);
    fine.constraints[0].displacement_mm[0] = Some(0.1);

    assert_eq!(
        solve_convergence_study(&[coarse, fine], FeaSolveSettings::default()),
        Err(FeaConvergenceError::ConstraintMismatch)
    );
}
