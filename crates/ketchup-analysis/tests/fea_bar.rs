//! A bar of linear tetrahedra in tension, large enough to pass the old dense
//! solver's limits; shared by the integration tests and the memory test.

use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaElement, FeaElementKind, FeaLoad, FeaMaterial, FeaModel,
    FeaNode,
};
use std::collections::BTreeMap;

pub const YOUNG_MPA: f64 = 70_000.0;
pub const POISSON: f64 = 0.33;
pub const STRESS_MPA: f64 = 50.0;

/// A bar of `cells` × `side` × `side` cubes of `cell_mm`, each cut into six
/// tetrahedra around its main diagonal (every cube the same way, so shared
/// faces match).
pub fn tetrahedral_bar(cells: usize, side: usize, cell_mm: f64) -> FeaModel {
    let index = |x: usize, y: usize, z: usize| (x * (side + 1) + y) * (side + 1) + z;
    let mut nodes = Vec::new();
    for x in 0..=cells {
        for y in 0..=side {
            for z in 0..=side {
                nodes.push(FeaNode {
                    position_mm: [x, y, z]
                        .map(|value| cell_mm * f64::from(u32::try_from(value).unwrap())),
                });
            }
        }
    }
    let mut elements = Vec::new();
    for x in 0..cells {
        for y in 0..side {
            for z in 0..side {
                let corner = |bits: [usize; 3]| index(x + bits[0], y + bits[1], z + bits[2]);
                for axes in [
                    [0, 1, 2],
                    [0, 2, 1],
                    [1, 0, 2],
                    [1, 2, 0],
                    [2, 0, 1],
                    [2, 1, 0],
                ] {
                    let mut bits = [0; 3];
                    let mut path = [corner(bits); 4];
                    for (step, axis) in axes.into_iter().enumerate() {
                        bits[axis] = 1;
                        path[step + 1] = corner(bits);
                    }
                    elements.push(FeaElement {
                        id: u64::try_from(elements.len()).unwrap() + 1,
                        kind: FeaElementKind::LinearTetrahedron4 {
                            nodes: path,
                            material_id: 1,
                        },
                    });
                }
            }
        }
    }
    let length = cell_mm * f64::from(u32::try_from(cells).unwrap());
    // The fixed end only stops the bar moving along it; one corner stops it
    // sliding sideways and the next one stops it turning about its axis, so
    // the bar contracts freely.
    let constraints = (0..=side)
        .flat_map(|y| (0..=side).map(move |z| (y, z)))
        .map(|(y, z)| FeaConstraint {
            node: index(0, y, z),
            displacement_mm: [
                Some(0.0),
                (y == 0 && z == 0).then_some(0.0),
                (z == 0 && (y == 0 || y == side)).then_some(0.0),
            ],
            rotation_rad: [None; 3],
        })
        .collect();
    let mut face_count = BTreeMap::<[usize; 3], usize>::new();
    for element in &elements {
        let FeaElementKind::LinearTetrahedron4 { nodes, .. } = element.kind else {
            unreachable!()
        };
        for skipped in 0..4 {
            let mut face = [0; 3];
            let mut slot = 0;
            for (position, node) in nodes.into_iter().enumerate() {
                if position != skipped {
                    face[slot] = node;
                    slot += 1;
                }
            }
            face.sort_unstable();
            *face_count.entry(face).or_default() += 1;
        }
    }
    let loads = face_count
        .into_iter()
        .filter(|(face, count)| {
            *count == 1
                && face
                    .iter()
                    .all(|node| nodes[*node].position_mm[0] == length)
        })
        .map(|(face, _)| FeaLoad::SurfaceTraction {
            nodes: face,
            traction_n_per_mm2: [STRESS_MPA, 0.0, 0.0],
        })
        .collect();
    FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "tetrahedral-bar-in-tension".into(),
        nodes,
        materials: vec![FeaMaterial {
            id: 1,
            youngs_modulus_mpa: YOUNG_MPA,
            poisson_ratio: POISSON,
            yield_strength_mpa: Some(250.0),
            density_kg_per_m3: None,
        }],
        elements,
        constraints,
        loads,
        couplings: Vec::new(),
    }
}
