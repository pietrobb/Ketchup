//! R1121ac-FEA-1000: a shelf bonded between two sides meshed coarser than it.
//! The shelf is bonded in two joints and each side in one, and the coarse
//! sides followed the fine shelf: the shelf's end nodes between the coarse
//! nodes stayed free and the joint came out far too soft. The finer face
//! follows, so the assembly bends close to the one conforming body.

use super::*;
use ketchup_analysis::fea::FeaSolveSettings;

/// A box of hexahedra, each cut into six tetrahedra about its main diagonal,
/// with its nodes appended to `nodes`. Face ordinals 0..6 are -x, +x, -y, +y,
/// -z, +z; each face's triangles are the tetrahedra's faces on it.
fn grid_box(
    nodes: &mut Vec<FeaNode>,
    origin: [f64; 3],
    size: [f64; 3],
    cells: [usize; 3],
) -> (PlacedPart, Vec<[usize; 4]>) {
    let first = nodes.len();
    let at = |i: usize, j: usize, k: usize| first + i + (cells[0] + 1) * (j + (cells[1] + 1) * k);
    for k in 0..=cells[2] {
        for j in 0..=cells[1] {
            for i in 0..=cells[0] {
                let step = [i, j, k];
                nodes.push(FeaNode {
                    position_mm: std::array::from_fn(|axis| {
                        origin[axis] + size[axis] * step[axis] as f64 / cells[axis] as f64
                    }),
                });
            }
        }
    }
    let mut tetrahedra = Vec::new();
    for k in 0..cells[2] {
        for j in 0..cells[1] {
            for i in 0..cells[0] {
                let corner = |bits: usize| at(i + (bits & 1), j + (bits >> 1 & 1), k + (bits >> 2));
                for [p, q] in [[0, 1], [0, 2], [1, 0], [1, 2], [2, 0], [2, 1]] {
                    let second = 1 << p;
                    let mut tetrahedron = [
                        corner(0),
                        corner(second),
                        corner(second | 1 << q),
                        corner(7),
                    ];
                    let [a, b, c, d] = tetrahedron.map(|node| nodes[node].position_mm);
                    if dot(cross(sub(b, a), sub(c, a)), sub(d, a)) < 0.0 {
                        tetrahedron.swap(1, 2);
                    }
                    tetrahedra.push(tetrahedron);
                }
            }
        }
    }
    let mut faces = BTreeMap::new();
    for axis in 0..3 {
        let (u, v) = match axis {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        };
        for (side, level) in [(0, 0), (1, cells[axis])] {
            let mut triangles = Vec::new();
            for b in 0..cells[v] {
                for a in 0..cells[u] {
                    let node = |da: usize, db: usize| {
                        let mut step = [0; 3];
                        step[axis] = level;
                        step[u] = a + da;
                        step[v] = b + db;
                        at(step[0], step[1], step[2])
                    };
                    triangles.push([node(0, 0), node(1, 0), node(1, 1)]);
                    triangles.push([node(0, 0), node(1, 1), node(0, 1)]);
                }
            }
            faces.insert(u32::try_from(2 * axis + side).unwrap(), triangles);
        }
    }
    (PlacedPart { faces }, tetrahedra)
}

const SIDE_MM: [f64; 3] = [4.0, 12.0, 2.0];
const SHELF_MM: [f64; 3] = [12.0, 4.0, 2.0];
const SHELF_Y_MM: f64 = 4.0;

/// The bonded model of the two sides and the shelf, meshed `side_mm` and
/// `shelf_mm`, and the shelf's nodes.
fn shelf_between_sides(side_mm: f64, shelf_mm: f64) -> (FeaModel, std::ops::Range<usize>) {
    let cells = |size: [f64; 3], mm: f64| size.map(|length| (length / mm).round() as usize);
    let mut nodes = Vec::new();
    let mut elements = Vec::new();
    let mut placed = Vec::new();
    let mut shelf_nodes = 0..0;
    for (origin, size, mm, material_id) in [
        ([0.0, 0.0, 0.0], SIDE_MM, side_mm, 2),
        ([SIDE_MM[0], SHELF_Y_MM, 0.0], SHELF_MM, shelf_mm, 1),
        ([SIDE_MM[0] + SHELF_MM[0], 0.0, 0.0], SIDE_MM, side_mm, 2),
    ] {
        let first = nodes.len();
        let (part, tetrahedra) = grid_box(&mut nodes, origin, size, cells(size, mm));
        if placed.len() == 1 {
            shelf_nodes = first..nodes.len();
        }
        for nodes in tetrahedra {
            elements.push(FeaElement {
                id: elements.len() as u64 + 1,
                kind: FeaElementKind::LinearTetrahedron4 { nodes, material_id },
            });
        }
        placed.push(part);
    }
    let held: BTreeSet<usize> = [&placed[0], &placed[2]]
        .into_iter()
        .flat_map(|part| part.face_nodes(2))
        .collect();
    let face = |part, face_ordinal| ExactFeaJointFace { part, face_ordinal };
    // Each joint in its own face order.
    let bonded = |faces| ExactFeaJoint {
        faces,
        kind: ExactFeaJointKind::Bonded,
    };
    let (couplings, _) = joints(
        &mut nodes,
        &placed,
        &held,
        &[
            bonded([face(0, 1), face(1, 0)]),
            bonded([face(1, 1), face(2, 0)]),
        ],
    )
    .unwrap();
    let loads = placed[1].faces[&3]
        .iter()
        .map(|nodes| FeaLoad::SurfaceTraction {
            nodes: *nodes,
            traction_n_per_mm2: [0.0, -1.0, 0.0],
        })
        .collect();
    let model = FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.to_owned(),
        case_id: "shelf-between-sides".into(),
        nodes,
        // The sides a thousand times stiffer than the shelf, so how they are
        // meshed does not matter and the sag tells how well the joints hold.
        materials: [(1, 200_000.0), (2, 2.0e8)]
            .map(|(id, youngs_modulus_mpa)| FeaMaterial {
                id,
                youngs_modulus_mpa,
                poisson_ratio: 0.3,
                yield_strength_mpa: None,
                density_kg_per_m3: None,
            })
            .into(),
        elements,
        constraints: held.into_iter().map(FeaConstraint::held).collect(),
        loads,
        couplings,
    };
    (model, shelf_nodes)
}

/// The shelf's largest sag and how many of its nodes follow a tie.
fn sag(side_mm: f64, shelf_mm: f64) -> (f64, usize) {
    let (model, shelf) = shelf_between_sides(side_mm, shelf_mm);
    let solution = model.solve(FeaSolveSettings::default()).unwrap();
    let sag = solution.displacements_mm[shelf.clone()]
        .iter()
        .map(|moved| -moved[1])
        .fold(0.0, f64::max);
    let followers = model
        .couplings
        .iter()
        .filter(|coupling| shelf.contains(&coupling.nodes[1]))
        .count();
    (sag, followers)
}

#[test]
fn a_shelf_bonded_between_coarser_sides_bends_as_one_body() {
    for (shelf_mm, side_mm) in [(0.5, 2.0), (0.5, 1.0), (2.0 / 3.0, 2.0)] {
        let (one_body, _) = sag(shelf_mm, shelf_mm);
        let (bonded, followers) = sag(side_mm, shelf_mm);
        let [_, rows, columns] =
            [0, 1, 2].map(|axis| (SHELF_MM[axis] / shelf_mm).round() as usize + 1);
        let case = format!("shelf {shelf_mm:.2} mm, sides {side_mm} mm");
        // With the coarse sides following, the 0.5 / 2 mm shelf sagged 79 % more.
        let off = bonded / one_body - 1.0;
        assert!(off.abs() <= 0.05, "{case}: {:+.1} %", 100.0 * off);
        // Every node of both shelf ends follows a side.
        assert_eq!(followers, 2 * rows * columns, "{case}");
    }
}
