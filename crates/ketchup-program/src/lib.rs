//! Rule programs for Ketchup.
//!
//! A program is a Starlark source file that describes parameters, parts,
//! machining and joints. Evaluating it yields a [`ProgramModel`]; the model is
//! checked by [`validate`], listed by [`bom`] and turned into canonical panel
//! operations by [`cad`]. Evaluation needs neither OCCT nor a GUI.
//!
//! Domain vocabulary (boards, dowels, grooves) lives in
//! `library/prelude.star`, not in Rust.

pub mod bom;
pub mod cad;
pub mod eval;
pub mod model;
pub mod validate;

pub use bom::{Bom, bom};
pub use eval::{Evaluated, ProgramError, evaluate};
pub use model::{Face, ProgramModel};
pub use validate::{Issue, Severity, validate};

use serde::Serialize;
use std::collections::BTreeMap;

/// Everything a caller (person, AI agent, UI) needs after one evaluation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub ok: bool,
    pub errors: usize,
    pub warnings: usize,
    pub issues: Vec<Issue>,
    pub params: Vec<model::Param>,
    pub bom: Bom,
    pub log: Vec<String>,
    pub unused_overrides: Vec<String>,
}

/// Evaluates, validates and lists a program in one call.
///
/// # Errors
/// Returns the interpreter error when the program cannot be evaluated.
pub fn run(
    file_name: &str,
    source: &str,
    overrides: &BTreeMap<String, f64>,
) -> Result<(Evaluated, Report), ProgramError> {
    let evaluated = evaluate(file_name, source, overrides)?;
    let issues = validate(&evaluated.model);
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    let report = Report {
        ok: errors == 0,
        errors,
        warnings: issues.len() - errors,
        issues,
        params: evaluated.model.params.clone(),
        bom: bom(&evaluated.model),
        log: evaluated.log.clone(),
        unused_overrides: evaluated.unused_overrides.clone(),
    };
    Ok((evaluated, report))
}
