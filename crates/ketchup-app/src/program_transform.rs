//! A move or turn of parts a program owns is written into the program and
//! published like any program edit, so the program keeps owning the
//! document and Undo takes the edit back in one step.
use super::*;
use crate::transform_operation::TransformTarget;
use ketchup_assistant::sidecar::AssistantCadRotation;
use ketchup_model::document::RuleProgramSource;

impl KetchupApp {
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
            .and_then(|program| {
                ketchup_application::rewrite_rule_program_transform(program, &parts, world_edit)
                    .map_err(|error| failed("program.transform", error))
            });
        let result = match rewritten {
            Ok(Some(rewritten)) => self
                .apply_program_source(rewritten, false)
                .map(|_| ())
                .map_err(|error| failed("program.apply", error)),
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

    /// The document's program with every typed `transform` of `program`
    /// written into it, as the window writes a move or turn. `None` when no
    /// program owns the document, `program` does more than move and turn, or
    /// it moves something that is not a part the program makes.
    pub(crate) fn program_transform_source(
        &self,
        program: &AssistantCadEditProgram,
    ) -> Option<Result<RuleProgramSource, Rejection>> {
        let mut source = self.document.current_rule_program()?.clone();
        let motions = program
            .operations
            .iter()
            .map(|operation| match operation {
                AssistantCadEditOperation::Transform {
                    translation_mm,
                    rotation,
                    ..
                } => Some((operation, *translation_mm, rotation.as_ref())),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .filter(|motions| !motions.is_empty())?;
        let snapshot = self.document.current();
        for (operation, translation_mm, rotation) in motions {
            // The planner resolves the selector exactly as for a detached edit.
            let batch = match self.plan_assistant_cad_edit_program(&AssistantCadEditProgram {
                operations: vec![operation.clone()],
            }) {
                Ok(batch) => batch,
                Err(diagnostic) => return Some(Err(Rejection::from(*diagnostic))),
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
            let rewritten = world_motion(translation_mm, rotation)
                .map_err(|error| failed("program.transform", error))
                .and_then(|world_edit| {
                    ketchup_application::rewrite_rule_program_transform(&source, &parts, world_edit)
                        .map_err(|error| failed("program.transform", error))
                });
            match rewritten {
                Ok(Some(rewritten)) => source = rewritten,
                // Not parts the program makes: the edit is planned as it is.
                Ok(None) => return None,
                Err(rejection) => return Some(Err(rejection)),
            }
        }
        Some(Ok(source))
    }

    /// The canonical edit that brings the document to what `source` makes,
    /// for a caller that commits it and binds `source` in the same step.
    pub(crate) fn program_source_proposal(
        &self,
        source: &RuleProgramSource,
    ) -> Result<Proposal, Rejection> {
        let plan = ketchup_application::plan_rule_program(&self.document, source)
            .map_err(|error| failed("program.apply", error))?;
        let ketchup_application::RuleProgramChange::Incremental(batch) = plan.change else {
            return Err(Rejection::new("program.apply", RejectionPhase::Planning)
                .reason("the program edit does not change the model in place"));
        };
        self.document
            .prepare_proposal_with_context(batch, ProposalContext::rule_program())
            .map_err(|error| failed("program.apply", error))
    }
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
