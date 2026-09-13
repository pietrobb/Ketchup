use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

pub const FEA_MODEL_SCHEMA_V1: &str = "ketchup.fea-model.v1";
pub const FEA_SOLUTION_SCHEMA_V1: &str = "ketchup.fea-solution.v1";
pub const FEA_SOLVER_VERSION_V1: &str = "ketchup.linear-static-fem.v1";

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
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaConstraint {
    pub node: usize,
    pub displacement_mm: [Option<f64>; 3],
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
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FeaSolveSettings {
    pub maximum_nodes: usize,
    pub relative_pivot_tolerance: f64,
    pub maximum_small_deformation_ratio: f64,
    pub convergence_relative_tolerance: f64,
}

impl Default for FeaSolveSettings {
    fn default() -> Self {
        Self {
            maximum_nodes: 256,
            relative_pivot_tolerance: 1.0e-12,
            maximum_small_deformation_ratio: 0.05,
            convergence_relative_tolerance: 0.02,
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
    DuplicateElementId,
    InvalidElementNode,
    RepeatedElementNode,
    DegenerateElement,
    MissingMaterial,
    InvalidArea,
    MissingConstraint,
    InvalidConstraint,
    ConflictingConstraint,
    MissingLoad,
    InvalidLoad,
    NonBoundaryTraction,
    InvalidSettings,
    SingularSystem,
    NonFiniteSolution,
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
    pub excluded_physics: [&'static str; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeaSolution {
    pub schema: &'static str,
    pub solver_version: &'static str,
    pub case_id: String,
    pub model_digest: String,
    pub displacements_mm: Vec<[f64; 3]>,
    pub reaction_forces_n: Vec<[f64; 3]>,
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

#[derive(Clone, Debug, PartialEq)]
pub struct FeaConvergenceEvidence {
    pub case_id: String,
    pub levels: Vec<FeaConvergenceLevel>,
    pub final_displacement_relative_change: f64,
    pub final_energy_relative_change: f64,
    pub required_relative_tolerance: f64,
    pub converged: bool,
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
    Solve(FeaError),
}

impl FeaModel {
    pub fn solve(&self, settings: FeaSolveSettings) -> Result<FeaSolution, FeaError> {
        self.validate(settings)?;
        let dof_count = self.nodes.len() * 3;
        let mut stiffness = vec![vec![0.0; dof_count]; dof_count];
        let mut forces = vec![0.0; dof_count];

        for element in &self.elements {
            match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => self.assemble_truss(nodes, material_id, area_mm2, &mut stiffness),
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                    self.assemble_tetrahedron(nodes, material_id, &mut stiffness)
                }
            }
        }
        self.assemble_loads(&mut forces);

        let original_stiffness = stiffness.clone();
        let original_forces = forces.clone();
        let constrained = self.apply_constraints(&mut stiffness, &mut forces)?;
        let displacement = solve_dense(stiffness, forces, settings.relative_pivot_tolerance)?;
        if displacement.iter().any(|value| !value.is_finite()) {
            return Err(FeaError::NonFiniteSolution);
        }

        let residual = matrix_vector(&original_stiffness, &displacement)
            .into_iter()
            .zip(&original_forces)
            .map(|(internal, applied)| internal - applied)
            .collect::<Vec<_>>();
        let mut maximum_free_dof_residual_n: f64 = 0.0;
        let mut reactions = vec![[0.0; 3]; self.nodes.len()];
        for (dof, value) in residual.iter().copied().enumerate() {
            if constrained.contains(&dof) {
                reactions[dof / 3][dof % 3] = value;
            } else {
                maximum_free_dof_residual_n = maximum_free_dof_residual_n.max(value.abs());
            }
        }

        let displacements_mm = displacement
            .chunks_exact(3)
            .map(|values| [values[0], values[1], values[2]])
            .collect::<Vec<_>>();
        let maximum_displacement_mm = displacements_mm
            .iter()
            .map(|value| norm(*value))
            .fold(0.0, f64::max);
        let element_results = self.element_results(&displacement);
        let maximum_von_mises_stress_mpa = element_results
            .iter()
            .map(|result| result.von_mises_stress_mpa)
            .fold(0.0, f64::max);
        let total_strain_energy_n_mm = element_results
            .iter()
            .map(|result| result.strain_energy_n_mm)
            .sum();
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

        let total_applied = vector_sum(&original_forces);
        let total_reaction = reactions.iter().copied().fold([0.0; 3], add3);
        Ok(FeaSolution {
            schema: FEA_SOLUTION_SCHEMA_V1,
            solver_version: FEA_SOLVER_VERSION_V1,
            case_id: self.case_id.clone(),
            model_digest: self.digest(),
            displacements_mm,
            reaction_forces_n: reactions,
            element_results,
            maximum_displacement_mm,
            maximum_von_mises_stress_mpa,
            total_strain_energy_n_mm,
            maximum_free_dof_residual_n,
            force_balance_n: add3(total_applied, total_reaction),
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
                excluded_physics: [
                    "geometric nonlinearity",
                    "plasticity",
                    "contact",
                    "buckling and dynamics",
                ],
            },
        })
    }

    fn validate(&self, settings: FeaSolveSettings) -> Result<(), FeaError> {
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
            || self.nodes.len() > 256
        {
            return Err(FeaError::ModelTooLarge);
        }
        if !finite_positive(settings.relative_pivot_tolerance)
            || settings.relative_pivot_tolerance >= 1.0
            || !finite_positive(settings.maximum_small_deformation_ratio)
            || settings.maximum_small_deformation_ratio >= 1.0
            || !finite_positive(settings.convergence_relative_tolerance)
            || settings.convergence_relative_tolerance >= 1.0
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
                    .is_some_and(|value| !finite_positive(value))
            {
                return Err(FeaError::InvalidMaterial);
            }
        }

        let mut element_ids = BTreeSet::new();
        let mut tetrahedron_faces = BTreeMap::<[usize; 3], usize>::new();
        for element in &self.elements {
            if !element_ids.insert(element.id) {
                return Err(FeaError::DuplicateElementId);
            }
            let (nodes, material_id) = match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => {
                    if !finite_positive(area_mm2) {
                        return Err(FeaError::InvalidArea);
                    }
                    if nodes[0] == nodes[1] {
                        return Err(FeaError::RepeatedElementNode);
                    }
                    if self.truss_geometry(nodes).is_none() {
                        return Err(FeaError::DegenerateElement);
                    }
                    (nodes.to_vec(), material_id)
                }
                FeaElementKind::LinearTetrahedron4 { nodes, material_id } => {
                    if nodes.iter().copied().collect::<BTreeSet<_>>().len() != 4 {
                        return Err(FeaError::RepeatedElementNode);
                    }
                    if self.tetrahedron_geometry(nodes).is_none() {
                        return Err(FeaError::DegenerateElement);
                    }
                    for face in tetrahedron_faces_of(nodes) {
                        *tetrahedron_faces.entry(face).or_default() += 1;
                    }
                    (nodes.to_vec(), material_id)
                }
            };
            if nodes.iter().any(|node| *node >= self.nodes.len()) {
                return Err(FeaError::InvalidElementNode);
            }
            if !material_ids.contains(&material_id) {
                return Err(FeaError::MissingMaterial);
            }
        }

        if self.constraints.is_empty() {
            return Err(FeaError::MissingConstraint);
        }
        let mut prescribed = BTreeMap::<usize, f64>::new();
        for constraint in &self.constraints {
            if constraint.node >= self.nodes.len()
                || constraint.displacement_mm.iter().all(Option::is_none)
            {
                return Err(FeaError::InvalidConstraint);
            }
            for (axis, value) in constraint.displacement_mm.iter().copied().enumerate() {
                if let Some(value) = value {
                    if !value.is_finite() {
                        return Err(FeaError::InvalidConstraint);
                    }
                    if prescribed
                        .insert(constraint.node * 3 + axis, value)
                        .is_some_and(|previous| previous.to_bits() != value.to_bits())
                    {
                        return Err(FeaError::ConflictingConstraint);
                    }
                }
            }
        }

        if self.loads.is_empty() {
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
                    ) <= 1.0e-12
                    {
                        return Err(FeaError::InvalidLoad);
                    }
                }
            }
        }
        Ok(())
    }

    fn material(&self, id: u64) -> &FeaMaterial {
        self.materials
            .iter()
            .find(|material| material.id == id)
            .expect("validated material reference")
    }

    fn truss_geometry(&self, nodes: [usize; 2]) -> Option<(f64, [f64; 3])> {
        if nodes.iter().any(|node| *node >= self.nodes.len()) {
            return None;
        }
        let delta = sub3(
            self.nodes[nodes[1]].position_mm,
            self.nodes[nodes[0]].position_mm,
        );
        let length = norm(delta);
        (length > 1.0e-12).then(|| (length, scale3(delta, 1.0 / length)))
    }

    fn tetrahedron_geometry(&self, nodes: [usize; 4]) -> Option<(f64, [[f64; 3]; 4])> {
        if nodes.iter().any(|node| *node >= self.nodes.len()) {
            return None;
        }
        let origin = self.nodes[nodes[0]].position_mm;
        let jacobian = [
            sub3(self.nodes[nodes[1]].position_mm, origin),
            sub3(self.nodes[nodes[2]].position_mm, origin),
            sub3(self.nodes[nodes[3]].position_mm, origin),
        ];
        let determinant = determinant3(jacobian);
        let inverse = inverse3(jacobian)?;
        let gradients_reference = [
            [-1.0, -1.0, -1.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [0.0, 0.0, 1.0],
        ];
        let gradients = gradients_reference.map(|gradient| matrix_vector3(inverse, gradient));
        Some((determinant.abs() / 6.0, gradients))
    }

    fn assemble_truss(
        &self,
        nodes: [usize; 2],
        material_id: u64,
        area_mm2: f64,
        stiffness: &mut [Vec<f64>],
    ) {
        let (length, direction) = self.truss_geometry(nodes).expect("validated truss");
        let factor = self.material(material_id).youngs_modulus_mpa * area_mm2 / length;
        for local_i in 0..6 {
            for local_j in 0..6 {
                let sign_i = if local_i < 3 { -1.0 } else { 1.0 };
                let sign_j = if local_j < 3 { -1.0 } else { 1.0 };
                let global_i = nodes[local_i / 3] * 3 + local_i % 3;
                let global_j = nodes[local_j / 3] * 3 + local_j % 3;
                stiffness[global_i][global_j] +=
                    factor * sign_i * sign_j * direction[local_i % 3] * direction[local_j % 3];
            }
        }
    }

    fn assemble_tetrahedron(
        &self,
        nodes: [usize; 4],
        material_id: u64,
        stiffness: &mut [Vec<f64>],
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
                stiffness[global_i][global_j] += value;
            }
        }
    }

    fn assemble_loads(&self, forces: &mut [f64]) {
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
            }
        }
    }

    fn apply_constraints(
        &self,
        stiffness: &mut [Vec<f64>],
        forces: &mut [f64],
    ) -> Result<BTreeSet<usize>, FeaError> {
        let mut prescribed = BTreeMap::<usize, f64>::new();
        for constraint in &self.constraints {
            for (axis, value) in constraint.displacement_mm.iter().copied().enumerate() {
                if let Some(value) = value {
                    prescribed.insert(constraint.node * 3 + axis, value);
                }
            }
        }
        let constraint_scale = stiffness
            .iter()
            .flat_map(|row| row.iter())
            .map(|value| value.abs())
            .fold(0.0, f64::max)
            .max(1.0);
        for (&dof, &value) in &prescribed {
            for row in 0..stiffness.len() {
                if row != dof {
                    forces[row] -= stiffness[row][dof] * value;
                    stiffness[row][dof] = 0.0;
                    stiffness[dof][row] = 0.0;
                }
            }
            stiffness[dof][dof] = constraint_scale;
            forces[dof] = constraint_scale * value;
        }
        if prescribed.is_empty() {
            return Err(FeaError::MissingConstraint);
        }
        Ok(prescribed.into_keys().collect())
    }

    fn element_results(&self, displacement: &[f64]) -> Vec<FeaElementResult> {
        self.elements
            .iter()
            .map(|element| match element.kind {
                FeaElementKind::LinearTruss2 {
                    nodes,
                    material_id,
                    area_mm2,
                } => {
                    let material = self.material(material_id);
                    let (length, direction) = self.truss_geometry(nodes).expect("validated truss");
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
                        local_displacement[local] = displacement[nodes[local / 3] * 3 + local % 3];
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
            })
            .collect()
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
        norm(sub3(maximum, minimum)).max(1.0e-12)
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

    fn resultant_load(&self) -> [f64; 3] {
        let mut forces = vec![0.0; self.nodes.len() * 3];
        self.assemble_loads(&mut forces);
        vector_sum(&forces)
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
    let resultant_load = levels[0].resultant_load();
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
    if levels
        .iter()
        .skip(1)
        .any(|level| !near3(level.resultant_load(), resultant_load, 1.0e-9))
    {
        return Err(FeaConvergenceError::LoadMismatch);
    }
    let solutions = levels
        .iter()
        .map(|level| level.solve(settings).map_err(FeaConvergenceError::Solve))
        .collect::<Result<Vec<_>, _>>()?;
    let previous = &solutions[solutions.len() - 2];
    let final_solution = solutions.last().expect("at least two solutions");
    let final_displacement_relative_change = relative_change(
        previous.maximum_displacement_mm,
        final_solution.maximum_displacement_mm,
    );
    let final_energy_relative_change = relative_change(
        previous.total_strain_energy_n_mm,
        final_solution.total_strain_energy_n_mm,
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
        final_displacement_relative_change,
        final_energy_relative_change,
        required_relative_tolerance: settings.convergence_relative_tolerance,
        converged: final_displacement_relative_change <= settings.convergence_relative_tolerance
            && final_energy_relative_change <= settings.convergence_relative_tolerance,
        solutions,
    })
}

fn solve_dense(
    mut matrix: Vec<Vec<f64>>,
    mut right_hand_side: Vec<f64>,
    relative_tolerance: f64,
) -> Result<Vec<f64>, FeaError> {
    let size = matrix.len();
    let scale = matrix
        .iter()
        .flat_map(|row| row.iter())
        .map(|value| value.abs())
        .fold(0.0, f64::max)
        .max(1.0);
    let threshold = scale * relative_tolerance;
    for column in 0..size {
        let pivot = (column..size)
            .max_by(|left, right| {
                matrix[*left][column]
                    .abs()
                    .total_cmp(&matrix[*right][column].abs())
            })
            .expect("non-empty pivot range");
        if matrix[pivot][column].abs() <= threshold {
            return Err(FeaError::SingularSystem);
        }
        matrix.swap(column, pivot);
        right_hand_side.swap(column, pivot);
        let pivot_row = matrix[column].clone();
        let pivot_value = pivot_row[column];
        let pivot_right_hand_side = right_hand_side[column];
        for (row, row_right_hand_side) in
            matrix.iter_mut().zip(&mut right_hand_side).skip(column + 1)
        {
            let factor = row[column] / pivot_value;
            row[column] = 0.0;
            for (value, pivot_value) in row.iter_mut().zip(&pivot_row).skip(column + 1) {
                *value -= factor * pivot_value;
            }
            *row_right_hand_side -= factor * pivot_right_hand_side;
        }
    }
    let mut solution = vec![0.0; size];
    for row in (0..size).rev() {
        let known = ((row + 1)..size)
            .map(|column| matrix[row][column] * solution[column])
            .sum::<f64>();
        solution[row] = (right_hand_side[row] - known) / matrix[row][row];
    }
    Ok(solution)
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

fn determinant3(matrix: [[f64; 3]; 3]) -> f64 {
    matrix[0][0] * (matrix[1][1] * matrix[2][2] - matrix[1][2] * matrix[2][1])
        - matrix[1][0] * (matrix[0][1] * matrix[2][2] - matrix[0][2] * matrix[2][1])
        + matrix[2][0] * (matrix[0][1] * matrix[1][2] - matrix[0][2] * matrix[1][1])
}

fn inverse3(matrix: [[f64; 3]; 3]) -> Option<[[f64; 3]; 3]> {
    let determinant = determinant3(matrix);
    if !determinant.is_finite() || determinant.abs() <= 1.0e-12 {
        return None;
    }
    let inverse_determinant = 1.0 / determinant;
    Some([
        [
            (matrix[1][1] * matrix[2][2] - matrix[2][1] * matrix[1][2]) * inverse_determinant,
            (matrix[2][1] * matrix[0][2] - matrix[0][1] * matrix[2][2]) * inverse_determinant,
            (matrix[0][1] * matrix[1][2] - matrix[1][1] * matrix[0][2]) * inverse_determinant,
        ],
        [
            (matrix[2][0] * matrix[1][2] - matrix[1][0] * matrix[2][2]) * inverse_determinant,
            (matrix[0][0] * matrix[2][2] - matrix[2][0] * matrix[0][2]) * inverse_determinant,
            (matrix[1][0] * matrix[0][2] - matrix[0][0] * matrix[1][2]) * inverse_determinant,
        ],
        [
            (matrix[1][0] * matrix[2][1] - matrix[2][0] * matrix[1][1]) * inverse_determinant,
            (matrix[2][0] * matrix[0][1] - matrix[0][0] * matrix[2][1]) * inverse_determinant,
            (matrix[0][0] * matrix[1][1] - matrix[1][0] * matrix[0][1]) * inverse_determinant,
        ],
    ])
}

fn matrix_vector3(matrix: [[f64; 3]; 3], vector: [f64; 3]) -> [f64; 3] {
    // The stored Jacobian is transposed, so its inverse is already J^-T.
    [
        matrix[0][0] * vector[0] + matrix[0][1] * vector[1] + matrix[0][2] * vector[2],
        matrix[1][0] * vector[0] + matrix[1][1] * vector[1] + matrix[1][2] * vector[2],
        matrix[2][0] * vector[0] + matrix[2][1] * vector[1] + matrix[2][2] * vector[2],
    ]
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

fn matrix_vector(matrix: &[Vec<f64>], vector: &[f64]) -> Vec<f64> {
    matrix
        .iter()
        .map(|row| {
            row.iter()
                .zip(vector)
                .map(|(left, right)| left * right)
                .sum()
        })
        .collect()
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
    norm(cross(sub3(second, first), sub3(third, first))) * 0.5
}

fn relative_change(previous: f64, current: f64) -> f64 {
    (current - previous).abs() / current.abs().max(previous.abs()).max(1.0e-15)
}

fn near_bounds(left: ([f64; 3], [f64; 3]), right: ([f64; 3], [f64; 3])) -> bool {
    near3(left.0, right.0, 1.0e-9) && near3(left.1, right.1, 1.0e-9)
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

fn hash_f64s(hasher: &mut Sha256, values: &[f64]) {
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
}

fn dot(left: [f64; 3], right: [f64; 3]) -> f64 {
    left[0] * right[0] + left[1] * right[1] + left[2] * right[2]
}

fn dot6(left: [f64; 6], right: [f64; 6]) -> f64 {
    left.into_iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum()
}

fn norm(value: [f64; 3]) -> f64 {
    dot(value, value).sqrt()
}

fn add3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] + right[0], left[1] + right[1], left[2] + right[2]]
}

fn sub3(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [left[0] - right[0], left[1] - right[1], left[2] - right[2]]
}

fn scale3(value: [f64; 3], factor: f64) -> [f64; 3] {
    [value[0] * factor, value[1] * factor, value[2] * factor]
}

fn cross(left: [f64; 3], right: [f64; 3]) -> [f64; 3] {
    [
        left[1] * right[2] - left[2] * right[1],
        left[2] * right[0] - left[0] * right[2],
        left[0] * right[1] - left[1] * right[0],
    ]
}
