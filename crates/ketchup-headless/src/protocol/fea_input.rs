use ketchup_analysis::fea::{FeaMaterial, FeaSolveSettings};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeaMaterialInput {
    pub(super) id: u64,
    youngs_modulus_mpa: f64,
    poisson_ratio: f64,
    yield_strength_mpa: Option<f64>,
}

impl From<FeaMaterialInput> for FeaMaterial {
    /// A static study needs no density.
    fn from(input: FeaMaterialInput) -> Self {
        Self {
            id: input.id,
            youngs_modulus_mpa: input.youngs_modulus_mpa,
            poisson_ratio: input.poisson_ratio,
            yield_strength_mpa: input.yield_strength_mpa,
            density_kg_per_m3: None,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct FeaSolveSettingsInput {
    maximum_nodes: usize,
    relative_pivot_tolerance: f64,
    maximum_small_deformation_ratio: f64,
    convergence_relative_tolerance: f64,
}

impl From<FeaSolveSettingsInput> for FeaSolveSettings {
    /// The factored matrix keeps the default memory budget.
    fn from(input: FeaSolveSettingsInput) -> Self {
        Self {
            maximum_nodes: input.maximum_nodes,
            relative_pivot_tolerance: input.relative_pivot_tolerance,
            maximum_small_deformation_ratio: input.maximum_small_deformation_ratio,
            convergence_relative_tolerance: input.convergence_relative_tolerance,
            ..Self::default()
        }
    }
}
