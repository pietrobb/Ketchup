use crate::fea_coupling::{
    Joints, add_springs, joint_definition, spring_energy, spring_entry_count, turning_leaders,
    validate_couplings, validate_turns,
};
use crate::fea_frame::{
    ElementAxes, FRAME_DOFS, FrameError, consistent_load, local_stiffness, multiply,
    peak_displacement, peak_von_mises, valid_section,
};
pub use crate::fea_modal::{
    FEA_MODAL_SOLVER_VERSION_V2, FEA_MODES_SCHEMA_V1, FeaMode, FeaModes, MAX_FEA_MODES,
};
use crate::sparse::{ENTRY_BYTES, FactorError, MemoryBudget, SymmetricMatrix};
use ketchup_geometry::linalg::{Mat3, add, cross, dot, length, scale, sub};
use ketchup_model::tolerance::{NEGLIGIBLE, ROUNDING};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

pub const FEA_MODEL_SCHEMA_V1: &str = "ketchup.fea-model.v1";
pub const FEA_SOLUTION_SCHEMA_V1: &str = "ketchup.fea-solution.v1";
pub const FEA_SOLVER_VERSION_V1: &str = "ketchup.linear-static-fem.v1";
pub const MAX_FEA_MATERIALS: usize = 4_096;
pub const MAX_FEA_NODES: usize = 200_000;
pub const MAX_FEA_ELEMENTS: usize = 1_000_000;
pub const MAX_FEA_CONSTRAINTS: usize = MAX_FEA_NODES;
pub const MAX_FEA_LOADS: usize = 1_000_000;
pub const MAX_FEA_COUPLINGS: usize = MAX_FEA_NODES;
/// The default memory for the factored stiffness matrix: 512 MiB.
pub const DEFAULT_FEA_FACTOR_BYTES: usize = 512 << 20;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeaNode {
    pub position_mm: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaMaterial {
    pub id: u64,
    pub youngs_modulus_mpa: f64,
    pub poisson_ratio: f64,
    pub yield_strength_mpa: Option<f64>,
    /// Needed for natural frequencies; a static solve does without it.
    pub density_kg_per_m3: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaElement {
    pub id: u64,
    pub kind: FeaElementKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeaElementKind {
    LinearTruss2 {
        nodes: [usize; 2],
        material_id: u64,
        area_mm2: f64,
    },
    LinearTetrahedron4 {
        nodes: [usize; 4],
        material_id: u64,
    },
    /// A straight 3D Euler–Bernoulli frame element between two nodes; its nodes also
    /// turn. `orientation` is any vector off the element's line: the section's
    /// `y` axis lies in the plane of the line and that vector.
    Frame2 {
        nodes: [usize; 2],
        material_id: u64,
        section: FeaFrameSection,
        orientation: [f64; 3],
    },
}

/// A frame element's cross-section, in the element's own axes (`x` along it).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeaFrameSection {
    pub area_mm2: f64,
    /// Second moment of area about the section's `y` axis (bending in `x-z`).
    pub second_moment_y_mm4: f64,
    /// Second moment of area about the section's `z` axis (bending in `x-y`).
    pub second_moment_z_mm4: f64,
    /// Saint-Venant torsion constant: twist rate = torque / (G·J).
    pub torsion_constant_mm4: f64,
    /// Distance from the centroid to the farthest fibre along `y`, and along `z`.
    pub extreme_fibre_y_mm: f64,
    pub extreme_fibre_z_mm: f64,
    /// Torsional section modulus: the largest shear stress is torque / modulus.
    pub torsion_modulus_mm3: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaConstraint {
    pub node: usize,
    pub displacement_mm: [Option<f64>; 3],
    /// Turns about the model's axes; only a node a frame element ends at turns.
    pub rotation_rad: [Option<f64>; 3],
}

impl FeaConstraint {
    /// The node held in place and left free to turn.
    #[must_use]
    pub fn held(node: usize) -> Self {
        Self {
            node,
            displacement_mm: [Some(0.0); 3],
            rotation_rad: [None; 3],
        }
    }
}

/// A joint between two nodes, usually of different parts.
#[derive(Clone, Debug, PartialEq)]
pub struct FeaCoupling {
    pub nodes: [usize; 2],
    pub kind: FeaCouplingKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeaCouplingKind {
    /// The second node follows the first as if a rigid link joined them: it
    /// moves by the first's move plus the first's turn about the offset
    /// between them, and turns with it. A first node another sits away from,
    /// or one whose follower turns, turns even when no frame element ends at it.
    Rigid,
    /// Springs along the model's axes between two nodes at one place; a turn
    /// stiffness needs both nodes to turn. A zero stiffness leaves that axis free.
    Spring {
        translation_n_per_mm: [f64; 3],
        rotation_n_mm_per_rad: [f64; 3],
    },
    /// The second node moves with the point of the triangle of the first node
    /// and `others` that `weights` interpolate (the first node's weight first):
    /// its move is the weighted sum of theirs. It ties a node of one face to the
    /// face it lies on, so two meshes that do not share nodes deform together
    /// where they touch and nowhere else. The weights are the point's
    /// barycentric coordinates, each in 0..=1 and adding up to one; the three
    /// nodes neither follow nor are tied themselves.
    Tie {
        others: [usize; 2],
        weights: [f64; 3],
    },
}

/// Why a coupling cannot be built.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeaCouplingProblem {
    /// A node is outside the mesh, or both nodes are the same.
    InvalidNode,
    /// A stiffness is not finite or negative, or every stiffness is zero.
    InvalidStiffness,
    /// A spring joins nodes at different places.
    SpringNodesApart,
    /// A turn stiffness joins a node that does not turn.
    TurnSpringNeedsTurningNodes,
    /// The second node already follows another rigid coupling.
    FollowsTwice,
    /// Rigid couplings follow each other round in a circle.
    Cycle,
    /// A constraint holds the node that follows; hold the node it follows.
    ConstrainedFollower,
    /// A tie's weights are not barycentric coordinates of a point of its
    /// triangle, or its triangle repeats a node.
    InvalidTie,
    /// A node a tie follows follows another coupling itself, or a tied node
    /// leads one.
    TieChain,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FeaLoad {
    NodalForce {
        node: usize,
        force_n: [f64; 3],
    },
    SurfaceTraction {
        nodes: [usize; 3],
        traction_n_per_mm2: [f64; 3],
    },
    /// A moment (N·mm, model axes) at a node a frame element ends at.
    NodalMoment {
        node: usize,
        moment_n_mm: [f64; 3],
    },
    /// A uniform load per length (N/mm, model axes) along the whole element
    /// element with this id.
    FrameDistributed {
        element: u64,
        load_n_per_mm: [f64; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaModel {
    pub schema: String,
    pub case_id: String,
    pub nodes: Vec<FeaNode>,
    pub materials: Vec<FeaMaterial>,
    pub elements: Vec<FeaElement>,
    pub constraints: Vec<FeaConstraint>,
    pub loads: Vec<FeaLoad>,
    pub couplings: Vec<FeaCoupling>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeaSolveSettings {
    pub maximum_nodes: usize,
    pub relative_pivot_tolerance: f64,
    pub maximum_small_deformation_ratio: f64,
    pub convergence_relative_tolerance: f64,
    /// The memory a solve or a modes study may hold at once in its entries,
    /// matrices and factor; a larger model is refused with what it would
    /// need, before the allocation that would pass the budget
    /// (R1120ac-FEA-902). Vectors of one value per unknown come on top.
    pub maximum_factor_bytes: usize,
}

impl Default for FeaSolveSettings {
    fn default() -> Self {
        Self {
            maximum_nodes: MAX_FEA_NODES,
            relative_pivot_tolerance: NEGLIGIBLE,
            maximum_small_deformation_ratio: 0.05,
            convergence_relative_tolerance: 0.02,
            maximum_factor_bytes: DEFAULT_FEA_FACTOR_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeaError {
    UnsupportedSchema,
    EmptyCaseId,
    EmptyMesh,
    ModelTooLarge,
    InvalidNode,
    DuplicateMaterialId,
    InvalidMaterial,
    /// Each element error names the element by its id.
    DuplicateElementId {
        element: u64,
    },
    DuplicateElement {
        element: u64,
    },
    InvalidElementNode {
        element: u64,
    },
    RepeatedElementNode {
        element: u64,
    },
    DegenerateElement {
        element: u64,
    },
    MissingMaterial {
        element: u64,
    },
    InvalidArea {
        element: u64,
    },
    /// A frame section value is missing, zero or negative.
    InvalidFrameSection {
        element: u64,
    },
    /// A frame element's orientation vector is not finite or runs along the element.
    InvalidFrameOrientation {
        element: u64,
    },
    MissingConstraint,
    InvalidConstraint,
    ConflictingConstraint,
    /// The coupling at index `coupling` of the model's couplings.
    InvalidCoupling {
        coupling: usize,
        problem: FeaCouplingProblem,
    },
    MissingLoad,
    InvalidLoad,
    NonBoundaryTraction,
    InvalidSettings,
    SingularSystem,
    /// The entries, matrices and factor held at once would need
    /// `needed_bytes`, more than the `budget_bytes` the settings allow.
    FactorTooLarge {
        needed_bytes: usize,
        budget_bytes: usize,
    },
    NonFiniteSolution,
    /// Natural frequencies need the density of the material with this id.
    MissingDensity {
        material: u64,
    },
    /// No modes asked for, more than `MAX_FEA_MODES`, or more than the model
    /// has free unknowns with mass.
    InvalidModeCount,
    /// The subspace iteration did not settle on the lowest modes.
    ModesNotConverged,
    /// The caller's cancellation flag was set while solving.
    Cancelled,
}

impl FeaError {
    /// What a failed factorization under `settings` means for the model, with
    /// `held_bytes` of matrices kept beside the factor.
    pub(crate) const fn from_factor(
        error: FactorError,
        settings: FeaSolveSettings,
        held_bytes: usize,
    ) -> Self {
        match error {
            FactorError::Singular => Self::SingularSystem,
            FactorError::Cancelled => Self::Cancelled,
            FactorError::TooLarge { needed_bytes } => {
                Self::too_large(needed_bytes.saturating_add(held_bytes), settings)
            }
        }
    }

    /// The model would need `needed_bytes` held at once, more than `settings` allow.
    pub(crate) const fn too_large(needed_bytes: usize, settings: FeaSolveSettings) -> Self {
        Self::FactorTooLarge {
            needed_bytes,
            budget_bytes: settings.maximum_factor_bytes,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaElementResult {
    pub element_id: u64,
    pub strain_energy_n_mm: f64,
    pub von_mises_stress_mpa: f64,
    pub yield_utilization: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaValidityEvidence {
    pub characteristic_length_mm: f64,
    pub maximum_displacement_ratio: f64,
    pub maximum_allowed_displacement_ratio: f64,
    pub within_small_deformation_limit: bool,
    pub all_yield_strengths_supplied: bool,
    pub maximum_yield_utilization: Option<f64>,
    pub within_linear_elastic_stress_limit: Option<bool>,
    pub excluded_physics: [&'static str; 5],
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaSolution {
    pub schema: &'static str,
    pub solver_version: &'static str,
    pub case_id: String,
    pub model_digest: String,
    pub displacements_mm: Vec<[f64; 3]>,
    pub reaction_forces_n: Vec<[f64; 3]>,
    /// Turns about the model's axes; `None` at a node no frame element ends at.
    pub rotations_rad: Vec<Option<[f64; 3]>>,
    pub reaction_moments_n_mm: Vec<Option<[f64; 3]>>,
    pub element_results: Vec<FeaElementResult>,
    pub maximum_displacement_mm: f64,
    pub maximum_von_mises_stress_mpa: f64,
    pub total_strain_energy_n_mm: f64,
    pub maximum_free_dof_residual_n: f64,
    pub force_balance_n: [f64; 3],
    pub validity: FeaValidityEvidence,
}

impl FeaSolution {
    #[must_use]
    pub fn is_within_declared_limits(&self) -> bool {
        self.validity.within_small_deformation_limit
            && self
                .validity
                .within_linear_elastic_stress_limit
                .unwrap_or(false)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaConvergenceLevel {
    pub node_count: usize,
    pub element_count: usize,
    pub maximum_displacement_mm: f64,
    pub total_strain_energy_n_mm: f64,
}

/// The least ratio of element counts between the last two levels of a convergence study
/// for it to count as converged: from one level to one hardly finer the results change
/// little whatever their error (og:R1119og-FEA-803: 20 to 21 elements changed the
/// deflection by 1.9 % while it was 69 % of the true one).
pub const MIN_FEA_REFINEMENT_RATIO: f64 = 2.0;

/// How the finest level of a convergence study compares with the level before it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeaConvergenceJudgement {
    pub final_displacement_relative_change: f64,
    pub final_energy_relative_change: f64,
    /// Of the largest von Mises stress: a stress that keeps rising as the mesh is refined,
    /// as at a sharp inner corner, has not converged however still the deflection is.
    pub final_stress_relative_change: f64,
    /// The finest level's elements over those of the level before.
    pub final_refinement_ratio: f64,
    /// Every change within the tolerance and the finest level at least
    /// [`MIN_FEA_REFINEMENT_RATIO`] times finer.
    pub converged: bool,
}

impl FeaConvergenceJudgement {
    /// Compares the solution `last` on `last_elements` elements with `previous` on
    /// `previous_elements`, against the relative `tolerance`.
    #[must_use]
    pub fn between(
        (previous, previous_elements): (&FeaSolution, usize),
        (last, last_elements): (&FeaSolution, usize),
        tolerance: f64,
    ) -> Self {
        let displacement = relative_change(
            previous.maximum_displacement_mm,
            last.maximum_displacement_mm,
        );
        let energy = relative_change(
            previous.total_strain_energy_n_mm,
            last.total_strain_energy_n_mm,
        );
        let stress = relative_change(
            previous.maximum_von_mises_stress_mpa,
            last.maximum_von_mises_stress_mpa,
        );
        let elements = |count: usize| f64::from(u32::try_from(count).unwrap_or(u32::MAX));
        let ratio = elements(last_elements) / elements(previous_elements).max(1.0);
        Self {
            final_displacement_relative_change: displacement,
            final_energy_relative_change: energy,
            final_stress_relative_change: stress,
            final_refinement_ratio: ratio,
            converged: displacement <= tolerance
                && energy <= tolerance
                && stress <= tolerance
                && ratio >= MIN_FEA_REFINEMENT_RATIO,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaConvergenceEvidence {
    pub case_id: String,
    pub levels: Vec<FeaConvergenceLevel>,
    pub judgement: FeaConvergenceJudgement,
    pub required_relative_tolerance: f64,
    pub solutions: Vec<FeaSolution>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeaConvergenceError {
    TooFewLevels,
    CaseMismatch,
    MeshNotRefined,
    DomainMismatch,
    MaterialMismatch,
    LoadMismatch,
    ConstraintMismatch,
    Solve(FeaError),
}

#[derive(Default)]
struct GeometricMoments {
    measure: f64,
    first: [f64; 3],
    second: [f64; 9],
}

#[derive(Default)]
struct LoadDefinition {
    nodal: Vec<([u64; 3], [u64; 3])>,
    moments: Vec<([u64; 3], [u64; 3])>,
    surface: BTreeMap<[u64; 3], GeometricMoments>,
    /// Uniform frame loads by their value, over the lines they act along.
    lines: BTreeMap<[u64; 3], GeometricMoments>,
}

#[derive(Default)]
struct ConstraintSupport {
    lines: GeometricMoments,
    surfaces: GeometricMoments,
    points: Vec<[u64; 3]>,
}

type ConstraintDefinition = BTreeMap<(usize, u64), ConstraintSupport>;

pub(crate) struct DofLayout {
    /// Three moves per node come first.
    pub(crate) translations: usize,
    /// The turning nodes, in slot order.
    owners: Vec<usize>,
    /// Each turning node's place among the turning nodes.
    pub(crate) slots: Vec<Option<usize>>,
}

impl DofLayout {
    pub(crate) fn count(&self) -> usize {
        self.translations + self.owners.len() * 3
    }

    /// The unknown of `axis` (0–2 moves, 3–5 turns) at `node`.
    pub(crate) fn dof(&self, node: usize, axis: usize) -> Option<usize> {
        if axis < 3 {
            Some(node * 3 + axis)
        } else {
            self.slots[node].map(|slot| self.translations + slot * 3 + axis - 3)
        }
    }

    /// The unknowns of a frame element; both its nodes turn by construction.
    pub(crate) fn frame_dofs(
        &self,
        element: u64,
        nodes: [usize; 2],
    ) -> Result<[usize; FRAME_DOFS], FeaError> {
        let mut dofs = [0; FRAME_DOFS];
        for (local, dof) in dofs.iter_mut().enumerate() {
            *dof = self
                .dof(nodes[local / 6], local % 6)
                .ok_or(FeaError::InvalidElementNode { element })?;
        }
        Ok(dofs)
    }

    fn turn_of(&self, dof: usize) -> (usize, usize) {
        let offset = dof - self.translations;
        (self.owners[offset / 3], offset % 3)
    }

    fn per_turning_node(&self, value: [f64; 3]) -> Vec<Option<[f64; 3]>> {
        self.slots.iter().map(|slot| slot.map(|_| value)).collect()
    }
}

impl FeaModel {
    pub fn solve(&self, settings: FeaSolveSettings) -> Result<FeaSolution, FeaError> {
        self.solve_with_cancellation(settings, &AtomicBool::new(false))
    }

    /// [`Self::solve`], stopping with [`FeaError::Cancelled`] soon after
    /// `cancelled` is set (og:R1119og-FEA-806).
    pub fn solve_with_cancellation(
        &self,
        settings: FeaSolveSettings,
        cancelled: &AtomicBool,
    ) -> Result<FeaSolution, FeaError> {
        self.validate(settings)?;
        let layout = self.dof_layout();
        let dof_count = layout.count();
        let too_large = |needed| FeaError::too_large(needed, settings);
        let mut budget = MemoryBudget::new(settings.maximum_factor_bytes);
        let entries = self.stiffness_entries_within(&layout, &mut budget, settings)?;
        let mut forces = vec![0.0; dof_count];
        self.assemble_loads(&layout, &mut forces)?;
        // A rigid coupling's follower is no unknown of its own: its rows and
        // forces go to the unknowns it follows, and it is set from them after.
        let joints = Joints::new(self, &layout)?;
        let entries =
            reduce_within(&joints, entries, &mut forces, &mut budget).map_err(too_large)?;

        let original_stiffness = matrix_within(dof_count, entries, &mut budget, settings)?;
        budget.take(original_stiffness.bytes()).map_err(too_large)?;
        let mut stiffness = original_stiffness.clone();
        let original_forces = forces.clone();
        let follower_scale = stiffness.largest_magnitude().max(1.0);
        for dof in joints.follower_dofs() {
            stiffness.prescribe(dof, 0.0, follower_scale, &mut forces);
        }
        let constrained = self.apply_constraints(&layout, &mut stiffness, &mut forces)?;
        let threshold = stiffness.largest_magnitude().max(1.0) * settings.relative_pivot_tolerance;
        let mut displacement = stiffness
            .factor(threshold, budget.left(), cancelled)
            .map_err(|error| FeaError::from_factor(error, settings, budget.held()))?
            .solve(&forces);
        if displacement.iter().any(|value| !value.is_finite()) {
            return Err(FeaError::NonFiniteSolution);
        }

        let residual = original_stiffness
            .multiply(&displacement)
            .into_iter()
            .zip(&original_forces)
            .map(|(internal, applied)| internal - applied)
            .collect::<Vec<_>>();
        joints.recover(&mut displacement);
        let mut maximum_free_dof_residual_n: f64 = 0.0;
        let mut reactions = vec![[0.0; 3]; self.nodes.len()];
        let mut reaction_moments = layout.per_turning_node([0.0; 3]);
        for (dof, value) in residual.iter().copied().enumerate() {
            if !constrained.contains(&dof) {
                maximum_free_dof_residual_n = maximum_free_dof_residual_n.max(value.abs());
            } else if dof < layout.translations {
                reactions[dof / 3][dof % 3] = value;
            } else {
                let (node, axis) = layout.turn_of(dof);
                if let Some(moment) = &mut reaction_moments[node] {
                    moment[axis] = value;
                }
            }
        }

        let displacements_mm = displacement[..layout.translations]
            .chunks_exact(3)
            .map(|values| [values[0], values[1], values[2]])
            .collect::<Vec<_>>();
        let rotations_rad = layout
            .slots
            .iter()
            .map(|slot| {
                slot.map(|slot| {
                    let start = layout.translations + slot * 3;
                    [
                        displacement[start],
                        displacement[start + 1],
                        displacement[start + 2],
                    ]
                })
            })
            .collect::<Vec<_>>();
        let (element_results, frame_peak_mm) = self.element_results(&layout, &displacement)?;
        let maximum_displacement_mm = displacements_mm
            .iter()
            .map(|value| length(*value))
            .fold(frame_peak_mm, f64::max);
        let maximum_von_mises_stress_mpa = element_results
            .iter()
            .map(|result| result.von_mises_stress_mpa)
            .fold(0.0, f64::max);
        let total_strain_energy_n_mm = element_results
            .iter()
            .map(|result| result.strain_energy_n_mm)
            .sum::<f64>()
            + spring_energy(self, &layout, &displacement);
        // A load near the end of the number range (1e300 N) solves to finite
        // displacements whose stresses and energy overflow, written as null
        // (review 2026-11-20 ac P3).
        if !(maximum_displacement_mm.is_finite()
            && total_strain_energy_n_mm.is_finite()
            && maximum_free_dof_residual_n.is_finite()
            && element_results
                .iter()
                .all(|result| result.von_mises_stress_mpa.is_finite())
            && reactions.iter().flatten().all(|value| value.is_finite()))
        {
            return Err(FeaError::NonFiniteSolution);
        }
        let characteristic_length_mm = self.characteristic_length();
        let maximum_displacement_ratio = maximum_displacement_mm / characteristic_length_mm;
        let all_yield_strengths_supplied = self
            .materials
            .iter()
            .all(|material| material.yield_strength_mpa.is_some());
        let maximum_yield_utilization = all_yield_strengths_supplied.then(|| {
            element_results
                .iter()
                .filter_map(|result| result.yield_utilization)
                .fold(0.0, f64::max)
        });

        let total_applied = vector_sum(&original_forces[..layout.translations]);
        let total_reaction = reactions.iter().copied().fold([0.0; 3], add);
        Ok(FeaSolution {
            schema: FEA_SOLUTION_SCHEMA_V1,
            solver_version: FEA_SOLVER_VERSION_V1,
            case_id: self.case_id.clone(),
            model_digest: self.digest(),
            displacements_mm,
            reaction_forces_n: reactions,
            rotations_rad,
            reaction_moments_n_mm: reaction_moments,
            element_results,
            maximum_displacement_mm,
            maximum_von_mises_stress_mpa,
            total_strain_energy_n_mm,
            maximum_free_dof_residual_n,
            force_balance_n: add(total_applied, total_reaction),
            validity: FeaValidityEvidence {
                characteristic_length_mm,
                maximum_displacement_ratio,
                maximum_allowed_displacement_ratio: settings.maximum_small_deformation_ratio,
                within_small_deformation_limit: maximum_displacement_ratio
                    <= settings.maximum_small_deformation_ratio,
                all_yield_strengths_supplied,
                maximum_yield_utilization,
                within_linear_elastic_stress_limit: maximum_yield_utilization
                    .map(|utilization| utilization <= 1.0),
                // Review 2026-11-08 PG-246: one isotropic material with von
                // Mises against a yield strength, which says nothing of a
                // material whose strength depends on the direction.
                excluded_physics: [
                    "geometric nonlinearity",
                    "plasticity",
                    "contact",
                    // Review 2026-11-19 og FEA-809: the answer may carry natural
                    // frequencies, and the parts' own weight was silently left out.
                    "buckling, forced and transient dynamics (natural frequencies come only from a modes study), and self-weight: only the given loads act",
                    "anisotropic materials (timber, wood panels; checked by timber_design): the material is isotropic and its von Mises stress is compared with one yield strength",
                ],
            },
        })
    }

    /// How many entries [`Self::stiffness_entries`] makes: a diagonal for
    /// every unknown, a square for every element and four for every spring.
    pub(crate) fn stiffness_entry_count(&self, layout: &DofLayout) -> Result<usize, FeaError> {
        let springs = spring_entry_count(self, layout)?;
        Ok(self
            .elements
            .iter()
            .map(|element| match element.kind {
                FeaElementKind::LinearTruss2 { .. } => 6 * 6,
                FeaElementKind::LinearTetrahedron4 { .. } => 12 * 12,
                FeaElementKind::Frame2 { .. } => FRAME_DOFS * FRAME_DOFS,
            })
            .fold(
                layout.count().saturating_add(springs),
                usize::saturating_add,
            ))
    }

    /// [`Self::stiffness_entries`], taken from `budget` before any is
    /// assembled (R1120ac-FEA-902: a million tetrahedra took ~5.8 GB in the
    /// window before the factor's 512 MiB check).
    pub(crate) fn stiffness_entries_within(
        &self,
        layout: &DofLayout,
        budget: &mut MemoryBudget,
        settings: FeaSolveSettings,
    ) -> Result<Vec<(usize, usize, f64)>, FeaError> {
        let count = self.stiffness_entry_count(layout)?;
        budget
            .take(count.saturating_mul(ENTRY_BYTES))
            .map_err(|needed| FeaError::too_large(needed, settings))?;
        self.stiffness_entries(layout)
    }

    /// The stiffness of every element and spring as `(row, column, value)`
    /// entries, with a diagonal entry for every unknown so a constraint can set it.
    pub(crate) fn stiffness_entries(
        &self,
        layout: &DofLayout,
    ) -> Result<Vec<(usize, usize, f64)>, FeaError> {
        // Exactly as many as counted, so the budget taken for them holds.
        let mut entries = Vec::with_capacity(self.stiffness_entry_count(layout)?);
        entries.extend((0..layout.count()).map(|dof| (dof, dof, 0.0)));
        for element in &self.elements {
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => self.assemble_truss(nodes, material_id, area_mm2, &mut entries),
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                    self.assemble_tetrahedron(nodes, material_id, &mut entries)
                }
                FeaElementKind::Frame2 {
                    nodes,
                    material_id,
                    section,
                    orientation,
                } => {
                    let frame = self.element_axes(element.id, nodes, orientation)?;
                    let stiffness = frame.global_stiffness(&self.frame_local_stiffness(
                        material_id,
                        &section,
                        frame.length_mm,
                    ));
                    let dofs = layout.frame_dofs(element.id, nodes)?;
                    for (row, values) in stiffness.iter().enumerate() {
                        for (column, value) in values.iter().enumerate() {
                            entries.push((dofs[row], dofs[column], *value));
                        }
                    }
                }
            }
        }
        add_springs(self, layout, &mut entries)?;
        debug_assert_eq!(entries.len(), entries.capacity());
        Ok(entries)
    }

    fn validate(&self, settings: FeaSolveSettings) -> Result<(), FeaError> {
        self.validate_model(settings, true)
    }

    /// Everything [`Self::solve`] checks; natural frequencies need no load.
    pub(crate) fn validate_model(
        &self,
        settings: FeaSolveSettings,
        loads_required: bool,
    ) -> Result<(), FeaError> {
        if self.schema != FEA_MODEL_SCHEMA_V1 {
            return Err(FeaError::UnsupportedSchema);
        }
        if self.case_id.trim().is_empty() {
            return Err(FeaError::EmptyCaseId);
        }
        if self.nodes.is_empty() || self.elements.is_empty() {
            return Err(FeaError::EmptyMesh);
        }
        if settings.maximum_nodes == 0
            || self.nodes.len() > settings.maximum_nodes
            || self.nodes.len() > MAX_FEA_NODES
            || self.materials.len() > MAX_FEA_MATERIALS
            || self.elements.len() > MAX_FEA_ELEMENTS
            || self.constraints.len() > MAX_FEA_CONSTRAINTS
            || self.loads.len() > MAX_FEA_LOADS
            || self.couplings.len() > MAX_FEA_COUPLINGS
        {
            return Err(FeaError::ModelTooLarge);
        }
        if !finite_positive(settings.relative_pivot_tolerance)
            || settings.relative_pivot_tolerance >= 1.0
            || !finite_positive(settings.maximum_small_deformation_ratio)
            || settings.maximum_small_deformation_ratio >= 1.0
            || !finite_positive(settings.convergence_relative_tolerance)
            || settings.convergence_relative_tolerance >= 1.0
            || settings.maximum_factor_bytes == 0
        {
            return Err(FeaError::InvalidSettings);
        }
        if self
            .nodes
            .iter()
            .flat_map(|node| node.position_mm)
            .any(|value| !value.is_finite() || value.abs() > 1.0e9)
        {
            return Err(FeaError::InvalidNode);
        }

        let mut material_ids = BTreeSet::new();
        for material in &self.materials {
            if !material_ids.insert(material.id) {
                return Err(FeaError::DuplicateMaterialId);
            }
            if !finite_positive(material.youngs_modulus_mpa)
                || !material.poisson_ratio.is_finite()
                || !(-1.0..0.5).contains(&material.poisson_ratio)
                || material
                    .yield_strength_mpa
                    .into_iter()
                    .chain(material.density_kg_per_m3)
                    .any(|value| !finite_positive(value))
            {
                return Err(FeaError::InvalidMaterial);
            }
        }

        let mut element_ids = BTreeSet::new();
        let mut truss_connectivities = BTreeSet::new();
        let mut tetrahedron_connectivities = BTreeSet::new();
        let mut frame_connectivities = BTreeSet::new();
        let mut frames = BTreeSet::new();
        let mut tetrahedron_faces = BTreeMap::<[usize; 3], usize>::new();
        for element in &self.elements {
            let element_id = element.id;
            if !element_ids.insert(element_id) {
                return Err(FeaError::DuplicateElementId {
                    element: element_id,
                });
            }
            let (nodes, material_id) = match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes, material_id, ..
                } => (nodes.to_vec(), material_id),
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                    (nodes.to_vec(), material_id)
                }
                FeaElementKind::Frame2 {
                    nodes, material_id, ..
                } => (nodes.to_vec(), material_id),
            };
            // A node outside the mesh is named as such, not as a degenerate shape.
            if nodes.iter().any(|node| *node >= self.nodes.len()) {
                return Err(FeaError::InvalidElementNode {
                    element: element_id,
                });
            }
            if nodes.iter().copied().collect::<BTreeSet<_>>().len() != nodes.len() {
                return Err(FeaError::RepeatedElementNode {
                    element: element_id,
                });
            }
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes, area_mm2, ..
                } => {
                    if !finite_positive(area_mm2) {
                        return Err(FeaError::InvalidArea {
                            element: element_id,
                        });
                    }
                    let mut canonical_nodes = nodes;
                    canonical_nodes.sort_unstable();
                    if !truss_connectivities.insert(canonical_nodes) {
                        return Err(FeaError::DuplicateElement {
                            element: element_id,
                        });
                    }
                    if self.truss_geometry(nodes).is_none() {
                        return Err(FeaError::DegenerateElement {
                            element: element_id,
                        });
                    }
                }
                FeaElementKind::LinearTetrahedron4 { nodes, .. } => {
                    let mut canonical_nodes = nodes;
                    canonical_nodes.sort_unstable();
                    if !tetrahedron_connectivities.insert(canonical_nodes) {
                        return Err(FeaError::DuplicateElement {
                            element: element_id,
                        });
                    }
                    if self.tetrahedron_geometry(nodes).is_none() {
                        return Err(FeaError::DegenerateElement {
                            element: element_id,
                        });
                    }
                    for face in tetrahedron_faces_of(nodes) {
                        *tetrahedron_faces.entry(face).or_default() += 1;
                    }
                }
                FeaElementKind::Frame2 {
                    nodes,
                    section,
                    orientation,
                    ..
                } => {
                    if !valid_section(&section) {
                        return Err(FeaError::InvalidFrameSection {
                            element: element_id,
                        });
                    }
                    let mut canonical_nodes = nodes;
                    canonical_nodes.sort_unstable();
                    if !frame_connectivities.insert(canonical_nodes) {
                        return Err(FeaError::DuplicateElement {
                            element: element_id,
                        });
                    }
                    self.element_axes(element_id, nodes, orientation)?;
                    frames.insert(element_id);
                }
            }
            if !material_ids.contains(&material_id) {
                return Err(FeaError::MissingMaterial {
                    element: element_id,
                });
            }
        }

        validate_couplings(self)?;
        let layout = self.dof_layout();
        validate_turns(self, &layout)?;
        if self.constraints.is_empty() {
            return Err(FeaError::MissingConstraint);
        }
        let turning = layout.slots;
        let mut prescribed = BTreeMap::<(usize, usize), f64>::new();
        for constraint in &self.constraints {
            if constraint.node >= self.nodes.len()
                || constraint
                    .displacement_mm
                    .iter()
                    .chain(&constraint.rotation_rad)
                    .all(Option::is_none)
                || (turning[constraint.node].is_none()
                    && constraint.rotation_rad.iter().any(Option::is_some))
            {
                return Err(FeaError::InvalidConstraint);
            }
            for (axis, value) in constraint
                .displacement_mm
                .iter()
                .chain(&constraint.rotation_rad)
                .copied()
                .enumerate()
            {
                if let Some(value) = value {
                    if !value.is_finite() {
                        return Err(FeaError::InvalidConstraint);
                    }
                    if prescribed
                        .insert((constraint.node, axis), value)
                        .is_some_and(|previous| previous.to_bits() != value.to_bits())
                    {
                        return Err(FeaError::ConflictingConstraint);
                    }
                }
            }
        }

        if loads_required && self.loads.is_empty() {
            return Err(FeaError::MissingLoad);
        }
        for load in &self.loads {
            match *load {
                FeaLoad::NodalForce { node, force_n } => {
                    if node >= self.nodes.len() || !finite3(force_n) {
                        return Err(FeaError::InvalidLoad);
                    }
                }
                FeaLoad::SurfaceTraction {
                    mut nodes,
                    traction_n_per_mm2,
                } => {
                    if nodes.iter().any(|node| *node >= self.nodes.len())
                        || !finite3(traction_n_per_mm2)
                    {
                        return Err(FeaError::InvalidLoad);
                    }
                    nodes.sort_unstable();
                    if tetrahedron_faces.get(&nodes) != Some(&1) {
                        return Err(FeaError::NonBoundaryTraction);
                    }
                    if triangle_area(
                        self.nodes[nodes[0]].position_mm,
                        self.nodes[nodes[1]].position_mm,
                        self.nodes[nodes[2]].position_mm,
                    ) <= NEGLIGIBLE
                    {
                        return Err(FeaError::InvalidLoad);
                    }
                }
                FeaLoad::NodalMoment { node, moment_n_mm } => {
                    if node >= self.nodes.len() || turning[node].is_none() || !finite3(moment_n_mm)
                    {
                        return Err(FeaError::InvalidLoad);
                    }
                }
                FeaLoad::FrameDistributed {
                    element,
                    load_n_per_mm,
                } => {
                    if !frames.contains(&element) || !finite3(load_n_per_mm) {
                        return Err(FeaError::InvalidLoad);
                    }
                }
            }
        }
        Ok(())
    }

    /// Unknowns: three moves per node, then three turns per node a frame element
    /// ends at, then per node only rigid couplings make turn.
    pub(crate) fn dof_layout(&self) -> DofLayout {
        let mut slots = vec![None; self.nodes.len()];
        let mut owners = Vec::new();
        for element in &self.elements {
            if let FeaElementKind::Frame2 { nodes, .. } = element.kind {
                for node in nodes {
                    if let Some(slot) = slots.get_mut(node)
                        && slot.is_none()
                    {
                        *slot = Some(owners.len());
                        owners.push(node);
                    }
                }
            }
        }
        let mut turning = slots.iter().map(Option::is_some).collect::<Vec<_>>();
        turning_leaders(self, &mut turning);
        for (node, turns) in turning.into_iter().enumerate() {
            if turns && slots[node].is_none() {
                slots[node] = Some(owners.len());
                owners.push(node);
            }
        }
        DofLayout {
            translations: self.nodes.len() * 3,
            owners,
            slots,
        }
    }

    pub(crate) fn element_axes(
        &self,
        element: u64,
        nodes: [usize; 2],
        orientation: [f64; 3],
    ) -> Result<ElementAxes, FeaError> {
        ElementAxes::new(
            self.nodes[nodes[0]].position_mm,
            self.nodes[nodes[1]].position_mm,
            orientation,
        )
        .map_err(|error| match error {
            FrameError::Degenerate => FeaError::DegenerateElement { element },
            FrameError::Orientation => FeaError::InvalidFrameOrientation { element },
        })
    }

    fn frame_local_stiffness(
        &self,
        material_id: u64,
        section: &FeaFrameSection,
        length_mm: f64,
    ) -> [[f64; FRAME_DOFS]; FRAME_DOFS] {
        let material = self.material(material_id);
        local_stiffness(
            material.youngs_modulus_mpa,
            material.youngs_modulus_mpa / (2.0 * (1.0 + material.poisson_ratio)),
            section,
            length_mm,
        )
    }

    /// The uniform load on each frame element, summed, in model axes.
    fn frame_loads(&self) -> BTreeMap<u64, [f64; 3]> {
        let mut loads = BTreeMap::<u64, [f64; 3]>::new();
        for load in &self.loads {
            if let FeaLoad::FrameDistributed {
                element,
                load_n_per_mm,
            } = *load
            {
                let total = loads.entry(element).or_default();
                *total = add(*total, load_n_per_mm);
            }
        }
        loads
    }

    pub(crate) fn material(&self, id: u64) -> &FeaMaterial {
        self.materials
            .iter()
            .find(|material| material.id == id)
            .expect("validated material reference")
    }

    fn material_distribution(&self) -> BTreeMap<u64, GeometricMoments> {
        let mut distribution = BTreeMap::<u64, GeometricMoments>::new();
        for element in &self.elements {
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => distribution
                    .entry(material_id)
                    .or_default()
                    .add_weighted_segment(
                        self.nodes[nodes[0]].position_mm,
                        self.nodes[nodes[1]].position_mm,
                        area_mm2,
                    ),
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => distribution
                    .entry(material_id)
                    .or_default()
                    .add_tetrahedron(nodes.map(|node| self.nodes[node].position_mm)),
                FeaElementKind::Frame2 {
                    nodes,
                    material_id,
                    section,
                    ..
                } => distribution
                    .entry(material_id)
                    .or_default()
                    .add_weighted_segment(
                        self.nodes[nodes[0]].position_mm,
                        self.nodes[nodes[1]].position_mm,
                        section.area_mm2,
                    ),
            }
        }
        distribution
    }

    pub(crate) fn truss_geometry(&self, nodes: [usize; 2]) -> Option<(f64, [f64; 3])> {
        if nodes.iter().any(|node| *node >= self.nodes.len()) {
            return None;
        }
        let delta = sub(
            self.nodes[nodes[1]].position_mm,
            self.nodes[nodes[0]].position_mm,
        );
        let length = length(delta);
        (length > ROUNDING).then(|| (length, scale(delta, 1.0 / length)))
    }

    pub(crate) fn tetrahedron_geometry(&self, nodes: [usize; 4]) -> Option<(f64, [[f64; 3]; 4])> {
        if nodes.iter().any(|node| *node >= self.nodes.len()) {
            return None;
        }
        let origin = self.nodes[nodes[0]].position_mm;
        let jacobian = [
            sub(self.nodes[nodes[1]].position_mm, origin),
            sub(self.nodes[nodes[2]].position_mm, origin),
            sub(self.nodes[nodes[3]].position_mm, origin),
        ];
        // The stored Jacobian is transposed, so its inverse is already J^-T.
        let jacobian = Mat3::from_rows(jacobian);
        let determinant = jacobian.determinant();
        let inverse = jacobian.inverse().ok()?;
        let gradients_reference = [
            [-1.0, -1.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let gradients =
            gradients_reference.map(|gradient| inverse.mul_vec(gradient.into()).to_array());
        Some((determinant.abs() / 6.0, gradients))
    }

    fn assemble_truss(
        &self,
        nodes: [usize; 2],
        material_id: u64,
        area_mm2: f64,
        stiffness: &mut Vec<(usize, usize, f64)>,
    ) {
        let (length, direction) = self.truss_geometry(nodes).expect("validated truss");
        let factor = self.material(material_id).youngs_modulus_mpa * area_mm2 / length;
        for local_i in 0..6 {
            for local_j in 0..6 {
                let sign_i = if local_i < 3 { -1.0 } else { 1.0 };
                let sign_j = if local_j < 3 { -1.0 } else { 1.0 };
                let global_i = nodes[local_i / 3] * 3 + local_i % 3;
                let global_j = nodes[local_j / 3] * 3 + local_j % 3;
                stiffness.push((
                    global_i,
                    global_j,
                    factor * sign_i * sign_j * direction[local_i % 3] * direction[local_j % 3],
                ));
            }
        }
    }

    fn assemble_tetrahedron(
        &self,
        nodes: [usize; 4],
        material_id: u64,
        stiffness: &mut Vec<(usize, usize, f64)>,
    ) {
        let (volume, gradients) = self
            .tetrahedron_geometry(nodes)
            .expect("validated tetrahedron");
        let b = strain_displacement(gradients);
        let d = elasticity(self.material(material_id));
        for local_i in 0..12 {
            for local_j in 0..12 {
                let value = (0..6)
                    .flat_map(|row| {
                        (0..6).map(move |column| {
                            b[row][local_i] * d[row][column] * b[column][local_j]
                        })
                    })
                    .sum::<f64>()
                    * volume;
                let global_i = nodes[local_i / 3] * 3 + local_i % 3;
                let global_j = nodes[local_j / 3] * 3 + local_j % 3;
                stiffness.push((global_i, global_j, value));
            }
        }
    }

    fn assemble_loads(&self, layout: &DofLayout, forces: &mut [f64]) -> Result<(), FeaError> {
        let frame_loads = self.frame_loads();
        for element in &self.elements {
            if let FeaElementKind::Frame2 {
                nodes, orientation, ..
            } = element.kind
                && let Some(load) = frame_loads.get(&element.id)
            {
                let frame = self.element_axes(element.id, nodes, orientation)?;
                let nodal = frame.global12(consistent_load(frame.local(*load), frame.length_mm));
                for (dof, value) in layout.frame_dofs(element.id, nodes)?.into_iter().zip(nodal) {
                    forces[dof] += value;
                }
            }
        }
        for load in &self.loads {
            match *load {
                FeaLoad::NodalForce { node, force_n } => {
                    for axis in 0..3 {
                        forces[node * 3 + axis] += force_n[axis];
                    }
                }
                FeaLoad::SurfaceTraction {
                    nodes,
                    traction_n_per_mm2,
                } => {
                    let area = triangle_area(
                        self.nodes[nodes[0]].position_mm,
                        self.nodes[nodes[1]].position_mm,
                        self.nodes[nodes[2]].position_mm,
                    );
                    for node in nodes {
                        for axis in 0..3 {
                            forces[node * 3 + axis] += traction_n_per_mm2[axis] * area / 3.0;
                        }
                    }
                }
                FeaLoad::NodalMoment { node, moment_n_mm } => {
                    for (axis, moment) in moment_n_mm.into_iter().enumerate() {
                        let dof = layout.dof(node, axis + 3).ok_or(FeaError::InvalidLoad)?;
                        forces[dof] += moment;
                    }
                }
                FeaLoad::FrameDistributed { .. } => {}
            }
        }
        Ok(())
    }

    fn apply_constraints(
        &self,
        layout: &DofLayout,
        stiffness: &mut SymmetricMatrix,
        forces: &mut [f64],
    ) -> Result<BTreeSet<usize>, FeaError> {
        let mut prescribed = BTreeMap::<usize, f64>::new();
        for constraint in &self.constraints {
            for (axis, value) in constraint
                .displacement_mm
                .iter()
                .chain(&constraint.rotation_rad)
                .copied()
                .enumerate()
            {
                if let Some(value) = value {
                    let dof = layout
                        .dof(constraint.node, axis)
                        .ok_or(FeaError::InvalidConstraint)?;
                    prescribed.insert(dof, value);
                }
            }
        }
        let constraint_scale = stiffness.largest_magnitude().max(1.0);
        for (&dof, &value) in &prescribed {
            stiffness.prescribe(dof, value, constraint_scale, forces);
        }
        if prescribed.is_empty() {
            return Err(FeaError::MissingConstraint);
        }
        Ok(prescribed.into_keys().collect())
    }

    /// Each element's result, and the largest move inside any frame element.
    fn element_results(
        &self,
        layout: &DofLayout,
        displacement: &[f64],
    ) -> Result<(Vec<FeaElementResult>, f64), FeaError> {
        let frame_loads = self.frame_loads();
        let mut frame_peak_mm: f64 = 0.0;
        let results = self
            .elements
            .iter()
            .map(|element| {
                Ok(match element.kind {
                    FeaElementKind::LinearTruss2 {
                        nodes,
                        material_id,
                        area_mm2,
                    } => {
                        let material = self.material(material_id);
                        let (length, direction) =
                            self.truss_geometry(nodes).expect("validated truss");
                        let relative_displacement = [
                            displacement[nodes[1] * 3] - displacement[nodes[0] * 3],
                            displacement[nodes[1] * 3 + 1] - displacement[nodes[0] * 3 + 1],
                            displacement[nodes[1] * 3 + 2] - displacement[nodes[0] * 3 + 2],
                        ];
                        let strain = dot(relative_displacement, direction) / length;
                        let stress = material.youngs_modulus_mpa * strain;
                        FeaElementResult {
                            element_id: element.id,
                            strain_energy_n_mm: 0.5 * stress * strain * area_mm2 * length,
                            von_mises_stress_mpa: stress.abs(),
                            yield_utilization: material
                                .yield_strength_mpa
                                .map(|limit| stress.abs() / limit),
                        }
                    }
                    FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                        let material = self.material(material_id);
                        let (volume, gradients) = self
                            .tetrahedron_geometry(nodes)
                            .expect("validated tetrahedron");
                        let b = strain_displacement(gradients);
                        let d = elasticity(material);
                        let mut local_displacement = [0.0; 12];
                        for local in 0..12 {
                            local_displacement[local] =
                                displacement[nodes[local / 3] * 3 + local % 3];
                        }
                        let strain = matrix_vector6x12(b, local_displacement);
                        let stress = matrix_vector6(d, strain);
                        let von_mises = von_mises(stress);
                        FeaElementResult {
                            element_id: element.id,
                            strain_energy_n_mm: 0.5 * volume * dot6(strain, stress),
                            von_mises_stress_mpa: von_mises,
                            yield_utilization: material
                                .yield_strength_mpa
                                .map(|limit| von_mises / limit),
                        }
                    }
                    FeaElementKind::Frame2 {
                        nodes,
                        material_id,
                        section,
                        orientation,
                    } => {
                        let material = self.material(material_id);
                        let frame = self.element_axes(element.id, nodes, orientation)?;
                        let stiffness =
                            self.frame_local_stiffness(material_id, &section, frame.length_mm);
                        let dofs = layout.frame_dofs(element.id, nodes)?;
                        let local_displacement = frame.local12(dofs.map(|dof| displacement[dof]));
                        let internal = multiply(&stiffness, local_displacement);
                        // A frame element without a load of its own carries none along it.
                        let load = frame_loads
                            .get(&element.id)
                            .map_or([0.0; 3], |load| frame.local(*load));
                        // What the nodes put on the element: its stiffness response
                        // less the share of the load it carries to them.
                        let equivalent = consistent_load(load, frame.length_mm);
                        let mut end_forces = internal;
                        for (value, share) in end_forces.iter_mut().zip(equivalent) {
                            *value -= share;
                        }
                        let von_mises = peak_von_mises(&section, end_forces, load, frame.length_mm);
                        frame_peak_mm = frame_peak_mm.max(peak_displacement(
                            material.youngs_modulus_mpa,
                            &section,
                            local_displacement,
                            load,
                            frame.length_mm,
                        ));
                        FeaElementResult {
                            element_id: element.id,
                            strain_energy_n_mm: 0.5
                                * local_displacement
                                    .iter()
                                    .zip(internal)
                                    .map(|(moved, force)| moved * force)
                                    .sum::<f64>(),
                            von_mises_stress_mpa: von_mises,
                            yield_utilization: material
                                .yield_strength_mpa
                                .map(|limit| von_mises / limit),
                        }
                    }
                })
            })
            .collect::<Result<Vec<_>, FeaError>>()?;
        Ok((results, frame_peak_mm))
    }

    fn characteristic_length(&self) -> f64 {
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        for node in &self.nodes {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(node.position_mm[axis]);
                maximum[axis] = maximum[axis].max(node.position_mm[axis]);
            }
        }
        length(sub(maximum, minimum)).max(ROUNDING)
    }

    fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        let mut minimum = [f64::INFINITY; 3];
        let mut maximum = [f64::NEG_INFINITY; 3];
        for node in &self.nodes {
            for axis in 0..3 {
                minimum[axis] = minimum[axis].min(node.position_mm[axis]);
                maximum[axis] = maximum[axis].max(node.position_mm[axis]);
            }
        }
        (minimum, maximum)
    }

    fn load_definition(&self) -> LoadDefinition {
        let mut definition = LoadDefinition::default();
        for load in &self.loads {
            match *load {
                FeaLoad::NodalForce { node, force_n } => definition.nodal.push((
                    self.nodes[node].position_mm.map(f64::to_bits),
                    force_n.map(f64::to_bits),
                )),
                FeaLoad::SurfaceTraction {
                    nodes,
                    traction_n_per_mm2,
                } => {
                    definition
                        .surface
                        .entry(traction_n_per_mm2.map(f64::to_bits))
                        .or_default()
                        .add_triangle(
                            self.nodes[nodes[0]].position_mm,
                            self.nodes[nodes[1]].position_mm,
                            self.nodes[nodes[2]].position_mm,
                        );
                }
                FeaLoad::NodalMoment { node, moment_n_mm } => definition.moments.push((
                    self.nodes[node].position_mm.map(f64::to_bits),
                    moment_n_mm.map(f64::to_bits),
                )),
                FeaLoad::FrameDistributed {
                    element,
                    load_n_per_mm,
                } => {
                    if let Some(FeaElementKind::Frame2 { nodes, .. }) = self
                        .elements
                        .iter()
                        .find(|candidate| candidate.id == element)
                        .map(|candidate| &candidate.kind)
                    {
                        definition
                            .lines
                            .entry(load_n_per_mm.map(f64::to_bits))
                            .or_default()
                            .add_segment(
                                self.nodes[nodes[0]].position_mm,
                                self.nodes[nodes[1]].position_mm,
                            );
                    }
                }
            }
        }
        definition.nodal.sort_unstable();
        definition.moments.sort_unstable();
        definition
    }

    fn constraint_definition(&self) -> ConstraintDefinition {
        // Axes 0–2 are moves, 3–5 turns; a held turn is kept as a point.
        let mut prescribed = vec![[None; 6]; self.nodes.len()];
        let mut definition = ConstraintDefinition::new();
        for constraint in &self.constraints {
            for (axis, value) in constraint
                .displacement_mm
                .iter()
                .chain(&constraint.rotation_rad)
                .copied()
                .enumerate()
            {
                if let Some(value) = value {
                    prescribed[constraint.node][axis] = Some(value);
                    definition.entry((axis, value.to_bits())).or_default();
                }
            }
        }

        let mut covered = BTreeSet::new();
        for element in &self.elements {
            if let FeaElementKind::LinearTruss2 { nodes, .. }
            | FeaElementKind::Frame2 { nodes, .. } = element.kind
            {
                for (axis, left) in prescribed[nodes[0]].iter().copied().enumerate().take(3) {
                    if let (Some(left), Some(right)) = (left, prescribed[nodes[1]][axis])
                        && left.to_bits() == right.to_bits()
                    {
                        definition
                            .get_mut(&(axis, left.to_bits()))
                            .expect("constraint definition exists")
                            .lines
                            .add_segment(
                                self.nodes[nodes[0]].position_mm,
                                self.nodes[nodes[1]].position_mm,
                            );
                        covered.insert((nodes[0], axis));
                        covered.insert((nodes[1], axis));
                    }
                }
            }
        }

        let mut face_counts = BTreeMap::<[usize; 3], usize>::new();
        for element in &self.elements {
            if let FeaElementKind::LinearTetrahedron4 { nodes, .. } = element.kind {
                for face in tetrahedron_faces_of(nodes) {
                    *face_counts.entry(face).or_default() += 1;
                }
            }
        }
        for (nodes, count) in face_counts {
            if count != 1 {
                continue;
            }
            for (axis, first) in prescribed[nodes[0]].iter().copied().enumerate().take(3) {
                if let (Some(first), Some(second), Some(third)) = (
                    first,
                    prescribed[nodes[1]][axis],
                    prescribed[nodes[2]][axis],
                ) && first.to_bits() == second.to_bits()
                    && first.to_bits() == third.to_bits()
                {
                    definition
                        .get_mut(&(axis, first.to_bits()))
                        .expect("constraint definition exists")
                        .surfaces
                        .add_triangle(
                            self.nodes[nodes[0]].position_mm,
                            self.nodes[nodes[1]].position_mm,
                            self.nodes[nodes[2]].position_mm,
                        );
                    for node in nodes {
                        covered.insert((node, axis));
                    }
                }
            }
        }

        for (node, values) in prescribed.into_iter().enumerate() {
            for (axis, value) in values.into_iter().enumerate() {
                if let Some(value) = value
                    && !covered.contains(&(node, axis))
                {
                    definition
                        .get_mut(&(axis, value.to_bits()))
                        .expect("constraint definition exists")
                        .points
                        .push(self.nodes[node].position_mm.map(f64::to_bits));
                }
            }
        }
        for support in definition.values_mut() {
            support.points.sort_unstable();
        }
        definition
    }

    #[must_use]
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        hash_text(&mut hasher, &self.schema);
        hash_text(&mut hasher, &self.case_id);
        for node in &self.nodes {
            hash_f64s(&mut hasher, &node.position_mm);
        }
        for material in &self.materials {
            hasher.update(material.id.to_le_bytes());
            hash_f64s(
                &mut hasher,
                &[material.youngs_modulus_mpa, material.poisson_ratio],
            );
            match material.yield_strength_mpa {
                Some(value) => {
                    hasher.update([1]);
                    hasher.update(value.to_bits().to_le_bytes());
                }
                None => hasher.update([0]),
            }
        }
        for element in &self.elements {
            hasher.update(element.id.to_le_bytes());
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => {
                    hasher.update([1]);
                    for node in nodes {
                        hasher.update((node as u64).to_le_bytes());
                    }
                    hasher.update(material_id.to_le_bytes());
                    hasher.update(area_mm2.to_bits().to_le_bytes());
                }
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                    hasher.update([2]);
                    for node in nodes {
                        hasher.update((node as u64).to_le_bytes());
                    }
                    hasher.update(material_id.to_le_bytes());
                }
                FeaElementKind::Frame2 {
                    nodes,
                    material_id,
                    section,
                    orientation,
                } => {
                    hasher.update([3]);
                    for node in nodes {
                        hash_index(&mut hasher, node);
                    }
                    hasher.update(material_id.to_le_bytes());
                    hash_f64s(
                        &mut hasher,
                        &[
                            section.area_mm2,
                            section.second_moment_y_mm4,
                            section.second_moment_z_mm4,
                            section.torsion_constant_mm4,
                            section.extreme_fibre_y_mm,
                            section.extreme_fibre_z_mm,
                            section.torsion_modulus_mm3,
                        ],
                    );
                    hash_f64s(&mut hasher, &orientation);
                }
            }
        }
        for constraint in &self.constraints {
            hasher.update((constraint.node as u64).to_le_bytes());
            for value in constraint.displacement_mm {
                match value {
                    Some(value) => {
                        hasher.update([1]);
                        hasher.update(value.to_bits().to_le_bytes());
                    }
                    None => hasher.update([0]),
                }
            }
        }
        for load in &self.loads {
            match *load {
                FeaLoad::NodalForce { node, force_n } => {
                    hasher.update([1]);
                    hasher.update((node as u64).to_le_bytes());
                    hash_f64s(&mut hasher, &force_n);
                }
                FeaLoad::SurfaceTraction {
                    nodes,
                    traction_n_per_mm2,
                } => {
                    hasher.update([2]);
                    for node in nodes {
                        hasher.update((node as u64).to_le_bytes());
                    }
                    hash_f64s(&mut hasher, &traction_n_per_mm2);
                }
                FeaLoad::NodalMoment { node, moment_n_mm } => {
                    hasher.update([3]);
                    hash_index(&mut hasher, node);
                    hash_f64s(&mut hasher, &moment_n_mm);
                }
                FeaLoad::FrameDistributed {
                    element,
                    load_n_per_mm,
                } => {
                    hasher.update([4]);
                    hasher.update(element.to_le_bytes());
                    hash_f64s(&mut hasher, &load_n_per_mm);
                }
            }
        }
        // Held turns come last, and only when there are any, so a model
        // without frame elements keeps the digest it had before they existed.
        let turns = self
            .constraints
            .iter()
            .enumerate()
            .filter(|(_, constraint)| constraint.rotation_rad.iter().any(Option::is_some))
            .collect::<Vec<_>>();
        if !turns.is_empty() {
            hasher.update(b"rotations");
            for (index, constraint) in turns {
                hash_index(&mut hasher, index);
                for value in constraint.rotation_rad {
                    match value {
                        Some(value) => {
                            hasher.update([1]);
                            hasher.update(value.to_bits().to_le_bytes());
                        }
                        None => hasher.update([0]),
                    }
                }
            }
        }
        // Densities likewise come last and only when there are any.
        if self
            .materials
            .iter()
            .any(|material| material.density_kg_per_m3.is_some())
        {
            hasher.update(b"densities");
            for material in &self.materials {
                match material.density_kg_per_m3 {
                    Some(value) => {
                        hasher.update([1]);
                        hasher.update(value.to_bits().to_le_bytes());
                    }
                    None => hasher.update([0]),
                }
            }
        }
        // Couplings likewise come last and only when there are any.
        if !self.couplings.is_empty() {
            hasher.update(b"couplings");
            for coupling in &self.couplings {
                for node in coupling.nodes {
                    hash_index(&mut hasher, node);
                }
                match coupling.kind {
                    FeaCouplingKind::Rigid => hasher.update([1]),
                    FeaCouplingKind::Spring {
                        translation_n_per_mm,
                        rotation_n_mm_per_rad,
                    } => {
                        hasher.update([2]);
                        hash_f64s(&mut hasher, &translation_n_per_mm);
                        hash_f64s(&mut hasher, &rotation_n_mm_per_rad);
                    }
                    FeaCouplingKind::Tie { others, weights } => {
                        hasher.update([3]);
                        for node in others {
                            hash_index(&mut hasher, node);
                        }
                        hash_f64s(&mut hasher, &weights);
                    }
                }
            }
        }
        format!("{:x}", hasher.finalize())
    }
}

pub fn solve_convergence_study(
    levels: &[FeaModel],
    settings: FeaSolveSettings,
) -> Result<FeaConvergenceEvidence, FeaConvergenceError> {
    if levels.len() < 2 {
        return Err(FeaConvergenceError::TooFewLevels);
    }
    for level in levels {
        level
            .validate(settings)
            .map_err(FeaConvergenceError::Solve)?;
    }
    let case_id = levels[0].case_id.clone();
    let bounds = levels[0].bounds();
    let materials = &levels[0].materials;
    let load_definition = levels[0].load_definition();
    let constraint_definition = levels[0].constraint_definition();
    for pair in levels.windows(2) {
        if pair[1].case_id != case_id {
            return Err(FeaConvergenceError::CaseMismatch);
        }
        if pair[1].elements.len() <= pair[0].elements.len() {
            return Err(FeaConvergenceError::MeshNotRefined);
        }
    }
    if levels
        .iter()
        .skip(1)
        .any(|level| !near_bounds(level.bounds(), bounds))
    {
        return Err(FeaConvergenceError::DomainMismatch);
    }
    if levels
        .iter()
        .skip(1)
        .any(|level| level.materials.as_slice() != materials.as_slice())
    {
        return Err(FeaConvergenceError::MaterialMismatch);
    }
    let material_distribution = levels[0].material_distribution();
    if levels.iter().skip(1).any(|level| {
        let candidate = level.material_distribution();
        candidate.len() != material_distribution.len()
            || candidate.iter().any(|(material_id, moments)| {
                material_distribution
                    .get(material_id)
                    .is_none_or(|expected| !same_geometric_moments(moments, expected))
            })
    }) {
        return Err(FeaConvergenceError::MaterialMismatch);
    }
    if levels
        .iter()
        .skip(1)
        .any(|level| !same_load_definition(&level.load_definition(), &load_definition))
    {
        return Err(FeaConvergenceError::LoadMismatch);
    }
    if levels.iter().skip(1).any(|level| {
        !same_constraint_definition(&level.constraint_definition(), &constraint_definition)
    }) {
        return Err(FeaConvergenceError::ConstraintMismatch);
    }
    let joints = joint_definition(&levels[0]);
    if levels
        .iter()
        .skip(1)
        .any(|level| joint_definition(level) != joints)
    {
        return Err(FeaConvergenceError::ConstraintMismatch);
    }
    let solutions = levels
        .iter()
        .map(|level| level.solve(settings).map_err(FeaConvergenceError::Solve))
        .collect::<Result<Vec<_>, _>>()?;
    let last = levels.len() - 1;
    let judgement = FeaConvergenceJudgement::between(
        (&solutions[last - 1], levels[last - 1].elements.len()),
        (&solutions[last], levels[last].elements.len()),
        settings.convergence_relative_tolerance,
    );
    Ok(FeaConvergenceEvidence {
        case_id,
        levels: levels
            .iter()
            .zip(&solutions)
            .map(|(model, solution)| FeaConvergenceLevel {
                node_count: model.nodes.len(),
                element_count: model.elements.len(),
                maximum_displacement_mm: solution.maximum_displacement_mm,
                total_strain_energy_n_mm: solution.total_strain_energy_n_mm,
            })
            .collect(),
        judgement,
        required_relative_tolerance: settings.convergence_relative_tolerance,
        solutions,
    })
}

/// [`Joints::reduce`] with the reduced entries taken from `budget` while the
/// entries they come from are still held; those are given back after. `Err`
/// says how much all would need.
pub(crate) fn reduce_within(
    joints: &Joints,
    entries: Vec<(usize, usize, f64)>,
    forces: &mut [f64],
    budget: &mut MemoryBudget,
) -> Result<Vec<(usize, usize, f64)>, usize> {
    let Some(count) = joints.reduced_len(&entries) else {
        return Ok(entries);
    };
    budget.take(count.saturating_mul(ENTRY_BYTES))?;
    budget.give(entries.capacity() * ENTRY_BYTES);
    Ok(joints.reduce(entries, forces))
}

/// The matrix of `entries`, taken from `budget` while they are still held;
/// they are given back once it is built.
pub(crate) fn matrix_within(
    size: usize,
    entries: Vec<(usize, usize, f64)>,
    budget: &mut MemoryBudget,
    settings: FeaSolveSettings,
) -> Result<SymmetricMatrix, FeaError> {
    let most = SymmetricMatrix::bytes_for(size, entries.len());
    budget
        .take(most)
        .map_err(|needed| FeaError::too_large(needed, settings))?;
    let entry_bytes = entries.capacity() * ENTRY_BYTES;
    let matrix = SymmetricMatrix::from_entries(size, entries);
    budget.give((entry_bytes + most).saturating_sub(matrix.bytes()));
    Ok(matrix)
}

fn elasticity(material: &FeaMaterial) -> [[f64; 6]; 6] {
    let young = material.youngs_modulus_mpa;
    let poisson = material.poisson_ratio;
    let lambda = young * poisson / ((1.0 + poisson) * (1.0 - 2.0 * poisson));
    let shear = young / (2.0 * (1.0 + poisson));
    let mut matrix = [[0.0; 6]; 6];
    for (row, values) in matrix.iter_mut().enumerate().take(3) {
        for (column, value) in values.iter_mut().enumerate().take(3) {
            *value = if row == column {
                lambda + 2.0 * shear
            } else {
                lambda
            };
        }
    }
    matrix[3][3] = shear;
    matrix[4][4] = shear;
    matrix[5][5] = shear;
    matrix
}

fn strain_displacement(gradients: [[f64; 3]; 4]) -> [[f64; 12]; 6] {
    let mut matrix = [[0.0; 12]; 6];
    for (node, [x, y, z]) in gradients.into_iter().enumerate() {
        let column = node * 3;
        matrix[0][column] = x;
        matrix[1][column + 1] = y;
        matrix[2][column + 2] = z;
        matrix[3][column] = y;
        matrix[3][column + 1] = x;
        matrix[4][column + 1] = z;
        matrix[4][column + 2] = y;
        matrix[5][column] = z;
        matrix[5][column + 2] = x;
    }
    matrix
}

fn tetrahedron_faces_of(nodes: [usize; 4]) -> [[usize; 3]; 4] {
    let mut faces = [
        [nodes[0], nodes[1], nodes[2]],
        [nodes[0], nodes[1], nodes[3]],
        [nodes[0], nodes[2], nodes[3]],
        [nodes[1], nodes[2], nodes[3]],
    ];
    for face in &mut faces {
        face.sort_unstable();
    }
    faces
}

fn matrix_vector6(matrix: [[f64; 6]; 6], vector: [f64; 6]) -> [f64; 6] {
    let mut result = [0.0; 6];
    for row in 0..6 {
        result[row] = (0..6)
            .map(|column| matrix[row][column] * vector[column])
            .sum();
    }
    result
}

fn matrix_vector6x12(matrix: [[f64; 12]; 6], vector: [f64; 12]) -> [f64; 6] {
    let mut result = [0.0; 6];
    for row in 0..6 {
        result[row] = (0..12)
            .map(|column| matrix[row][column] * vector[column])
            .sum();
    }
    result
}

fn vector_sum(vector: &[f64]) -> [f64; 3] {
    let mut sum = [0.0; 3];
    for (index, value) in vector.iter().copied().enumerate() {
        sum[index % 3] += value;
    }
    sum
}

fn von_mises(stress: [f64; 6]) -> f64 {
    let normal = 0.5
        * ((stress[0] - stress[1]).powi(2)
            + (stress[1] - stress[2]).powi(2)
            + (stress[2] - stress[0]).powi(2));
    (normal + 3.0 * (stress[3].powi(2) + stress[4].powi(2) + stress[5].powi(2))).sqrt()
}

fn triangle_area(first: [f64; 3], second: [f64; 3], third: [f64; 3]) -> f64 {
    length(cross(sub(second, first), sub(third, first))) * 0.5
}

impl GeometricMoments {
    fn add_segment(&mut self, first: [f64; 3], second: [f64; 3]) {
        self.add_weighted_segment(first, second, 1.0);
    }

    fn add_weighted_segment(&mut self, first: [f64; 3], second: [f64; 3], weight: f64) {
        let measure = length(sub(second, first)) * weight;
        self.measure += measure;
        for axis in 0..3 {
            self.first[axis] += measure * (first[axis] + second[axis]) / 2.0;
            for other in 0..3 {
                self.second[axis * 3 + other] += measure
                    * (2.0 * first[axis] * first[other]
                        + first[axis] * second[other]
                        + second[axis] * first[other]
                        + 2.0 * second[axis] * second[other])
                    / 6.0;
            }
        }
    }

    fn add_tetrahedron(&mut self, vertices: [[f64; 3]; 4]) {
        let volume = Mat3::from_rows([
            sub(vertices[1], vertices[0]),
            sub(vertices[2], vertices[0]),
            sub(vertices[3], vertices[0]),
        ])
        .determinant()
        .abs()
            / 6.0;
        self.measure += volume;
        for axis in 0..3 {
            let sum = vertices.iter().map(|vertex| vertex[axis]).sum::<f64>();
            self.first[axis] += volume * sum / 4.0;
            for other in 0..3 {
                let other_sum = vertices.iter().map(|vertex| vertex[other]).sum::<f64>();
                let diagonal = vertices
                    .iter()
                    .map(|vertex| vertex[axis] * vertex[other])
                    .sum::<f64>();
                self.second[axis * 3 + other] += volume * (sum * other_sum + diagonal) / 20.0;
            }
        }
    }

    fn add_triangle(&mut self, first: [f64; 3], second: [f64; 3], third: [f64; 3]) {
        let measure = triangle_area(first, second, third);
        self.measure += measure;
        for axis in 0..3 {
            let sum = first[axis] + second[axis] + third[axis];
            self.first[axis] += measure * sum / 3.0;
            for other in 0..3 {
                let other_sum = first[other] + second[other] + third[other];
                let diagonal = first[axis] * first[other]
                    + second[axis] * second[other]
                    + third[axis] * third[other];
                self.second[axis * 3 + other] += measure * (sum * other_sum + diagonal) / 12.0;
            }
        }
    }
}

fn same_load_definition(left: &LoadDefinition, right: &LoadDefinition) -> bool {
    let same_spread = |left: &BTreeMap<[u64; 3], GeometricMoments>,
                       right: &BTreeMap<[u64; 3], GeometricMoments>| {
        left.len() == right.len()
            && left.iter().all(|(load, moments)| {
                right
                    .get(load)
                    .is_some_and(|other| same_geometric_moments(moments, other))
            })
    };
    left.nodal == right.nodal
        && left.moments == right.moments
        && same_spread(&left.surface, &right.surface)
        && same_spread(&left.lines, &right.lines)
}

fn same_constraint_definition(left: &ConstraintDefinition, right: &ConstraintDefinition) -> bool {
    left.len() == right.len()
        && left.iter().all(|(key, support)| {
            right.get(key).is_some_and(|other| {
                support.points == other.points
                    && same_geometric_moments(&support.lines, &other.lines)
                    && same_geometric_moments(&support.surfaces, &other.surfaces)
            })
        })
}

fn same_geometric_moments(left: &GeometricMoments, right: &GeometricMoments) -> bool {
    near_relative(left.measure, right.measure)
        && left
            .first
            .into_iter()
            .zip(right.first)
            .all(|(left, right)| near_relative(left, right))
        && left
            .second
            .into_iter()
            .zip(right.second)
            .all(|(left, right)| near_relative(left, right))
}

fn near_relative(left: f64, right: f64) -> bool {
    (left - right).abs() <= ROUNDING * left.abs().max(right.abs()).max(1.0)
}

fn relative_change(previous: f64, current: f64) -> f64 {
    (current - previous).abs() / current.abs().max(previous.abs()).max(NEGLIGIBLE)
}

fn near_bounds(left: ([f64; 3], [f64; 3]), right: ([f64; 3], [f64; 3])) -> bool {
    near3(left.0, right.0, ROUNDING) && near3(left.1, right.1, ROUNDING)
}

fn near3(left: [f64; 3], right: [f64; 3], tolerance: f64) -> bool {
    left.into_iter()
        .zip(right)
        .all(|(left, right)| (left - right).abs() <= tolerance)
}

fn finite_positive(value: f64) -> bool {
    value.is_finite() && value > 0.0
}

fn finite3(value: [f64; 3]) -> bool {
    value.into_iter().all(f64::is_finite)
}

fn hash_text(hasher: &mut Sha256, value: &str) {
    hasher.update((value.len() as u64).to_le_bytes());
    hasher.update(value.as_bytes());
}

fn hash_index(hasher: &mut Sha256, index: usize) {
    // An index past 64 bits cannot occur on a platform the crate builds for.
    hasher.update(u64::try_from(index).unwrap_or(u64::MAX).to_le_bytes());
}

fn hash_f64s(hasher: &mut Sha256, values: &[f64]) {
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
}

fn dot6(left: [f64; 6], right: [f64; 6]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}
