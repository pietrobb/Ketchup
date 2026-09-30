use crate::{DocumentSession, SessionError};
use ketchup_geometry::sketch::{PadOperation, PadSpec};
use ketchup_model::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, DocumentStore, EdgeFinishKind,
    FeatureKind, FeatureParameterTarget, OccurrenceId, ParameterValueType, RuleProgramSource,
    Snapshot, Transform,
};
use ketchup_model::tolerance::ACCUMULATED_ROUNDING;
use ketchup_program::model::ProgramOperation;
use ketchup_program::{ProgramFeatureKind, ProgramModel, ProgramParameterValueType, Report};

#[derive(Debug)]
pub enum RuleProgramApplyError {
    Program { code: String, message: String },
    IncrementalUnsupported,
    ReplacementConfirmationRequired,
    UnsavedChanges,
    Session(SessionError),
}

impl std::fmt::Display for RuleProgramApplyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Program { message, .. } => f.write_str(message),
            Self::IncrementalUnsupported => f.write_str(
                "this change cannot yet update the existing program model; the document and undo history were not replaced",
            ),
            Self::ReplacementConfirmationRequired => f.write_str(
                "the existing document is not owned by this program; pass discard_unsaved=true to replace it",
            ),
            Self::UnsavedChanges => f.write_str(
                "replacing the document would lose unsaved changes; save first or pass discard_unsaved=true",
            ),
            Self::Session(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RuleProgramApplyError {}

impl From<SessionError> for RuleProgramApplyError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}

pub struct RuleProgramApplyResult {
    pub snapshot: Snapshot,
    pub model: ProgramModel,
    pub report: Report,
    pub replaced_document: bool,
}

/// Program lines that created or changed each part, keyed by part (occurrence) name.
pub fn rule_program_part_sources(
    source: &RuleProgramSource,
) -> Result<std::collections::BTreeMap<String, Vec<ketchup_program::SourceLines>>, String> {
    ketchup_program::evaluate(&source.file_name, &source.source, &source.overrides)
        .map(|evaluated| evaluated.part_sources)
        .map_err(|error| error.message)
}

/// Rewrites a manual Push/Pull into the program source that owns the part.
/// A uniquely driving parameter is overridden; otherwise a named operation is appended.
pub fn rewrite_rule_program_push_pull(
    source: &RuleProgramSource,
    part_name: &str,
    face_name: &str,
    distance_mm: f64,
) -> Result<RuleProgramSource, RuleProgramApplyError> {
    if !distance_mm.is_finite() || distance_mm.abs() <= f64::EPSILON {
        return Err(RuleProgramApplyError::Program {
            code: "invalid_push_pull".to_owned(),
            message: "Push/Pull distance must be finite and non-zero".to_owned(),
        });
    }
    let (evaluated, _) = evaluate(source)?;
    let part = evaluated
        .model
        .part(part_name)
        .ok_or_else(|| RuleProgramApplyError::Program {
            code: "unknown_part".to_owned(),
            message: format!("program part {part_name:?} does not exist"),
        })?;
    let controlled_value = |model: &ProgramModel| {
        let part = model.part(part_name)?;
        if let Some(offset) = part
            .face_offsets()
            .filter(|offset| offset.face == face_name)
            .last()
        {
            return Some(offset.distance_mm);
        }
        match (&part.body, face_name) {
            (ketchup_program::model::ProgramPartBody::Extrusion { distance_mm, .. }, "end") => {
                Some(*distance_mm)
            }
            // A board is padded along its third size component.
            (ketchup_program::model::ProgramPartBody::Panel, "end" | "z+") => Some(part.size_mm[2]),
            _ => None,
        }
    };
    let baseline = controlled_value(&evaluated.model);
    let mut drivers = Vec::new();
    if let Some(baseline) = baseline {
        for parameter in &evaluated.model.params {
            let current = source
                .overrides
                .get(&parameter.name)
                .copied()
                .unwrap_or(parameter.value);
            // not a tolerance: a probe step large enough to change the evaluated model.
            let step = current.abs().mul_add(1.0e-4, 1.0e-3);
            let probed = if parameter
                .max
                .is_none_or(|maximum| current + step <= maximum)
            {
                current + step
            } else if parameter
                .min
                .is_none_or(|minimum| current - step >= minimum)
            {
                current - step
            } else {
                continue;
            };
            let mut probe_source = source.clone();
            probe_source
                .overrides
                .insert(parameter.name.clone(), probed);
            let (probe, _) = evaluate(&probe_source)?;
            let Some(probe_value) = controlled_value(&probe.model) else {
                continue;
            };
            let slope = (probe_value - baseline) / (probed - current);
            if slope.abs() > ACCUMULATED_ROUNDING {
                let replacement = current + distance_mm / slope;
                if parameter.min.is_none_or(|minimum| replacement >= minimum)
                    && parameter.max.is_none_or(|maximum| replacement <= maximum)
                {
                    drivers.push((parameter.name.clone(), replacement));
                }
            }
        }
    }
    let mut rewritten = source.clone();
    if let [(parameter, value)] = drivers.as_slice() {
        rewritten.overrides.insert(parameter.clone(), *value);
    } else {
        let quote = |value: &str| serde_json::to_string(value).expect("strings serialize");
        let feature_name = format!("GUI Push/Pull {}", part.face_offsets().count() + 1);
        rewritten.source.push_str(&format!(
            "\npush_pull({}, face={}, distance={}, name={})\n",
            quote(part_name),
            quote(face_name),
            distance_mm,
            quote(&feature_name)
        ));
    }
    evaluate(&rewritten)?;
    Ok(rewritten)
}

/// What publishing a rule program changes in a document.
#[derive(Debug)]
pub enum RuleProgramChange {
    /// The document already holds exactly this source.
    Unchanged,
    /// Only the source changes; geometry and identities stay as they are.
    SourceOnly,
    /// Canonical edits of the affected parts only, published with the source as one Undo step.
    Incremental(CommandBatch),
    /// The document is not owned by a program; its content would be replaced.
    Replacement,
}

pub struct RuleProgramPlan {
    pub evaluated: ketchup_program::Evaluated,
    pub report: Report,
    pub change: RuleProgramChange,
}

/// Plans how `source` updates `document`. This is the one reconciliation path shared by the
/// headless session and the GUI window.
///
/// # Errors
/// Returns the program error, or `IncrementalUnsupported` when the existing program model
/// cannot be updated in place.
pub fn plan_rule_program(
    document: &DocumentStore,
    source: &RuleProgramSource,
) -> Result<RuleProgramPlan, RuleProgramApplyError> {
    let (evaluated, report) = evaluate(source)?;
    let change = match document.current_rule_program() {
        Some(current) if current == source => RuleProgramChange::Unchanged,
        Some(current) => {
            let (old, _) = evaluate(current)?;
            let batch = incremental_batch(document, &old.model, &evaluated.model)?
                .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
            if batch.commands().is_empty() {
                RuleProgramChange::SourceOnly
            } else {
                RuleProgramChange::Incremental(batch)
            }
        }
        None => RuleProgramChange::Replacement,
    };
    Ok(RuleProgramPlan {
        evaluated,
        report,
        change,
    })
}

impl DocumentSession {
    /// Evaluates and publishes a rule program through the canonical document session.
    /// Incremental updates preserve identities; replacing an unrelated document requires
    /// explicit permission and still publishes the source and geometry atomically.
    pub fn apply_rule_program(
        &mut self,
        source: RuleProgramSource,
        allow_replacement: bool,
    ) -> Result<RuleProgramApplyResult, RuleProgramApplyError> {
        let RuleProgramPlan {
            evaluated,
            report,
            change,
        } = plan_rule_program(self.document_store(), &source)?;
        let snapshot = match change {
            RuleProgramChange::Unchanged => self.snapshot(),
            RuleProgramChange::SourceOnly => self.replace_rule_program_source(source)?,
            RuleProgramChange::Incremental(batch) => {
                self.apply_rule_commands_with_source(batch, &[], source)?
            }
            RuleProgramChange::Replacement => {
                if self.snapshot().definitions().next().is_some() && !allow_replacement {
                    return Err(if self.is_modified() {
                        RuleProgramApplyError::UnsavedChanges
                    } else {
                        RuleProgramApplyError::ReplacementConfirmationRequired
                    });
                }
                let snapshot = self.replace_with_rule_parts(&evaluated.model.parts, source)?;
                return Ok(RuleProgramApplyResult {
                    snapshot,
                    model: evaluated.model,
                    report,
                    replaced_document: true,
                });
            }
        };
        Ok(RuleProgramApplyResult {
            snapshot,
            model: evaluated.model,
            report,
            replaced_document: false,
        })
    }
}

/// Commands that turn the document of `old` into the document of `new`, or `None` when the
/// document cannot be updated in place. Program joints are not stored in the document (their
/// holes belong to the parts), so they never block an update.
fn incremental_batch(
    document: &DocumentStore,
    old: &ProgramModel,
    new: &ProgramModel,
) -> Result<Option<CommandBatch>, RuleProgramApplyError> {
    let snapshot = document.current();
    // A manually added part is not described by the program; do not guess its fate.
    // A shape only drawn (a profile without a solid) is left as it is.
    let drawing_only = |definition_id| {
        snapshot
            .definition(definition_id)
            .is_some_and(|definition| {
                definition.feature_ids().iter().all(|id| {
                    snapshot.feature(*id).is_some_and(|feature| {
                        matches!(feature.kind(), FeatureKind::Profile { .. })
                    })
                })
            })
    };
    if snapshot
        .occurrences()
        .filter(|occurrence| !drawing_only(occurrence.definition_id()))
        .count()
        != old.parts.len()
    {
        return Ok(None);
    }
    let mut removals = Vec::new();
    append_removed_parts(&snapshot, old, new, &mut removals)?;
    let added = added_parts(old, new);
    if old.parts.iter().all(|before| {
        new.part(&before.name)
            .is_none_or(|after| program_feature_references_match(before, after))
    }) && let Some(mut commands) = feature_level_changes(&snapshot, old, new)
    {
        commands.extend(removals);
        let additions = crate::planner::plan_rule_part_batch(document, &added)
            .map_err(SessionError::Planning)?;
        commands.extend(additions.commands().iter().cloned());
        return Ok(Some(CommandBatch::new(commands)));
    }
    let Some(replacements) = part_replacements(&snapshot, old, new) else {
        return Ok(None);
    };
    replacement_batch(document, &snapshot, &replacements, &added, removals).map(Some)
}

/// Parameter edits of parts whose feature trees keep their shape.
fn feature_level_changes(
    snapshot: &Snapshot,
    old: &ProgramModel,
    new: &ProgramModel,
) -> Option<Vec<CanonicalCommand>> {
    let mut commands = Vec::new();
    for before in &old.parts {
        let Some(after) = new.part(&before.name) else {
            continue;
        };
        let mut comparable = after.clone();
        comparable.at_mm = before.at_mm;
        comparable.rotation = before.rotation;
        comparable.size_mm = before.size_mm;
        comparable.body = before.body.clone();
        comparable.operations = before.operations.clone();
        comparable.holes = before.holes.clone();
        comparable.pockets = before.pockets.clone();
        comparable.features = before.features.clone();
        // Tools are placed in the part's frame, so moving a part with booleans rebuilds it.
        if &comparable != before
            || !after.booleans().eq(before.booleans())
            || (after.booleans().next().is_some()
                && before.transform_matrix() != after.transform_matrix())
        {
            return None;
        }
        let occurrence = snapshot.occurrences().find(|o| o.name() == before.name)?;
        if occurrence.transform() != Transform::from_matrix(before.transform_matrix()).ok()? {
            return None;
        }
        commands.extend(program_feature_commands(
            snapshot,
            occurrence.definition_id(),
            before,
            after,
        )?);
        if before.transform_matrix() != after.transform_matrix() {
            let transform = Transform::from_matrix(after.transform_matrix()).ok()?;
            commands.push(CanonicalCommand::SetOccurrenceTransform {
                id: occurrence.id(),
                transform,
            });
        }
    }
    Some(commands)
}

/// Parts that must be rebuilt, each keeping its occurrence identity.
fn part_replacements(
    snapshot: &Snapshot,
    old: &ProgramModel,
    new: &ProgramModel,
) -> Option<Vec<(OccurrenceId, ketchup_program::model::Part)>> {
    let mut replacements = Vec::new();
    for before in &old.parts {
        let Some(after) = new.part(&before.name) else {
            continue;
        };
        let mut comparable = after.clone();
        comparable.at_mm = before.at_mm;
        comparable.rotation = before.rotation;
        comparable.size_mm = before.size_mm;
        comparable.operations = before.operations.clone();
        comparable.holes = before.holes.clone();
        comparable.pockets = before.pockets.clone();
        comparable.body = before.body.clone();
        comparable.features = before.features.clone();
        if comparable != *before {
            return None;
        }
        let occurrence = snapshot.occurrences().find(|o| o.name() == before.name)?;
        if occurrence.transform() != Transform::from_matrix(before.transform_matrix()).ok()? {
            return None;
        }
        if before != after {
            replacements.push((occurrence.id(), after.clone()));
        }
    }
    Some(replacements)
}

/// Rebuilds `replacements` under their existing occurrence IDs and adds `added` parts.
fn replacement_batch(
    document: &DocumentStore,
    snapshot: &Snapshot,
    replacements: &[(OccurrenceId, ketchup_program::model::Part)],
    added: &[ketchup_program::model::Part],
    other_commands: Vec<CanonicalCommand>,
) -> Result<CommandBatch, RuleProgramApplyError> {
    let parts = replacements
        .iter()
        .map(|(_, part)| part.clone())
        .chain(added.iter().cloned())
        .collect::<Vec<_>>();
    let additions =
        crate::planner::plan_rule_part_batch(document, &parts).map_err(SessionError::Planning)?;
    let definitions = additions
        .commands()
        .iter()
        .filter_map(|command| match command {
            CanonicalCommand::CreateDefinition { id, .. } => Some(*id),
            _ => None,
        })
        .collect::<Vec<_>>();
    let temporary_occurrences = additions
        .commands()
        .iter()
        .filter_map(|command| match command {
            CanonicalCommand::CreateOccurrence { id, transform, .. } => Some((*id, *transform)),
            _ => None,
        })
        .collect::<Vec<_>>();
    let mut commands = additions.commands().to_vec();
    for ((old_id, _), (definition_id, (temporary_id, transform))) in replacements
        .iter()
        .zip(definitions.into_iter().zip(temporary_occurrences))
    {
        commands.push(CanonicalCommand::SetOccurrenceTransform {
            id: *old_id,
            transform,
        });
        let old = snapshot
            .occurrence(*old_id)
            .ok_or_else(|| SessionError::Persistence("program part is missing".into()))?;
        if snapshot
            .occurrences()
            .filter(|item| item.definition_id() == old.definition_id())
            .count()
            != 1
        {
            return Err(SessionError::Persistence(
                "cannot rebuild a shared program definition".into(),
            )
            .into());
        }
        commands.push(CanonicalCommand::DeleteOccurrence { id: temporary_id });
        commands.push(CanonicalCommand::RepointOccurrence {
            id: *old_id,
            definition_id,
        });
        commands.push(CanonicalCommand::DeleteDefinition {
            id: old.definition_id(),
        });
    }
    commands.extend(other_commands);
    Ok(CommandBatch::new(commands))
}

fn evaluate(
    source: &RuleProgramSource,
) -> Result<(ketchup_program::Evaluated, Report), RuleProgramApplyError> {
    ketchup_program::run(&source.file_name, &source.source, &source.overrides).map_err(|error| {
        RuleProgramApplyError::Program {
            code: error.code.to_owned(),
            message: error.message,
        }
    })
}

fn append_removed_parts(
    snapshot: &Snapshot,
    before: &ProgramModel,
    after: &ProgramModel,
    commands: &mut Vec<CanonicalCommand>,
) -> Result<(), RuleProgramApplyError> {
    for removed in before
        .parts
        .iter()
        .filter(|part| after.part(&part.name).is_none())
    {
        let occurrence = snapshot
            .occurrences()
            .find(|item| item.name() == removed.name)
            .ok_or(RuleProgramApplyError::IncrementalUnsupported)?;
        commands.push(CanonicalCommand::DeleteOccurrence {
            id: occurrence.id(),
        });
        commands.push(CanonicalCommand::DeleteDefinition {
            id: occurrence.definition_id(),
        });
    }
    Ok(())
}

fn added_parts(before: &ProgramModel, after: &ProgramModel) -> Vec<ketchup_program::model::Part> {
    after
        .parts
        .iter()
        .filter(|part| before.part(&part.name).is_none())
        .cloned()
        .collect()
}

fn program_feature_references_match(
    before: &ketchup_program::model::Part,
    after: &ketchup_program::model::Part,
) -> bool {
    // Names plus line/arc and arc direction: points and centres can change
    // through parameters, the segment kind cannot.
    fn segment_names(
        segments: &[ketchup_program::model::ProgramProfileSegment],
    ) -> Vec<(&str, Option<bool>)> {
        segments
            .iter()
            .map(|segment| (segment.name.as_str(), segment.arc.map(|arc| arc.clockwise)))
            .collect()
    }
    let body_matches = match (&before.body, &after.body) {
        (
            ketchup_program::model::ProgramPartBody::Panel,
            ketchup_program::model::ProgramPartBody::Panel,
        ) => true,
        (
            ketchup_program::model::ProgramPartBody::Extrusion { segments: left, .. },
            ketchup_program::model::ProgramPartBody::Extrusion {
                segments: right, ..
            },
        )
        | (
            ketchup_program::model::ProgramPartBody::Revolve { segments: left, .. },
            ketchup_program::model::ProgramPartBody::Revolve {
                segments: right, ..
            },
        ) => segment_names(left) == segment_names(right),
        // Swept and lofted bodies have no editable parameters: equal or rebuilt.
        (
            ketchup_program::model::ProgramPartBody::Sweep { .. }
            | ketchup_program::model::ProgramPartBody::Loft { .. },
            _,
        ) => before.body == after.body,
        _ => false,
    };
    // Same operations in the same order, naming the same faces; only numbers may differ.
    let same_operation = |left: &ProgramOperation, right: &ProgramOperation| match (left, right) {
        (ProgramOperation::Cut(left), ProgramOperation::Cut(right)) => {
            left.name == right.name
                && segment_names(&left.segments) == segment_names(&right.segments)
        }
        (ProgramOperation::Finish(left), ProgramOperation::Finish(right)) => {
            left.name == right.name && left.kind == right.kind && left.edges == right.edges
        }
        (ProgramOperation::FaceOffset(left), ProgramOperation::FaceOffset(right)) => {
            left.name == right.name && left.face == right.face
        }
        (ProgramOperation::Boolean(left), ProgramOperation::Boolean(right)) => left == right,
        (ProgramOperation::Mirror(left), ProgramOperation::Mirror(right)) => left == right,
        (ProgramOperation::Shell(left), ProgramOperation::Shell(right)) => left == right,
        _ => false,
    };
    body_matches
        && before.operations.len() == after.operations.len()
        && before
            .operations
            .iter()
            .zip(&after.operations)
            .all(|(left, right)| same_operation(left, right))
}

fn program_feature_commands(
    snapshot: &Snapshot,
    definition_id: DefinitionId,
    before: &ketchup_program::model::Part,
    after: &ketchup_program::model::Part,
) -> Option<Vec<CanonicalCommand>> {
    let definition = snapshot.definition(definition_id)?;
    let before_features = before
        .features
        .iter()
        .filter(|feature| feature.kind != ProgramFeatureKind::Transform)
        .collect::<Vec<_>>();
    let after_features = after
        .features
        .iter()
        .filter(|feature| feature.kind != ProgramFeatureKind::Transform)
        .collect::<Vec<_>>();
    if before_features.len() != after_features.len() {
        return None;
    }
    let mut commands = Vec::new();
    for old in before_features {
        let mut matching_new = after_features
            .iter()
            .filter(|new| new.name == old.name && new.kind == old.kind);
        let new = *matching_new.next()?;
        if matching_new.next().is_some() || old.parameters.len() != new.parameters.len() {
            return None;
        }
        let mut matching_ids = definition.feature_ids().iter().filter(|id| {
            snapshot
                .feature(**id)
                .is_some_and(|feature| feature.name() == old.name)
        });
        let feature_id = *matching_ids.next()?;
        if matching_ids.next().is_some()
            || !program_feature_kind_matches(old.kind, snapshot.feature(feature_id)?.kind())
        {
            return None;
        }
        for old_parameter in &old.parameters {
            let mut matching_parameters = new.parameters.iter().filter(|parameter| {
                parameter.path == old_parameter.path
                    && parameter.value_type == old_parameter.value_type
            });
            let new_parameter = matching_parameters.next()?;
            if matching_parameters.next().is_some() {
                return None;
            }
            let value_type = match old_parameter.value_type {
                ProgramParameterValueType::Length => ParameterValueType::Length,
                ProgramParameterValueType::Angle => ParameterValueType::Angle,
                ProgramParameterValueType::Scalar => ParameterValueType::Scalar,
            };
            let target =
                FeatureParameterTarget::new(feature_id, &old_parameter.path, value_type).ok()?;
            if snapshot.feature_parameter_value(&target)? != old_parameter.value {
                return None;
            }
            if old_parameter.value != new_parameter.value {
                commands.push(CanonicalCommand::SetFeatureParameter {
                    target,
                    dimension: Dimension::new(new_parameter.value.to_string(), new_parameter.value)
                        .ok()?,
                });
            }
        }
    }
    Some(commands)
}

fn program_feature_kind_matches(expected: ProgramFeatureKind, actual: &FeatureKind) -> bool {
    matches!(
        (expected, actual),
        (ProgramFeatureKind::Workplane, FeatureKind::Workplane(_))
            | (
                ProgramFeatureKind::Sketch,
                FeatureKind::Sketch(_) | FeatureKind::Profile { .. }
            )
            | (
                ProgramFeatureKind::Pad,
                FeatureKind::Pad(PadSpec {
                    operation: PadOperation::NewBody,
                    ..
                })
            )
            | (
                ProgramFeatureKind::Cut,
                FeatureKind::Pad(PadSpec {
                    operation: PadOperation::Cut { .. },
                    ..
                })
            )
            | (ProgramFeatureKind::Revolve, FeatureKind::Revolve { .. })
            | (ProgramFeatureKind::Sweep, FeatureKind::Sweep { .. })
            | (ProgramFeatureKind::Loft, FeatureKind::Loft { .. })
            | (
                ProgramFeatureKind::Fillet,
                FeatureKind::EdgeFinish {
                    kind: EdgeFinishKind::Fillet,
                    ..
                }
            )
            | (
                ProgramFeatureKind::Chamfer,
                FeatureKind::EdgeFinish {
                    kind: EdgeFinishKind::Chamfer,
                    ..
                }
            )
            | (
                ProgramFeatureKind::FaceOffset,
                FeatureKind::FaceOffset { .. }
            )
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SessionSettings;
    use std::collections::BTreeMap;

    fn source(width: f64) -> RuleProgramSource {
        RuleProgramSource {
            file_name: "program.star".to_owned(),
            source: "W = param(\"width\", 100)\nbox(\"part\", [W, 20, 10])".to_owned(),
            overrides: BTreeMap::from([("width".to_owned(), width)]),
        }
    }

    fn profile_source(source: &str, parameter: &str, value: f64) -> RuleProgramSource {
        RuleProgramSource {
            file_name: "profile.star".to_owned(),
            source: source.to_owned(),
            overrides: BTreeMap::from([(parameter.to_owned(), value)]),
        }
    }

    fn exact_graph_of(
        session: &DocumentSession,
        occurrence_index: usize,
    ) -> ketchup_model::exact_brep_graph::ExactBRepGraph {
        let snapshot = session.snapshot();
        let occurrence = snapshot.occurrences().nth(occurrence_index).unwrap();
        let definition = snapshot.definition(occurrence.definition_id()).unwrap();
        let producer = *definition.feature_ids().last().unwrap();
        ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
            &snapshot,
            occurrence.definition_id(),
            producer,
        )
        .unwrap()
    }

    fn exact_signature(
        session: &DocumentSession,
    ) -> ketchup_model::exact_brep_graph::ExactBRepGraph {
        exact_signature_of(session, 0)
    }

    /// The exact graph of one occurrence without document-assigned IDs.
    fn exact_signature_of(
        session: &DocumentSession,
        occurrence_index: usize,
    ) -> ketchup_model::exact_brep_graph::ExactBRepGraph {
        let mut graph = exact_graph_of(session, occurrence_index);
        graph.document_id = 0;
        graph.source_revision = 0;
        graph.source_digest.clear();
        graph.definition_id = 0;
        graph.producer_feature_id = 0;
        graph.graph_digest.clear();
        graph.canonical_input_digest.clear();
        for profile in &mut graph.profiles {
            profile.source_feature_id = 0;
        }
        for node in &mut graph.nodes {
            node.source_feature_id = 0;
            if let ketchup_model::exact_brep_graph::ExactBRepOperation::SpatialSweep {
                path, ..
            } = &mut node.operation
            {
                path.source_feature_id = 0;
            }
        }
        graph
    }

    #[test]
    fn adding_changing_and_removing_a_doweled_board_keeps_every_other_part() {
        const TABLE: &str = include_str!("../../../examples/programs/table.star");
        const APRON: &str = "\nAPRON = param(\"apron_height\", 80, min = 40, max = 150)\napron = board(\"table/apron-front\", (WIDTH - 2 * (INSET + LEG), 20, APRON), at = (INSET + LEG, INSET + 25, HEIGHT - APRON - 20))\ndowels(legs[0], apron, dowel = \"8x30\", margin = 15)\n";
        let program = |text: String, overrides: &[(&str, f64)]| RuleProgramSource {
            file_name: "table.star".to_owned(),
            source: text,
            overrides: overrides
                .iter()
                .map(|(name, value)| ((*name).to_owned(), *value))
                .collect(),
        };
        let ids = |snapshot: &Snapshot| {
            snapshot
                .occurrences()
                .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
                .collect::<BTreeMap<_, _>>()
        };
        let mut session = DocumentSession::new(SessionSettings::default());
        let table = session
            .apply_rule_program(program(TABLE.to_owned(), &[]), false)
            .unwrap();
        let original = ids(&table.snapshot);
        assert_eq!(original.len(), 5);

        let with_apron = program(format!("{TABLE}{APRON}"), &[]);
        let plan = plan_rule_program(session.document_store(), &with_apron).unwrap();
        assert!(matches!(plan.change, RuleProgramChange::Incremental(_)));
        assert!(plan.report.ok, "{:?}", plan.report.issues);
        let added = session.apply_rule_program(with_apron, false).unwrap();
        assert!(!added.replaced_document);
        let after_add = ids(&added.snapshot);
        assert_eq!(after_add.len(), 6);
        assert!(original.iter().all(|(name, id)| after_add[name] == *id));
        let apron_id = after_add["table/apron-front"];

        let taller = session
            .apply_rule_program(
                program(format!("{TABLE}{APRON}"), &[("apron_height", 120.0)]),
                false,
            )
            .unwrap();
        assert_eq!(ids(&taller.snapshot)["table/apron-front"], apron_id);
        assert_eq!(ids(&taller.snapshot).len(), 6);

        let removed = session
            .apply_rule_program(program(TABLE.to_owned(), &[]), false)
            .unwrap();
        assert_eq!(ids(&removed.snapshot), original);

        session.undo().unwrap();
        assert_eq!(ids(&session.snapshot())["table/apron-front"], apron_id);
        assert_eq!(
            session.rule_program().unwrap().overrides["apron_height"],
            120.0
        );
    }

    #[test]
    fn application_session_owns_rule_program_reconciliation() {
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session.apply_rule_program(source(100.0), false).unwrap();
        let occurrence_id = first.snapshot.occurrences().next().unwrap().id();
        let first_digest = first.snapshot.canonical_digest().to_owned();
        assert!(first.replaced_document);

        let second = session.apply_rule_program(source(140.0), false).unwrap();
        assert!(!second.replaced_document);
        assert_eq!(
            second.snapshot.occurrences().next().unwrap().id(),
            occurrence_id
        );
        assert_ne!(second.snapshot.canonical_digest(), first_digest);

        session.undo().unwrap();
        assert_eq!(session.rule_program().unwrap().overrides["width"], 100.0);
    }

    #[test]
    fn tilted_part_is_placed_at_its_frame_and_retilting_keeps_its_identity() {
        let program =
            "X = param(\"splay\", 100)\nmember(\"leg\", (0, 0, 0), (X, 0, 450), (40, 40))";
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session
            .apply_rule_program(profile_source(program, "splay", 100.0), false)
            .unwrap();
        let (evaluated, _) = evaluate(&profile_source(program, "splay", 100.0)).unwrap();
        let occurrence = first.snapshot.occurrences().next().unwrap();
        let occurrence_id = occurrence.id();
        assert_eq!(
            occurrence.transform().matrix(),
            &evaluated.model.parts[0].transform_matrix()
        );

        let retilted = session
            .apply_rule_program(profile_source(program, "splay", 150.0), false)
            .unwrap();
        let (evaluated, _) = evaluate(&profile_source(program, "splay", 150.0)).unwrap();
        let occurrence = retilted.snapshot.occurrences().next().unwrap();
        assert!(!retilted.replaced_document);
        assert_eq!(occurrence.id(), occurrence_id);
        assert_eq!(
            occurrence.transform().matrix(),
            &evaluated.model.parts[0].transform_matrix()
        );
    }

    #[test]
    fn boolean_tools_become_consumed_hidden_bodies_and_follow_parameter_changes() {
        const STOOL: &str = include_str!("../../../examples/programs/round_stool.star");
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session
            .apply_rule_program(profile_source(STOOL, "splay", 260.0), false)
            .unwrap();
        let ids = |snapshot: &Snapshot| {
            snapshot
                .occurrences()
                .map(|occurrence| (occurrence.name().to_owned(), occurrence.id()))
                .collect::<BTreeMap<_, _>>()
        };
        let original = ids(&first.snapshot);
        assert_eq!(original.len(), 7, "tools are not published as parts");
        let seat_definition = |snapshot: &Snapshot| {
            let seat = snapshot
                .occurrences()
                .find(|occurrence| occurrence.name() == "seat")
                .unwrap();
            snapshot.definition(seat.definition_id()).unwrap().clone()
        };
        let seat = seat_definition(&first.snapshot);
        // Three legs, each carrying two trims and two rail notches of its own.
        assert_eq!(seat.bodies().count(), 1 + 3 * 5);
        assert_eq!(
            seat.bodies()
                .filter(|body| body.consumed_by().is_none())
                .count(),
            1
        );
        for occurrence in first.snapshot.occurrences() {
            let definition = first
                .snapshot
                .definition(occurrence.definition_id())
                .unwrap();
            let producer = *definition.feature_ids().last().unwrap();
            ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
                &first.snapshot,
                occurrence.definition_id(),
                producer,
            )
            .unwrap_or_else(|error| panic!("{}: {error:?}", occurrence.name()));
        }

        let wider = session
            .apply_rule_program(profile_source(STOOL, "splay", 300.0), false)
            .unwrap();
        assert!(!wider.replaced_document);
        assert_eq!(ids(&wider.snapshot), original);
        assert_eq!(seat_definition(&wider.snapshot).bodies().count(), 16);
    }

    #[test]
    fn turned_notched_and_doweled_parts_build_exact_bodies() {
        const PROGRAM: &str = "A = param(\"angle\", 37)\n\
            leg = box(\"leg\", (40, 40, 400))\n\
            rail = box(\"rail\", (300, 20, 30), at = (20, 10, 200))\n\
            subtract(leg, box(\"notch\", (20, 20, 30), at = (20, 10, 200), tool = True))\n\
            side = box(\"side\", (18, 400, 600), at = (0, 100, 0))\n\
            shelf = box(\"shelf\", (500, 400, 18), at = (18, 100, 300))\n\
            for p in (leg, rail, side, shelf):\n\
            \x20   rotate(p, axis = (1, 2, 3), angle = A, pivot = (0, 0, 0))\n\
            dowels(side, shelf)\n";
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session
            .apply_rule_program(profile_source(PROGRAM, "angle", 37.0), false)
            .unwrap();
        assert!(first.report.ok, "{:#?}", first.report.issues);
        let build_all = |snapshot: &Snapshot| {
            for occurrence in snapshot.occurrences() {
                let definition = snapshot.definition(occurrence.definition_id()).unwrap();
                let producer = *definition.feature_ids().last().unwrap();
                ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
                    snapshot,
                    occurrence.definition_id(),
                    producer,
                )
                .unwrap_or_else(|error| panic!("{}: {error:?}", occurrence.name()));
            }
        };
        build_all(&first.snapshot);
        let turned = session
            .apply_rule_program(profile_source(PROGRAM, "angle", 12.0), false)
            .unwrap();
        assert!(!turned.replaced_document);
        build_all(&turned.snapshot);
    }

    #[test]
    fn extruded_profile_update_preserves_identity_and_matches_fresh_build() {
        let program = "W = param(\"width\", 40)\npart = extrude(\"angle\", profile=[[\"bottom\", [0, 0], [W, 0]], [\"outer\", [W, 0], [W, 10]], [\"ledge\", [W, 10], [10, 10]], [\"inner\", [10, 10], [10, 40]], [\"top\", [10, 40], [0, 40]], [\"back\", [0, 40], [0, 0]]], distance=100)\nfillet(part, edges=[[\"bottom\", \"outer\"]], radius=2, name=\"outer round\")";
        let mut incremental = DocumentSession::new(SessionSettings::default());
        let first = incremental
            .apply_rule_program(profile_source(program, "width", 40.0), false)
            .unwrap();
        let occurrence_id = first.snapshot.occurrences().next().unwrap().id();
        let updated = incremental
            .apply_rule_program(profile_source(program, "width", 55.0), false)
            .unwrap();

        let mut fresh = DocumentSession::new(SessionSettings::default());
        fresh
            .apply_rule_program(profile_source(program, "width", 55.0), false)
            .unwrap();
        assert_eq!(
            updated.snapshot.occurrences().next().unwrap().id(),
            occurrence_id
        );
        assert_eq!(exact_signature(&incremental), exact_signature(&fresh));
    }

    #[test]
    fn swept_and_lofted_parts_rebuild_in_place_like_a_fresh_build() {
        let program = "B = param(\"bend\", 100)\n\
            sweep(\"rail\", profile=[(-15, 0), (15, 0), (15, 20), (-15, 20)], path=[(0, 0, 0), (600, 0, 0), (600, 400, 0)], bend=B)\n\
            loft(\"leg\", sections=[([(0, 0), (40, 0), (40, 40), (0, 40)], 0), ([(B / 10, B / 10), (40 - B / 10, B / 10), (40 - B / 10, 40 - B / 10), (B / 10, 40 - B / 10)], 700)], at=(0, 600, 0))";
        let mut incremental = DocumentSession::new(SessionSettings::default());
        let first = incremental
            .apply_rule_program(profile_source(program, "bend", 100.0), false)
            .unwrap();
        let ids: Vec<_> = first.snapshot.occurrences().map(|o| o.id()).collect();
        let updated = incremental
            .apply_rule_program(profile_source(program, "bend", 150.0), false)
            .unwrap();
        let mut fresh = DocumentSession::new(SessionSettings::default());
        fresh
            .apply_rule_program(profile_source(program, "bend", 150.0), false)
            .unwrap();
        assert_eq!(
            updated
                .snapshot
                .occurrences()
                .map(|o| o.id())
                .collect::<Vec<_>>(),
            ids
        );
        for index in 0..2 {
            assert_eq!(
                exact_signature_of(&incremental, index),
                exact_signature_of(&fresh, index)
            );
        }
        assert_ne!(
            exact_signature_of(&incremental, 1),
            exact_signature_of(&first_session(program), 1),
            "the loft follows the parameter"
        );
    }

    fn first_session(program: &str) -> DocumentSession {
        let mut session = DocumentSession::new(SessionSettings::default());
        session
            .apply_rule_program(profile_source(program, "bend", 100.0), false)
            .unwrap();
        session
    }

    #[test]
    fn named_cut_and_split_face_push_pull_graph_survives_parameter_changes() {
        let program = "L = param(\"length\", 500)\nG = param(\"groove\", 150)\nP = param(\"pull\", 10)\npart = extrude(\"board\", profile=[[\"front\", [0, 0], [L, 0]], [\"right\", [L, 0], [L, 300]], [\"back\", [L, 300], [0, 300]], [\"left\", [0, 300], [0, 0]]], distance=18)\ncut(part, profile=[[\"entry\", [G, -1], [G + 8, -1]], [\"wall_right\", [G + 8, -1], [G + 8, 301]], [\"exit\", [G + 8, 301], [G, 301]], [\"wall_left\", [G, 301], [G, -1]]], depth=6, name=\"groove\")\npush_pull(part, face=\"end#2\", distance=P, name=\"raise right half\")";
        let source = |length, groove, pull| RuleProgramSource {
            file_name: "split.star".to_owned(),
            source: program.to_owned(),
            overrides: BTreeMap::from([
                ("length".to_owned(), length),
                ("groove".to_owned(), groove),
                ("pull".to_owned(), pull),
            ]),
        };
        let mut incremental = DocumentSession::new(SessionSettings::default());
        let first = incremental
            .apply_rule_program(source(500.0, 150.0, 10.0), false)
            .unwrap();
        let occurrence_id = first.snapshot.occurrences().next().unwrap().id();
        for (length, groove, pull) in [(500.0, 320.0, 10.0), (700.0, 40.0, -4.0)] {
            let updated = incremental
                .apply_rule_program(source(length, groove, pull), false)
                .unwrap();
            assert_eq!(
                updated.snapshot.occurrences().next().unwrap().id(),
                occurrence_id
            );
            let mut fresh = DocumentSession::new(SessionSettings::default());
            fresh
                .apply_rule_program(source(length, groove, pull), false)
                .unwrap();
            assert_eq!(exact_signature(&incremental), exact_signature(&fresh));
        }
    }

    #[test]
    fn push_pull_rewrites_driver_or_appends_named_operation() {
        let driven = RuleProgramSource {
            file_name: "driven.star".to_owned(),
            source: "H = param(\"height\", 18)\npart = extrude(\"board\", profile=[[\"a\", [0, 0], [100, 0]], [\"b\", [100, 0], [100, 50]], [\"c\", [100, 50], [0, 50]], [\"d\", [0, 50], [0, 0]]], distance=H)".to_owned(),
            overrides: BTreeMap::new(),
        };
        let rewritten = rewrite_rule_program_push_pull(&driven, "board", "end", 7.0).unwrap();
        assert_eq!(rewritten.source, driven.source);
        assert_eq!(rewritten.overrides["height"], 25.0);
        let mut session = DocumentSession::new(SessionSettings::default());
        let first = session.apply_rule_program(driven.clone(), false).unwrap();
        let updated = session
            .apply_rule_program(rewritten.clone(), false)
            .unwrap();
        assert_eq!(
            updated.snapshot.revision_id(),
            first.snapshot.revision_id() + 1
        );
        assert_eq!(session.rule_program(), Some(&rewritten));
        session.undo().unwrap();
        assert_eq!(session.rule_program(), Some(&driven));

        let fixed = RuleProgramSource {
            file_name: "fixed.star".to_owned(),
            source: driven
                .source
                .replace("H = param(\"height\", 18)\n", "")
                .replace("distance=H", "distance=18"),
            overrides: BTreeMap::new(),
        };
        let appended = rewrite_rule_program_push_pull(&fixed, "board", "end", -3.0).unwrap();
        assert!(
            appended.source.contains(
                "push_pull(\"board\", face=\"end\", distance=-3, name=\"GUI Push/Pull 1\")"
            )
        );
        let (evaluated, _) = evaluate(&appended).unwrap();
        assert_eq!(
            evaluated
                .model
                .part("board")
                .unwrap()
                .face_offsets()
                .next()
                .unwrap()
                .distance_mm,
            -3.0
        );
    }

    #[test]
    fn revolved_profile_update_preserves_identity_and_matches_fresh_build() {
        let program = "A = param(\"angle\", 90)\npart = revolve(\"ring\", profile=[[\"spodok\", [40, 0], [60, 0]], [\"vonkajsi\", [60, 0], [60, 30]], [\"vrch\", [60, 30], [40, 30]], [\"vnutorny\", [40, 30], [40, 0]]], axis=[[0, 0], [0, 1]], angle=A)\nchamfer(part, edges=[[\"vrch\", \"vonkajsi\"]], distance=2, name=\"outer bevel\")";
        let mut incremental = DocumentSession::new(SessionSettings::default());
        let first = incremental
            .apply_rule_program(profile_source(program, "angle", 90.0), false)
            .unwrap();
        let occurrence_id = first.snapshot.occurrences().next().unwrap().id();
        let updated = incremental
            .apply_rule_program(profile_source(program, "angle", 270.0), false)
            .unwrap();

        let mut fresh = DocumentSession::new(SessionSettings::default());
        fresh
            .apply_rule_program(profile_source(program, "angle", 270.0), false)
            .unwrap();
        assert_eq!(
            updated.snapshot.occurrences().next().unwrap().id(),
            occurrence_id
        );
        assert_eq!(exact_signature(&incremental), exact_signature(&fresh));
    }
}
