use crate::{DocumentSession, SessionError};
use ketchup_core::document::{
    CanonicalCommand, CommandBatch, DefinitionId, Dimension, EdgeFinishKind, FeatureKind,
    FeatureParameterTarget, ParameterValueType, RuleProgramSource, Snapshot, Transform,
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
            .face_offsets
            .iter()
            .rev()
            .find(|offset| offset.face == face_name)
        {
            return Some(offset.distance_mm);
        }
        match (&part.body, face_name) {
            (ketchup_program::model::ProgramPartBody::Extrusion { distance_mm, .. }, "end") => {
                Some(*distance_mm)
            }
            // A board is padded along its third size component.
            (ketchup_program::model::ProgramPartBody::Panel, "end") => Some(part.size_mm[2]),
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
            if slope.abs() > 1.0e-8 {
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
        let feature_name = format!("GUI Push/Pull {}", part.face_offsets.len() + 1);
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
        if self.rule_program() == Some(&source) {
            return Ok(RuleProgramApplyResult {
                snapshot: self.snapshot(),
                report,
                replaced_document: false,
            });
        }
        let old_program = self.rule_program().map(evaluate).transpose()?;

        if let Some((old, _)) = &old_program
            && old.model.joints == evaluated.model.joints
            && old.model.parts.iter().all(|before| {
                evaluated
                    .model
                    .part(&before.name)
                    .is_some_and(|after| program_feature_references_match(before, after))
            })
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
                    comparable.body = before.body.clone();
                    comparable.fillets = before.fillets.clone();
                    comparable.cuts = before.cuts.clone();
                    comparable.face_offsets = before.face_offsets.clone();
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
                    self.apply_rule_commands_with_parts_source(
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
                        Some((before != after).then(|| (occurrence.id(), after.clone())))
                    })
                    .collect::<Option<Vec<_>>>();
                if let Some(replacements) = replacements {
                    let replacements = replacements.into_iter().flatten().collect::<Vec<_>>();
                    let mut commands = Vec::new();
                    append_removed_parts(&snapshot, &old.model, &evaluated.model, &mut commands)?;
                    let panels = added_parts(&old.model, &evaluated.model);
                    let snapshot = self.replace_rule_parts_with_source(
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
        let snapshot = self.replace_with_rule_parts(&evaluated.model.parts, source)?;
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
    fn segment_names(segments: &[ketchup_program::model::ProgramProfileSegment]) -> Vec<&str> {
        segments
            .iter()
            .map(|segment| segment.name.as_str())
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
        _ => false,
    };
    body_matches
        && before.cuts.len() == after.cuts.len()
        && before.cuts.iter().zip(&after.cuts).all(|(left, right)| {
            left.name == right.name
                && segment_names(&left.segments) == segment_names(&right.segments)
        })
        && before.fillets.len() == after.fillets.len()
        && before
            .fillets
            .iter()
            .zip(&after.fillets)
            .all(|(left, right)| {
                left.name == right.name && left.kind == right.kind && left.edges == right.edges
            })
        && before.face_offsets.len() == after.face_offsets.len()
        && before
            .face_offsets
            .iter()
            .zip(&after.face_offsets)
            .all(|(left, right)| left.name == right.name && left.face == right.face)
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
                FeatureKind::Sketch(_) | FeatureKind::SegmentProfile { .. }
            )
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
                FeatureKind::TopologyEdgeFinish {
                    kind: EdgeFinishKind::Fillet,
                    ..
                }
            )
            | (
                ProgramFeatureKind::Chamfer,
                FeatureKind::TopologyEdgeFinish {
                    kind: EdgeFinishKind::Chamfer,
                    ..
                }
            )
            | (
                ProgramFeatureKind::FaceOffset,
                FeatureKind::TopologyFaceOffset { .. }
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

    fn exact_graph(session: &DocumentSession) -> ketchup_core::exact_brep_graph::ExactBRepGraph {
        let snapshot = session.snapshot();
        let occurrence = snapshot.occurrences().next().unwrap();
        let definition = snapshot.definition(occurrence.definition_id()).unwrap();
        let producer = *definition.feature_ids().last().unwrap();
        ketchup_core::exact_brep_graph::ExactBRepGraph::from_snapshot(
            &snapshot,
            occurrence.definition_id(),
            producer,
        )
        .unwrap()
    }

    fn exact_signature(
        session: &DocumentSession,
    ) -> ketchup_core::exact_brep_graph::ExactBRepGraph {
        let mut graph = exact_graph(session);
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
            evaluated.model.part("board").unwrap().face_offsets[0].distance_mm,
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
