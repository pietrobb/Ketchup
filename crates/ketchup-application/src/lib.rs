//! GUI-independent CAD planning, exact evaluation, validation and document sessions.

mod append_feature;
pub mod batch_task;
pub mod cam_workflow;
mod collision;
mod creation;
pub mod diagnostics;
pub mod evaluation;
pub mod fea_workflow;
pub mod mesh_conversion;
pub mod model_query;
pub mod pdm_workflow;
mod planner;
mod rule_booleans;
mod rule_exact_collisions;
mod rule_program;
mod sketch;
pub mod topology;
pub mod transforms;
pub mod validation;
mod worker_pool;
pub mod workflow_trace;

pub use ketchup_program::SourceLines;
pub use planner::{
    AssistantCadProgramPlan, AssistantCadResolvedProgramOutput, plan_assistant_cad_edit_program,
    plan_assistant_cad_edit_program_with_outputs, plan_panel_batch, plan_rule_part_batch,
};
pub use rule_exact_collisions::{apply_exact_collisions, verify_rule_program_collisions};
pub use rule_program::{
    RuleProgramApplyError, RuleProgramApplyResult, RuleProgramChange, RuleProgramPlan,
    plan_rule_program, rewrite_rule_program_push_pull, rule_program_part_sources,
};

mod session;
pub use session::{DocumentSession, RecoveryState, SaveOptions, SessionError, SessionSettings};
pub use validation::{
    AssistantValidationSelection, StructuralValidationScope, scoped_static_load_report,
};

pub use ketchup_core::assistant_sidecar::{AssistantCadEditOperation, AssistantCadEditProgram};
