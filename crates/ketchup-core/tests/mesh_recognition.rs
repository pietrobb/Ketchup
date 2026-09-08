use ketchup_core::document::{MESH_BODY_SCHEMA_V1, MeshAuthority, MeshBodySpec};
use ketchup_core::mesh_recognition::{
    MeshRecognition, MeshRecognitionCandidate, RecognizedMeshKind, recognize_mesh_body,
};

fn prism(profile: &[[f64; 2]], height: f64) -> MeshBodySpec {
    let side_count = profile.len();
    let mut vertices_mm = profile
        .iter()
        .map(|point| [point[0], point[1], 0.0])
        .collect::<Vec<_>>();
    vertices_mm.extend(profile.iter().map(|point| [point[0], point[1], height]));
    let mut triangles = Vec::new();
    for index in 1..side_count - 1 {
        triangles.push([0, (index + 1) as u32, index as u32]);
        triangles.push([
            side_count as u32,
            (side_count + index) as u32,
            (side_count + index + 1) as u32,
        ]);
    }
    for index in 0..side_count {
        let next = (index + 1) % side_count;
        triangles.push([index as u32, next as u32, (side_count + next) as u32]);
        triangles.push([
            index as u32,
            (side_count + next) as u32,
            (side_count + index) as u32,
        ]);
    }
    MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm,
        triangles,
        authority: MeshAuthority::Authored {
            provenance: "mesh-recognition-test".to_owned(),
        },
    }
}

fn regular_polygon(side_count: usize, radius: f64) -> Vec<[f64; 2]> {
    (0..side_count)
        .map(|index| {
            let angle = std::f64::consts::TAU * index as f64 / side_count as f64;
            [radius * angle.cos(), radius * angle.sin()]
        })
        .collect()
}

fn reindexed(mesh: &MeshBodySpec, order: &[usize]) -> MeshBodySpec {
    let mut inverse = vec![0_u32; order.len()];
    for (new, old) in order.iter().copied().enumerate() {
        inverse[old] = new as u32;
    }
    MeshBodySpec {
        schema: mesh.schema.clone(),
        vertices_mm: order.iter().map(|old| mesh.vertices_mm[*old]).collect(),
        triangles: mesh
            .triangles
            .iter()
            .map(|triangle| {
                [
                    inverse[triangle[0] as usize],
                    inverse[triangle[1] as usize],
                    inverse[triangle[2] as usize],
                ]
            })
            .collect(),
        authority: mesh.authority.clone(),
    }
}

#[test]
fn recognizes_oriented_box_with_measured_residuals() {
    let mesh = prism(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]], 8.0);

    let MeshRecognition::Candidate {
        candidate: MeshRecognitionCandidate::Box(candidate),
        residuals,
    } = recognize_mesh_body(&mesh, 1.0e-6)
    else {
        panic!("expected an unambiguous box")
    };

    assert_eq!(candidate.dimensions_mm.iter().product::<f64>(), 64.0);
    assert_eq!(candidate.center_mm, [0.0, 0.0, 4.0]);
    assert_eq!(residuals.maximum_mm(), 0.0);
}

#[test]
fn recognition_is_canonical_under_vertex_reindexing() {
    let angle: f64 = 0.37;
    let rotate = |point: [f64; 2]| {
        [
            point[0] * angle.cos() - point[1] * angle.sin(),
            point[0] * angle.sin() + point[1] * angle.cos(),
        ]
    };
    let mesh = prism(
        &[
            rotate([-2.0, -1.0]),
            rotate([2.0, -1.0]),
            rotate([2.0, 1.0]),
            rotate([-2.0, 1.0]),
        ],
        8.0,
    );
    let permuted = reindexed(&mesh, &[3, 7, 0, 5, 2, 6, 1, 4]);

    assert_eq!(
        recognize_mesh_body(&mesh, 1.0e-6),
        recognize_mesh_body(&permuted, 1.0e-6)
    );
}

#[test]
fn recognizes_sufficiently_fine_cylinder_and_reports_tessellation_error() {
    let mesh = prism(&regular_polygon(16, 10.0), 25.0);

    let MeshRecognition::Candidate {
        candidate: MeshRecognitionCandidate::Cylinder(candidate),
        residuals,
    } = recognize_mesh_body(&mesh, 0.2)
    else {
        panic!("expected an unambiguous cylinder")
    };

    assert_eq!(candidate.tessellated_side_count, 16);
    assert!((candidate.radius_mm - 10.0).abs() < 1.0e-12);
    assert_eq!(candidate.height_mm, 25.0);
    assert!(residuals.max_surface_distance_mm > 0.19);
    assert!(residuals.max_surface_distance_mm <= residuals.tolerance_mm);
}

#[test]
fn low_polygon_cylinder_is_ambiguous_with_general_extrusion() {
    let mesh = prism(&regular_polygon(6, 2.0), 3.0);

    let MeshRecognition::Ambiguous {
        candidates, reason, ..
    } = recognize_mesh_body(&mesh, 0.3)
    else {
        panic!("expected a deliberately ambiguous low-polygon cylinder")
    };

    assert_eq!(candidates.len(), 2);
    assert_eq!(candidates[0].kind(), RecognizedMeshKind::Cylinder);
    assert_eq!(candidates[1].kind(), RecognizedMeshKind::LinearExtrusion);
    assert!(reason.contains("low polygon count"));
}

#[test]
fn recognizes_general_linear_extrusion_and_exposes_profile() {
    let profile = [
        [-3.0, -2.0],
        [2.0, -2.0],
        [4.0, 0.5],
        [1.0, 3.0],
        [-2.0, 2.0],
    ];
    let mesh = prism(&profile, 7.0);

    let MeshRecognition::Candidate {
        candidate: MeshRecognitionCandidate::LinearExtrusion(candidate),
        residuals,
    } = recognize_mesh_body(&mesh, 1.0e-6)
    else {
        panic!("expected a general linear extrusion")
    };

    assert_eq!(candidate.profile_mm.len(), profile.len());
    assert_eq!(candidate.axis, [0.0, 0.0, 1.0]);
    assert_eq!(candidate.height_mm, 7.0);
    assert_eq!(residuals.maximum_mm(), 0.0);
}

#[test]
fn tolerance_boundary_is_reported_and_deterministic_without_mutation() {
    let profile = [
        [-3.0, -2.0],
        [2.0, -2.0],
        [4.0, 0.5],
        [1.0, 3.0],
        [-2.0, 2.0],
    ];
    let mut mesh = prism(&profile, 7.0);
    let first_high_vertex = profile.len();
    mesh.vertices_mm[first_high_vertex + 1][2] -= 0.02;
    let original = mesh.clone();

    let first = recognize_mesh_body(&mesh, 0.05);
    let second = recognize_mesh_body(&mesh, 0.05);
    assert_eq!(first, second);
    assert_eq!(mesh, original);
    let MeshRecognition::Candidate { residuals, .. } = first else {
        panic!("expected the perturbed extrusion inside tolerance")
    };
    assert!((residuals.max_layer_distance_mm - 0.02).abs() < 1.0e-12);

    assert!(matches!(
        recognize_mesh_body(&mesh, 0.01),
        MeshRecognition::NoMatch { .. }
    ));
}

#[test]
fn open_caps_and_twisted_side_band_have_no_match() {
    let profile = [[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]];
    let complete = prism(&profile, 5.0);
    let mut open = complete.clone();
    open.triangles.truncate(2 * (profile.len() - 2));
    assert!(matches!(
        recognize_mesh_body(&open, 1.0e-6),
        MeshRecognition::NoMatch { .. }
    ));

    let mut twisted = complete;
    let side_start = 2 * (profile.len() - 2);
    twisted.triangles.truncate(side_start);
    let count = profile.len();
    for index in 0..count {
        let next = (index + 1) % count;
        let high_a = count + (index + 1) % count;
        let high_b = count + (next + 1) % count;
        twisted
            .triangles
            .push([index as u32, next as u32, high_b as u32]);
        twisted
            .triangles
            .push([index as u32, high_b as u32, high_a as u32]);
    }
    assert!(matches!(
        recognize_mesh_body(&twisted, 1.0e-6),
        MeshRecognition::NoMatch { .. }
    ));
}

#[test]
fn disconnected_closed_shell_is_not_silently_dropped() {
    let mut mesh = prism(&[[-2.0, -1.0], [2.0, -1.0], [2.0, 1.0], [-2.0, 1.0]], 5.0);
    let first = mesh.vertices_mm.len() as u32;
    mesh.vertices_mm.extend([
        [10.0, 0.0, 0.0],
        [10.01, 0.0, 0.0],
        [10.0, 0.01, 0.0],
        [10.0, 0.0, 0.01],
    ]);
    mesh.triangles.extend([
        [first, first + 2, first + 1],
        [first, first + 1, first + 3],
        [first + 1, first + 2, first + 3],
        [first + 2, first, first + 3],
    ]);

    assert!(matches!(
        recognize_mesh_body(&mesh, 0.02),
        MeshRecognition::NoMatch { .. }
    ));
}

#[test]
fn non_prismatic_closed_mesh_has_no_match() {
    let mesh = MeshBodySpec {
        schema: MESH_BODY_SCHEMA_V1.to_owned(),
        vertices_mm: vec![
            [0.0, 0.0, 0.0],
            [2.0, 0.0, 0.0],
            [0.0, 2.0, 0.0],
            [0.5, 0.5, 3.0],
        ],
        triangles: vec![[0, 2, 1], [0, 1, 3], [1, 2, 3], [2, 0, 3]],
        authority: MeshAuthority::Authored {
            provenance: "mesh-recognition-test".to_owned(),
        },
    };

    assert!(matches!(
        recognize_mesh_body(&mesh, 1.0e-6),
        MeshRecognition::NoMatch { .. }
    ));
}
