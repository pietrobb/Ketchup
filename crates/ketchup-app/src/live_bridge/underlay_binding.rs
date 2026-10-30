//! Persistent selectors for generated program sections. Refresh through the existing
//! program planner before publishing an underlay edit, never as a second mutation.
use super::*;
use ketchup_application::{RuleProgramApplyError, RuleProgramChange, SessionError};
use ketchup_model::document::{CanonicalCommand, RuleProgramSource};
use ketchup_model::underlay::object_query::{ObjectFilter, ObjectIndex};
use ketchup_model::underlay::{Underlay, UnderlayLines};

pub(super) const PREFIX: &str = "# underlay binding ";
/// How the heading of a bound section ends, and of one unlinked from its drawing.
pub(super) const FOLLOWS: &str = "Follows the referenced underlay.";
const UNLINKED: &str = "Unlinked from the drawing, kept as it was.";
use super::underlay_program::BEGIN;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Binding {
    pub name: String,
    pub underlay_id: u64,
    pub point: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub layers: Option<Vec<String>>,
    pub tolerance: Option<f64>,
    pub closures: Vec<[[f64; 2]; 2]>,
    object_id: Option<u32>,
    handle: Option<u64>,
    pub block: Option<String>,
    /// Document millimetres, not scaled with the drawing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thickness: Option<f64>,
    /// Written with `NAME_FRAME`, where the view the drawing is set up as puts the outline
    /// in space, for `drawing action=solid_from_views`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub frame: bool,
    /// The whole figure around `point`, its outer edge, not the innermost region.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub whole: bool,
}

impl Binding {
    pub fn new(
        app: &KetchupApp,
        snapshot: &Snapshot,
        underlay: &Underlay,
        request: Request,
    ) -> Result<Self, &'static str> {
        let Request::ProgramFromUnderlay {
            name,
            point_mm: point,
            label,
            layers,
            tolerance_mm: tolerance,
            closing_segments_mm: closures,
            object_id,
            block,
            thickness_mm: thickness,
            ..
        } = request
        else {
            return Err("invalid_params");
        };
        let layers = super::underlay_regions::chosen_layers(snapshot, underlay, layers.as_deref())?
            .into_iter()
            .map(|i| underlay.layers[i].name.clone())
            .collect();
        let handle = if let Some(id) = object_id {
            let lines = app
                .cached_underlay_lines(underlay)
                .ok_or("entity_not_found")?;
            Some(
                ObjectIndex::new(&lines)
                    .object(id)
                    .ok_or("entity_not_found")?
                    .handle,
            )
        } else {
            None
        };
        let drawing_point = |p| {
            let local = underlay.to_local(p);
            let offset = underlay.offset_mm();
            [local[0] + offset[0], local[1] + offset[1]]
        };
        Ok(Self {
            name,
            underlay_id: underlay.id.0,
            point: point.map(drawing_point),
            label,
            layers: Some(layers),
            tolerance: Some(
                tolerance.unwrap_or(ketchup_model::underlay::regions::DEFAULT_JOIN_MM)
                    / underlay.scale(),
            ),
            closures: closures
                .into_iter()
                .map(|ends| ends.map(drawing_point))
                .collect(),
            object_id,
            handle,
            block,
            thickness,
            frame: false,
            whole: false,
        })
    }

    pub fn object(&self, lines: &UnderlayLines) -> Result<Option<u32>, &'static str> {
        let Some(handle) = self.handle.filter(|handle| *handle != 0) else {
            return Ok(self.object_id);
        };
        let index = ObjectIndex::new(lines);
        let found = index
            .find(&ObjectFilter::default())
            .into_iter()
            .filter(|id| {
                index
                    .object(*id)
                    .is_some_and(|object| object.handle == handle)
            })
            .collect::<Vec<_>>();
        match found.as_slice() {
            [id] => Ok(Some(*id)),
            _ => Err(super::program_access::invalid(
                "object_id",
                "The linked object handle is missing or ambiguous.",
                "Restore the source object or select the intended object with from_underlay.",
            )),
        }
    }

    pub fn written(&self, app: &KetchupApp, snapshot: &Snapshot) -> Result<String, &'static str> {
        super::underlay_program::binding_data(app, snapshot, self)
    }
}

/// The rejection recorded for `code`.
fn recorded_rejection(code: &'static str) -> Rejection {
    let mut record = rejection_result(code, take_error_details());
    if let Some(fields) = record.as_object_mut() {
        fields.remove("details");
    }
    serde_json::from_value::<Rejection>(record)
        .unwrap_or_else(|error| rejected(code).caused_by(error))
}

fn selection_error(code: &'static str) -> RuleProgramApplyError {
    RuleProgramApplyError::Rejected(Box::new(recorded_rejection(code)))
}

/// The refusal `code` of the section `name` that follows the drawing and finds no outline
/// in it after the edit: what was not found, with the section named and how to keep it as
/// it is (review 2026-10-29 UO-64).
fn followed_section_lost(name: &str, code: &'static str) -> RuleProgramApplyError {
    let cause = recorded_rejection(code);
    let reason = format!(
        "{name} follows this drawing, and the edit leaves it no outline: {}",
        cause.reason_text()
    );
    let fix_hint = format!(
        "{} To keep {name} as it is, first unlink the areas from the drawing: Tools > {UNLINK_MENU} in the window, or program action=patch removing the line starting \"{PREFIX}\" under \"{BEGIN} {name}:\".",
        cause.fix_hint_text()
    );
    RuleProgramApplyError::Rejected(Box::new(
        cause.reason(reason).fix_hint(fix_hint.trim_start()),
    ))
}

/// The window's command that unlinks pulled areas from their drawing.
const UNLINK_MENU: &str = "Unlink pulled areas from the drawing";

/// What the edit changed of a section that follows a drawing: its measure before and after
/// (review 2026-10-29 UO-64), so a trim that joins two rooms does not grow a solid
/// unsaid.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FollowedChange {
    pub name: String,
    pub was: String,
    pub now: String,
}

/// The area, perimeter and holes the heading of the section `name` in `source` gives.
fn section_measure(source: &str, name: &str) -> Option<String> {
    let begin = format!("{BEGIN} {name}:");
    let heading = source.lines().find(|line| line.starts_with(&begin))?;
    let start = heading.find(", area ")? + 2;
    let end = start + heading[start..].find(" holes")? + " holes".len();
    Some(heading[start..end].to_owned())
}

/// The program `source` with every underlay binding line taken out: its sections keep the
/// outlines they have and no longer follow a drawing. None when nothing is bound.
pub(crate) fn unlinked(source: &str) -> Option<String> {
    let follows = format!(". {FOLLOWS}");
    let mut bound = false;
    let kept = source
        .split_inclusive('\n')
        .filter(|line| {
            let binding = line.starts_with(PREFIX);
            bound |= binding;
            !binding
        })
        .map(|line| {
            if line.starts_with(BEGIN) {
                line.replacen(&follows, &format!(". {UNLINKED}"), 1)
            } else {
                line.to_owned()
            }
        })
        .collect::<String>();
    bound.then_some(kept)
}

impl KetchupApp {
    /// Returns a normal program plan only when this batch changes a referenced underlay,
    /// with what it changes of the sections that follow it.
    pub(crate) fn plan_underlay_program_update(
        &self,
        batch: &CommandBatch,
    ) -> Result<
        Option<(
            RuleProgramSource,
            ketchup_application::RuleProgramPlan,
            Vec<FollowedChange>,
        )>,
        RuleProgramApplyError,
    > {
        let Some(program) = self.document.current_rule_program() else {
            return Ok(None);
        };
        let changed = batch
            .commands()
            .iter()
            .filter_map(|command| match command {
                CanonicalCommand::UpsertUnderlay(underlay) => Some(underlay.id.0),
                CanonicalCommand::DeleteUnderlay { id } => Some(id.0),
                _ => None,
            })
            .collect::<BTreeSet<_>>();
        if changed.is_empty() {
            return Ok(None);
        }
        let mut bindings = Vec::new();
        for json in program
            .source
            .lines()
            .filter_map(|line| line.strip_prefix(PREFIX))
        {
            let binding: Binding = serde_json::from_str(json).map_err(|error| {
                RuleProgramApplyError::Rejected(Box::new(
                    rejected("invalid_params")
                        .target("program.underlay_binding")
                        .reason("The underlay binding is not valid JSON for a source selection.")
                        .fix_hint("Restore the generated source section with from_underlay.")
                        .caused_by(error),
                ))
            })?;
            if changed.contains(&binding.underlay_id) {
                bindings.push(binding);
            }
        }
        if bindings.is_empty() {
            return Ok(None);
        }
        let snapshot = self
            .document
            .preview_batch(batch)
            .map_err(|error| RuleProgramApplyError::Session(SessionError::Canonical(error)))?;
        let mut source = program.clone();
        let mut changes = Vec::new();
        for binding in bindings {
            // Positional object IDs belong to one import blob, not to a later import.
            let id = ketchup_model::underlay::UnderlayId(binding.underlay_id);
            if binding.object_id.is_some()
                && binding.handle.is_none_or(|handle| handle == 0)
                && self
                    .document
                    .current()
                    .underlay(id)
                    .map(Underlay::lines_blob)
                    != snapshot.underlay(id).map(Underlay::lines_blob)
            {
                return Err(selection_error(super::program_access::invalid(
                    "object_id",
                    "The linked object has no handle; its identity is lost when the drawing data changes.",
                    "Restore the original drawing data, or unlink the generated section before replacing the drawing and select the intended object again with from_underlay.",
                )));
            }
            let written = binding
                .written(self, &snapshot)
                .map_err(|code| followed_section_lost(&binding.name, code))?;
            let was = section_measure(&source.source, &binding.name);
            source.source =
                super::underlay_program::with_data(&source.source, &binding.name, &written);
            if let (Some(was), Some(now)) = (was, section_measure(&written, &binding.name))
                && was != now
            {
                changes.push(FollowedChange {
                    name: binding.name,
                    was,
                    now,
                });
            }
        }
        if source == *program {
            return Ok(None);
        }
        let plan = ketchup_application::plan_rule_program(&self.document, &source)?;
        if matches!(plan.change, RuleProgramChange::Replacement) {
            return Err(RuleProgramApplyError::IncrementalUnsupported);
        }
        Ok(Some((source, plan, changes)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::error::Error;

    #[test]
    fn selection_keeps_the_recorded_rejection_and_cause_chain() {
        let rejection = rejected("invalid_params")
            .target("object_id")
            .reason("The linked object is ambiguous.")
            .fix_hint("Select a unique source object.")
            .caused_by("invalid".parse::<u64>().unwrap_err());
        record_rejection(&rejection, json!({"published":false}));
        let error = selection_error("invalid_params");
        assert_eq!(error.rejection(), rejection);
        let retained = error.source().unwrap().downcast_ref::<Rejection>().unwrap();
        assert_eq!(retained, &rejection);
        assert!(retained.source().is_some());
        let code = program_failure(error);
        let response = rejection_result(code, take_error_details());
        assert_eq!(response["target"], "object_id");
        assert_eq!(response["fix_hint"], rejection.fix_hint_text());
        assert_eq!(response["causes"], json!(rejection.causes()));
    }
}
