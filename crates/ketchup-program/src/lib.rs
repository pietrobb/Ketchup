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
mod continuous_span;
pub mod cut_list;
pub mod cutting;
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
mod material;
pub mod member_check;
pub mod model;
pub mod motion;
mod opposing_holes;
pub mod path;
pub mod quote;
pub mod relations;
pub mod shop_drawing;
pub mod source_edit;
pub mod table;
pub mod takeoff;
pub mod validate;
mod wire_names;

pub use bom::{Bom, bom};
pub use eval::{
    CallFrame, CancelToken, ERROR_CODES, ErrorLocation, Evaluated, MetricThread, NumberedLine,
    PRELUDE, Position, ProgramError, SourceLines, evaluate, evaluate_cancellable,
    library_material_weights, library_metric_threads, use_prelude,
};
pub use exact::{ExactPair, ExactShapes, exact_candidates, rechecked};
pub use execution_budget::{MAX_HEAP_BYTES as EVALUATION_HEAP_BYTES, MAX_TIME as EVALUATION_TIME};
pub use model::{
    ProgramFeature, ProgramFeatureKind, ProgramFeatureParameter, ProgramModel,
    ProgramParameterValueType, ProgramPartBody, ProgramProfileSegment,
};
pub use relations::{OverlapStatus, Relation, RelationKind, relations, relations_with};
pub use validate::{
    CHECKS_UNVERIFIED, COLLISION_UNVERIFIED, Issue, REPORT_TRUNCATED, Severity, validate,
    validate_with,
};

use serde::{Deserialize, Serialize};
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
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Report {
    pub ok: bool,
    pub errors: usize,
    pub warnings: usize,
    #[serde(deserialize_with = "validate::report_issues")]
    pub issues: Vec<Issue>,
    /// How parts touch, reach into each other or nearly meet.
    pub relations: Vec<Relation>,
    pub params: Vec<model::Param>,
    pub bom: Bom,
    /// Every bearing joint with its rating, or why it is not verified.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub joints: Vec<joint_check::JointCheck>,
    /// Characteristic loads on every load-path member, when the program declares them.
    #[serde(default, skip_serializing_if = "loads::LoadReport::is_empty")]
    pub loads: loads::LoadReport,
    /// EN 1995-1-1 check of every load-path member, when timber strength classes are declared.
    #[serde(default, skip_serializing_if = "member_check::DesignReport::is_empty")]
    pub design: member_check::DesignReport,
    pub log: Vec<String>,
    pub unused_overrides: Vec<String>,
    /// The counts of the whole report when this one lists only its first issues
    /// and relations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<Truncated>,
    /// The program lines of every part, to name the lines of later issues.
    #[serde(skip)]
    part_sources: BTreeMap<String, Vec<SourceLines>>,
    /// Made by [`report_before_exact`]: only the collisions are listed yet.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    checks_pending: bool,
}

/// How many issues (errors among them) and relations a whole report holds,
/// kept on a report that lists only its first ones.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Truncated {
    pub issues: usize,
    pub errors: usize,
    pub relations: usize,
}

impl Report {
    /// Replaces the issues (e.g. after an exact check) and recounts them. A
    /// report whose checks did not finish is not `ok`: nothing says it is.
    pub fn set_issues(&mut self, mut issues: Vec<Issue>) {
        locate_issues(&mut issues, &self.part_sources);
        issues.sort_by_key(|issue| issue.severity);
        self.errors = issues
            .iter()
            .filter(|issue| issue.severity == Severity::Error)
            .count();
        self.warnings = issues.len() - self.errors;
        self.ok = self.errors == 0 && !issues.iter().any(|issue| issue.kind == CHECKS_UNVERIFIED);
        relations::sync_with_issues(&mut self.relations, &issues);
        self.issues = issues;
    }

    /// A report read back from the program worker names the program lines of
    /// its parts again from the evaluation it describes.
    #[must_use]
    pub fn with_part_sources(mut self, evaluated: &Evaluated) -> Self {
        self.part_sources.clone_from(&evaluated.part_sources);
        self
    }

    /// The report of `evaluated` whose checks stopped at `stopped` (a limit of
    /// the program worker): its parameters, bill of materials and log, and one
    /// issue saying that no collision, relation, joint or load was checked.
    /// Validation reports and does not block editing, so the edit stands.
    #[must_use]
    pub fn unfinished(evaluated: &Evaluated, stopped: &ProgramError) -> Self {
        let mut report = Self {
            ok: true,
            errors: 0,
            warnings: 0,
            issues: Vec::new(),
            relations: Vec::new(),
            params: evaluated.model.params.clone(),
            bom: bom(&evaluated.model),
            joints: Vec::new(),
            loads: loads::LoadReport::default(),
            design: member_check::DesignReport::default(),
            log: evaluated.log.clone(),
            unused_overrides: evaluated.unused_overrides.clone(),
            truncated: None,
            part_sources: evaluated.part_sources.clone(),
            checks_pending: false,
        };
        report.set_issues(vec![Issue {
            severity: Severity::Warning,
            kind: CHECKS_UNVERIFIED,
            parts: Vec::new(),
            message: format!(
                "The checks of the program did not finish ({}: {}); the edit was applied, but no collision, relation, joint or load was checked.",
                stopped.code, stopped.message
            ),
            where_mm: None,
            hint: "Ask for the report again, or check fewer parts at a time.".to_owned(),
            source_lines: Vec::new(),
        }]);
        report
    }

    /// Takes the issues and relations [`refine`] re-derived. Refining checks
    /// collisions and relations only, so a report whose checks stopped keeps
    /// saying that joints and loads were not checked.
    pub fn apply_refined(&mut self, refined: Refined) {
        let mut issues = refined.issues;
        issues.extend(
            self.issues
                .iter()
                .filter(|issue| issue.kind == CHECKS_UNVERIFIED)
                .cloned(),
        );
        self.relations = refined.relations;
        self.truncated = refined.truncated;
        self.checks_pending = false;
        self.set_issues(issues);
    }

    /// Whether this report lists only its collisions ([`report_before_exact`]):
    /// before it is published without the exact check that re-derives its
    /// issues and relations, [`Self::apply_refined`] has to give them the
    /// result of [`refine`] without exact answers.
    #[must_use]
    pub fn checks_pending(&self) -> bool {
        self.checks_pending
    }

    /// The checks [`report_before_exact`] left out did not finish: the report
    /// keeps its collisions, joints and loads, and says what is missing.
    pub fn stop_pending_checks(&mut self, stopped: &ProgramError) {
        self.checks_pending = false;
        let mut issues = std::mem::take(&mut self.issues);
        issues.push(Issue {
            severity: Severity::Warning,
            kind: CHECKS_UNVERIFIED,
            parts: Vec::new(),
            message: format!(
                "The checks of the program did not finish ({}: {}); the edit was applied, and its collisions, joints and loads were checked, but no relation and no other issue.",
                stopped.code, stopped.message
            ),
            where_mm: None,
            hint: "Ask for the report again, or check fewer parts at a time.".to_owned(),
            source_lines: Vec::new(),
        });
        self.set_issues(issues);
    }

    /// This report listing only its first `issues` issues, errors first, and
    /// first `relations` relations, plus one issue naming what is left out,
    /// for a reply that cannot hold the whole report.
    #[must_use]
    pub fn first(&self, issues: usize, relations: usize) -> Self {
        let mut report = self.clone();
        let listed = first_listed(
            &self.issues,
            &self.relations,
            self.truncated,
            issues,
            relations,
        );
        report.truncated = listed.truncated;
        report.set_issues(listed.issues);
        report.relations = listed.relations;
        report
    }
}

impl Refined {
    /// [`Report::first`] for the issues and relations [`refine`] re-derived.
    #[must_use]
    pub fn first(&self, issues: usize, relations: usize) -> Self {
        first_listed(
            &self.issues,
            &self.relations,
            self.truncated,
            issues,
            relations,
        )
    }
}

/// The first `kept` issues, errors first, and `kept_relations` relations, and
/// one more issue naming what is left out: an error when an error is, so a
/// shortened report is never `ok` when the whole one is not. The counts of the
/// whole report go along (`whole` when the issues were shortened before).
fn first_listed(
    issues: &[Issue],
    relations: &[Relation],
    whole: Option<Truncated>,
    kept: usize,
    kept_relations: usize,
) -> Refined {
    let mut listed = issues.to_vec();
    listed.sort_by_key(|issue| issue.severity);
    let left_out = listed.split_off(kept.min(listed.len()));
    let first_relations = relations[..kept_relations.min(relations.len())].to_vec();
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    listed.push(Issue {
        severity: if left_out.iter().any(|issue| issue.severity == Severity::Error) {
            Severity::Error
        } else {
            Severity::Warning
        },
        kind: REPORT_TRUNCATED,
        parts: Vec::new(),
        message: format!(
            "The report lists {} of its {} issues ({errors} errors) and {} of its {} relations; the whole one did not fit the program worker's reply.",
            listed.len(),
            issues.len(),
            first_relations.len(),
            relations.len(),
        ),
        where_mm: None,
        hint: "Fix the issues listed and ask for the report again, or check fewer parts at a time."
            .to_owned(),
        source_lines: Vec::new(),
    });
    Refined {
        issues: listed,
        relations: first_relations,
        truncated: Some(whole.unwrap_or(Truncated {
            issues: issues.len(),
            errors,
            relations: relations.len(),
        })),
    }
}

/// The issues and relations of a model re-derived with exact answers.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Refined {
    #[serde(deserialize_with = "validate::report_issues")]
    pub issues: Vec<Issue>,
    pub relations: Vec<Relation>,
    /// The counts of the whole list when this one holds only its first entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<Truncated>,
}

/// Re-derives the issues and relations of `model` with the exact answers for
/// the pairs whose boxes misstate their solids. It costs as much as the checks
/// of [`report`], so the editor runs it in the program worker too.
#[must_use]
pub fn refine(model: &ProgramModel, exact: &ExactShapes) -> Refined {
    Refined {
        relations: relations_with(model, &[], exact),
        issues: validate_with(model, exact),
        truncated: None,
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
    // The loads reuse the contacts the load path measured.
    let mut faces = contact::ContactFaces::default();
    let mut issues =
        validate::validate_with_faces(&evaluated.model, &ExactShapes::default(), &mut faces);
    locate_issues(&mut issues, &evaluated.part_sources);
    let errors = issues
        .iter()
        .filter(|issue| issue.severity == Severity::Error)
        .count();
    let loads = loads::loads_with(&evaluated.model, &mut faces);
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
        truncated: None,
        part_sources: evaluated.part_sources.clone(),
        checks_pending: false,
    }
}

/// [`report`] of an edit whose exact check follows and re-derives every issue
/// and relation anyway: it lists only the collisions on the boxes, which say
/// what that check cleared, and no relation. Its joints, loads and the rest
/// are those of [`report`]. Published without the exact check, it needs
/// [`refine`] without exact answers first ([`Report::checks_pending`]).
#[must_use]
pub fn report_before_exact(evaluated: &Evaluated) -> Report {
    let loads = loads::loads(&evaluated.model);
    let mut report = Report {
        ok: true,
        errors: 0,
        warnings: 0,
        issues: Vec::new(),
        relations: Vec::new(),
        params: evaluated.model.params.clone(),
        bom: bom(&evaluated.model),
        joints: joint_check::joint_checks(&evaluated.model, &loads),
        design: member_check::member_checks(&evaluated.model, &loads),
        loads,
        log: evaluated.log.clone(),
        unused_overrides: evaluated.unused_overrides.clone(),
        truncated: None,
        part_sources: evaluated.part_sources.clone(),
        checks_pending: true,
    };
    report.set_issues(validate::collisions_only(&evaluated.model));
    report
}

/// Names on every issue the program lines that define its parts, in order and
/// without repeats; an issue that already names its lines keeps them.
fn locate_issues(issues: &mut [Issue], part_sources: &BTreeMap<String, Vec<SourceLines>>) {
    for issue in issues
        .iter_mut()
        .filter(|issue| issue.source_lines.is_empty())
    {
        let mut lines = issue
            .parts
            .iter()
            .filter_map(|part| part_sources.get(part))
            .flatten()
            .copied()
            .collect::<Vec<_>>();
        lines.sort_unstable();
        lines.dedup();
        issue.source_lines = lines;
    }
}
