use crate::{DocumentSession, SessionError};
use ketchup_core::assistant_sidecar::AssistantCadEditOperation;
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, FeatureKind, FeatureParameterTarget,
    ParameterValueType, RuleProgramSource, Snapshot, Transform,
};
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
    pub report: Report,
    pub replaced_document: bool,
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
        let (evaluated, report) = evaluate(&source)?;
        let old_program = self.rule_program().map(evaluate).transpose()?;

        if let Some((old, _)) = &old_program
            && old.model.joints == evaluated.model.joints
            && self.snapshot().occurrences().count() == old.model.parts.len()
        {
            let snapshot = self.snapshot();
            let changes = old
                .model
                .parts
                .iter()
                .filter(|before| evaluated.model.part(&before.name).is_some())
                .map(|before| {
                    let after = evaluated.model.part(&before.name)?;
                    let mut comparable = after.clone();
                    comparable.at_mm = before.at_mm;
                    comparable.size_mm = before.size_mm;
                    comparable.holes = before.holes.clone();
                    comparable.pockets = before.pockets.clone();
                    comparable.features = before.features.clone();
                    if &comparable != before {
                        return None;
                    }
                    let occurrence = snapshot.occurrences().find(|o| o.name() == before.name)?;
                    let old_position = Transform::from_translation(
                        before.at_mm[0],
                        before.at_mm[1],
                        before.at_mm[2],
                    )
                    .ok()?;
                    if occurrence.transform() != old_position {
                        return None;
                    }
                    let mut commands = program_feature_commands(
                        &snapshot,
                        occurrence.definition_id(),
                        before,
                        after,
                    )?;
                    if before.at_mm != after.at_mm {
                        let transform = Transform::from_translation(
                            after.at_mm[0],
                            after.at_mm[1],
                            after.at_mm[2],
                        )
                        .ok()?;
                        commands.push(CanonicalCommand::SetOccurrenceTransform {
                            id: occurrence.id(),
                            transform,
                        });
                    }
                    Some(commands)
                })
                .collect::<Option<Vec<Vec<CanonicalCommand>>>>();
            if let Some(changes) = changes {
                let mut commands = changes.into_iter().flatten().collect::<Vec<_>>();
                append_removed_parts(&snapshot, &old.model, &evaluated.model, &mut commands)?;
                let panels = added_parts(&old.model, &evaluated.model);
                let snapshot = if commands.is_empty() && panels.is_empty() {
                    self.replace_rule_program_source(source)?
                } else {
                    self.apply_rule_commands_with_source(
                        CommandBatch::new(commands),
                        &panels,
                        source,
                    )?
                };
                return Ok(RuleProgramApplyResult {
                    snapshot,
                    report,
                    replaced_document: false,
                });
            }
        }

        if let Some((old, _)) = old_program {
            let snapshot = self.snapshot();
            if old.model.joints == evaluated.model.joints
                && snapshot.occurrences().count() == old.model.parts.len()
            {
                let replacements = old
                    .model
                    .parts
                    .iter()
                    .map(|before| {
                        let Some(after) = evaluated.model.part(&before.name) else {
                            return Some(None);
                        };
                        let mut comparable = after.clone();
                        comparable.at_mm = before.at_mm;
                        comparable.size_mm = before.size_mm;
                        comparable.holes = before.holes.clone();
                        comparable.pockets = before.pockets.clone();
                        comparable.body = before.body.clone();
                        comparable.features = before.features.clone();
                        if comparable != *before {
                            return None;
                        }
                        let occurrence =
                            snapshot.occurrences().find(|o| o.name() == before.name)?;
                        let position = Transform::from_translation(
                            before.at_mm[0],
                            before.at_mm[1],
                            before.at_mm[2],
                        )
                        .ok()?;
                        if occurrence.transform() != position {
                            return None;
                        }
                        Some(
                            (before != after)
                                .then(|| (occurrence.id(), ketchup_program::cad::part(after))),
                        )
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(replacements) = replacements {
                    let replacements = replacements.into_iter().flatten().collect::<Vec<_>>();
                    let mut commands = Vec::new();
                    append_removed_parts(&snapshot, &old.model, &evaluated.model, &mut commands)?;
                    let panels = added_parts(&old.model, &evaluated.model);
                    let snapshot = self.replace_rule_panels_with_source(
                        &replacements,
                        &panels,
                        CommandBatch::new(commands),
                        source,
                    )?;
                    return Ok(RuleProgramApplyResult {
                        snapshot,
                        report,
                        replaced_document: false,
                    });
                }
            }
            return Err(RuleProgramApplyError::IncrementalUnsupported);
        }

        let nonempty = self.snapshot().definitions().next().is_some();
        if nonempty && !allow_replacement {
            return Err(if self.is_modified() {
                RuleProgramApplyError::UnsavedChanges
            } else {
                RuleProgramApplyError::ReplacementConfirmationRequired
            });
        }
        let panels = ketchup_program::cad::part_operations(&evaluated.model);
        let snapshot = self.replace_with_rule_panels(&panels, source)?;
        Ok(RuleProgramApplyResult {
            snapshot,
            report,
            replaced_document: true,
        })
    }
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

fn added_parts(before: &ProgramModel, after: &ProgramModel) -> Vec<AssistantCadEditOperation> {
    after
        .parts
        .iter()
        .filter(|part| before.part(&part.name).is_none())
        .map(ketchup_program::cad::part)
        .collect()
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
            | (ProgramFeatureKind::Sketch, FeatureKind::Sketch(_))
            | (
                ProgramFeatureKind::Pad,
                FeatureKind::Pad(_) | FeatureKind::Extrusion { .. }
            )
            | (
                ProgramFeatureKind::Cut,
                FeatureKind::SketchPocket(_) | FeatureKind::Pocket { .. }
            )
            | (ProgramFeatureKind::Revolve, FeatureKind::Revolve { .. })
            | (
                ProgramFeatureKind::Fillet,
                FeatureKind::TopologyEdgeFinish { .. }
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

    fn exact_signature(
        session: &DocumentSession,
    ) -> ketchup_core::exact_brep_graph::ExactBRepGraph {
        let snapshot = session.snapshot();
        let occurrence = snapshot.occurrences().next().unwrap();
        let definition = snapshot.definition(occurrence.definition_id()).unwrap();
        let producer = *definition.feature_ids().last().unwrap();
        let mut graph = ketchup_core::exact_brep_graph::ExactBRepGraph::from_snapshot(
            &snapshot,
            occurrence.definition_id(),
            producer,
        )
        .unwrap();
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
        }
        graph
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
    fn extruded_profile_update_preserves_identity_and_matches_fresh_build() {
        let program = "W = param(\"width\", 40)\nextrude(\"angle\", profile=[[\"bottom\", [0, 0], [W, 0]], [\"outer\", [W, 0], [W, 10]], [\"ledge\", [W, 10], [10, 10]], [\"inner\", [10, 10], [10, 40]], [\"top\", [10, 40], [0, 40]], [\"back\", [0, 40], [0, 0]]], distance=100)";
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
    fn revolved_profile_update_preserves_identity_and_matches_fresh_build() {
        let program = "R = param(\"radius\", 20)\nrevolve(\"knob\", profile=[[0, 0], [R, 0], [R, 30], [0, 30]], axis=[[0, 0], [0, 30]], angle=270)";
        let mut incremental = DocumentSession::new(SessionSettings::default());
        let first = incremental
            .apply_rule_program(profile_source(program, "radius", 20.0), false)
            .unwrap();
        let occurrence_id = first.snapshot.occurrences().next().unwrap().id();
        let updated = incremental
            .apply_rule_program(profile_source(program, "radius", 25.0), false)
            .unwrap();

        let mut fresh = DocumentSession::new(SessionSettings::default());
        fresh
            .apply_rule_program(profile_source(program, "radius", 25.0), false)
            .unwrap();
        assert_eq!(
            updated.snapshot.occurrences().next().unwrap().id(),
            occurrence_id
        );
        assert_eq!(exact_signature(&incremental), exact_signature(&fresh));
    }
}
