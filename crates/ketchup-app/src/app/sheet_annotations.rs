//! Exact sheet annotation authoring shared by the window and clients.
use crate::*;
use ketchup_application::rejections::rejected;
use ketchup_model::drawing::annotation::{DrawingAnnotation, annotate_sheet};
use ketchup_model::drawing::{
    DrawingSheetId, OrthographicDrawing, prepare_edit_drawing_sheet,
    project_orthographic_drawing_in,
};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_rejection::Rejection;

impl KetchupApp {
    pub(crate) fn drawing_sheet_geometry(
        &self,
        id: DrawingSheetId,
    ) -> Result<OrthographicDrawing, Rejection> {
        if self.exact.task.is_some() {
            return Err(rejected("busy"));
        }
        let snapshot = self.document.current();
        let sheet = snapshot
            .drawing_sheet(id)
            .ok_or_else(|| rejected("drawing_sheet_not_found"))?;
        project_orthographic_drawing_in(
            &snapshot,
            &self.exact.results,
            sheet,
            &self.parts_list_words(),
        )
        .map_err(|error| {
            rejected("invalid_params")
                .target("sheet_id")
                .caused_by(error)
        })
    }

    pub(crate) fn annotate_drawing_sheet(
        &mut self,
        id: DrawingSheetId,
        annotations: &[DrawingAnnotation],
        hole_callouts: bool,
    ) -> Result<usize, Rejection> {
        if self.exact.task.is_some() {
            return Err(rejected("busy"));
        }
        let snapshot = self.document.current();
        let sheet = snapshot
            .drawing_sheet(id)
            .ok_or_else(|| rejected("drawing_sheet_not_found"))?;
        // Do not silently detach a rule program and lose its parametric ownership.
        if self.document.current_rule_program().is_some() {
            return Err(rejected("program_owned_document"));
        }
        let mut edited = if annotations.is_empty() && hole_callouts {
            sheet.clone()
        } else {
            annotate_sheet(sheet, annotations).map_err(|error| {
                rejected("invalid_params")
                    .target("annotations")
                    .caused_by(error)
            })?
        };
        let mut removed_sources = 0;
        if hole_callouts {
            let report = ketchup_model::drawing::hole_callouts::generate_hole_callouts(
                &snapshot,
                &self.exact.results,
                &edited,
            )
            .map_err(|error| {
                rejected("invalid_params")
                    .target("hole_callouts")
                    .caused_by(error)
            })?;
            use ketchup_model::drawing::hole_callouts::HoleCalloutDiagnosticKind;
            if report.diagnostics.iter().any(|diagnostic| {
                diagnostic.kind == HoleCalloutDiagnosticKind::UnsupportedProvenance
            }) {
                return Err(rejected("invalid_params")
                    .target("hole_callouts")
                    .reason("Some cuts cannot produce verified hole callouts; nothing was changed.")
                    .fix_hint("Review the reported cuts or use a manual annotation.")
                    .caused_by_messages(report.diagnostics.iter().map(|diagnostic| {
                        format!(
                            "feature {}: {}; {}",
                            diagnostic.source.feature_id.0, diagnostic.reason, diagnostic.fix_hint
                        )
                    })));
            }
            removed_sources = report.diagnostics.len();
            edited = report.sheet;
        }
        let (proposal, _) = prepare_edit_drawing_sheet(&self.document, &self.exact.results, edited)
            .map_err(|error| {
                rejected("invalid_params")
                    .target("annotations")
                    .caused_by(error)
            })?;
        self.commit_verified_proposal_with_work_recovery(&proposal)
            .map_err(|error| rejected("commit_rejected").caused_by(error))?;
        self.exact.results =
            ExactResultRegistry::carried_forward(&self.document.current(), &self.exact.results);
        self.reconcile_selection();
        if removed_sources > 0 {
            self.digest = self.catalog.text("sheet-callouts-removed").to_owned();
        }
        Ok(removed_sources)
    }
}
