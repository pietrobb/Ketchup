use super::*;
use crate::app::skp_import::{SkpConversion, model_glb};
use ketchup_model::import::{GlbNodeTags, ParsedGlbScene, inspect_glb, plan_glb_import_tagged};

#[derive(Clone, Debug, PartialEq)]
struct GlbImportPreviewPlan {
    source: ImportSourcePlan,
    review: ParsedGlbScene,
    batch: CommandBatch,
    /// The conversion of a SketchUp file, held while the review is open.
    skp: Option<SkpConversion>,
}

#[derive(Clone, Debug)]
pub(super) struct PendingGlbImport {
    plan: GlbImportPreviewPlan,
    pub(super) invalidated: bool,
}

impl KetchupApp {
    fn choose_glb_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-glb-skp");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Glb,
            filter_label: &filter_label,
            extensions: &["glb", "skp"],
        })
    }

    fn prepare_glb_import_preview_plan(
        &self,
        source: ImportSourcePlan,
    ) -> Result<GlbImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let (glb, skp) = model_glb(&source.path, &source.source)?;
        let (notes, tags) = match &skp {
            Some(conversion) => (
                conversion.notes().map_err(|error| source.failed(error))?,
                conversion.node_tags(),
            ),
            None => (Vec::new(), GlbNodeTags::default()),
        };
        let review = crate::bounded_parser::run_bounded_parser(source.format, || inspect_glb(&glb))
            .map_err(|error| source.failed(error))?
            .map_err(|error| source.failed(error))?
            .with_notes(&notes);
        let batch = crate::bounded_parser::run_bounded_parser(source.format, || {
            plan_glb_import_tagged(&snapshot, &glb, source_name, &notes, &tags)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        Ok(GlbImportPreviewPlan {
            source,
            review,
            batch,
            skp,
        })
    }

    fn import_glb_from(&mut self, pending: &PendingGlbImport) -> Result<(), ImportError> {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            source.reread_unchanged()?;
            if self.prepare_glb_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.apply_batch_with_work_recovery(&pending.plan.batch)
                .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    /// Imports `path` without the review window; the answer is the review summary.
    pub(crate) fn import_glb_file(
        &mut self,
        path: &Path,
    ) -> Result<serde_json::Value, ImportError> {
        self.import_glb_source(path, read_import_source(ImportFormat::Glb, path)?)
    }

    /// [`Self::import_glb_file`] of `source`, the bytes of the `.glb` or `.skp` file at
    /// `path` a thread has read already.
    pub(crate) fn import_glb_source(
        &mut self,
        path: &Path,
        source: Vec<u8>,
    ) -> Result<serde_json::Value, ImportError> {
        let source = ImportSourcePlan::seal(
            ImportFormat::Glb,
            path.to_owned(),
            source,
            (),
            &self.document.current(),
        );
        let plan = self.prepare_glb_import_preview_plan(source)?;
        let (diagnostics, diagnostics_total) =
            crate::app::import_file::bounded(plan.review.diagnostics());
        let summary = serde_json::json!({
            "meshes": plan.review.mesh_primitive_count(),
            "nodes": plan.review.node_count(),
            "instances": plan.review.instance_count(),
            "triangles": plan.review.triangle_count(),
            "diagnostics": diagnostics,
            "diagnostics_total": diagnostics_total,
        });
        self.import_glb_from(&PendingGlbImport {
            plan,
            invalidated: false,
        })?;
        Ok(summary)
    }

    pub(super) fn begin_glb_import(&mut self) {
        let Some(path) = self.choose_glb_import_path() else {
            return;
        };
        // In a frame a SketchUp file is read on a thread while the window shows it.
        match self.file.activity.context.clone() {
            Some(context) if crate::app::skp_import::is_skp_path(&path) => {
                self.start_converted_read(path, &context);
            }
            _ => match read_import_source(ImportFormat::Glb, &path) {
                Ok(source) => self.open_glb_import(path, source),
                Err(reason) => self.report_import_failure(&path, &reason),
            },
        }
    }

    /// Opens the review of `source`, the bytes of the `.glb` or `.skp` file at `path`.
    pub(crate) fn open_glb_import(&mut self, path: PathBuf, source: Vec<u8>) {
        let source_plan = ImportSourcePlan::seal(
            ImportFormat::Glb,
            path.clone(),
            source,
            (),
            &self.document.current(),
        );
        let result = self
            .prepare_glb_import_preview_plan(source_plan)
            .map(|plan| PendingGlbImport {
                plan,
                invalidated: false,
            });
        match result {
            Ok(pending) => self.modal.open(pending),
            Err(reason) => {
                self.modal.close::<PendingGlbImport>();
                self.report_import_failure(&path, &reason);
            }
        }
    }

    pub(super) fn show_glb_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingGlbImport>() else {
            return;
        };
        let path = pending.plan.source.path.display().to_string();
        let mesh_count = pending.plan.review.mesh_primitive_count().to_string();
        let node_count = pending.plan.review.node_count().to_string();
        let instance_count = pending.plan.review.instance_count().to_string();
        let triangle_count = pending.plan.review.triangle_count().to_string();
        let diagnostics = pending
            .plan
            .review
            .diagnostics()
            .iter()
            .map(|diagnostic| {
                (
                    diagnostic.severity(),
                    diagnostic.code().to_owned(),
                    diagnostic.count(),
                )
            })
            .collect::<Vec<_>>();
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-glb-title"))
            .id(egui::Id::new("glb-import-review"))
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-glb-source",
                    &BTreeMap::from([("path", path.clone())]),
                ));
                ui.label(self.catalog.format(
                    "dialog-import-glb-summary",
                    &BTreeMap::from([
                        ("meshes", mesh_count.clone()),
                        ("nodes", node_count.clone()),
                        ("instances", instance_count.clone()),
                        ("triangles", triangle_count.clone()),
                    ]),
                ));
                ui.label(self.catalog.text("dialog-import-glb-preserved"));
                ui.separator();
                ui.label(self.catalog.text("dialog-import-glb-losses"));
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (severity, code, count) in &diagnostics {
                            let severity = match severity {
                                ImportDiagnosticSeverity::Info => {
                                    self.catalog.text("dialog-import-glb-diagnostic-info")
                                }
                                ImportDiagnosticSeverity::Warning => {
                                    self.catalog.text("dialog-import-glb-diagnostic-warning")
                                }
                            };
                            ui.label(format!("{severity} · {code} · {count}"));
                        }
                    });
                ui.colored_label(
                    Color32::YELLOW,
                    self.catalog.text("dialog-import-glb-warning"),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-glb-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-glb-cancel"))
                        .clicked();
                });
            });
        if cancel {
            self.modal.close::<PendingGlbImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingGlbImport>()
                .expect("the GLB review window has a pending import");
            if let Err(error) = self.import_glb_from(&pending) {
                // The import has shown the refusal; logged with its cause (TST-140).
                ketchup_rejection::log!("GLB import refused: {error}");
            }
        }
    }
}
