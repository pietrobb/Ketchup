//! A move, turn, delete, colour, rename, mirror or parameter value of parts a
//! program owns is written into the program and published like any program
//! edit, so the program keeps owning the document and Undo takes the edit
//! back in one step.
use super::*;
use crate::transform_operation::TransformTarget;
use ketchup_application::PartChange;
use ketchup_assistant::sidecar::AssistantCadRotation;
use ketchup_model::document::RuleProgramSource;

impl KetchupApp {
    /// Commits a batch of the editor that deletes, recolours or renames
    /// parts: into the program when the parts are a program's, else as it
    /// is. A refusal is shown in the digest.
    pub(crate) fn commit_part_batch(&mut self, batch: CommandBatch) -> bool {
        self.commit_program_part_batch(batch.commands())
            .unwrap_or_else(|| self.apply_batch_with_work_recovery(&batch).is_ok())
    }

    /// Writes `commands` into the program and publishes it; a refusal is
    /// shown in the digest. `None` when no program owns the document, or a
    /// command does something else than delete, recolour or rename a part the
    /// program makes or set a value of its feature.
    pub(crate) fn commit_program_part_batch(
        &mut self,
        commands: &[CanonicalCommand],
    ) -> Option<bool> {
        let program = self.document.current_rule_program()?.clone();
        let changes = part_changes(&self.document.current(), commands)?;
        let result = self.publish_part_changes(&program, &changes)?;
        Some(self.refused(result).is_some())
    }

    /// Writes `changes` into `program` and publishes it. `None` when a change
    /// names something that is not a part the program makes.
    pub(crate) fn publish_part_changes(
        &mut self,
        program: &RuleProgramSource,
        changes: &[(String, PartChange)],
    ) -> Option<Result<(), Rejection>> {
        let rewritten = rewrite_parts(program, changes).transpose()?;
        Some(rewritten.and_then(|source| self.publish_program_source(source)))
    }

    fn publish_program_source(&mut self, source: RuleProgramSource) -> Result<(), Rejection> {
        self.apply_program_source(source, false)
            .map(|_| ())
            .map_err(|error| error.rejection())
    }

    /// `None` when no program owns the document or the edit moves something
    /// that is not a part the program makes: the caller commits the edit as
    /// it is. With `replace_last` the edit takes the place of the last one
    /// (a typed value correcting a move just made) instead of adding to it.
    pub(crate) fn program_transform(
        &mut self,
        target: &TransformTarget,
        world_edit: Transform,
        replace_last: bool,
    ) -> Option<Result<(), Rejection>> {
        self.document.current_rule_program()?;
        let snapshot = self.document.current();
        let parts = program_part_names(&snapshot, target);
        if replace_last && !self.mutate_history_with_work_recovery(DocumentStore::undo) {
            return Some(Err(Rejection::new(
                "program.transform_correction",
                RejectionPhase::Planning,
            )
            .reason("the edit to correct is no longer the last one")));
        }
        let rewritten = self
            .document
            .current_rule_program()
            .ok_or_else(|| {
                Rejection::new("program.transform_correction", RejectionPhase::Planning)
                    .reason("the program did not own the document before the edit to correct")
            })
            .and_then(|program| rewrite_parts(program, &transformed(&parts, world_edit)));
        let result = match rewritten {
            Ok(Some(rewritten)) => self.publish_program_source(rewritten),
            // Not parts the program makes: the edit is committed as it is.
            Ok(None) if !replace_last => return None,
            Ok(None) => Err(Rejection::new(
                "program.transform_correction",
                RejectionPhase::Planning,
            )
            .reason("the edit to correct was not written into the program")),
            Err(rejection) => Err(rejection),
        };
        if result.is_err() && replace_last {
            self.mutate_history_with_work_recovery(DocumentStore::redo);
        }
        Some(result)
    }

    /// The document's program with every typed operation of `program`
    /// written into it, as the window writes a move, turn, delete, colour or
    /// mirror. `None` when no program owns the document, `program` does
    /// anything else, or it changes something that is not a part the program
    /// makes.
    pub(crate) fn program_edit_source(
        &self,
        program: &AssistantCadEditProgram,
    ) -> Option<Result<RuleProgramSource, Rejection>> {
        let mut source = self.document.current_rule_program()?.clone();
        if program.operations.is_empty() {
            return None;
        }
        let snapshot = self.document.current();
        for operation in &program.operations {
            let rewritten = match operation {
                AssistantCadEditOperation::Transform {
                    translation_mm,
                    rotation,
                    ..
                } => {
                    let batch = match self.plan_one(operation) {
                        Ok(batch) => batch,
                        Err(rejection) => return Some(Err(rejection)),
                    };
                    let moved = batch
                        .commands()
                        .iter()
                        .filter_map(|command| match command {
                            CanonicalCommand::SetOccurrenceTransform { id, .. } => {
                                Some(InstancePath::root(*id))
                            }
                            _ => None,
                        })
                        .collect();
                    let parts = program_part_names(&snapshot, &TransformTarget::Occurrences(moved));
                    world_motion(*translation_mm, rotation.as_ref())
                        .map_err(|error| failed("program.transform", error))
                        .and_then(|world_edit| {
                            rewrite_parts(&source, &transformed(&parts, world_edit))
                        })
                }
                AssistantCadEditOperation::Delete { .. }
                | AssistantCadEditOperation::SetColor { .. }
                | AssistantCadEditOperation::SetFeatureParameter { .. } => {
                    let batch = match self.plan_one(operation) {
                        Ok(batch) => batch,
                        Err(rejection) => return Some(Err(rejection)),
                    };
                    let changes = part_changes(&snapshot, batch.commands())?;
                    rewrite_parts(&source, &changes)
                }
                AssistantCadEditOperation::Mirror {
                    selector: AssistantCadEntitySelector::Occurrences { occurrence_ids },
                    plane_origin_mm,
                    plane_normal,
                } => {
                    let mut taken = snapshot
                        .occurrences()
                        .map(|occurrence| occurrence.name().to_owned())
                        .collect::<BTreeSet<_>>();
                    let changes = occurrence_ids
                        .iter()
                        .map(|id| {
                            let name = ketchup_application::rule_program_part_name(
                                &snapshot,
                                &InstancePath::root(OccurrenceId(*id)),
                            )?;
                            let copy = (1..)
                                .map(|n| match n {
                                    1 => format!("{name} mirrored"),
                                    n => format!("{name} mirrored {n}"),
                                })
                                .find(|copy| !taken.contains(copy))?;
                            taken.insert(copy.clone());
                            let change = PartChange::Mirror {
                                copy,
                                point: *plane_origin_mm,
                                normal: *plane_normal,
                            };
                            Some((name, change))
                        })
                        .collect::<Option<Vec<_>>>()?;
                    rewrite_parts(&source, &changes)
                }
                _ => return None,
            };
            match rewritten {
                Ok(Some(rewritten)) => source = rewritten,
                // Not parts the program makes: the edit is planned as it is.
                Ok(None) => return None,
                Err(rejection) => return Some(Err(rejection)),
            }
        }
        Some(Ok(source))
    }

    /// The canonical edit of one typed operation: the planner resolves its
    /// selector exactly as for a detached edit.
    fn plan_one(&self, operation: &AssistantCadEditOperation) -> Result<CommandBatch, Rejection> {
        self.plan_assistant_cad_edit_program(&AssistantCadEditProgram {
            operations: vec![operation.clone()],
        })
        .map_err(|diagnostic| Rejection::from(*diagnostic))
    }

    /// The canonical edit that brings the document to what `source` makes,
    /// for a caller that commits it and binds `source` in the same step.
    pub(crate) fn program_source_proposal(
        &self,
        source: &RuleProgramSource,
    ) -> Result<Proposal, Rejection> {
        let plan = ketchup_application::plan_rule_program(&self.document, source)
            .map_err(|error| error.rejection())?;
        let ketchup_application::RuleProgramChange::Incremental(batch) = plan.change else {
            return Err(Rejection::new("program.apply", RejectionPhase::Planning)
                .reason("the program edit does not change the model in place"));
        };
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::rule_program())
            .map_err(|error| failed("program.apply", error))
    }
}

/// The part changes `commands` make, when each deletes, recolours or renames
/// a part of the document, or sets a value of its feature, by its program name.
fn part_changes(
    snapshot: &Snapshot,
    commands: &[CanonicalCommand],
) -> Option<Vec<(String, PartChange)>> {
    commands
        .iter()
        .map(|command| {
            let (id, change) = match command {
                CanonicalCommand::DeleteOccurrence { id } => (*id, PartChange::Delete),
                CanonicalCommand::SetOccurrenceColor { id, color } => {
                    (*id, PartChange::Color(*color))
                }
                CanonicalCommand::RenameEntity { id, name } => {
                    (*id, PartChange::Rename(name.clone()))
                }
                CanonicalCommand::SetFeatureParameter { target, dimension } => {
                    // The one part whose definition holds the feature.
                    let mut owners = snapshot.occurrences().filter(|occurrence| {
                        snapshot
                            .definition(occurrence.definition_id())
                            .is_some_and(|made| made.feature_ids().contains(&target.feature_id))
                    });
                    let (Some(owner), None) = (owners.next(), owners.next()) else {
                        return None;
                    };
                    let change = PartChange::Parameter {
                        feature: snapshot.feature(target.feature_id)?.name().to_owned(),
                        path: target.path.as_str().to_owned(),
                        value: dimension.millimetres(),
                    };
                    (owner.id(), change)
                }
                _ => return None,
            };
            let name =
                ketchup_application::rule_program_part_name(snapshot, &InstancePath::root(id))?;
            Some((name, change))
        })
        .collect()
}

fn rewrite_parts(
    source: &RuleProgramSource,
    changes: &[(String, PartChange)],
) -> Result<Option<RuleProgramSource>, Rejection> {
    ketchup_application::rewrite_rule_program_parts(source, changes)
        .map_err(|error| error.rejection())
}

fn transformed(parts: &[String], world_edit: Transform) -> Vec<(String, PartChange)> {
    let change = PartChange::Transform(world_edit);
    parts
        .iter()
        .map(|name| (name.clone(), change.clone()))
        .collect()
}

/// The world motion of a typed transform: the shift first, then the turn.
fn world_motion(
    translation_mm: [f64; 3],
    rotation: Option<&AssistantCadRotation>,
) -> Result<Transform, CanonicalError> {
    let [x, y, z] = translation_mm;
    let shift = Transform::from_translation(x, y, z)?;
    let Some(rotation) = rotation else {
        return Ok(shift);
    };
    let [px, py, pz] = rotation.pivot_mm;
    let [ax, ay, az] = rotation.axis;
    let turn = ketchup_application::transforms::world_axis_rotation_transform(
        Vec3::new(px, py, pz),
        Vec3::new(ax, ay, az),
        rotation.angle_degrees,
    )?;
    Ok(turn.compose(shift))
}

/// The program names of the parts a transform moves, in scene order.
fn program_part_names(snapshot: &Snapshot, target: &TransformTarget) -> Vec<String> {
    let mut names = Vec::new();
    for item in snapshot.scene_query() {
        let moved = match target {
            TransformTarget::Occurrences(paths) => paths
                .iter()
                .any(|path| path.root_occurrence() == item.instance_path.root_occurrence()),
            TransformTarget::Group(group_id) => KetchupApp::group_contains_occurrence(
                snapshot,
                *group_id,
                item.instance_path.root_occurrence(),
            ),
            TransformTarget::Underlay(_) => false,
        };
        if moved
            && let Some(name) =
                ketchup_application::rule_program_part_name(snapshot, &item.instance_path)
            && !names.contains(&name)
        {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
#[path = "program_transform_tests.rs"]
mod tests;
