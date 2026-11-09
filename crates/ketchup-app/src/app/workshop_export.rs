//! File > Manufacturing > Send to workshop: every shop file of the model in
//! one ZIP the workshop can check (review 2026-11-08 §5.2).

use crate::app_state::TakeoffError;
use crate::*;
use ketchup_manufacturing::workshop_package::{
    NotMade, WorkshopPackage, WorkshopPackageError, WorkshopPackageOptions,
};
use ketchup_program::table::DecimalSeparator;

impl KetchupApp {
    pub(crate) fn export_current_workshop_package_to(
        &mut self,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let options = WorkshopPackageOptions {
            btlx: self.btlx_export_options(),
            woodwop: Default::default(),
        };
        let result = (|| -> Result<Written, ExportError> {
            if !path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| extension.eq_ignore_ascii_case("zip"))
            {
                return Err(ExportError::Destination {
                    requirement: "an explicit .zip destination",
                });
            }
            let zip = self.workshop_package_zip(&snapshot, options)?;
            let expected_sha256 = export_target_sha256(path)?;
            let title = self.catalog.text("dialog-export-workshop-package-title");
            let risk = self.catalog.text("dialog-export-workshop-package-risk");
            self.authorize_path_side_effect(
                HighRiskClass::ReleaseManufacturingExportWithWarnings,
                "release-workshop-package",
                &title,
                &risk,
                path,
                &zip,
            )?;
            if expected_sha256.is_some() {
                let title = self.catalog.text("dialog-export-overwrite-title");
                let risk = self.catalog.text("dialog-export-overwrite-risk");
                self.authorize_path_side_effect(
                    HighRiskClass::Overwrite,
                    "overwrite-workshop-package",
                    &title,
                    &risk,
                    path,
                    &zip,
                )?;
            }
            let current = self.document.current();
            if current.document_id() != snapshot.document_id()
                || current.revision_id() != snapshot.revision_id()
                || current.canonical_digest() != snapshot.canonical_digest()
                || self.workshop_package_zip(&current, options)? != zip
            {
                return Err(ExportError::ChangedDuringConsent);
            }
            write_export_artifact_if_unchanged(path, &zip, expected_sha256.as_deref())?;
            Ok(Written::alone(path))
        })();
        self.report_export_outcome("workshop-package", path, result)
    }

    /// The shop files of the made parts and, under `cnc/`, the CNC drawings
    /// of the program's flat parts with their table; a part that cannot be
    /// drawn is named in `not_made.txt` with its reason.
    fn workshop_package_zip(
        &mut self,
        snapshot: &Snapshot,
        options: WorkshopPackageOptions,
    ) -> Result<Vec<u8>, ExportError> {
        let mut package = self
            .current_general_fabrication_projection()?
            .workshop_package(snapshot, options);
        self.add_cnc_drawings(&mut package)?;
        package.zip().map_err(|error| match error {
            WorkshopPackageError::Empty => ExportError::NothingToExport {
                subject: "shop file",
            },
            error => ExportError::failed(error),
        })
    }

    fn add_cnc_drawings(&self, package: &mut WorkshopPackage) -> Result<(), ExportError> {
        let cnc = match self.cnc_drawings() {
            Ok(cnc) => cnc,
            // A model made without a program has no CNC drawings to send.
            Err(TakeoffError::NoProgram) => return Ok(()),
            Err(TakeoffError::Program(error)) => {
                package.not_made.push(NotMade {
                    output: "CNC DXF".to_owned(),
                    reason: Arc::new(error),
                });
                return Ok(());
            }
            // The package would not be of the program the window shows.
            Err(TakeoffError::Planning | TakeoffError::NoPrices | TakeoffError::Write(_)) => {
                return Err(ExportError::NothingToExport {
                    subject: "CNC drawing of the evaluated program",
                });
            }
        };
        if cnc.parts.is_empty() && cnc.not_drawn.is_empty() {
            return Ok(());
        }
        package.files.extend(
            cnc.parts
                .iter()
                .map(|part| (format!("cnc/{}", part.file), part.dxf())),
        );
        package.files.push((
            "cnc/parts.csv".to_owned(),
            cnc.table().csv(DecimalSeparator::Point).into_bytes(),
        ));
        package
            .not_made
            .extend(cnc.not_drawn.into_iter().map(|part| NotMade {
                output: format!("CNC DXF of {}", part.part),
                reason: Arc::new(part.reason),
            }));
        Ok(())
    }
}
