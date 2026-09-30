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
pub mod document;
pub mod eval;
pub mod exact;
pub mod expect;
mod face_at;
pub mod faces;
pub mod frame;
pub mod model;
pub mod path;
pub mod relations;
pub mod validate;

pub use bom::{Bom, bom};
pub use eval::{Evaluated, ProgramError, SourceLines, evaluate};
pub use exact::{ExactPair, ExactShapes, exact_candidates};
pub use model::{
    Face, ProgramFeature, ProgramFeatureKind, ProgramFeatureParameter, ProgramModel,
    ProgramParameterValueType, ProgramPartBody, ProgramProfileSegment,
};
pub use relations::{OverlapStatus, Relation, RelationKind, relations, relations_with};
pub use validate::{COLLISION_UNVERIFIED, Issue, Severity, validate, validate_with};

use serde::Serialize;
use std::collections::BTreeMap;

/// The library's validation rules: material stiffness and the normative
/// limits (passage width, anchoring height, hole material, ...) that the
/// document validators evaluate. Norms differ by country and product, so
/// they are data here, not constants in the validators.
#[must_use]
pub fn library_validation_rules() -> &'static str {
    include_str!("../library/validation_rules.json")
}

/// Everything a caller (person, AI agent, UI) needs after one evaluation.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Report {
    pub ok: bool,
    pub errors: usize,
    pub warnings: usize,
    pub issues: Vec<Issue>,
    /// How parts touch, reach into each other or nearly meet.
    pub relations: Vec<Relation>,
    pub params: Vec<model::Param>,
    pub bom: Bom,
    pub log: Vec<String>,
    pub unused_overrides: Vec<String>,
}

impl Report {
    /// Replaces the issues (e.g. after an exact check) and recounts them.
    pub fn set_issues(&mut self, mut issues: Vec<Issue>) {
        issues.sort_by_key(|issue| issue.severity);
        self.errors = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count();
        self.warnings = issues.len() - self.errors;
        self.ok = self.errors == 0;
        relations::sync_with_issues(&mut self.relations, &issues);
        self.issues = issues;
    }

    /// Re-derives the issues and relations of `model` with the exact answers
    /// for the pairs whose boxes misstate their solids.
    pub fn refine(&mut self, model: &ProgramModel, exact: &ExactShapes) {
        self.relations = relations_with(model, &[], exact);
        self.set_issues(validate_with(model, exact));
    }
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
        relations: relations(&evaluated.model, &issues),
        issues,
        params: evaluated.model.params.clone(),
        bom: bom(&evaluated.model),
        log: evaluated.log.clone(),
        unused_overrides: evaluated.unused_overrides.clone(),
    };
    Ok((evaluated, report))
}
