//! Natural frequencies and mode shapes against the hand formulas: a spring
//! and its mass, a bar's axial modes, a cantilever's bending modes, and a
//! cantilever of two parts joined rigidly.

use ketchup_analysis::fea::{
    FEA_MODEL_SCHEMA_V1, FeaConstraint, FeaCoupling, FeaCouplingKind, FeaElement, FeaElementKind,
    FeaError, FeaFrameSection, FeaMaterial, FeaModel, FeaNode, FeaSolveSettings,
};
use std::f64::consts::PI;

const YOUNG_MPA: f64 = 210_000.0;
const DENSITY_KG_PER_M3: f64 = 7_850.0;
/// The same density in tonnes per cubic millimetre, so that with N and mm a
/// frequency comes out per second.
const DENSITY_T_PER_MM3: f64 = DENSITY_KG_PER_M3 * 1.0e-12;
const LENGTH_MM: f64 = 1_000.0;

fn steel() -> FeaMaterial {
    FeaMaterial {
        id: 1,
        youngs_modulus_mpa: YOUNG_MPA,
        poisson_ratio: 0.3,
        yield_strength_mpa: Some(355.0),
        density_kg_per_m3: Some(DENSITY_KG_PER_M3),
    }
}

fn count(value: usize) -> f64 {
    f64::from(u32::try_from(value).unwrap())
}

fn model(
    nodes: Vec<FeaNode>,
    elements: Vec<FeaElement>,
    constraints: Vec<FeaConstraint>,
) -> FeaModel {
    FeaModel {
        schema: FEA_MODEL_SCHEMA_V1.into(),
        case_id: "modes".into(),
        nodes,
        materials: vec![steel()],
        elements,
        constraints,
        loads: Vec::new(),
        couplings: Vec::new(),
    }
}

fn along_x(segments: usize) -> Vec<FeaNode> {
    (0..=segments)
        .map(|index| FeaNode {
            position_mm: [LENGTH_MM * count(index) / count(segments), 0.0, 0.0],
        })
        .collect()
}

/// Truss elements along x, held at the origin and kept on the line.
fn axial_bar(segments: usize, area_mm2: f64) -> FeaModel {
    let elements = (0..segments)
        .map(|index| FeaElement {
            id: u64::try_from(index).unwrap() + 1,
            kind: FeaElementKind::LinearTruss2 {
                nodes: [index, index + 1],
                material_id: 1,
                area_mm2,
            },
        })
        .collect();
    let constraints = (0..=segments)
        .map(|node| FeaConstraint {
            node,
            displacement_mm: [(node == 0).then_some(0.0), Some(0.0), Some(0.0)],
            rotation_rad: [None; 3],
        })
        .collect();
    model(along_x(segments), elements, constraints)
}

/// A `width_z` × `height_y` rectangle.
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

fn frame(id: usize, nodes: [usize; 2]) -> FeaElement {
    FeaElement {
        id: u64::try_from(id).unwrap(),
        kind: FeaElementKind::Frame2 {
            nodes,
            material_id: 1,
            section: rectangle(20.0, 40.0),
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

fn cantilever(segments: usize) -> FeaModel {
    let elements = (0..segments)
        .map(|index| frame(index + 1, [index, index + 1]))
        .collect();
    model(along_x(segments), elements, vec![clamp(0)])
}

/// The cantilever's bending frequency for the root `beta_l` of
/// `cos βL · cosh βL = −1`, with `second_moment` the bending one.
fn bending_hz(beta_l: f64, second_moment: f64) -> f64 {
    let area = rectangle(20.0, 40.0).area_mm2;
    beta_l.powi(2) / (2.0 * PI)
        * (YOUNG_MPA * second_moment / (DENSITY_T_PER_MM3 * area * LENGTH_MM.powi(4))).sqrt()
}

fn relative(found: f64, expected: f64) -> f64 {
    (found - expected).abs() / expected.abs()
}

#[test]
fn one_bar_on_its_spring_vibrates_at_the_root_of_three_e_over_rho_l_squared() {
    // A single element: stiffness E·A/L, the free end's consistent mass ρ·A·L/3.
    let modes = axial_bar(1, 50.0)
        .modes(FeaSolveSettings::default(), 1)
        .unwrap();
    let expected = (3.0 * YOUNG_MPA / DENSITY_T_PER_MM3).sqrt() / (2.0 * PI * LENGTH_MM);
    assert!(relative(modes.modes[0].frequency_hz, expected) <= 1.0e-10);
    assert!(relative(modes.modes[0].angular_frequency_rad_s, 2.0 * PI * expected) <= 1.0e-12);
    assert!(
        relative(
            modes.total_mass_kg,
            DENSITY_KG_PER_M3 * 50.0 * LENGTH_MM * 1.0e-9
        ) <= 1.0e-12
    );
    assert_eq!(modes.modes[0].displacements_mm[1], [1.0, 0.0, 0.0]);
}

#[test]
fn a_bar_held_at_one_end_has_axial_modes_at_odd_quarter_waves() {
    let modes = axial_bar(200, 50.0)
        .modes(FeaSolveSettings::default(), 3)
        .unwrap();
    let wave_speed = (YOUNG_MPA / DENSITY_T_PER_MM3).sqrt();
    for (index, mode) in modes.modes.iter().enumerate() {
        let quarter_waves = 2.0 * count(index) + 1.0;
        let expected = quarter_waves * wave_speed / (4.0 * LENGTH_MM);
        assert!(
            relative(mode.frequency_hz, expected) <= 1.0e-4,
            "mode {index}: {} against {expected}",
            mode.frequency_hz
        );
        assert!(mode.relative_residual <= 1.0e-8);
    }
    assert!(
        modes
            .modes
            .windows(2)
            .all(|pair| pair[0].frequency_hz < pair[1].frequency_hz)
    );
}

#[test]
fn a_cantilever_bends_at_its_first_two_frequencies_about_each_axis() {
    let modes = cantilever(40)
        .modes(FeaSolveSettings::default(), 4)
        .unwrap();
    let section = rectangle(20.0, 40.0);
    let (weak, strong) = (section.second_moment_y_mm4, section.second_moment_z_mm4);
    // f = (βL)²/(2π)·√(EI/(ρAL⁴)); the strong axis is twice as stiff in frequency.
    let expected = [
        bending_hz(1.875_104_068_711_961, weak),
        bending_hz(1.875_104_068_711_961, strong),
        bending_hz(4.694_091_132_974_175, weak),
        bending_hz(4.694_091_132_974_175, strong),
    ];
    for ((mode, expected), index) in modes.modes.iter().zip(expected).zip(0..) {
        assert!(
            relative(mode.frequency_hz, expected) <= 1.0e-6,
            "mode {index}: {} against {expected}",
            mode.frequency_hz
        );
        assert!(mode.relative_residual <= 1.0e-9);
    }
    // The weak modes swing the tip along z, the strong ones along y; each
    // shape's largest move is one.
    let tip = |mode: usize| modes.modes[mode].displacements_mm[40];
    assert!((tip(0)[2].abs() - 1.0).abs() <= 1.0e-9 && tip(0)[1].abs() <= 1.0e-9);
    assert!((tip(1)[1].abs() - 1.0).abs() <= 1.0e-9 && tip(1)[2].abs() <= 1.0e-9);
    assert!(modes.modes[0].rotations_rad[40].is_some());
}

#[test]
fn a_cantilever_of_two_parts_joined_rigidly_vibrates_as_one() {
    let whole = cantilever(20)
        .modes(FeaSolveSettings::default(), 3)
        .unwrap();
    // The same cantilever with its tenth node doubled and the halves joined.
    let mut nodes = along_x(20);
    nodes.insert(11, nodes[10]);
    let elements = (0..20)
        .map(|index| {
            let start = if index < 10 { index } else { index + 1 };
            frame(index + 1, [start, start + 1])
        })
        .collect();
    let mut joined = model(nodes, elements, vec![clamp(0)]);
    joined.couplings = vec![FeaCoupling {
        nodes: [10, 11],
        kind: FeaCouplingKind::Rigid,
    }];
    let joined = joined.modes(FeaSolveSettings::default(), 3).unwrap();
    for (one, two) in whole.modes.iter().zip(&joined.modes) {
        assert!(relative(two.frequency_hz, one.frequency_hz) <= 1.0e-9);
    }
    assert!(relative(joined.total_mass_kg, whole.total_mass_kg) <= 1.0e-12);
}

#[test]
fn a_solid_block_weighs_its_density_times_its_volume() {
    // A 10 mm cube of six tetrahedra around its diagonal, held at its base.
    let corner = |bits: usize| FeaNode {
        position_mm: [
            10.0 * count(bits & 1),
            10.0 * count((bits >> 1) & 1),
            10.0 * count((bits >> 2) & 1),
        ],
    };
    let nodes = (0..8).map(corner).collect();
    let paths = [[1, 2], [1, 4], [2, 1], [2, 4], [4, 1], [4, 2]];
    let elements = paths
        .iter()
        .enumerate()
        .map(|(index, [first, second])| FeaElement {
            id: u64::try_from(index).unwrap() + 1,
            kind: FeaElementKind::LinearTetrahedron4 {
                nodes: [0, *first, first | second, 7],
                material_id: 1,
            },
        })
        .collect();
    let constraints = [0, 1, 2, 3].map(FeaConstraint::held).to_vec();
    let modes = model(nodes, elements, constraints)
        .modes(FeaSolveSettings::default(), 2)
        .unwrap();
    assert!(relative(modes.total_mass_kg, DENSITY_KG_PER_M3 * 1.0e-6) <= 1.0e-12);
    assert!(modes.modes.iter().all(|mode| mode.frequency_hz > 0.0));
    assert!(
        modes
            .modes
            .iter()
            .all(|mode| mode.relative_residual <= 1.0e-8)
    );
}

/// Every count from one to `MAX_FEA_MODES` misses no axial or bending mode
/// below the highest frequency it returns (review 2026-11-19 og:R1119og-FEA-801:
/// a subspace without a component of a mode never finds it, and the next one
/// up takes its place).
#[test]
fn no_count_skips_a_mode_below_the_highest_one_it_returns() {
    let section = rectangle(20.0, 40.0);
    let wave_speed = (YOUNG_MPA / DENSITY_T_PER_MM3).sqrt();
    let roots = [
        1.875_104_068_711_961,
        4.694_091_132_974_175,
        7.854_757_438_237_613,
        10.995_540_734_875_467,
        14.137_168_391_046_47,
    ];
    let mut expected = (1..=3)
        .map(|wave| (2.0 * count(wave) - 1.0) * wave_speed / (4.0 * LENGTH_MM))
        .collect::<Vec<_>>();
    for root in roots {
        expected.push(bending_hz(root, section.second_moment_y_mm4));
        expected.push(bending_hz(root, section.second_moment_z_mm4));
    }
    let model = cantilever(40);
    for wanted in [1, 2, 5, 10, 11, 12, 16, 20, 24, 30] {
        let modes = model
            .modes(FeaSolveSettings::default(), wanted)
            .unwrap_or_else(|error| panic!("{wanted} modes: {error:?}"));
        let highest = modes.modes.last().unwrap().frequency_hz;
        for frequency in expected
            .iter()
            .filter(|frequency| **frequency < 0.99 * highest)
        {
            assert!(
                modes
                    .modes
                    .iter()
                    .any(|mode| relative(mode.frequency_hz, *frequency) <= 0.01),
                "{wanted} modes up to {highest} Hz miss the one at {frequency} Hz"
            );
        }
    }
}

/// A 1000 × 50 × 100 mm cantilever of 1920 tetrahedra: the starting vectors of
/// its neighbouring unknowns all turn into the first bending shape, and the
/// iteration must still find every requested mode (og:R1119og-FEA-802).
#[test]
fn a_slender_solid_cantilever_finds_every_requested_mode() {
    let cells = [40, 2, 4];
    let size = [1_000.0, 50.0, 100.0];
    let index = |[x, y, z]: [usize; 3]| (z * (cells[1] + 1) + y) * (cells[0] + 1) + x;
    let mut nodes = Vec::new();
    for z in 0..=cells[2] {
        for y in 0..=cells[1] {
            for x in 0..=cells[0] {
                let at = [x, y, z];
                nodes.push(FeaNode {
                    position_mm: [0, 1, 2]
                        .map(|axis| size[axis] * count(at[axis]) / count(cells[axis])),
                });
            }
        }
    }
    // Six tetrahedra around each cell's diagonal; neighbouring cells share faces.
    let paths = [[1, 2], [1, 4], [2, 1], [2, 4], [4, 1], [4, 2]];
    let mut elements = Vec::new();
    for z in 0..cells[2] {
        for y in 0..cells[1] {
            for x in 0..cells[0] {
                let corner = |bits: usize| {
                    index([x + (bits & 1), y + ((bits >> 1) & 1), z + ((bits >> 2) & 1)])
                };
                for [first, second] in paths {
                    elements.push(FeaElement {
                        id: u64::try_from(elements.len()).unwrap() + 1,
                        kind: FeaElementKind::LinearTetrahedron4 {
                            nodes: [corner(0), corner(first), corner(first | second), corner(7)],
                            material_id: 1,
                        },
                    });
                }
            }
        }
    }
    assert_eq!(elements.len(), 1_920);
    let constraints = (0..nodes.len())
        .filter(|node| node % (cells[0] + 1) == 0)
        .map(FeaConstraint::held)
        .collect();
    let model = model(nodes, elements, constraints);
    for wanted in [4, 6, 8, 10, 14] {
        let modes = model
            .modes(FeaSolveSettings::default(), wanted)
            .unwrap_or_else(|error| panic!("{wanted} modes: {error:?}"));
        assert_eq!(modes.modes.len(), wanted);
        assert!(
            modes
                .modes
                .windows(2)
                .all(|pair| pair[0].frequency_hz <= pair[1].frequency_hz)
        );
        assert!(
            modes
                .modes
                .iter()
                .all(|mode| mode.relative_residual <= 1.0e-8)
        );
    }
}

#[test]
fn modes_without_a_density_or_with_a_bad_count_are_refused() {
    let mut no_density = cantilever(4);
    no_density.materials[0].density_kg_per_m3 = None;
    assert_eq!(
        no_density.modes(FeaSolveSettings::default(), 1),
        Err(FeaError::MissingDensity { material: 1 })
    );
    let mut bad_density = cantilever(4);
    bad_density.materials[0].density_kg_per_m3 = Some(-1.0);
    assert_eq!(
        bad_density.modes(FeaSolveSettings::default(), 1),
        Err(FeaError::InvalidMaterial)
    );
    assert_eq!(
        cantilever(4).modes(FeaSolveSettings::default(), 0),
        Err(FeaError::InvalidModeCount)
    );
    // One element held at one end has a single free move along it.
    assert_eq!(
        axial_bar(1, 50.0).modes(FeaSolveSettings::default(), 2),
        Err(FeaError::InvalidModeCount)
    );
    // A bar free to swing sideways has no stiffness against it.
    let mut loose = axial_bar(2, 50.0);
    loose.constraints.truncate(1);
    assert_eq!(
        loose.modes(FeaSolveSettings::default(), 1),
        Err(FeaError::SingularSystem)
    );
}
