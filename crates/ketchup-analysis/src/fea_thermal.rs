//! Steady heat conduction on linear tetrahedra: conductivity per material,
//! held temperatures at nodes, and heat flux or convection on boundary
//! triangles. Lengths are in millimetres, conductivity in W/(m·K), flux in
//! W/m² and film coefficients in W/(m²·K), as engineers quote them.

use crate::fea::{FeaNode, MAX_FEA_ELEMENTS, MAX_FEA_LOADS, MAX_FEA_MATERIALS, MAX_FEA_NODES};
use crate::sparse::{
    ENTRY_BYTES, FactorError, MemoryBudget, SymmetricMatrix, distinct_positions, sort_entries,
};
use ketchup_geometry::linalg::{Mat3, Singular, cross, dot, length, sub};
use ketchup_model::tolerance::NEGLIGIBLE;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::AtomicBool;

pub const THERMAL_MODEL_SCHEMA_V1: &str = "ketchup.thermal-model.v1";
pub const THERMAL_SOLUTION_SCHEMA_V1: &str = "ketchup.thermal-solution.v1";
pub const THERMAL_SOLVER_VERSION_V1: &str = "ketchup.steady-conduction-fem.v1";
/// The memory the factored conduction matrix may take by default: 512 MiB.
pub const DEFAULT_THERMAL_FACTOR_BYTES: usize = 512 << 20;
/// Absolute zero in °C; no held or ambient temperature may be below it.
pub const ABSOLUTE_ZERO_C: f64 = -273.15;

/// Millimetres to metres, and square millimetres to square metres.
const PER_MM: f64 = 1.0 / 1000.0;
const PER_MM2: f64 = PER_MM * PER_MM;

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalMaterial {
    pub id: u64,
    pub conductivity_w_per_m_k: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalElement {
    pub id: u64,
    pub nodes: [usize; 4],
    pub material_id: u64,
}

/// A node held at a temperature.
#[derive(Clone, Debug, PartialEq)]
pub struct ThermalTemperature {
    pub node: usize,
    pub temperature_c: f64,
}

/// A condition on one boundary triangle of the mesh.
#[derive(Clone, Debug, PartialEq)]
pub enum ThermalBoundary {
    /// Heat entering the body through the triangle (negative leaves it).
    HeatFlux {
        nodes: [usize; 3],
        flux_w_per_m2: f64,
    },
    /// Heat exchanged with a surrounding fluid: `h (T∞ − T)` enters the body.
    Convection {
        nodes: [usize; 3],
        film_coefficient_w_per_m2_k: f64,
        ambient_c: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalModel {
    pub schema: String,
    pub case_id: String,
    pub nodes: Vec<FeaNode>,
    pub materials: Vec<ThermalMaterial>,
    pub elements: Vec<ThermalElement>,
    pub temperatures: Vec<ThermalTemperature>,
    pub boundaries: Vec<ThermalBoundary>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ThermalSolveSettings {
    pub maximum_nodes: usize,
    pub relative_pivot_tolerance: f64,
    /// The memory the factored matrix may take; a larger model is refused
    /// with the size it would need.
    pub maximum_factor_bytes: usize,
}

impl Default for ThermalSolveSettings {
    fn default() -> Self {
        Self {
            maximum_nodes: MAX_FEA_NODES,
            relative_pivot_tolerance: NEGLIGIBLE,
            maximum_factor_bytes: DEFAULT_THERMAL_FACTOR_BYTES,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThermalError {
    UnsupportedSchema,
    EmptyCaseId,
    EmptyMesh,
    ModelTooLarge,
    InvalidNode,
    DuplicateMaterialId,
    InvalidMaterial,
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
    /// A held temperature names no node, is not finite or is below absolute zero.
    InvalidTemperature,
    /// One node is held at two different temperatures.
    ConflictingTemperature,
    /// A flux or film value is not finite, a film coefficient is not
    /// positive, or the triangle is degenerate.
    InvalidBoundary,
    /// A flux or convection triangle is not a face of exactly one element.
    NonBoundaryFace,
    /// Nothing sets the temperature level: no held node and no convection.
    NoReferenceTemperature,
    InvalidSettings,
    /// A part of the mesh has no held node and no convection.
    SingularSystem,
    FactorTooLarge {
        needed_bytes: usize,
        budget_bytes: usize,
    },
    NonFiniteSolution,
    /// The caller's cancellation flag was set while solving.
    Cancelled,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalElementResult {
    pub element_id: u64,
    pub temperature_gradient_k_per_mm: [f64; 3],
    /// `−k ∇T`: where heat flows in the element.
    pub heat_flux_w_per_m2: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThermalSolution {
    pub schema: &'static str,
    pub solver_version: &'static str,
    pub case_id: String,
    pub model_digest: String,
    pub temperatures_c: Vec<f64>,
    /// Heat supplied to the body at each held node (negative is removed);
    /// `None` at a node that is not held.
    pub held_node_heat_w: Vec<Option<f64>>,
    pub element_results: Vec<ThermalElementResult>,
    pub minimum_temperature_c: f64,
    pub maximum_temperature_c: f64,
    pub heat_into_held_nodes_w: f64,
    pub heat_out_of_held_nodes_w: f64,
    pub flux_heat_in_w: f64,
    pub convection_heat_in_w: f64,
    /// Heat in through held nodes, flux and convection together; zero
    /// within rounding for a solved steady state.
    pub heat_balance_w: f64,
    pub maximum_free_node_residual_w: f64,
    pub excluded_physics: [&'static str; 5],
}

/// Solves steady conduction `K T = f` over the model.
///
/// # Errors
/// Each invalid input is refused by name; a mesh part without a held node
/// or convection is singular; a factor past the memory budget is refused
/// with the size it would need.
pub fn solve_steady_conduction(
    model: &ThermalModel,
    settings: ThermalSolveSettings,
) -> Result<ThermalSolution, ThermalError> {
    solve_steady_conduction_with_cancellation(model, settings, &AtomicBool::new(false))
}

/// [`solve_steady_conduction`], stopping with [`ThermalError::Cancelled`]
/// soon after `cancelled` is set.
///
/// # Errors
/// As [`solve_steady_conduction`], or cancelled.
pub fn solve_steady_conduction_with_cancellation(
    model: &ThermalModel,
    settings: ThermalSolveSettings,
    cancelled: &AtomicBool,
) -> Result<ThermalSolution, ThermalError> {
    validate_settings(settings)?;
    let geometry = model.validate(settings)?;
    let held = model.held_temperatures()?;
    let boundary_faces = model.boundary_faces(&geometry);
    model.validate_boundaries(&boundary_faces)?;
    let has_film = model
        .boundaries
        .iter()
        .any(|boundary| matches!(boundary, ThermalBoundary::Convection { .. }));
    if held.is_empty() && !has_film {
        return Err(ThermalError::NoReferenceTemperature);
    }

    let count = model.nodes.len();
    let conductivity = model
        .materials
        .iter()
        .map(|material| (material.id, material.conductivity_w_per_m_k))
        .collect::<BTreeMap<_, _>>();
    // The entries, both matrices and the factor are taken from the budget
    // before each is allocated, as for stiffness (R1121ac P3: the entries and
    // matrices were built whole before the factor's own check).
    let too_large = |needed_bytes| ThermalError::FactorTooLarge {
        needed_bytes,
        budget_bytes: settings.maximum_factor_bytes,
    };
    let mut budget = MemoryBudget::new(settings.maximum_factor_bytes);
    let films = model
        .boundaries
        .iter()
        .filter(|boundary| matches!(boundary, ThermalBoundary::Convection { .. }))
        .count();
    let entry_count = model
        .elements
        .len()
        .saturating_mul(16)
        .saturating_add(films.saturating_mul(9));
    budget
        .take(entry_count.saturating_mul(ENTRY_BYTES))
        .map_err(too_large)?;
    let mut entries = Vec::with_capacity(entry_count);
    for (element, &(volume, gradients)) in model.elements.iter().zip(&geometry) {
        let scale = conductivity[&element.material_id] * PER_MM * volume;
        for (row, left) in gradients.iter().enumerate() {
            for (column, right) in gradients.iter().enumerate() {
                entries.push((
                    element.nodes[row],
                    element.nodes[column],
                    scale * dot(*left, *right),
                ));
            }
        }
    }
    let mut heat = vec![0.0; count];
    for boundary in &model.boundaries {
        match *boundary {
            ThermalBoundary::HeatFlux {
                nodes,
                flux_w_per_m2,
            } => {
                let share = flux_w_per_m2 * PER_MM2 * model.triangle_area(nodes) / 3.0;
                for node in nodes {
                    heat[node] += share;
                }
            }
            ThermalBoundary::Convection {
                nodes,
                film_coefficient_w_per_m2_k,
                ambient_c,
            } => {
                let film = film_coefficient_w_per_m2_k * PER_MM2 * model.triangle_area(nodes);
                for (row, &left) in nodes.iter().enumerate() {
                    heat[left] += film * ambient_c / 3.0;
                    for (column, &right) in nodes.iter().enumerate() {
                        let weight = if row == column { 2.0 } else { 1.0 };
                        entries.push((left, right, film * weight / 12.0));
                    }
                }
            }
        }
    }

    sort_entries(&mut entries);
    let matrix_bytes =
        SymmetricMatrix::bytes_for(count, distinct_positions(entries.iter().copied()));
    budget
        .take(matrix_bytes.saturating_mul(2))
        .map_err(too_large)?;
    let original = SymmetricMatrix::from_sorted(count, entries.iter().copied());
    budget.give(entries.capacity() * ENTRY_BYTES);
    drop(entries);
    let mut matrix = original.clone();
    let mut rhs = heat.clone();
    let scale = original.largest_magnitude();
    for (&node, &temperature) in &held {
        matrix.prescribe(node, temperature, scale, &mut rhs);
    }
    let threshold = scale * settings.relative_pivot_tolerance;
    let temperatures = matrix
        .factor(threshold, budget.left(), cancelled)
        .map_err(|error| match error {
            FactorError::Singular => ThermalError::SingularSystem,
            FactorError::Cancelled => ThermalError::Cancelled,
            FactorError::TooLarge { needed_bytes } => {
                too_large(needed_bytes.saturating_add(budget.held()))
            }
        })?
        .solve(&rhs);
    if temperatures.iter().any(|value| !value.is_finite()) {
        return Err(ThermalError::NonFiniteSolution);
    }

    let residual = original
        .multiply(&temperatures)
        .into_iter()
        .zip(&heat)
        .map(|(conducted, applied)| conducted - applied)
        .collect::<Vec<_>>();
    let mut held_node_heat_w = vec![None; count];
    let mut heat_into_held_nodes_w = 0.0;
    let mut heat_out_of_held_nodes_w = 0.0;
    let mut maximum_free_node_residual_w: f64 = 0.0;
    for (node, value) in residual.iter().copied().enumerate() {
        if held.contains_key(&node) {
            held_node_heat_w[node] = Some(value);
            if value > 0.0 {
                heat_into_held_nodes_w += value;
            } else {
                heat_out_of_held_nodes_w -= value;
            }
        } else {
            maximum_free_node_residual_w = maximum_free_node_residual_w.max(value.abs());
        }
    }
    let (flux_heat_in_w, convection_heat_in_w) = model.boundary_heat(&temperatures);

    let element_results = model
        .elements
        .iter()
        .zip(&geometry)
        .map(|(element, (_, gradients))| {
            let mut gradient = [0.0; 3];
            for (node, node_gradient) in element.nodes.iter().zip(gradients) {
                for axis in 0..3 {
                    gradient[axis] += temperatures[*node] * node_gradient[axis];
                }
            }
            let k = conductivity[&element.material_id];
            ThermalElementResult {
                element_id: element.id,
                temperature_gradient_k_per_mm: gradient,
                heat_flux_w_per_m2: gradient.map(|value| -k * value / PER_MM),
            }
        })
        .collect();
    Ok(ThermalSolution {
        schema: THERMAL_SOLUTION_SCHEMA_V1,
        solver_version: THERMAL_SOLVER_VERSION_V1,
        case_id: model.case_id.clone(),
        model_digest: model.digest(),
        minimum_temperature_c: temperatures.iter().copied().fold(f64::INFINITY, f64::min),
        maximum_temperature_c: temperatures
            .iter()
            .copied()
            .fold(f64::NEG_INFINITY, f64::max),
        temperatures_c: temperatures,
        held_node_heat_w,
        element_results,
        heat_into_held_nodes_w,
        heat_out_of_held_nodes_w,
        flux_heat_in_w,
        convection_heat_in_w,
        heat_balance_w: heat_into_held_nodes_w - heat_out_of_held_nodes_w
            + flux_heat_in_w
            + convection_heat_in_w,
        maximum_free_node_residual_w,
        excluded_physics: [
            "transient",
            "radiation",
            "internal heat generation",
            "temperature-dependent conductivity",
            "contact resistance between parts",
        ],
    })
}

fn validate_settings(settings: ThermalSolveSettings) -> Result<(), ThermalError> {
    let tolerance = settings.relative_pivot_tolerance;
    if settings.maximum_nodes == 0
        || !tolerance.is_finite()
        || tolerance <= 0.0
        || tolerance >= 1.0
        || settings.maximum_factor_bytes == 0
    {
        return Err(ThermalError::InvalidSettings);
    }
    Ok(())
}

type TetrahedronGeometry = (f64, [[f64; 3]; 4]);

impl ThermalModel {
    /// Checks the mesh and materials; returns each element's volume (mm³)
    /// and shape-function gradients (1/mm).
    fn validate(
        &self,
        settings: ThermalSolveSettings,
    ) -> Result<Vec<TetrahedronGeometry>, ThermalError> {
        if self.schema != THERMAL_MODEL_SCHEMA_V1 {
            return Err(ThermalError::UnsupportedSchema);
        }
        if self.case_id.trim().is_empty() {
            return Err(ThermalError::EmptyCaseId);
        }
        if self.nodes.is_empty() || self.elements.is_empty() {
            return Err(ThermalError::EmptyMesh);
        }
        if self.nodes.len() > settings.maximum_nodes.min(MAX_FEA_NODES)
            || self.elements.len() > MAX_FEA_ELEMENTS
            || self.materials.len() > MAX_FEA_MATERIALS
            || self.temperatures.len() > MAX_FEA_NODES
            || self.boundaries.len() > MAX_FEA_LOADS
        {
            return Err(ThermalError::ModelTooLarge);
        }
        if self
            .nodes
            .iter()
            .any(|node| !node.position_mm.iter().all(|value| value.is_finite()))
        {
            return Err(ThermalError::InvalidNode);
        }
        let mut material_ids = BTreeSet::new();
        for material in &self.materials {
            if !material_ids.insert(material.id) {
                return Err(ThermalError::DuplicateMaterialId);
            }
            let conductivity = material.conductivity_w_per_m_k;
            if !conductivity.is_finite() || conductivity <= 0.0 {
                return Err(ThermalError::InvalidMaterial);
            }
        }
        let mut element_ids = BTreeSet::new();
        let mut connectivities = BTreeSet::new();
        let mut geometry = Vec::with_capacity(self.elements.len());
        for element in &self.elements {
            let id = element.id;
            if !element_ids.insert(id) {
                return Err(ThermalError::DuplicateElementId { element: id });
            }
            if element.nodes.iter().any(|node| *node >= self.nodes.len()) {
                return Err(ThermalError::InvalidElementNode { element: id });
            }
            let mut canonical = element.nodes;
            canonical.sort_unstable();
            if canonical.windows(2).any(|pair| pair[0] == pair[1]) {
                return Err(ThermalError::RepeatedElementNode { element: id });
            }
            if !connectivities.insert(canonical) {
                return Err(ThermalError::DuplicateElement { element: id });
            }
            if !material_ids.contains(&element.material_id) {
                return Err(ThermalError::MissingMaterial { element: id });
            }
            let (volume, gradients) = self
                .tetrahedron_geometry(element.nodes)
                .map_err(|Singular| ThermalError::DegenerateElement { element: id })?;
            geometry.push((volume, gradients));
        }
        Ok(geometry)
    }

    /// A flat tetrahedron has a singular Jacobian.
    fn tetrahedron_geometry(&self, nodes: [usize; 4]) -> Result<TetrahedronGeometry, Singular> {
        let origin = self.nodes[nodes[0]].position_mm;
        // The rows are the Jacobian's columns, so its inverse is already J⁻ᵀ.
        let jacobian = Mat3::from_rows(
            [1, 2, 3].map(|corner| sub(self.nodes[nodes[corner]].position_mm, origin)),
        );
        let volume = jacobian.determinant().abs() / 6.0;
        let inverse = jacobian.inverse()?;
        let gradients = [
            [-1.0, -1.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ]
        .map(|gradient| inverse.mul_vec(gradient.into()).to_array());
        Ok((volume, gradients))
    }

    fn held_temperatures(&self) -> Result<BTreeMap<usize, f64>, ThermalError> {
        let mut held = BTreeMap::new();
        for temperature in &self.temperatures {
            let value = temperature.temperature_c;
            if temperature.node >= self.nodes.len() || !value.is_finite() || value < ABSOLUTE_ZERO_C
            {
                return Err(ThermalError::InvalidTemperature);
            }
            if let Some(previous) = held.insert(temperature.node, value)
                && previous.to_bits() != value.to_bits()
            {
                return Err(ThermalError::ConflictingTemperature);
            }
        }
        Ok(held)
    }

    /// The sorted triangles that bound exactly one element.
    fn boundary_faces(&self, geometry: &[TetrahedronGeometry]) -> BTreeSet<[usize; 3]> {
        debug_assert_eq!(geometry.len(), self.elements.len());
        let mut seen = BTreeMap::<[usize; 3], usize>::new();
        for element in &self.elements {
            for skipped in 0..4 {
                let mut face = [0; 3];
                let mut slot = 0;
                for (position, node) in element.nodes.into_iter().enumerate() {
                    if position != skipped {
                        face[slot] = node;
                        slot += 1;
                    }
                }
                face.sort_unstable();
                *seen.entry(face).or_default() += 1;
            }
        }
        seen.into_iter()
            .filter_map(|(face, count)| (count == 1).then_some(face))
            .collect()
    }

    fn validate_boundaries(&self, faces: &BTreeSet<[usize; 3]>) -> Result<(), ThermalError> {
        for boundary in &self.boundaries {
            let (nodes, valid) = match *boundary {
                ThermalBoundary::HeatFlux {
                    nodes,
                    flux_w_per_m2,
                } => (nodes, flux_w_per_m2.is_finite()),
                ThermalBoundary::Convection {
                    nodes,
                    film_coefficient_w_per_m2_k,
                    ambient_c,
                } => (
                    nodes,
                    film_coefficient_w_per_m2_k.is_finite()
                        && film_coefficient_w_per_m2_k > 0.0
                        && ambient_c.is_finite()
                        && ambient_c >= ABSOLUTE_ZERO_C,
                ),
            };
            if !valid || nodes.iter().any(|node| *node >= self.nodes.len()) {
                return Err(ThermalError::InvalidBoundary);
            }
            let mut sorted = nodes;
            sorted.sort_unstable();
            if !faces.contains(&sorted) {
                return Err(ThermalError::NonBoundaryFace);
            }
            if self.triangle_area(nodes) <= 0.0 {
                return Err(ThermalError::InvalidBoundary);
            }
        }
        Ok(())
    }

    fn triangle_area(&self, nodes: [usize; 3]) -> f64 {
        let [first, second, third] = nodes.map(|node| self.nodes[node].position_mm);
        0.5 * length(cross(sub(second, first), sub(third, first)))
    }

    /// Heat entering through flux triangles and through convection.
    fn boundary_heat(&self, temperatures: &[f64]) -> (f64, f64) {
        let mut flux = 0.0;
        let mut convection = 0.0;
        for boundary in &self.boundaries {
            match *boundary {
                ThermalBoundary::HeatFlux {
                    nodes,
                    flux_w_per_m2,
                } => flux += flux_w_per_m2 * PER_MM2 * self.triangle_area(nodes),
                ThermalBoundary::Convection {
                    nodes,
                    film_coefficient_w_per_m2_k,
                    ambient_c,
                } => {
                    let mean = nodes.iter().map(|node| temperatures[*node]).sum::<f64>() / 3.0;
                    convection += film_coefficient_w_per_m2_k
                        * PER_MM2
                        * self.triangle_area(nodes)
                        * (ambient_c - mean);
                }
            }
        }
        (flux, convection)
    }

    fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        // An index past 64 bits cannot occur on a platform the crate builds for.
        let index = |hasher: &mut Sha256, value: usize| {
            hasher.update(u64::try_from(value).unwrap_or(u64::MAX).to_le_bytes());
        };
        for text in [&self.schema, &self.case_id] {
            index(&mut hasher, text.len());
            hasher.update(text.as_bytes());
        }
        let number = |hasher: &mut Sha256, value: f64| hasher.update(value.to_bits().to_le_bytes());
        index(&mut hasher, self.nodes.len());
        for node in &self.nodes {
            node.position_mm
                .iter()
                .for_each(|value| number(&mut hasher, *value));
        }
        index(&mut hasher, self.materials.len());
        for material in &self.materials {
            hasher.update(material.id.to_le_bytes());
            number(&mut hasher, material.conductivity_w_per_m_k);
        }
        index(&mut hasher, self.elements.len());
        for element in &self.elements {
            hasher.update(element.id.to_le_bytes());
            hasher.update(element.material_id.to_le_bytes());
            element
                .nodes
                .iter()
                .for_each(|node| index(&mut hasher, *node));
        }
        index(&mut hasher, self.temperatures.len());
        for temperature in &self.temperatures {
            index(&mut hasher, temperature.node);
            number(&mut hasher, temperature.temperature_c);
        }
        index(&mut hasher, self.boundaries.len());
        for boundary in &self.boundaries {
            let (tag, nodes, values) = match *boundary {
                ThermalBoundary::HeatFlux {
                    nodes,
                    flux_w_per_m2,
                } => (0_u8, nodes, [flux_w_per_m2, 0.0]),
                ThermalBoundary::Convection {
                    nodes,
                    film_coefficient_w_per_m2_k,
                    ambient_c,
                } => (1, nodes, [film_coefficient_w_per_m2_k, ambient_c]),
            };
            hasher.update([tag]);
            nodes.iter().for_each(|node| index(&mut hasher, *node));
            values.iter().for_each(|value| number(&mut hasher, *value));
        }
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}
