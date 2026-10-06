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
pub mod clearance;
mod connectivity;
pub mod contact;
pub mod document;
pub mod eval;
pub mod exact;
mod execution_budget;
pub mod expect;
mod face_at;
pub mod faces;
pub mod frame;
pub mod joint_check;
pub mod load_path;
pub mod loads;
pub mod member_check;
pub mod model;
pub mod motion;
mod opposing_holes;
pub mod path;
pub mod relations;
pub mod takeoff;
pub mod validate;

pub use bom::{Bom, bom};
pub use eval::{Evaluated, PRELUDE, ProgramError, SourceLines, evaluate, use_prelude};
pub use exact::{ExactPair, ExactShapes, exact_candidates};
pub use model::{
    ProgramFeature, ProgramFeatureKind, ProgramFeatureParameter, ProgramModel,
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
    /// Every bearing joint with its rating, or why it is not verified.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<joint_check::JointCheck>,
    /// Characteristic loads on every load-path member, when the program declares them.
    #[serde(skip_serializing_if = "loads::LoadReport::is_empty")]
    pub loads: loads::LoadReport,
    /// EN 1995-1-1 check of every load-path member, when timber strength classes are declared.
    #[serde(skip_serializing_if = "member_check::DesignReport::is_empty")]
    pub design: member_check::DesignReport,
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
    let report = report(&evaluated);
    Ok((evaluated, report))
}

/// Validates and lists an evaluated program. On a large model this takes far
/// longer than the evaluation itself, so callers that only publish geometry
/// skip it.
#[must_use]
pub fn report(evaluated: &Evaluated) -> Report {
    let issues = validate(&evaluated.model);
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    let loads = loads::loads(&evaluated.model);
    Report {
        ok: errors == 0,
        errors,
        warnings: issues.len() - errors,
        relations: relations(&evaluated.model, &issues),
        issues,
        params: evaluated.model.params.clone(),
        bom: bom(&evaluated.model),
        joints: joint_check::joint_checks(&evaluated.model, &loads),
        design: member_check::member_checks(&evaluated.model, &loads),
        loads,
        log: evaluated.log.clone(),
        unused_overrides: evaluated.unused_overrides.clone(),
    }
}
