use super::*;

#[derive(Clone, Debug, PartialEq)]
struct GlbImportPreviewPlan {
    source: ImportSourcePlan,
    review: ParsedGlbScene,
    batch: CommandBatch,
}

#[derive(Clone, Debug)]
pub(super) struct PendingGlbImport {
    plan: GlbImportPreviewPlan,
    pub(super) invalidated: bool,
}

impl KetchupApp {
    fn choose_glb_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-glb");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Glb,
            filter_label: &filter_label,
            extensions: &["glb"],
        })
    }

    fn prepare_glb_import_preview_plan(
        &self,
        source: ImportSourcePlan,
    ) -> Result<GlbImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let review = crate::bounded_parser::run_bounded_parser(source.format, || {
            inspect_glb(&source.source)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let batch = crate::bounded_parser::run_bounded_parser(source.format, || {
            plan_glb_import(&snapshot, &source.source, source_name)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        Ok(GlbImportPreviewPlan {
            source,
            review,
            batch,
        })
    }

    fn import_glb_from(&mut self, pending: &PendingGlbImport) -> bool {
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

    pub(super) fn begin_glb_import(&mut self) {
        let Some(path) = self.choose_glb_import_path() else {
            return;
        };
        let result = read_import_source(ImportFormat::Glb, &path).and_then(|source| {
            let source_plan = ImportSourcePlan::seal(
                ImportFormat::Glb,
                path.clone(),
                source,
                (),
                &self.document.current(),
            );
            let plan = self.prepare_glb_import_preview_plan(source_plan)?;
            Ok(PendingGlbImport {
                plan,
                invalidated: false,
            })
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
            self.import_glb_from(&pending);
        }
    }
}
