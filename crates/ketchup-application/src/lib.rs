//! GUI-independent, observational planning of generic canonical CAD programs.
//!
//! This bounded extraction reuses the core's serializable Assistant program
//! contract. It does not initialize a GUI, assistant, worker, or document session.

mod append_feature;
mod creation;
pub mod diagnostics;
mod planner;
mod sketch;
pub mod topology;
pub mod transforms;

pub use planner::plan_assistant_cad_edit_program;
