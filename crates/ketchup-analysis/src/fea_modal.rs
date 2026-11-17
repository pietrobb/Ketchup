//! Natural frequencies and mode shapes of a held model: the lowest pairs of
//! `K·φ = ω²·M·φ`, by subspace iteration on the sparse Cholesky factor of `K`
//! with a consistent mass matrix `M`.

use crate::fea::{DofLayout, FeaElementKind, FeaError, FeaModel, FeaSolveSettings};
use crate::fea_coupling::Joints;
use crate::fea_frame::local_mass;
use crate::sparse::{FactorError, SymmetricMatrix};
use ketchup_model::tolerance::{NEGLIGIBLE, ROUNDING};
use std::collections::BTreeSet;
use std::f64::consts::PI;

pub const FEA_MODES_SCHEMA_V1: &str = "ketchup.fea-modes.v1";
pub const FEA_MODAL_SOLVER_VERSION_V1: &str = "ketchup.subspace-iteration-modes.v1";
pub const MAX_FEA_MODES: usize = 64;
/// Rounds of subspace iteration before the lowest modes count as unsettled.
const MAX_ITERATIONS: usize = 1_000;
/// Sweeps of the small eigenproblem's Jacobi rotations.
const MAX_SWEEPS: usize = 100;
// not a tolerance: kilograms per cubic metre in tonnes per cubic millimetre.
const T_PER_MM3_PER_KG_PER_M3: f64 = 1.0e-12;
const KG_PER_T: f64 = 1_000.0;

/// Matrix entries as `(row, column, value)`.
type Entries = Vec<(usize, usize, f64)>;

#[derive(Clone, Debug, PartialEq)]
pub struct FeaMode {
    pub frequency_hz: f64,
    pub angular_frequency_rad_s: f64,
    /// The shape, scaled so that its largest move component is one (or, when
    /// nothing moves, its largest turn component).
    pub displacements_mm: Vec<[f64; 3]>,
    /// Turns of the shape; `None` at a node that does not turn.
    pub rotations_rad: Vec<Option<[f64; 3]>>,
    /// `|K·φ − ω²·M·φ| / |(|K| + ω²·|M|)·|φ||` over the free unknowns: the
    /// backward error, whose rounding floor does not grow with how far the
    /// stiffnesses of the model spread.
    pub relative_residual: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaModes {
    pub schema: &'static str,
    pub solver_version: &'static str,
    pub case_id: String,
    pub model_digest: String,
    pub total_mass_kg: f64,
    pub iterations: usize,
    /// From the lowest frequency up.
    pub modes: Vec<FeaMode>,
}

impl FeaModel {
    /// The `count` lowest natural frequencies with their shapes. Every
    /// material needs a density; loads are not needed and play no part.
    pub fn modes(&self, settings: FeaSolveSettings, count: usize) -> Result<FeaModes, FeaError> {
        self.validate_model(settings, false)?;
        if count == 0 || count > MAX_FEA_MODES {
            return Err(FeaError::InvalidModeCount);
        }
        let layout = self.dof_layout();
        let size = layout.count();
        let (mass_entries, total_mass_t) = self.mass_entries(&layout)?;
        let joints = Joints::new(self, &layout)?;
        let mut unused = vec![0.0; size];
        let stiffness_entries = joints.reduce(self.stiffness_entries(&layout)?, &mut unused);
        let mass_entries = joints.reduce(mass_entries, &mut unused);
        let mut fixed = self.held_dofs(&layout);
        fixed.extend(joints.follower_dofs());

        let free = |&(row, column, _): &(usize, usize, f64)| {
            !fixed.contains(&row) && !fixed.contains(&column)
        };
        let stiffness_diagonal = diagonal(size, &stiffness_entries);
        let mass_diagonal = diagonal(size, &mass_entries);
        let free_stiffness = SymmetricMatrix::from_entries(
            size,
            stiffness_entries.iter().copied().filter(free).collect(),
        );
        let magnitudes = |entries: &[(usize, usize, f64)]| {
            SymmetricMatrix::from_entries(
                size,
                entries
                    .iter()
                    .copied()
                    .filter(free)
                    .map(|(row, column, value)| (row, column, value.abs()))
                    .collect(),
            )
        };
        let rounding_scale = Magnitudes {
            stiffness: magnitudes(&stiffness_entries),
            mass: magnitudes(&mass_entries),
        };
        let mass =
            SymmetricMatrix::from_entries(size, mass_entries.into_iter().filter(free).collect());
        let mut stiffness = SymmetricMatrix::from_entries(size, stiffness_entries);
        let scale = stiffness.largest_magnitude().max(1.0);
        for &dof in &fixed {
            stiffness.prescribe(dof, 0.0, scale, &mut unused);
        }
        let threshold = stiffness.largest_magnitude().max(1.0) * settings.relative_pivot_tolerance;
        let factor = stiffness
            .factor(threshold, settings.maximum_factor_bytes)
            .map_err(|error| match error {
                FactorError::Singular => FeaError::SingularSystem,
                FactorError::TooLarge { needed_bytes } => FeaError::FactorTooLarge {
                    needed_bytes,
                    budget_bytes: settings.maximum_factor_bytes,
                },
            })?;

        // Unknowns that move a mass, the most flexible first.
        let mut massive = (0..size)
            .filter(|dof| !fixed.contains(dof) && mass_diagonal[*dof] > 0.0)
            .collect::<Vec<_>>();
        if count > massive.len() {
            return Err(FeaError::InvalidModeCount);
        }
        massive.sort_by(|left, right| {
            (stiffness_diagonal[*left] / mass_diagonal[*left])
                .total_cmp(&(stiffness_diagonal[*right] / mass_diagonal[*right]))
        });
        let width = (2 * count).max(count + 8).min(massive.len());
        let mut start = vec![0.0; size];
        for dof in &massive {
            start[*dof] = mass_diagonal[*dof];
        }
        let mut vectors = vec![start];
        vectors.extend(massive.iter().take(width - 1).map(|dof| {
            let mut unit = vec![0.0; size];
            unit[*dof] = 1.0;
            unit
        }));

        let mut previous: Option<Vec<f64>> = None;
        for iteration in 1..=MAX_ITERATIONS {
            let loads = vectors
                .iter()
                .map(|vector| mass.multiply(vector))
                .collect::<Vec<_>>();
            let solved = loads
                .iter()
                .map(|load| {
                    let mut solved = factor.solve(load);
                    for &dof in &fixed {
                        solved[dof] = 0.0;
                    }
                    solved
                })
                .collect::<Vec<_>>();
            let reduced_stiffness = symmetric_gram(&solved, &loads);
            let moved_mass = solved
                .iter()
                .map(|vector| mass.multiply(vector))
                .collect::<Vec<_>>();
            let reduced_mass = symmetric_gram(&solved, &moved_mass);
            let (values, weights) = generalized_eigen(reduced_stiffness, reduced_mass)
                .ok_or(FeaError::ModesNotConverged)?;
            vectors = weights
                .iter()
                .map(|weight| combine(&solved, weight))
                .collect();
            if values[..count]
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
            {
                return Err(FeaError::ModesNotConverged);
            }
            let settled = previous.as_ref().is_some_and(|previous| {
                values[..count]
                    .iter()
                    .zip(previous)
                    .all(|(value, before)| (value - before).abs() <= ROUNDING * value)
            });
            if settled {
                // A shape settles more slowly than its frequency.
                let modes = values[..count]
                    .iter()
                    .zip(&vectors)
                    .map(|(value, vector)| {
                        self.mode(
                            &layout,
                            &joints,
                            [&free_stiffness, &mass],
                            &rounding_scale,
                            *value,
                            vector,
                        )
                    })
                    .collect::<Vec<_>>();
                if modes.iter().all(|mode| mode.relative_residual <= ROUNDING) {
                    return Ok(FeaModes {
                        schema: FEA_MODES_SCHEMA_V1,
                        solver_version: FEA_MODAL_SOLVER_VERSION_V1,
                        case_id: self.case_id.clone(),
                        model_digest: self.digest(),
                        total_mass_kg: total_mass_t * KG_PER_T,
                        iterations: iteration,
                        modes,
                    });
                }
            }
            previous = Some(values);
        }
        Err(FeaError::ModesNotConverged)
    }

    fn mode(
        &self,
        layout: &DofLayout,
        joints: &Joints,
        [free_stiffness, mass]: [&SymmetricMatrix; 2],
        rounding_scale: &Magnitudes,
        value: f64,
        vector: &[f64],
    ) -> FeaMode {
        let internal = free_stiffness.multiply(vector);
        let inertial = mass.multiply(vector);
        let residual = internal
            .iter()
            .zip(&inertial)
            .map(|(internal, inertial)| (internal - value * inertial).powi(2))
            .sum::<f64>()
            .sqrt();
        // The size the products would have with no cancellation: rounding in
        // them is relative to this, not to |K·φ|.
        let magnitude = vector.iter().map(|value| value.abs()).collect::<Vec<_>>();
        let reference = rounding_scale
            .stiffness
            .multiply(&magnitude)
            .iter()
            .zip(rounding_scale.mass.multiply(&magnitude))
            .map(|(stiffness, mass)| (stiffness + value * mass).powi(2))
            .sum::<f64>()
            .sqrt();
        let mut shape = vector.to_vec();
        joints.recover(&mut shape);
        let largest = |values: &[f64]| {
            values
                .iter()
                .copied()
                .max_by(|left, right| left.abs().total_cmp(&right.abs()))
                .filter(|value| value.abs() > NEGLIGIBLE)
        };
        let translations = layout.translations;
        if let Some(largest) =
            largest(&shape[..translations]).or_else(|| largest(&shape[translations..]))
        {
            for value in &mut shape {
                *value /= largest;
            }
        }
        let angular = value.sqrt();
        FeaMode {
            frequency_hz: angular / (2.0 * PI),
            angular_frequency_rad_s: angular,
            displacements_mm: shape[..translations]
                .chunks_exact(3)
                .map(|values| [values[0], values[1], values[2]])
                .collect(),
            rotations_rad: layout
                .slots
                .iter()
                .map(|slot| {
                    slot.map(|slot| {
                        let first = translations + slot * 3;
                        [shape[first], shape[first + 1], shape[first + 2]]
                    })
                })
                .collect(),
            relative_residual: if reference > 0.0 {
                residual / reference
            } else {
                residual
            },
        }
    }

    /// The unknowns a constraint holds; a natural frequency holds them at zero.
    fn held_dofs(&self, layout: &DofLayout) -> BTreeSet<usize> {
        let mut held = BTreeSet::new();
        for constraint in &self.constraints {
            for (axis, value) in constraint
                .displacement_mm
                .iter()
                .chain(&constraint.rotation_rad)
                .enumerate()
            {
                if value.is_some()
                    && let Some(dof) = layout.dof(constraint.node, axis)
                {
                    held.insert(dof);
                }
            }
        }
        held
    }

    /// The consistent mass of every element, in tonnes so that with newtons
    /// and millimetres a frequency comes out per second, and their total.
    fn mass_entries(&self, layout: &DofLayout) -> Result<(Entries, f64), FeaError> {
        let mut entries = Vec::new();
        let mut total = 0.0;
        for element in &self.elements {
            let degenerate = FeaError::DegenerateElement {
                element: element.id,
            };
            let material = match element.kind {
                FeaElementKind::LinearTruss2 { material_id, .. }
                | FeaElementKind::LinearTetrahedron4 { material_id, .. }
                | FeaElementKind::Frame2 { material_id, .. } => material_id,
            };
            let density = self
                .material(material)
                .density_kg_per_m3
                .ok_or(FeaError::MissingDensity { material })?
                * T_PER_MM3_PER_KG_PER_M3;
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes, area_mm2, ..
                } => {
                    let (length, _) = self.truss_geometry(nodes).ok_or(degenerate)?;
                    let mass = density * area_mm2 * length;
                    total += mass;
                    for axis in 0..3 {
                        let [first, second] = nodes.map(|node| node * 3 + axis);
                        entries.extend([
                            (first, first, mass / 3.0),
                            (second, second, mass / 3.0),
                            (first, second, mass / 6.0),
                            (second, first, mass / 6.0),
                        ]);
                    }
                }
                FeaElementKind::LinearTetrahedron4 { nodes, .. } => {
                    let (volume, _) = self.tetrahedron_geometry(nodes).ok_or(degenerate)?;
                    let mass = density * volume;
                    total += mass;
                    for row in nodes {
                        for column in nodes {
                            let share = if row == column {
                                mass / 10.0
                            } else {
                                mass / 20.0
                            };
                            entries.extend(
                                (0..3).map(|axis| (row * 3 + axis, column * 3 + axis, share)),
                            );
                        }
                    }
                }
                FeaElementKind::Frame2 {
                    nodes,
                    section,
                    orientation,
                    ..
                } => {
                    let frame = self.element_axes(element.id, nodes, orientation)?;
                    total += density * section.area_mm2 * frame.length_mm;
                    let global =
                        frame.global_stiffness(&local_mass(density, &section, frame.length_mm));
                    let dofs = layout.frame_dofs(element.id, nodes)?;
                    for (row, values) in global.iter().enumerate() {
                        for (column, value) in values.iter().enumerate() {
                            entries.push((dofs[row], dofs[column], *value));
                        }
                    }
                }
            }
        }
        Ok((entries, total))
    }
}

/// Entry magnitudes of the free stiffness and mass.
struct Magnitudes {
    stiffness: SymmetricMatrix,
    mass: SymmetricMatrix,
}

fn diagonal(size: usize, entries: &[(usize, usize, f64)]) -> Vec<f64> {
    let mut diagonal = vec![0.0; size];
    for &(row, column, value) in entries {
        if row == column {
            diagonal[row] += value;
        }
    }
    diagonal
}

fn inner(left: &[f64], right: &[f64]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

/// `Aᵢⱼ = leftᵢ · rightⱼ`, made exactly symmetric.
fn symmetric_gram(left: &[Vec<f64>], right: &[Vec<f64>]) -> Vec<Vec<f64>> {
    let gram = left
        .iter()
        .map(|row| {
            right
                .iter()
                .map(|column| inner(row, column))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    (0..gram.len())
        .map(|row| {
            (0..gram.len())
                .map(|column| (gram[row][column] + gram[column][row]) / 2.0)
                .collect()
        })
        .collect()
}

/// `Σᵢ vectorsᵢ · weightsᵢ`.
fn combine(vectors: &[Vec<f64>], weights: &[f64]) -> Vec<f64> {
    let mut sum = vec![0.0; vectors.first().map_or(0, Vec::len)];
    for (vector, weight) in vectors.iter().zip(weights) {
        for (total, value) in sum.iter_mut().zip(vector) {
            *total += weight * value;
        }
    }
    sum
}

/// The pairs of `K·q = λ·M·q` for a small dense `K` and positive definite
/// `M`, from the lowest `λ` up, each `q` with `qᵀ·M·q = 1`.
fn generalized_eigen(
    stiffness: Vec<Vec<f64>>,
    mass: Vec<Vec<f64>>,
) -> Option<(Vec<f64>, Vec<Vec<f64>>)> {
    let size = stiffness.len();
    // M = L·Lᵀ.
    let mut lower = vec![vec![0.0; size]; size];
    for row in 0..size {
        for column in 0..=row {
            let sum = mass[row][column] - inner(&lower[row][..column], &lower[column][..column]);
            if row == column {
                if sum <= NEGLIGIBLE * mass[row][row].abs() || sum <= 0.0 {
                    return None;
                }
                lower[row][row] = sum.sqrt();
            } else {
                lower[row][column] = sum / lower[column][column];
            }
        }
    }
    let forward = |values: &[f64]| {
        let mut solved = vec![0.0; size];
        for row in 0..size {
            solved[row] =
                (values[row] - inner(&lower[row][..row], &solved[..row])) / lower[row][row];
        }
        solved
    };
    // C = L⁻¹·K·L⁻ᵀ, from L⁻¹ applied to K's columns and then to the rows.
    let half = (0..size)
        .map(|column| forward(&stiffness.iter().map(|row| row[column]).collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    let mut matrix = (0..size)
        .map(|row| forward(&half.iter().map(|column| column[row]).collect::<Vec<_>>()))
        .collect::<Vec<_>>();
    let mut turns = (0..size)
        .map(|row| {
            (0..size)
                .map(|column| if row == column { 1.0 } else { 0.0 })
                .collect::<Vec<f64>>()
        })
        .collect::<Vec<_>>();
    jacobi(&mut matrix, &mut turns)?;

    // q = L⁻ᵀ·v, by back substitution.
    let backward = |values: Vec<f64>| {
        let mut solved = vec![0.0; size];
        for row in (0..size).rev() {
            let known = (row + 1..size)
                .map(|below| lower[below][row] * solved[below])
                .sum::<f64>();
            solved[row] = (values[row] - known) / lower[row][row];
        }
        solved
    };
    let mut pairs = (0..size)
        .map(|index| {
            (
                matrix[index][index],
                backward(turns.iter().map(|row| row[index]).collect()),
            )
        })
        .collect::<Vec<_>>();
    pairs.sort_by(|left, right| left.0.total_cmp(&right.0));
    Some(pairs.into_iter().unzip())
}

/// Diagonalizes the symmetric `matrix` by Jacobi rotations, gathering them in
/// `turns`; `None` when it does not settle.
fn jacobi(matrix: &mut [Vec<f64>], turns: &mut [Vec<f64>]) -> Option<()> {
    let size = matrix.len();
    let scale = matrix
        .iter()
        .flatten()
        .map(|value| value.powi(2))
        .sum::<f64>()
        .sqrt();
    for _ in 0..MAX_SWEEPS {
        let off = (0..size)
            .flat_map(|row| {
                (0..size)
                    .filter(move |column| *column != row)
                    .map(move |column| (row, column))
            })
            .map(|(row, column)| matrix[row][column].powi(2))
            .sum::<f64>()
            .sqrt();
        if off <= NEGLIGIBLE * scale {
            return Some(());
        }
        for first in 0..size {
            for second in first + 1..size {
                let coupling = matrix[first][second];
                if coupling.abs() <= NEGLIGIBLE * NEGLIGIBLE * scale {
                    continue;
                }
                let ratio = (matrix[second][second] - matrix[first][first]) / (2.0 * coupling);
                let tangent = ratio.signum() / (ratio.abs() + ratio.mul_add(ratio, 1.0).sqrt());
                let cosine = 1.0 / tangent.mul_add(tangent, 1.0).sqrt();
                let sine = tangent * cosine;
                let rotate = |rows: &mut [Vec<f64>]| {
                    for row in rows.iter_mut() {
                        let (left, right) = (row[first], row[second]);
                        row[first] = cosine * left - sine * right;
                        row[second] = sine * left + cosine * right;
                    }
                };
                rotate(matrix);
                rotate(turns);
                let (upper, lower) = matrix.split_at_mut(second);
                for (left, right) in upper[first].iter_mut().zip(lower[0].iter_mut()) {
                    let (top, bottom) = (*left, *right);
                    *left = cosine * top - sine * bottom;
                    *right = sine * top + cosine * bottom;
                }
            }
        }
    }
    None
}
