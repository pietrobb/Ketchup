//! File > Manufacturing > Send to workshop: every shop file of the model in
//! one ZIP the workshop can check (review 2026-11-08 §5.2).

use crate::*;
use ketchup_manufacturing::workshop_package::{WorkshopPackageError, WorkshopPackageOptions};

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
            let render = |projection: &GeneralFabricationProjection, snapshot: &Snapshot| {
                projection
                    .workshop_package(snapshot, options)
                    .zip()
                    .map_err(|error| match error {
                        WorkshopPackageError::Empty => ExportError::NothingToExport {
                            subject: "shop file",
                        },
                        error => ExportError::failed(error),
                    })
            };
            let projection = self.current_general_fabrication_projection()?;
            let zip = render(&projection, &snapshot)?;
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
            let current_projection = self.current_general_fabrication_projection()?;
            if current.document_id() != snapshot.document_id()
                || current.revision_id() != snapshot.revision_id()
                || current.canonical_digest() != snapshot.canonical_digest()
                || render(&current_projection, &current)? != zip
            {
                return Err(ExportError::ChangedDuringConsent);
            }
            write_export_artifact_if_unchanged(path, &zip, expected_sha256.as_deref())?;
            Ok(Written::alone(path))
        })();
        self.report_export_outcome("workshop-package", path, result)
    }
}
