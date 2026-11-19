//! The two-node 3D Euler–Bernoulli frame element: axial stretch, torsion and bending
//! about both section axes, with six unknowns (three moves, three turns) at
//! each end. Shear deformation and warping are left out, as bending theory does.

use crate::fea::FeaFrameSection;
use ketchup_geometry::linalg::{cross, dot, length, normalize, scale, sub};
use ketchup_model::tolerance::ROUNDING;

/// Unknowns per frame element: [u, v, w, θx, θy, θz] at the start, then at the end, in
/// the element's own axes.
pub(crate) const FRAME_DOFS: usize = 12;

pub(crate) type FrameMatrix = [[f64; FRAME_DOFS]; FRAME_DOFS];

/// The element's own axes: `x` along it from the first node, `y` in the plane of
/// the orientation vector, `z = x × y`.
#[derive(Clone, Copy)]
pub(crate) struct ElementAxes {
    pub(crate) length_mm: f64,
    pub(crate) axes: [[f64; 3]; 3],
}

#[derive(Debug)]
pub(crate) enum FrameError {
    Degenerate,
    Orientation,
}

impl ElementAxes {
    pub(crate) fn new(
        start: [f64; 3],
        end: [f64; 3],
        orientation: [f64; 3],
    ) -> Result<Self, FrameError> {
        let delta = sub(end, start);
        let length_mm = length(delta);
        if length_mm <= ROUNDING {
            return Err(FrameError::Degenerate);
        }
        let along = scale(delta, 1.0 / length_mm);
        let orientation = orientation
            .into_iter()
            .all(f64::is_finite)
            .then(|| normalize(orientation))
            .flatten()
            .ok_or(FrameError::Orientation)?;
        let normal = cross(along, orientation);
        if length(normal) <= ROUNDING {
            return Err(FrameError::Orientation);
        }
        let z = normalize(normal).ok_or(FrameError::Orientation)?;
        Ok(Self {
            length_mm,
            axes: [along, cross(z, along), z],
        })
    }

    /// A vector in the element's axes, from the model's.
    pub(crate) fn local(&self, vector: [f64; 3]) -> [f64; 3] {
        self.axes.map(|axis| dot(axis, vector))
    }

    /// A vector in the model's axes, from the element's.
    pub(crate) fn global(&self, vector: [f64; 3]) -> [f64; 3] {
        let mut result = [0.0; 3];
        for (axis, component) in self.axes.iter().zip(vector) {
            for (value, direction) in result.iter_mut().zip(axis) {
                *value += direction * component;
            }
        }
        result
    }

    /// Every three-component block of a 12-vector turned into the element's axes.
    pub(crate) fn local12(&self, vector: [f64; FRAME_DOFS]) -> [f64; FRAME_DOFS] {
        self.map_blocks(vector, Self::local)
    }

    pub(crate) fn global12(&self, vector: [f64; FRAME_DOFS]) -> [f64; FRAME_DOFS] {
        self.map_blocks(vector, Self::global)
    }

    fn map_blocks(
        &self,
        vector: [f64; FRAME_DOFS],
        map: fn(&Self, [f64; 3]) -> [f64; 3],
    ) -> [f64; FRAME_DOFS] {
        let mut result = [0.0; FRAME_DOFS];
        for block in 0..4 {
            let start = block * 3;
            let turned = map(self, [vector[start], vector[start + 1], vector[start + 2]]);
            result[start..start + 3].copy_from_slice(&turned);
        }
        result
    }

    /// `Tᵀ·K·T`: the element's stiffness in the model's axes, `T` turning each
    /// block of three into the element's axes.
    pub(crate) fn global_stiffness(&self, local: &FrameMatrix) -> FrameMatrix {
        let mut turned_columns = [[0.0; FRAME_DOFS]; FRAME_DOFS];
        for column in 0..FRAME_DOFS {
            let mut unit = [0.0; FRAME_DOFS];
            unit[column] = 1.0;
            let local_column = multiply(local, self.local12(unit));
            turned_columns[column] = self.global12(local_column);
        }
        let mut result = [[0.0; FRAME_DOFS]; FRAME_DOFS];
        for (column, values) in turned_columns.iter().enumerate() {
            for (row, value) in values.iter().enumerate() {
                result[row][column] = *value;
            }
        }
        result
    }
}

/// The element's stiffness in its own axes.
pub(crate) fn local_stiffness(
    youngs_modulus_mpa: f64,
    shear_modulus_mpa: f64,
    section: &FeaFrameSection,
    length_mm: f64,
) -> FrameMatrix {
    fn put(matrix: &mut FrameMatrix, row: usize, column: usize, value: f64) {
        matrix[row][column] += value;
        if row != column {
            matrix[column][row] += value;
        }
    }
    let mut matrix = [[0.0; FRAME_DOFS]; FRAME_DOFS];
    let axial = youngs_modulus_mpa * section.area_mm2 / length_mm;
    put(&mut matrix, 0, 0, axial);
    put(&mut matrix, 6, 6, axial);
    put(&mut matrix, 0, 6, -axial);
    let torsion = shear_modulus_mpa * section.torsion_constant_mm4 / length_mm;
    put(&mut matrix, 3, 3, torsion);
    put(&mut matrix, 9, 9, torsion);
    put(&mut matrix, 3, 9, -torsion);
    // Bending in the element's x-y plane: v with θz, about the z axis.
    // Bending in its x-z plane: w with θy, the turn opposite to dw/dx.
    for (bending, translation, rotation, sign) in [
        (section.second_moment_z_mm4, 1, 5, 1.0),
        (section.second_moment_y_mm4, 2, 4, -1.0),
    ] {
        let rigidity = youngs_modulus_mpa * bending;
        let shear = 12.0 * rigidity / length_mm.powi(3);
        let coupling = sign * 6.0 * rigidity / length_mm.powi(2);
        let near = 4.0 * rigidity / length_mm;
        let far = 2.0 * rigidity / length_mm;
        let (v1, r1, v2, r2) = (translation, rotation, translation + 6, rotation + 6);
        put(&mut matrix, v1, v1, shear);
        put(&mut matrix, v1, r1, coupling);
        put(&mut matrix, v1, v2, -shear);
        put(&mut matrix, v1, r2, coupling);
        put(&mut matrix, r1, r1, near);
        put(&mut matrix, r1, v2, -coupling);
        put(&mut matrix, r1, r2, far);
        put(&mut matrix, v2, v2, shear);
        put(&mut matrix, v2, r2, -coupling);
        put(&mut matrix, r2, r2, near);
    }
    matrix
}

/// The element's consistent mass in its own axes (tonnes and tonne·mm², so
/// that with newtons and millimetres a frequency comes out per second): the
/// same shape functions as its stiffness, the turn about its axis carrying
/// the section's polar moment of area.
pub(crate) fn local_mass(
    density_t_per_mm3: f64,
    section: &FeaFrameSection,
    length_mm: f64,
) -> FrameMatrix {
    fn put(matrix: &mut FrameMatrix, row: usize, column: usize, value: f64) {
        matrix[row][column] += value;
        if row != column {
            matrix[column][row] += value;
        }
    }
    let mut matrix = [[0.0; FRAME_DOFS]; FRAME_DOFS];
    let mass = density_t_per_mm3 * section.area_mm2 * length_mm;
    let polar =
        density_t_per_mm3 * (section.second_moment_y_mm4 + section.second_moment_z_mm4) * length_mm;
    for (first, second, total) in [(0, 6, mass), (3, 9, polar)] {
        put(&mut matrix, first, first, total / 3.0);
        put(&mut matrix, second, second, total / 3.0);
        put(&mut matrix, first, second, total / 6.0);
    }
    let unit = mass / 420.0;
    let length = length_mm;
    for (translation, rotation, sign) in [(1, 5, 1.0), (2, 4, -1.0)] {
        let (v1, r1, v2, r2) = (translation, rotation, translation + 6, rotation + 6);
        put(&mut matrix, v1, v1, 156.0 * unit);
        put(&mut matrix, v1, r1, sign * 22.0 * length * unit);
        put(&mut matrix, v1, v2, 54.0 * unit);
        put(&mut matrix, v1, r2, -sign * 13.0 * length * unit);
        put(&mut matrix, r1, r1, 4.0 * length.powi(2) * unit);
        put(&mut matrix, r1, v2, sign * 13.0 * length * unit);
        put(&mut matrix, r1, r2, -3.0 * length.powi(2) * unit);
        put(&mut matrix, v2, v2, 156.0 * unit);
        put(&mut matrix, v2, r2, -sign * 22.0 * length * unit);
        put(&mut matrix, r2, r2, 4.0 * length.powi(2) * unit);
    }
    matrix
}

/// The nodal forces and moments, in the element's axes, that do the same work
/// as a uniform load `load` (N/mm, element axes) along the whole element.
pub(crate) fn consistent_load(load: [f64; 3], length_mm: f64) -> [f64; FRAME_DOFS] {
    let half = length_mm / 2.0;
    let end_moment = length_mm.powi(2) / 12.0;
    let mut nodal = [0.0; FRAME_DOFS];
    for axis in 0..3 {
        nodal[axis] = load[axis] * half;
        nodal[6 + axis] = load[axis] * half;
    }
    nodal[5] = load[1] * end_moment;
    nodal[11] = -load[1] * end_moment;
    nodal[4] = -load[2] * end_moment;
    nodal[10] = load[2] * end_moment;
    nodal
}

pub(crate) fn multiply(matrix: &FrameMatrix, vector: [f64; FRAME_DOFS]) -> [f64; FRAME_DOFS] {
    matrix.map(|row| {
        row.iter()
            .zip(vector)
            .map(|(entry, value)| entry * value)
            .sum()
    })
}

/// The largest von Mises stress anywhere along the element, from the forces the
/// nodes put on it (element axes) and the uniform load on it.
///
/// At each section the normal stress is taken at the corner fibre where the
/// axial and both bending stresses add, and the torsional shear at its
/// largest; for a section that has no such corner this bounds the stress from
/// above.
pub(crate) fn peak_von_mises(
    section: &FeaFrameSection,
    end_forces: [f64; FRAME_DOFS],
    load: [f64; 3],
    length_mm: f64,
) -> f64 {
    // Internal forces at x from the start, from the balance of [0, x].
    let axial = |x: f64| -end_forces[0] - load[0] * x;
    let bending_z = |x: f64| -end_forces[5] + end_forces[1] * x + load[1] * x * x / 2.0;
    let bending_y = |x: f64| -end_forces[4] - end_forces[2] * x - load[2] * x * x / 2.0;
    let torque = end_forces[3].abs();
    let mut stations = vec![0.0, length_mm];
    // A bending moment under a uniform load peaks where the shear vanishes.
    for (shear, spread) in [(end_forces[1], load[1]), (end_forces[2], load[2])] {
        if spread != 0.0 {
            let station = -shear / spread;
            if (0.0..=length_mm).contains(&station) {
                stations.push(station);
            }
        }
    }
    let shear_stress = torque / section.torsion_modulus_mm3;
    stations
        .into_iter()
        .map(|x| {
            let normal = axial(x).abs() / section.area_mm2
                + bending_y(x).abs() * section.extreme_fibre_z_mm / section.second_moment_y_mm4
                + bending_z(x).abs() * section.extreme_fibre_y_mm / section.second_moment_z_mm4;
            (normal.powi(2) + 3.0 * shear_stress.powi(2)).sqrt()
        })
        .fold(0.0, f64::max)
}

/// Stations the element is first sampled at for its largest move; even, so the middle,
/// where a symmetric span moves most, is one of them.
const MOVE_SAMPLES: u32 = 32;
/// Golden-section steps that close in on the largest move between two samples.
const MOVE_REFINEMENTS: u32 = 40;

/// The largest move anywhere along the element (og:R1119og-FEA-804), from its ends'
/// moves and turns `ends` (element axes) carried along by its own shape functions, and
/// the bending of a span held at both ends under its uniform `load`, which no node
/// shows: one element of a simply supported beam sags 5·q·L⁴/(384·E·I) in its middle
/// with both nodes still.
pub(crate) fn peak_displacement(
    youngs_modulus_mpa: f64,
    section: &FeaFrameSection,
    ends: [f64; FRAME_DOFS],
    load: [f64; 3],
    length_mm: f64,
) -> f64 {
    let moved = |x: f64| {
        let ratio = x / length_mm;
        let rest = length_mm - x;
        let start = 1.0 - ratio * ratio * (3.0 - 2.0 * ratio);
        let start_turn = x * (1.0 - ratio).powi(2);
        let end_turn = -x * ratio * (1.0 - ratio);
        let held = x * x * rest * rest / (24.0 * youngs_modulus_mpa);
        let axial = (1.0 - ratio) * ends[0]
            + ratio * ends[6]
            + load[0] * x * rest / (2.0 * youngs_modulus_mpa * section.area_mm2);
        // Bending about z: v with θz = dv/dx; about y: w with θy = −dw/dx.
        let sideways = start * ends[1]
            + start_turn * ends[5]
            + (1.0 - start) * ends[7]
            + end_turn * ends[11]
            + load[1] * held / section.second_moment_z_mm4;
        let upwards = start * ends[2] - start_turn * ends[4] + (1.0 - start) * ends[8]
            - end_turn * ends[10]
            + load[2] * held / section.second_moment_y_mm4;
        length([axial, sideways, upwards])
    };
    let step = length_mm / f64::from(MOVE_SAMPLES);
    let (best, _) = (0..=MOVE_SAMPLES)
        .map(|index| (index, moved(step * f64::from(index))))
        .fold((0, f64::NEG_INFINITY), |found, sample| {
            if sample.1 > found.1 { sample } else { found }
        });
    let golden = (5.0_f64.sqrt() - 1.0) / 2.0;
    let mut low = step * f64::from(best.saturating_sub(1));
    let mut high = step * f64::from((best + 1).min(MOVE_SAMPLES));
    let mut peak = moved(step * f64::from(best));
    for _ in 0..MOVE_REFINEMENTS {
        let left = high - golden * (high - low);
        let right = low + golden * (high - low);
        let (on_left, on_right) = (moved(left), moved(right));
        peak = peak.max(on_left).max(on_right);
        if on_left >= on_right {
            high = right;
        } else {
            low = left;
        }
    }
    peak
}

#[must_use]
pub(crate) fn valid_section(section: &FeaFrameSection) -> bool {
    [
        section.area_mm2,
        section.second_moment_y_mm4,
        section.second_moment_z_mm4,
        section.torsion_constant_mm4,
        section.extreme_fibre_y_mm,
        section.extreme_fibre_z_mm,
        section.torsion_modulus_mm3,
    ]
    .into_iter()
    .all(|value| value.is_finite() && value > 0.0)
}
