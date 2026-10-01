//! Imports with review previews (STL, DXF, STEP, IGES, SketchUp scenes).

use crate::*;

impl KetchupApp {
    pub(crate) fn invalidate_pending_import_reviews(&mut self) {
        if let Some(pending) = self.modal.get_mut::<PendingStlImport>() {
            pending.invalidated = true;
        }
        if let Some(pending) = self.modal.get_mut::<PendingDxfImport>() {
            pending.invalidated = true;
        }
        if let Some(pending) = self.modal.get_mut::<PendingStepImport>() {
            pending.invalidated = true;
        }
        if let Some(pending) = self.modal.get_mut::<PendingIgesImport>() {
            pending.invalidated = true;
        }
        if let Some(pending) = self.modal.get_mut::<PendingSketchupSceneImport>() {
            pending.invalidated = true;
        }
        if let Some(pending) = self.modal.get_mut::<glb_import_ui::PendingGlbImport>() {
            pending.invalidated = true;
        }
    }

    pub(crate) fn choose_stl_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-stl");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Stl,
            filter_label: &filter_label,
            extensions: &["stl"],
        })
    }

    pub(crate) fn prepare_stl_import_preview_plan(
        &self,
        source: ImportSourcePlan<ImportLengthUnit>,
    ) -> Result<StlImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let units = ImportUnitDecision::new(source.unit, ImportUnitAuthority::UserDeclared);
        let review =
            bounded_parser::run_bounded_parser(source.format, || parse_stl(&source.source, units))
                .map_err(|error| source.failed(error))?
                .map_err(|error| source.failed(error))?;
        let batch = bounded_parser::run_bounded_parser(source.format, || {
            plan_stl_import(&snapshot, &source.source, source_name, units)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| source.failed(error))?;
        Ok(StlImportPreviewPlan {
            source,
            review,
            proposal,
        })
    }

    pub(crate) fn import_stl_from(&mut self, pending: &PendingStlImport) -> bool {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            if pending.review_error.is_some() {
                return Err(source.error(ImportFailure::ReviewIncomplete));
            }
            source.reread_unchanged()?;
            if self.prepare_stl_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.commit_verified_proposal_with_work_recovery(&pending.plan.proposal)
                .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    pub(crate) fn choose_dxf_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-dxf");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Dxf,
            filter_label: &filter_label,
            extensions: &["dxf"],
        })
    }

    pub(crate) fn read_dxf_source(path: &Path) -> Result<Vec<u8>, ImportError> {
        if !path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("dxf"))
        {
            return Err(ImportError::failed(
                ImportFormat::Dxf,
                "native DWG import is unavailable; convert through an audited DXF workflow",
            ));
        }
        read_import_source(ImportFormat::Dxf, path)
    }

    pub(crate) fn inspect_dxf_for_review(source: &[u8]) -> Result<ParsedDxf, ImportError> {
        let mut last_error = None;
        for unit in [
            ImportLengthUnit::Millimetre,
            ImportLengthUnit::Centimetre,
            ImportLengthUnit::Metre,
            ImportLengthUnit::Inch,
            ImportLengthUnit::Foot,
        ] {
            match bounded_parser::run_bounded_parser(ImportFormat::Dxf, || {
                inspect_dxf(source, DxfImportOptions::new(Some(unit)))
            })
            .map_err(|error| ImportError::failed(ImportFormat::Dxf, error))?
            {
                Ok(review) => return Ok(review),
                Err(error) => last_error = Some(error),
            }
        }
        Err(ImportError {
            format: ImportFormat::Dxf,
            failure: last_error.map_or(ImportFailure::ReviewIncomplete, |error| {
                ImportFailure::Failed(error.into())
            }),
        })
    }

    pub(crate) fn prepare_dxf_import_preview_plan(
        &self,
        source: ImportSourcePlan<ImportLengthUnit>,
    ) -> Result<DxfImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let options = DxfImportOptions::new(Some(source.unit));
        let review = bounded_parser::run_bounded_parser(source.format, || {
            inspect_dxf(&source.source, options)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let batch = bounded_parser::run_bounded_parser(source.format, || {
            plan_dxf_import(&snapshot, &source.source, source_name, options)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| source.failed(error))?;
        Ok(DxfImportPreviewPlan {
            source,
            review,
            proposal,
        })
    }

    pub(crate) fn import_dxf_from(&mut self, pending: &PendingDxfImport) -> bool {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            if !pending.unit_confirmed || pending.review_error.is_some() {
                return Err(source.error(ImportFailure::ReviewIncomplete));
            }
            source.reread_unchanged()?;
            if self.prepare_dxf_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.commit_verified_proposal_with_work_recovery(&pending.plan.proposal)
                .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    pub(crate) fn choose_step_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-step");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Step,
            filter_label: &filter_label,
            extensions: &["step", "stp"],
        })
    }

    pub(crate) fn read_step_source(path: &Path) -> Result<Vec<u8>, ImportError> {
        Self::validate_exact_exchange_extension(path, "STEP import", &["step", "stp"])
            .map_err(|error| ImportError::failed(ImportFormat::Step, error))?;
        read_import_source(ImportFormat::Step, path)
    }

    pub(crate) fn inspect_step_for_review(
        &mut self,
        path: &Path,
        source_sha256: &[u8; 32],
    ) -> Result<StepXdeImportEvidence, ImportError> {
        let executable = self
            .exact_worker_executable()
            .map_err(|error| ImportError::failed(ImportFormat::Step, error))?;
        let source_sha256 = source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let cancelled = AtomicBool::new(false);
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(executable, &cancelled)
            .map_err(|error| ImportError::failed(ImportFormat::Step, error))?;
        worker
            .inspect_step_xde_import_with_cancellation(path, &source_sha256, &cancelled)
            .map_err(|error| ImportError::failed(ImportFormat::Step, error))
    }

    pub(crate) fn prepare_step_import_preview_plan(
        &mut self,
        source: ImportSourcePlan,
    ) -> Result<StepImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let mut temporary = tempfile::Builder::new()
            .suffix(".step")
            .tempfile()
            .map_err(|error| source.failed(error))?;
        temporary
            .write_all(&source.source)
            .and_then(|()| temporary.flush())
            .map_err(|error| source.failed(error))?;
        let evidence = self.inspect_step_for_review(temporary.path(), &source.source_sha256)?;
        let batch = plan_step_xde_import(&snapshot, &source.source, source_name, &evidence)
            .map_err(|error| source.failed(error))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| source.failed(error))?;
        let mut staged_container = self.file.container_data.clone();
        let blob_hash = staged_container
            .insert_import_blob(source.source.clone())
            .map_err(|error| source.failed(error))?;
        if blob_hash != ketchup_model::graph::sha256_hex(&source.source) {
            return Err(source.error(ImportFailure::SealBroken));
        }
        Ok(StepImportPreviewPlan {
            source,
            evidence,
            proposal,
            blob_hash,
        })
    }

    pub(crate) fn import_step_from(&mut self, pending: &PendingStepImport) -> bool {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            source.reread_unchanged()?;
            if self.prepare_step_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            let mut staged_container = self.file.container_data.clone();
            let blob_hash = staged_container
                .insert_import_blob(source.source.clone())
                .map_err(|error| source.failed(error))?;
            if blob_hash != pending.plan.blob_hash {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.commit_verified_proposal_with_container_work_recovery(
                &pending.plan.proposal,
                staged_container,
            )
            .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    pub(crate) fn choose_iges_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-iges");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::Iges,
            filter_label: &filter_label,
            extensions: &["iges", "igs"],
        })
    }

    pub(crate) fn read_iges_source(path: &Path) -> Result<Vec<u8>, ImportError> {
        Self::validate_exact_exchange_extension(path, "IGES import", &["iges", "igs"])
            .map_err(|error| ImportError::failed(ImportFormat::Iges, error))?;
        read_import_source(ImportFormat::Iges, path)
    }

    pub(crate) fn prepare_iges_import_preview_plan(
        &mut self,
        source: ImportSourcePlan,
    ) -> Result<IgesImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let mut temporary = tempfile::Builder::new()
            .suffix(".iges")
            .tempfile()
            .map_err(|error| source.failed(error))?;
        temporary
            .write_all(&source.source)
            .and_then(|()| temporary.flush())
            .map_err(|error| source.failed(error))?;
        let executable = self
            .exact_worker_executable()
            .map_err(|error| source.failed(error))?;
        let source_sha256 = source
            .source_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let cancelled = AtomicBool::new(false);
        let mut worker = ExactWorkerSupervisor::spawn_with_cancellation(executable, &cancelled)
            .map_err(|error| source.failed(error))?;
        let evidence = worker
            .inspect_iges_xde_import_with_cancellation(temporary.path(), &source_sha256, &cancelled)
            .map_err(|error| source.failed(error))?;
        let batch = plan_iges_xde_import(&snapshot, &source.source, source_name, &evidence)
            .map_err(|error| source.failed(error))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| source.failed(error))?;
        let mut staged_container = self.file.container_data.clone();
        let blob_hash = staged_container
            .insert_import_blob(source.source.clone())
            .map_err(|error| source.failed(error))?;
        Ok(IgesImportPreviewPlan {
            source,
            evidence,
            proposal,
            blob_hash,
        })
    }

    pub(crate) fn import_iges_from(&mut self, pending: &PendingIgesImport) -> bool {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            let bytes = source.reread_unchanged()?;
            if self.prepare_iges_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            let mut staged_container = self.file.container_data.clone();
            if staged_container
                .insert_import_blob(bytes)
                .map_err(|error| source.failed(error))?
                != pending.plan.blob_hash
            {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.commit_verified_proposal_with_container_work_recovery(
                &pending.plan.proposal,
                staged_container,
            )
            .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    pub(crate) fn choose_sketchup_scene_import_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-sketchup-scene");
        self.dialogs.pick_import_path(ImportDialogRequest {
            format: ImportFormat::SketchupScene,
            filter_label: &filter_label,
            extensions: &["kscene"],
        })
    }

    pub(crate) fn prepare_sketchup_scene_import_preview_plan(
        &self,
        source: ImportSourcePlan,
    ) -> Result<SketchupSceneImportPreviewPlan, ImportError> {
        let snapshot = self.document.current();
        let source_name = source.verify_seal(&snapshot)?;
        let review = bounded_parser::run_bounded_parser(source.format, || {
            inspect_sketchup_scene(&source.source)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let batch = bounded_parser::run_bounded_parser(source.format, || {
            plan_sketchup_scene_import(&snapshot, &source.source, source_name)
        })
        .map_err(|error| source.failed(error))?
        .map_err(|error| source.failed(error))?;
        let proposal = self
            .document
            .prepare_proposal_with_context(batch, ProposalContext::canonical_preview())
            .map_err(|error| source.failed(error))?;
        Ok(SketchupSceneImportPreviewPlan {
            source,
            review,
            proposal,
        })
    }

    pub(crate) fn import_sketchup_scene_from(
        &mut self,
        pending: &PendingSketchupSceneImport,
    ) -> bool {
        let source = &pending.plan.source;
        let result = (|| {
            if pending.invalidated {
                return Err(source.error(ImportFailure::ReviewStale));
            }
            source.reread_unchanged()?;
            if self.prepare_sketchup_scene_import_preview_plan(source.clone())? != pending.plan {
                return Err(source.error(ImportFailure::ReviewChanged));
            }
            self.commit_verified_proposal_with_work_recovery(&pending.plan.proposal)
                .map_err(|error| source.failed(error))?;
            Ok(())
        })();
        self.report_import_outcome(source, result)
    }

    #[must_use]
    pub fn import_receipt_count(&self) -> usize {
        self.document.current().import_receipts().count()
    }

    #[must_use]
    pub fn undo_step_count(&self) -> usize {
        self.document.visible_undo_steps()
    }

    #[must_use]
    pub fn redo_step_count(&self) -> usize {
        self.document.visible_redo_steps()
    }

    pub(crate) fn show_stl_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingStlImport>() else {
            return;
        };
        let path = pending.plan.source.path.clone();
        let mut unit = pending.plan.source.unit;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-stl-title"))
            .id(egui::Id::new("stl-import-units"))
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-stl-source",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                ui.label(self.catalog.text("dialog-import-stl-units"));
                for (candidate, key) in [
                    (ImportLengthUnit::Millimetre, "unit-millimetre"),
                    (ImportLengthUnit::Centimetre, "unit-centimetre"),
                    (ImportLengthUnit::Metre, "unit-metre"),
                    (ImportLengthUnit::Inch, "unit-inch"),
                    (ImportLengthUnit::Foot, "unit-foot"),
                ] {
                    ui.radio_value(&mut unit, candidate, self.catalog.text(key));
                }
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-stl-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-stl-cancel"))
                        .clicked();
                });
            });
        let updated_source = self.modal.get::<PendingStlImport>().and_then(|pending| {
            (pending.plan.source.unit != unit).then(|| {
                let mut source = pending.plan.source.clone();
                source.unit = unit;
                source
            })
        });
        if let Some(source) = updated_source {
            let result = self.prepare_stl_import_preview_plan(source);
            let pending = self
                .modal
                .get_mut::<PendingStlImport>()
                .expect("the STL review window still has a pending import");
            match result {
                Ok(plan) => {
                    pending.plan = plan;
                    pending.review_error = None;
                }
                Err(error) => pending.review_error = Some(error.to_string()),
            }
        }
        if cancel {
            self.modal.close::<PendingStlImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingStlImport>()
                .expect("the STL review window has a pending import");
            self.import_stl_from(&pending);
        }
    }

    pub(crate) fn show_dxf_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingDxfImport>() else {
            return;
        };
        let path = pending.plan.source.path.clone();
        let review = pending.plan.review.clone();
        let review_error = pending.review_error.clone();
        let mut unit = pending.plan.source.unit;
        let mut unit_confirmed = pending.unit_confirmed;
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-dxf-title"))
            .id(egui::Id::new("dxf-import-review"))
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-dxf-source",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                ui.label(self.catalog.format(
                    "dialog-import-dxf-summary",
                    &BTreeMap::from([
                        ("profiles", review.profiles().len().to_string()),
                        ("layers", review.layers().len().to_string()),
                    ]),
                ));
                ui.label(self.catalog.text("dialog-import-dxf-origin"));
                ui.label(self.catalog.format(
                    "dialog-import-dxf-layers",
                    &BTreeMap::from([("layers", review.layers().join(", "))]),
                ));
                if review.units().authority() == ImportUnitAuthority::UserDeclared {
                    ui.label(self.catalog.text("dialog-import-dxf-units"));
                    for (candidate, key) in [
                        (ImportLengthUnit::Millimetre, "unit-millimetre"),
                        (ImportLengthUnit::Centimetre, "unit-centimetre"),
                        (ImportLengthUnit::Metre, "unit-metre"),
                        (ImportLengthUnit::Inch, "unit-inch"),
                        (ImportLengthUnit::Foot, "unit-foot"),
                    ] {
                        if ui
                            .radio_value(&mut unit, candidate, self.catalog.text(key))
                            .clicked()
                        {
                            unit_confirmed = true;
                        }
                    }
                } else {
                    let unit_key = match review.units().source_unit() {
                        ImportLengthUnit::Millimetre => "unit-millimetre",
                        ImportLengthUnit::Centimetre => "unit-centimetre",
                        ImportLengthUnit::Metre => "unit-metre",
                        ImportLengthUnit::Inch => "unit-inch",
                        ImportLengthUnit::Foot => "unit-foot",
                    };
                    ui.label(self.catalog.format(
                        "dialog-import-dxf-file-units",
                        &BTreeMap::from([("unit", self.catalog.text(unit_key))]),
                    ));
                }
                ui.separator();
                ui.label(self.catalog.text("dialog-import-dxf-losses"));
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for diagnostic in review.diagnostics() {
                            let severity_key = match diagnostic.severity() {
                                ImportDiagnosticSeverity::Info => {
                                    "dialog-import-dxf-diagnostic-info"
                                }
                                ImportDiagnosticSeverity::Warning => {
                                    "dialog-import-dxf-diagnostic-warning"
                                }
                            };
                            let subject = diagnostic
                                .subject()
                                .map(|value| format!(" · {value}"))
                                .unwrap_or_default();
                            ui.label(format!(
                                "{} · {}{} · {}",
                                self.catalog.text(severity_key),
                                diagnostic.code(),
                                subject,
                                diagnostic.count()
                            ));
                        }
                    });
                ui.label(self.catalog.text("dialog-import-dxf-subset-warning"));
                if let Some(error) = review_error.as_deref() {
                    ui.colored_label(Color32::LIGHT_RED, error);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .add_enabled(
                            review_error.is_none() && unit_confirmed,
                            egui::Button::new(self.catalog.text("dialog-import-dxf-confirm")),
                        )
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-dxf-cancel"))
                        .clicked();
                });
            });
        if let Some(pending) = self.modal.get_mut::<PendingDxfImport>() {
            pending.unit_confirmed = unit_confirmed;
        }
        let updated_source = self.modal.get::<PendingDxfImport>().and_then(|pending| {
            (pending.plan.source.unit != unit).then(|| {
                let mut source = pending.plan.source.clone();
                source.unit = unit;
                source
            })
        });
        if let Some(source) = updated_source {
            let result = self.prepare_dxf_import_preview_plan(source);
            let pending = self
                .modal
                .get_mut::<PendingDxfImport>()
                .expect("the DXF review window still has a pending import");
            match result {
                Ok(plan) => {
                    pending.plan = plan;
                    pending.review_error = None;
                }
                Err(error) => pending.review_error = Some(error.to_string()),
            }
        }
        if cancel {
            self.modal.close::<PendingDxfImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingDxfImport>()
                .expect("the DXF review window has a pending import");
            self.import_dxf_from(&pending);
        }
    }

    pub(crate) fn show_step_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingStepImport>() else {
            return;
        };
        let path = pending.plan.source.path.clone();
        let evidence = pending.plan.evidence.clone();
        let solid_count = evidence
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .map(|index| evidence.parts[index as usize].exact.solid_count)
            .sum::<u32>();
        let volume_mm3 = evidence
            .nodes
            .iter()
            .filter_map(|node| node.part_index)
            .map(|index| evidence.parts[index as usize].exact.volume_mm3)
            .sum::<f64>();
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-step-title"))
            .id(egui::Id::new("step-import-review"))
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-step-source",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                ui.label(self.catalog.format(
                    "dialog-import-step-summary",
                    &BTreeMap::from([
                        ("solids", solid_count.to_string()),
                        ("volume", format!("{volume_mm3:.6}")),
                    ]),
                ));
                ui.label(self.catalog.text("dialog-import-step-preserved"));
                ui.label(self.catalog.text("dialog-import-step-root-warning"));
                ui.label(self.catalog.text("dialog-import-step-metadata-warning"));
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-step-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-step-cancel"))
                        .clicked();
                });
            });
        if cancel {
            self.modal.close::<PendingStepImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingStepImport>()
                .expect("the STEP review window has a pending import");
            self.import_step_from(&pending);
        }
    }

    pub(crate) fn show_iges_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingIgesImport>() else {
            return;
        };
        let path = pending.plan.source.path.clone();
        let evidence = pending.plan.evidence.clone();
        let mut import = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("dialog-import-iges-title"))
            .id(egui::Id::new("iges-import-review"))
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-iges-source",
                    &BTreeMap::from([("path", path.display().to_string())]),
                ));
                ui.label(
                    self.catalog.format(
                        "dialog-import-iges-summary",
                        &BTreeMap::from([
                            (
                                "solids",
                                evidence
                                    .parts
                                    .iter()
                                    .map(|part| part.exact.solid_count)
                                    .sum::<u32>()
                                    .to_string(),
                            ),
                            (
                                "volume",
                                format!(
                                    "{:.6}",
                                    evidence
                                        .parts
                                        .iter()
                                        .map(|part| part.exact.volume_mm3)
                                        .sum::<f64>()
                                ),
                            ),
                        ]),
                    ),
                );
                ui.label(self.catalog.text("dialog-import-iges-preserved"));
                ui.colored_label(
                    Color32::YELLOW,
                    self.catalog.text("dialog-import-iges-loss-warning"),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-iges-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-iges-cancel"))
                        .clicked();
                });
            });
        if cancel {
            self.modal.close::<PendingIgesImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingIgesImport>()
                .expect("the IGES review window has a pending import");
            self.import_iges_from(&pending);
        }
    }

    pub(crate) fn show_sketchup_scene_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.modal.get::<PendingSketchupSceneImport>() else {
            return;
        };
        let path = pending.plan.source.path.display().to_string();
        let definition_count = pending.plan.review.definition_count().to_string();
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
        egui::Window::new(self.catalog.text("dialog-import-sketchup-scene-title"))
            .id(egui::Id::new("sketchup-scene-import-review"))
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(self.catalog.format(
                    "dialog-import-sketchup-scene-source",
                    &BTreeMap::from([("path", path.clone())]),
                ));
                ui.label(self.catalog.format(
                    "dialog-import-sketchup-scene-summary",
                    &BTreeMap::from([
                        ("definitions", definition_count.clone()),
                        ("instances", instance_count.clone()),
                        ("triangles", triangle_count.clone()),
                    ]),
                ));
                ui.label(self.catalog.text("dialog-import-sketchup-scene-preserved"));
                ui.separator();
                ui.label(self.catalog.text("dialog-import-sketchup-scene-losses"));
                egui::ScrollArea::vertical()
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for (diagnostic_severity, code, count) in &diagnostics {
                            let severity = match diagnostic_severity {
                                ImportDiagnosticSeverity::Info => self
                                    .catalog
                                    .text("dialog-import-sketchup-scene-diagnostic-info"),
                                ImportDiagnosticSeverity::Warning => self
                                    .catalog
                                    .text("dialog-import-sketchup-scene-diagnostic-warning"),
                            };
                            ui.label(format!("{severity} · {code} · {count}"));
                        }
                    });
                ui.colored_label(
                    Color32::YELLOW,
                    self.catalog.text("dialog-import-sketchup-scene-warning"),
                );
                ui.separator();
                ui.horizontal(|ui| {
                    import = ui
                        .button(self.catalog.text("dialog-import-sketchup-scene-confirm"))
                        .clicked();
                    cancel = ui
                        .button(self.catalog.text("dialog-import-sketchup-scene-cancel"))
                        .clicked();
                });
            });
        if cancel {
            self.modal.close::<PendingSketchupSceneImport>();
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .modal
                .remove::<PendingSketchupSceneImport>()
                .expect("the SketchUp scene review window has a pending import");
            self.import_sketchup_scene_from(&pending);
        }
    }
}
