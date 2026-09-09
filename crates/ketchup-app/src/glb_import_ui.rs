use super::*;

#[derive(Clone, Debug, PartialEq)]
struct GlbImportSourcePlan {
    path: PathBuf,
    source: Vec<u8>,
    document_id: DocumentId,
    revision_id: u64,
    canonical_digest: String,
    source_sha256: [u8; 32],
    source_byte_len: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct GlbImportPreviewPlan {
    source: GlbImportSourcePlan,
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

    fn read_glb_source(path: &Path) -> Result<Vec<u8>, String> {
        if std::fs::metadata(path)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_GLB_SOURCE_BYTES
        {
            return Err("GLB source exceeds the bounded 32 MiB envelope".to_owned());
        }
        let mut file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        let mut source = Vec::new();
        std::io::Read::by_ref(&mut file)
            .take(MAX_GLB_SOURCE_BYTES + 1)
            .read_to_end(&mut source)
            .map_err(|error| error.to_string())?;
        if source.len() as u64 > MAX_GLB_SOURCE_BYTES {
            return Err("GLB source exceeds the bounded 32 MiB envelope".to_owned());
        }
        Ok(source)
    }

    fn prepare_glb_import_preview_plan(
        &self,
        source: GlbImportSourcePlan,
    ) -> Result<GlbImportPreviewPlan, String> {
        let snapshot = self.document.current();
        if snapshot.document_id() != source.document_id
            || snapshot.revision_id() != source.revision_id
            || snapshot.canonical_digest() != source.canonical_digest
        {
            return Err("GLB import review is stale for the active document".to_owned());
        }
        if source.source.len() as u64 != source.source_byte_len
            || sha256_bytes(&source.source) != source.source_sha256
        {
            return Err("sealed GLB source identity does not match its bytes".to_owned());
        }
        let source_name = source
            .path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "GLB source name is not valid UTF-8".to_owned())?;
        let review = std::panic::catch_unwind(|| inspect_glb(&source.source))
            .map_err(|_| "bounded GLB parser stopped without publishing geometry".to_owned())?
            .map_err(|error| error.to_string())?;
        let batch =
            std::panic::catch_unwind(|| plan_glb_import(&snapshot, &source.source, source_name))
                .map_err(|_| "bounded GLB parser stopped without publishing geometry".to_owned())?
                .map_err(|error| error.to_string())?;
        Ok(GlbImportPreviewPlan {
            source,
            review,
            batch,
        })
    }

    fn import_glb_from(&mut self, pending: &PendingGlbImport) -> bool {
        let path = &pending.plan.source.path;
        let result = (|| {
            if pending.invalidated {
                return Err("GLB import review is stale for the active document".to_owned());
            }
            let source = Self::read_glb_source(path)?;
            if source.len() as u64 != pending.plan.source.source_byte_len
                || sha256_bytes(&source) != pending.plan.source.source_sha256
                || source != pending.plan.source.source
            {
                return Err("GLB source changed after it was selected for review".to_owned());
            }
            let rederived = self.prepare_glb_import_preview_plan(pending.plan.source.clone())?;
            if rederived != pending.plan {
                return Err("GLB review or canonical batch changed after preview".to_owned());
            }
            self.document
                .apply_batch(&pending.plan.batch)
                .map_err(|error| error.to_string())?;
            Ok::<(), String>(())
        })();
        match result {
            Ok(()) => {
                self.digest = self.catalog.format(
                    "digest-imported-glb",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                true
            }
            Err(reason) => {
                self.digest = self.catalog.format(
                    "error-import-glb",
                    &BTreeMap::from([("path", path.display().to_string()), ("reason", reason)]),
                );
                false
            }
        }
    }

    pub(super) fn begin_glb_import(&mut self) {
        let Some(path) = self.choose_glb_import_path() else {
            return;
        };
        let result = Self::read_glb_source(&path).and_then(|source| {
            let snapshot = self.document.current();
            let source_plan = GlbImportSourcePlan {
                path: path.clone(),
                source_sha256: sha256_bytes(&source),
                source_byte_len: source.len() as u64,
                source,
                document_id: snapshot.document_id(),
                revision_id: snapshot.revision_id(),
                canonical_digest: snapshot.canonical_digest(),
            };
            let plan = self.prepare_glb_import_preview_plan(source_plan)?;
            Ok(PendingGlbImport {
                plan,
                invalidated: false,
            })
        });
        match result {
            Ok(pending) => self.pending_glb_import = Some(pending),
            Err(reason) => {
                self.pending_glb_import = None;
                self.digest = self.catalog.format(
                    "error-import-glb",
                    &BTreeMap::from([("path", path.display().to_string()), ("reason", reason)]),
                );
            }
        }
    }

    pub(super) fn show_glb_import_window(&mut self, context: &egui::Context) {
        let Some(pending) = self.pending_glb_import.as_ref() else {
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
            self.pending_glb_import = None;
            self.digest = self.catalog.text("digest-cancelled");
        } else if import {
            let pending = self
                .pending_glb_import
                .take()
                .expect("the GLB review window has a pending import");
            self.import_glb_from(&pending);
        }
    }
}
