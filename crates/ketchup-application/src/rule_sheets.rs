//! Publish program-owned sheets with geometry in the same atomic edit.
use crate::{RuleProgramApplyError, SessionError, evaluation, rejections::rejected};
use ketchup_model::{
    document::{CanonicalCommand, CommandBatch, DocumentStore, Snapshot},
    drawing::{
        DrawingSheet, DrawingSheetId, DrawingSource, annotation::annotate_sheet,
        project_orthographic_drawing,
    },
    exact_product::ExactResultRegistry,
    persistence::ContainerData,
};
use ketchup_program::ProgramModel;
use std::collections::BTreeMap;

fn problem(target: &str, error: impl std::fmt::Display) -> RuleProgramApplyError {
    RuleProgramApplyError::Rejected(Box::new(
        rejected("program_rejected")
            .target(target)
            .reason(error.to_string())
            .fix_hint("Check the drawing sheet source, view and geometric selector in the program.")
            .caused_by_messages([error.to_string()]),
    ))
}

pub(crate) fn remove_owned(
    snapshot: &Snapshot,
    old: &ProgramModel,
) -> Result<Vec<CanonicalCommand>, RuleProgramApplyError> {
    let mut commands = Vec::new();
    for declaration in &old.drawing_sheets {
        let mut matches = snapshot
            .drawing_sheets()
            .filter(|sheet| sheet.name() == declaration.name);
        if let Some(sheet) = matches.next() {
            if matches.next().is_some() {
                return Err(problem(
                    &declaration.name,
                    "multiple sheets share this program-owned name; rename the manual sheet before updating the program",
                ));
            }
            commands.push(CanonicalCommand::DeleteDrawingSheet { id: sheet.id() });
        }
    }
    Ok(commands)
}

pub(crate) fn append(
    document: &DocumentStore,
    previous: &Snapshot,
    model: &ProgramModel,
    old: Option<&ProgramModel>,
    batch: CommandBatch,
) -> Result<CommandBatch, RuleProgramApplyError> {
    if model.drawing_sheets.is_empty() {
        return Ok(batch);
    }
    let mut staged = document.fork_for_planning();
    if !batch.commands().is_empty() {
        staged
            .apply_batch(&batch)
            .map_err(SessionError::Canonical)?;
    }
    let snapshot = staged.current();
    let by_name = snapshot
        .scene_query()
        .into_iter()
        .filter_map(|leaf| {
            crate::rule_program_part_name(&snapshot, &leaf.instance_path)
                .map(|name| (name, leaf.instance_path))
        })
        .collect::<BTreeMap<_, _>>();
    let mut sheets = Vec::new();
    let mut next_id = previous
        .drawing_sheets()
        .chain(snapshot.drawing_sheets())
        .map(|sheet| sheet.id().0)
        .max()
        .unwrap_or(0);
    for declaration in &model.drawing_sheets {
        let previous_sheet = previous
            .drawing_sheets()
            .find(|sheet| sheet.name() == declaration.name);
        let id = if let Some(sheet) = previous_sheet {
            if !old.is_some_and(|old| {
                old.drawing_sheets
                    .iter()
                    .any(|owned| owned.name == declaration.name)
            }) {
                return Err(problem(
                    &declaration.name,
                    "this sheet name belongs to a manually created sheet; choose a different name",
                ));
            }
            sheet.id()
        } else {
            next_id = next_id.checked_add(1).ok_or_else(|| {
                problem(&declaration.name, "drawing sheet identifiers are exhausted")
            })?;
            DrawingSheetId(next_id)
        };
        let path = by_name.get(&declaration.source).ok_or_else(|| {
            problem(
                &declaration.name,
                format!(
                    "unknown drawing source {:?}; name an existing program part",
                    declaration.source
                ),
            )
        })?;
        let sheet = DrawingSheet::new(
            id,
            &declaration.name,
            DrawingSource::for_instances(&snapshot, vec![path.clone()]),
        )
        .map_err(|error| problem(&declaration.name, error))?;
        sheets.push((declaration, sheet));
    }
    // Only evaluate the definitions selected by these sheets, not unrelated model parts.
    let mut scope = std::collections::BTreeSet::new();
    for (declaration, _) in &sheets {
        let path = &by_name[&declaration.source];
        let part = snapshot
            .resolve_instance_path(path)
            .map_err(|error| problem(&declaration.name, error))?;
        for feature_id in ketchup_model::exact_product::exact_body_terminal_features(
            &snapshot,
            part.definition_id,
        )
        .map_err(|error| problem(&declaration.name, error))?
        .into_values()
        {
            scope.insert(evaluation::ProducerKey {
                definition_id: part.definition_id,
                feature_id,
            });
        }
    }
    let empty = ExactResultRegistry::default();
    let task = evaluation::start_exact_evaluation_scoped(
        snapshot.clone(),
        &ContainerData::default(),
        &empty,
        &empty,
        evaluation::exact_worker_or_installed(evaluation::configured_exact_worker()),
        Some(&scope),
        || {},
    );
    let products = task
        .wait(ketchup_scheduler::program_worker::PROGRAM_CHECK_TIME)
        .map_err(|error| problem("drawing sheets", error))?;
    let (exact, _, report) =
        evaluation::materialize_exact_products(&snapshot, &empty, &empty, &task, products)
            .map_err(|error| problem("drawing sheets", error))?;
    if !report.complete || !report.topology_complete {
        let reasons = report
            .producers
            .iter()
            .flat_map(|entry| [&entry.render, &entry.topology])
            .filter_map(|status| match status {
                evaluation::EvidenceStatus::Failed { reason }
                | evaluation::EvidenceStatus::NotEvaluated { reason } => Some(reason.as_str()),
                _ => None,
            })
            .chain(report.not_evaluated.as_deref())
            .collect::<Vec<_>>()
            .join("; ");
        return Err(problem("drawing sheets", reasons));
    }
    let mut commands = batch.commands().to_vec();
    for (declaration, mut sheet) in sheets {
        let drawing = project_orthographic_drawing(&snapshot, &exact, &sheet)
            .map_err(|error| problem(&declaration.name, error))?;
        if !declaration.annotations.is_empty() {
            let additions = declaration
                .annotations
                .iter()
                .map(|annotation| {
                    let view = drawing
                        .views
                        .iter()
                        .find(|view| view.kind.stable_name() == annotation.view)
                        .ok_or_else(|| {
                            problem(
                                &annotation.name,
                                format!("unknown sheet view {:?}", annotation.view),
                            )
                        })?;
                    annotation
                        .resolve(view)
                        .map_err(|error| problem(&annotation.name, error))
                })
                .collect::<Result<Vec<_>, _>>()?;
            sheet = annotate_sheet(&sheet, &additions)
                .map_err(|error| problem(&declaration.name, error))?;
            project_orthographic_drawing(&snapshot, &exact, &sheet)
                .map_err(|error| problem(&declaration.name, error))?;
        }
        if declaration.hole_callouts {
            let report = ketchup_model::drawing::hole_callouts::generate_hole_callouts(
                &snapshot,
                &exact,
                &sheet,
                // A rule program's sheet declares no language.
                ketchup_model::drawing::SheetLanguage::English,
            )
            .map_err(|error| problem(&declaration.name, error))?;
            if !report.diagnostics.is_empty() {
                return Err(problem(
                    &declaration.name,
                    report
                        .diagnostics
                        .iter()
                        .map(|diagnostic| diagnostic.describe(&snapshot))
                        .collect::<Vec<_>>()
                        .join("; "),
                ));
            }
            sheet = report.sheet;
        }
        commands.push(CanonicalCommand::CreateDrawingSheet(sheet));
    }
    Ok(CommandBatch::new(commands))
}
