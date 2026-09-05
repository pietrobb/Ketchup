//! GUI-independent CAD planning, exact evaluation, validation and document sessions.

mod append_feature;
mod collision;
mod creation;
pub mod diagnostics;
pub mod evaluation;
mod planner;
mod sketch;
pub mod topology;
pub mod transforms;
pub mod validation;

pub use planner::plan_assistant_cad_edit_program;

mod session;
pub use session::{DocumentSession, SaveOptions, SessionError, SessionSettings};
pub use validation::AssistantValidationSelection;

pub use ketchup_core::assistant_sidecar::{AssistantCadEditOperation, AssistantCadEditProgram};
