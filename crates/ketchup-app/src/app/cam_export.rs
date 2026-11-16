//! The CAM program of the document's plan as a manufacturing export: simulated exactly,
//! written for a controller of the dialect library, after the user confirms its release.

use crate::*;
use ketchup_application::cam_workflow::CamReviewError;
use ketchup_model::cam::{CamGCodeDialect, CamGCodeDialectError, CamOperation};

/// The library's controller dialects as (id, name), in library order.
pub(crate) fn cam_dialect_choices() -> Result<Vec<(String, String)>, CamGCodeDialectError> {
    Ok(CamGCodeDialect::library()?
        .iter()
        .map(|dialect| (dialect.id().to_owned(), dialect.name().to_owned()))
        .collect())
}

/// What a user is told the program is for: the dialect's name, or its id when it has none.
pub(crate) fn cam_dialect_name(dialect: &CamPostprocessorDialect) -> String {
    match dialect {
        CamPostprocessorDialect::GCode(dialect) => dialect.name().to_owned(),
        CamPostprocessorDialect::ControllerNeutralJson => dialect.id().to_owned(),
    }
}

impl KetchupApp {
    /// Writes the program of the document's first CAM plan running `operations`, or facing
    /// its stock when there are none, for `dialect` to the new file `path`. The program is
    /// simulated exactly first, stopped when `cancelled` is set; the user confirms its release
    /// knowing the controller and the result.
    pub(crate) fn export_current_cam_program_to(
        &mut self,
        path: &Path,
        dialect: CamPostprocessorDialect,
        operations: Vec<CamOperation>,
        cancelled: &AtomicBool,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let result = self.write_cam_program(path, dialect, operations, cancelled);
        self.report_export_outcome("cam-nc", path, result)
    }

    fn write_cam_program(
        &mut self,
        path: &Path,
        dialect: CamPostprocessorDialect,
        mut operations: Vec<CamOperation>,
        cancelled: &AtomicBool,
    ) -> Result<Written, ExportError> {
        let new_file = || ExportError::Destination {
            requirement: "a new file in an existing folder",
        };
        // Refused before the simulation and the prompt: a program never replaces a file.
        if path.exists() {
            return Err(new_file());
        }
        let snapshot = self.document.current();
        let mutation_epoch = self.document.mutation_epoch();
        let plan = snapshot
            .cam_plans()
            .next()
            .ok_or(ExportError::NothingToExport {
                subject: "CAM plan",
            })?;
        let plan_id = plan.id();
        if operations.is_empty() {
            operations.push(
                plan.default_facing_operation(&snapshot, 1)
                    .map_err(ExportError::failed)?,
            );
        }
        let worker_path = self.exact_worker_executable()?;
        self.reviews.cam_reviews.set_worker_path(worker_path);
        let review = self
            .reviews
            .cam_reviews
            .preview(
                &snapshot,
                mutation_epoch,
                CamReviewRequest {
                    plan_id,
                    operations,
                    fixtures: Vec::new(),
                    dialect: dialect.clone(),
                },
                cancelled,
            )
            .map_err(ExportError::failed)?;
        let title = self.catalog.text("dialog-export-cam-title");
        let risk = self.catalog.format(
            "dialog-export-cam-risk",
            &BTreeMap::from([
                ("dialect", cam_dialect_name(&dialect)),
                ("removed", format!("{:.3}", review.removed_stock_mm3)),
                ("gouge", format!("{:.3}", review.gouge_mm3)),
            ]),
        );
        self.authorize_path_side_effect(
            HighRiskClass::ReleaseManufacturingExportWithWarnings,
            "release-cam-nc-program",
            &title,
            &risk,
            path,
            review.content_digest.as_bytes(),
        )?;
        self.reviews
            .cam_reviews
            .export(
                &snapshot,
                mutation_epoch,
                &review.token,
                path,
                true,
                cancelled,
            )
            .map_err(|error| match error {
                CamReviewError::FileExists | CamReviewError::InvalidPath => new_file(),
                other => ExportError::failed(other),
            })?;
        Ok(Written::alone(path))
    }
}
