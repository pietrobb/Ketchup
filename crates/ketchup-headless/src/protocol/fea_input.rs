use ketchup_analysis::fea::FeaSolveSettings;
use serde::Deserialize;

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
