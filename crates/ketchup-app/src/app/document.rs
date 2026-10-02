//! New, open and save, work recovery, revisions, undo and redo.

use crate::*;

impl KetchupApp {
    #[must_use]
    pub const fn build_version() -> &'static str {
        match option_env!("KETCHUP_BUILD_VERSION") {
            Some(version) => version,
            None => env!("CARGO_PKG_VERSION"),
        }
    }

    #[must_use]
    pub const fn is_manual_alpha_build() -> bool {
        cfg!(feature = "manual-alpha")
    }

    #[must_use]
    pub fn title() -> String {
        let title = LocaleCatalog::english().text("app-title");
        if Self::is_manual_alpha_build() {
            format!("{title} Manual Alpha")
        } else {
            title
        }
    }

    pub(crate) fn document_title(&self) -> String {
        let name = self
            .file
            .path
            .as_deref()
            .or_else(|| {
                self.file
                    .recovery_open
                    .as_ref()
                    .map(|recovery| recovery.requested_path.as_path())
            })
            .and_then(Path::file_name)
            .map_or_else(
                || self.catalog.text("document-untitled"),
                |name| name.to_string_lossy().into_owned(),
            );
        self.catalog.format(
            if self.is_dirty() {
                "document-title-dirty"
            } else {
                "document-title-clean"
            },
            &BTreeMap::from([("name", name)]),
        )
    }

    pub(crate) fn reset_document_presentation(&mut self) {
        self.invalidate_pending_import_reviews();
        self.cancel_mesh_conversion();
        self.tool_preview = None;
        self.push_pull.smart_proposal = None;
        self.push_pull.smart_planning = None;
        self.solid_tools.target = None;
        self.solid_tools.revolve = None;
        self.clear_helix_preview();
        self.solid_tools.loft_input_sections = None;
        self.solid_tools.pocket_editor_feature = None;
        self.solid_tools.pocket_depth_input.clear();
        self.parameter.editor_node = None;
        self.parameter.expression_input.clear();
        self.parameter.canonical_source.clear();
        self.parameter.provenance = None;
        self.parameter.last_recomputed_nodes.clear();
        self.assembly_editor = assembly_ui::AssemblyEditorState::default();
        self.body_editor = body_ui::BodyEditorState::default();
        self.face_workflow = face_workflow_ui::FaceWorkflowUiState::default();
        self.feature_history = feature_history_ui::FeatureHistoryUiState::default();
        self.selection = SelectionState::default();
        self.hover.target = None;
        self.hover.pick = None;
        self.hover.snap = None;
        self.hover.overlap_index = 0;
        self.hover.pointer = None;
        self.hover.projection_cache.get_mut().take();
        self.gesture.drag.close::<ZoomWindowDrag>();
        self.active_tool = ActiveTool::Select;
        self.cancel_pending_assistant_work();
        self.assistant.verification = None;
        self.side_effect_receipts.clear();
        self.gesture.drag.close::<PushPullDrag>();
        self.gesture.drag.close::<PushPullAnchor>();
        self.push_pull.last = None;
        self.reset_transform_interaction();
        self.end_transform_correction();
        self.clipboard.occurrences.clear();
        self.clipboard.cut_occurrences.clear();
        self.gesture.sketch.armed = false;
        self.gesture.sketch.start = None;
        self.gesture.sketch.end = None;
        self.gesture.sketch.cursor = None;
        self.gesture.sketch.chain_origin = None;
        self.gesture.sketch.chain_points.clear();
        self.gesture.sketch.chain_items.clear();
        self.gesture.sketch.axis_lock = None;
        self.clear_measurement();
        self.value_box.input.clear();
        self.value_box.focus = false;
        if let Some(task) = self.exact.task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact.results.clear();
        self.exact.topology_results.clear();
        self.exact.result_history.clear();
        self.exact.topology_result_history.clear();
        self.render.plan = None;
        self.exact.source = None;
        self.exact.retry_at = None;
        self.status_key = "status-ready";
    }

    pub(crate) fn new_document(&mut self) {
        self.cancel_pending_assistant_work();
        let mut live_bridge = self.live.bridge.take();
        if let Some(bridge) = live_bridge.as_mut() {
            bridge.invalidate_document_context();
        }
        let live_consent_broker = self.live.consent_broker.take();
        let live_consent_attached = self.live.consent_attached;
        let dialogs = std::mem::replace(&mut self.dialogs, Box::new(NativeFileDialogs::default()));
        let assistant_transport = Arc::clone(&self.assistant.transport);
        let assistant_request_sequence = self.assistant.request_sequence;
        let assistant_diagnostics_enabled = self.assistant.diagnostics_enabled;
        let assistant_inspector_tab = self.assistant.inspector_tab;
        let catalog = self.catalog.clone();
        let outliner_visible = self.panels.outliner_visible;
        let tags_visible = self.panels.tags_visible;
        let dimensions_visible = self.panels.dimensions_visible;
        let manual_cad_panels_visible = self.panels.manual_cad_panels_visible;
        let about_open = self.panels.about_open;
        let exact_worker_path = self.exact.worker_path.clone();
        let exact_worker_attempted = self.exact.worker_attempted;
        // The graphics device outlives the document: losing its target format
        // here would silently drop the whole instanced scene until restart.
        let wgpu_target_format = self.render.wgpu_target_format;
        let wgpu_device = self.render.wgpu_device.clone();
        let wgpu_queue = self.render.wgpu_queue.clone();
        *self = Self::with_catalog_and_initial_box(catalog, false)
            .with_dialogs(dialogs)
            .with_assistant_transport(assistant_transport);
        self.live.bridge = live_bridge;
        self.live.consent_broker = live_consent_broker;
        self.live.consent_attached = live_consent_attached;
        self.render.wgpu_target_format = wgpu_target_format;
        self.render.wgpu_device = wgpu_device;
        self.render.wgpu_queue = wgpu_queue;
        self.exact.worker_path = exact_worker_path;
        self.exact.worker_attempted = exact_worker_attempted;
        self.assistant.request_sequence = assistant_request_sequence;
        self.assistant.diagnostics_enabled = assistant_diagnostics_enabled;
        self.assistant.inspector_tab = assistant_inspector_tab;
        self.panels.outliner_visible = outliner_visible;
        self.panels.tags_visible = tags_visible;
        self.panels.dimensions_visible = dimensions_visible;
        self.panels.manual_cad_panels_visible = manual_cad_panels_visible;
        self.panels.about_open = about_open;
        self.digest = self.catalog.text("digest-new-document");
    }

    pub(crate) fn open_document_from(&mut self, path: &Path) -> bool {
        self.open_document_with_discard(path, false)
    }

    pub(crate) fn open_document_with_discard(
        &mut self,
        path: &Path,
        discard_confirmed: bool,
    ) -> bool {
        self.cancel_pending_assistant_work();
        let discard_owned = discard_confirmed
            && self.file.work_recovery_lock.is_some()
            && (self.file.path.as_deref() == Some(path)
                || self
                    .file
                    .recovery_open
                    .as_ref()
                    .is_some_and(|recovery| recovery.requested_path == path));
        // Keep ownership throughout a confirmed reopen, including failed loads.
        let recovery_lock = if discard_owned {
            let result = ketchup_model::persistence::check_work_recovery_identity(
                path,
                self.file.work_recovery_identity,
            )
            .and_then(|()| {
                ketchup_model::persistence::clear_work_recovery(
                    path,
                    self.file.work_recovery_identity,
                )
            });
            if let Err(error) = result {
                self.report_file_persistence_error("error-open-document", path, &error);
                return false;
            }
            self.file.work_recovery_identity = None;
            self.file.work_recovery_digest = None;
            None
        } else {
            match ketchup_model::persistence::WorkRecoveryLock::acquire(path) {
                Ok(lock) => Some(lock),
                Err(error) => {
                    self.report_file_persistence_error("error-open-document", path, &error);
                    return false;
                }
            }
        };
        match ketchup_model::persistence::load_file_with_source(path) {
            Ok(loaded_file) => {
                let (outcome, effective_path, source, work_recovery_identity) =
                    loaded_file.into_parts();
                if !outcome.is_editable() {
                    let plan = match self.prepare_migration_review_plan(
                        path,
                        effective_path,
                        source,
                        &outcome,
                    ) {
                        Ok(plan) => plan,
                        Err(reason) => {
                            self.file.review_candidate = None;
                            self.file.migration_review_plan = None;
                            self.digest = self.catalog.format(
                                "error-open-document",
                                &BTreeMap::from([
                                    ("path", path.display().to_string()),
                                    ("reason", reason.to_string()),
                                ]),
                            );
                            return false;
                        }
                    };
                    self.file.review_candidate = Some(outcome);
                    self.file.migration_review_plan = Some(plan);
                    self.digest = self.catalog.format(
                        "error-open-document",
                        &BTreeMap::from([
                            ("path", path.display().to_string()),
                            (
                                "reason",
                                "document requires read-only migration review".to_owned(),
                            ),
                        ]),
                    );
                    return false;
                }
                let file_identity = (effective_path == path)
                    .then(|| ketchup_model::persistence::FileIdentity::from_bytes(&source));
                let recovery_open = (effective_path != path).then(|| RecoveryOpenState {
                    requested_path: path.to_owned(),
                    source_path: effective_path,
                });
                let Ok((mut document, container_data)) = outcome.into_editable_with_container()
                else {
                    unreachable!("editable load outcome must contain an editable document");
                };
                document
                    .configure_human_confirmation_policy(
                        self.confirmation_surface.verifying_key(),
                        1,
                    )
                    .expect("an opened document accepts the application confirmation policy");
                self.document = document;
                self.file.container_data = container_data;
                self.file.review_candidate = None;
                self.file.migration_review_plan = None;
                self.file.path = recovery_open.is_none().then(|| path.to_owned());
                self.file.recovery_open = recovery_open;
                self.file.identity = file_identity;
                self.file.work_recovery_identity = work_recovery_identity;
                self.file.work_recovery_lock = work_recovery_identity
                    .and(recovery_lock.or_else(|| self.file.work_recovery_lock.take()));
                self.file.work_recovery_digest = None;
                self.file.saved_digest = self.document.history_digest();
                self.reset_document_presentation();
                self.camera.zoom_fit_pending = true;
                self.camera.zoom_fit_pending_quiet = true;
                self.load_assistant_conversation();
                self.load_assistant_memory();
                if let Some(recovery) = &self.file.recovery_open {
                    self.status_key = "status-recovery";
                    self.digest = self.catalog.format(
                        "digest-opened-recovery",
                        &BTreeMap::from([
                            ("source", recovery.source_path.display().to_string()),
                            ("requested", recovery.requested_path.display().to_string()),
                        ]),
                    );
                } else {
                    self.digest = self.catalog.format(
                        "digest-opened-document",
                        &BTreeMap::from([("path", path.display().to_string())]),
                    );
                }
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-open-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                false
            }
        }
    }

    pub(crate) fn authorize_path_side_effect(
        &mut self,
        class: HighRiskClass,
        operation: &str,
        title: &str,
        risk: &str,
        path: &Path,
        payload: &[u8],
    ) -> Result<(), Rejection> {
        let scope = HighRiskScope::new(class, None, None, Some(path.display().to_string()))
            .map_err(|error| failed("side_effect.scope", error))?;
        let proposal = self
            .document
            .prepare_high_risk_side_effect(
                operation,
                ProposalPrincipal::LocalAssistant,
                scope,
                payload,
            )
            .map_err(|error| failed("side_effect.prepare", error))?;
        let description = format!(
            "Risk: {risk}\nPath: {}\nDocument revision: {}\nPayload SHA-256: {}\nOperation digest: {}",
            path.display(),
            proposal.provenance_revision(),
            proposal.payload_digest(),
            proposal.operation_digest(),
        );
        let Some(approving_human) = self.dialogs.confirm_high_risk(HighRiskConfirmationRequest {
            title,
            description: &description,
        }) else {
            return Err(
                Rejection::new("side_effect.declined", RejectionPhase::Validation)
                    .reason(format!("authenticated human declined {risk}")),
            );
        };
        let now_ms = current_unix_time_ms()?;
        let approval = self
            .confirmation_surface
            .issue_side_effect(
                &proposal,
                AuthenticatedApprover::Human(approving_human),
                now_ms,
                now_ms
                    .checked_add(MAX_HUMAN_CONFIRMATION_LIFETIME_MS)
                    .ok_or_else(|| {
                        Rejection::new("side_effect.clock", RejectionPhase::Planning)
                            .reason("confirmation expiry exceeds the supported clock")
                    })?,
            )
            .map_err(|error| failed("side_effect.approval", error))?;
        let receipt = self
            .document
            .authorize_high_risk_side_effect(&proposal, &approval, now_ms)
            .map_err(|error| failed("side_effect.authorize", error))?;
        self.side_effect_receipts.push(receipt);
        Ok(())
    }

    pub(crate) fn authorize_overwrite(
        &mut self,
        path: &Path,
        payload: &[u8],
    ) -> Result<(), Rejection> {
        self.side_effect_receipts.clear();
        self.authorize_path_side_effect(
            HighRiskClass::Overwrite,
            "overwrite-native-document",
            "Confirm high-risk overwrite",
            "overwrite existing file",
            path,
            payload,
        )
    }

    pub(crate) fn retry_pending_work_recovery_cleanup(
        &mut self,
    ) -> Result<(), ketchup_model::persistence::FilePersistenceError> {
        let Some((path, identity)) = self.file.pending_work_recovery_cleanup.clone() else {
            return Ok(());
        };
        ketchup_model::persistence::clear_work_recovery(&path, Some(identity))?;
        self.file.pending_work_recovery_cleanup = None;
        Ok(())
    }

    fn report_file_persistence_error(
        &mut self,
        key: &str,
        path: &Path,
        error: &ketchup_model::persistence::FilePersistenceError,
    ) {
        let reason = match error {
            ketchup_model::persistence::FilePersistenceError::ExternalConflict => format!(
                "{error}; another window may own its unsaved recovery. Keep this window and use Save As to a different file, or close the other writer before reopening."
            ),
            _ => error.to_string(),
        };
        self.digest = self.catalog.format(
            key,
            &BTreeMap::from([("path", path.display().to_string()), ("reason", reason)]),
        );
    }

    fn acquire_work_recovery_lock(
        &mut self,
    ) -> Result<(), ketchup_model::persistence::FilePersistenceError> {
        if self.file.work_recovery_lock.is_none()
            && let Some(path) = &self.file.path
        {
            self.file.work_recovery_lock =
                Some(ketchup_model::persistence::WorkRecoveryLock::acquire(path)?);
        }
        Ok(())
    }

    pub(crate) fn refresh_work_recovery_checkpoint(&mut self) {
        if let Err(error) = self.retry_pending_work_recovery_cleanup() {
            let path = self
                .file
                .pending_work_recovery_cleanup
                .as_ref()
                .expect("failed cleanup remains pending")
                .0
                .clone();
            self.report_file_persistence_error("error-save-document", &path, &error);
            return;
        }
        let checkpoint_digest = format!(
            "{}:{}",
            self.document.history_digest(),
            assistant_conversation_digest(&self.assistant.messages)
        );
        if !self.is_dirty() {
            if let Some(path) = self.file.path.clone()
                && let Err(error) = ketchup_model::persistence::clear_work_recovery(
                    &path,
                    self.file.work_recovery_identity,
                )
            {
                self.report_file_persistence_error("error-save-document", &path, &error);
                return;
            }
            self.file.work_recovery_lock = None;
            self.file.work_recovery_identity = None;
            self.file.work_recovery_digest = None;
            return;
        }
        if self.file.work_recovery_digest.as_deref() == Some(&checkpoint_digest) {
            return;
        }
        let (Some(path), Some(identity)) = (self.file.path.clone(), self.file.identity) else {
            return;
        };
        self.store_assistant_conversation();
        self.store_assistant_memory();
        let result = self.acquire_work_recovery_lock().and_then(|()| {
            ketchup_model::persistence::save_work_recovery_document_store_with_container_if_unchanged(
                &path,
                &self.document,
                &self.file.container_data,
                identity,
                self.file.work_recovery_identity,
            )
        });
        match result {
            Ok(checkpoint_identity) => {
                self.file.work_recovery_identity = Some(checkpoint_identity);
                self.file.work_recovery_digest = Some(checkpoint_digest);
            }
            Err(error) => {
                if let Some(identity) =
                    error.published_identity(&ketchup_model::persistence::work_recovery_path(&path))
                {
                    self.file.work_recovery_identity = Some(identity);
                    self.file.work_recovery_digest = Some(checkpoint_digest);
                }
                self.report_file_persistence_error("error-save-document", &path, &error);
            }
        }
        if self.file.work_recovery_identity.is_none() {
            self.file.work_recovery_lock = None;
        }
    }

    pub(crate) fn save_document_to(&mut self, path: &Path) -> bool {
        self.save_document_to_while(path, || true)
    }

    pub(crate) fn save_document_to_while(
        &mut self,
        path: &Path,
        request_authorized: impl Fn() -> bool,
    ) -> bool {
        if !request_authorized() {
            return false;
        }
        if path.is_dir() {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    ("reason", "the target path is a directory".to_owned()),
                ]),
            );
            return false;
        }
        if let Err(error) = self.retry_pending_work_recovery_cleanup() {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    ("reason", error.to_string()),
                ]),
            );
            return false;
        }
        let active_target = self.file.path.as_deref() == Some(path)
            || self
                .file
                .recovery_open
                .as_ref()
                .is_some_and(|recovery| recovery.requested_path == path);
        let _target_lock = if active_target && self.file.work_recovery_lock.is_some() {
            None
        } else {
            match ketchup_model::persistence::WorkRecoveryLock::acquire(path) {
                Ok(lock) => Some(lock),
                Err(error) => {
                    self.report_file_persistence_error("error-save-document", path, &error);
                    return false;
                }
            }
        };
        // Save As must not overwrite an abandoned recovery branch either.
        let expected_recovery = active_target
            .then_some(self.file.work_recovery_identity)
            .flatten();
        if let Err(error) =
            ketchup_model::persistence::check_work_recovery_identity(path, expected_recovery)
        {
            self.report_file_persistence_error("error-save-document", path, &error);
            return false;
        }
        self.store_assistant_conversation();
        self.store_assistant_memory();
        let (prepared, truncate_history) = match ketchup_model::persistence::save_document_store(
            &self.document,
            &self.file.container_data,
        ) {
            Ok(bytes) => (bytes, false),
            Err(ketchup_model::persistence::PersistenceError::ResourceLimit) => {
                let bytes = match ketchup_model::persistence::save_document_store_current_snapshot(
                    &self.document,
                    &self.file.container_data,
                ) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        self.digest = self.catalog.format(
                            "error-save-document",
                            &BTreeMap::from([
                                ("path", path.display().to_string()),
                                ("reason", error.to_string()),
                            ]),
                        );
                        return false;
                    }
                };
                let title = self.catalog.text("dialog-save-current-only-title");
                let description = self.catalog.format(
                    "dialog-save-current-only-description",
                    &BTreeMap::from([
                        ("undo", self.document.visible_undo_steps().to_string()),
                        ("redo", self.document.visible_redo_steps().to_string()),
                    ]),
                );
                if !self
                    .dialogs
                    .confirm_history_truncation(HistoryTruncationRequest {
                        title: &title,
                        description: &description,
                    })
                {
                    self.digest = self.catalog.format(
                        "error-save-document",
                        &BTreeMap::from([
                            ("path", path.display().to_string()),
                            ("reason", self.catalog.text("save-current-only-refused")),
                        ]),
                    );
                    return false;
                }
                if !request_authorized() {
                    return false;
                }
                (bytes, true)
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-save-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                return false;
            }
        };
        let saved_identity = ketchup_model::persistence::FileIdentity::from_bytes(&prepared);
        let owned_identity = (self.file.path.as_deref() == Some(path))
            .then_some(self.file.identity)
            .flatten();
        let expected_identity =
            match ketchup_model::persistence::read_native_document_identity(path) {
                Ok(identity) => Some(identity),
                Err(ketchup_model::persistence::FilePersistenceError::Io(error))
                    if error.kind() == std::io::ErrorKind::NotFound =>
                {
                    None
                }
                Err(error) => {
                    self.digest = self.catalog.format(
                        "error-save-document",
                        &BTreeMap::from([
                            ("path", path.display().to_string()),
                            ("reason", error.to_string()),
                        ]),
                    );
                    return false;
                }
            };
        if owned_identity.is_some_and(|owned| Some(owned) != expected_identity) {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    (
                        "reason",
                        ketchup_model::persistence::FilePersistenceError::ExternalConflict
                            .to_string(),
                    ),
                ]),
            );
            return false;
        }
        // Saving over this document's own, unchanged file is a plain Save: only
        // replacing some other existing file needs the user's consent.
        if expected_identity.is_some()
            && owned_identity.is_none()
            && let Err(error) = self.authorize_overwrite(path, &prepared)
        {
            self.digest = self.catalog.format(
                "error-save-document",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    ("reason", error.reason_text().to_owned()),
                ]),
            );
            return false;
        }
        if !request_authorized() {
            return false;
        }
        let result = match (truncate_history, expected_identity) {
            (true, Some(expected)) => ketchup_model::persistence::save_atomic_document_store_current_snapshot_with_container_if_unchanged(
                path,
                &self.document,
                &self.file.container_data,
                expected,
            )
            .map(|_| ()),
            (false, Some(expected)) => ketchup_model::persistence::save_atomic_document_store_with_container_if_unchanged(
                path,
                &self.document,
                &self.file.container_data,
                expected,
            )
            .map(|_| ()),
            (true, None) => ketchup_model::persistence::save_atomic_document_store_current_snapshot_with_container_if_absent(
                path,
                &self.document,
                &self.file.container_data,
            )
            .map(|_| ()),
            (false, None) => ketchup_model::persistence::save_atomic_document_store_with_container_if_absent(
                path,
                &self.document,
                &self.file.container_data,
            )
            .map(|_| ()),
        };
        let (result, publication_error) = match result {
            Err(error) if error.published_identity(path).is_some() => (Ok(()), Some(error)),
            result => (result, None),
        };
        match result {
            Ok(()) => {
                if truncate_history {
                    self.document.discard_history_before_current();
                }
                let owned_recovery_path = self.file.path.clone().or_else(|| {
                    self.file
                        .recovery_open
                        .as_ref()
                        .map(|recovery| recovery.requested_path.clone())
                });
                if let (Some(recovery_path), Some(recovery_identity)) =
                    (owned_recovery_path, self.file.work_recovery_identity)
                    && ketchup_model::persistence::clear_work_recovery(
                        &recovery_path,
                        Some(recovery_identity),
                    )
                    .is_err()
                {
                    self.file.pending_work_recovery_cleanup =
                        Some((recovery_path, recovery_identity));
                }
                self.file.path = Some(path.to_owned());
                self.file.recovery_open = None;
                self.file.identity = Some(saved_identity);
                self.file.work_recovery_lock = None;
                self.file.work_recovery_identity = None;
                self.file.work_recovery_digest = None;
                self.file.saved_digest = self.document.history_digest();
                self.assistant.saved_conversation_digest =
                    assistant_conversation_digest(&self.assistant.messages);
                let digest_key = if truncate_history {
                    "digest-saved-document-current-only"
                } else {
                    "digest-saved-document"
                };
                self.digest = self.catalog.format(
                    digest_key,
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
                if let Some(error) = publication_error {
                    self.report_file_persistence_error("error-save-document", path, &error);
                    return false;
                }
                true
            }
            Err(error) => {
                self.digest = self.catalog.format(
                    "error-save-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                false
            }
        }
    }

    pub(crate) fn confirm_discard_if_dirty(&mut self) -> bool {
        if !self.is_dirty() {
            return true;
        }
        let title = self.catalog.text("dialog-unsaved-title");
        let description = self.catalog.text("dialog-unsaved-description");
        self.dialogs.confirm_discard(DiscardRequest {
            title: &title,
            description: &description,
        })
    }

    pub(crate) fn choose_save_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-ketchup");
        let title = self.document_title();
        let suggested_name = title.trim_end_matches(" *");
        self.dialogs.pick_save_path(SaveRequest {
            filter_label: &filter_label,
            suggested_name,
        })
    }

    pub(crate) fn dispatch_file_command(&mut self, id: AppCommand) {
        match id {
            AppCommand::New if self.confirm_discard_if_dirty() => self.new_document(),
            AppCommand::Open if self.confirm_discard_if_dirty() => {
                if let Some(path) = self.choose_open_path() {
                    self.open_document_with_discard(&path, true);
                }
            }
            AppCommand::Save => {
                if let Some(path) = self.file.path.clone().or_else(|| self.choose_save_path()) {
                    self.save_document_to(&path);
                }
            }
            AppCommand::SaveAs => {
                if let Some(path) = self.choose_save_path() {
                    self.save_document_to(&path);
                }
            }
            AppCommand::ImportMeshStl => {
                if let Some(path) = self.choose_stl_import_path() {
                    match read_import_source(ImportFormat::Stl, &path).and_then(|source| {
                        let source_plan = ImportSourcePlan::seal(
                            ImportFormat::Stl,
                            path.clone(),
                            source,
                            ImportLengthUnit::Millimetre,
                            &self.document.current(),
                        );
                        let plan = self.prepare_stl_import_preview_plan(source_plan)?;
                        Ok(PendingStlImport {
                            plan,
                            review_error: None,
                            invalidated: false,
                        })
                    }) {
                        Ok(pending) => self.modal.open(pending),
                        Err(reason) => {
                            self.modal.close::<PendingStlImport>();
                            self.report_import_failure(&path, &reason);
                        }
                    }
                }
            }
            AppCommand::ImportExactStep => {
                if let Some(path) = self.choose_step_import_path() {
                    let result = Self::read_step_source(&path).and_then(|source| {
                        let source_plan = ImportSourcePlan::seal(
                            ImportFormat::Step,
                            path.clone(),
                            source,
                            (),
                            &self.document.current(),
                        );
                        let plan = self.prepare_step_import_preview_plan(source_plan)?;
                        Ok(PendingStepImport {
                            plan,
                            invalidated: false,
                        })
                    });
                    match result {
                        Ok(pending) => self.modal.open(pending),
                        Err(reason) => {
                            self.modal.close::<PendingStepImport>();
                            self.report_import_failure(&path, &reason);
                        }
                    }
                }
            }
            AppCommand::ImportExactIges => {
                if let Some(path) = self.choose_iges_import_path() {
                    let result = Self::read_iges_source(&path).and_then(|source| {
                        let source_plan = ImportSourcePlan::seal(
                            ImportFormat::Iges,
                            path.clone(),
                            source,
                            (),
                            &self.document.current(),
                        );
                        let plan = self.prepare_iges_import_preview_plan(source_plan)?;
                        Ok(PendingIgesImport {
                            plan,
                            invalidated: false,
                        })
                    });
                    match result {
                        Ok(pending) => self.modal.open(pending),
                        Err(reason) => {
                            self.modal.close::<PendingIgesImport>();
                            self.report_import_failure(&path, &reason);
                        }
                    }
                }
            }
            AppCommand::ImportSketchupScene => {
                if let Some(path) = self.choose_sketchup_scene_import_path() {
                    let result =
                        read_import_source(ImportFormat::SketchupScene, &path).and_then(|source| {
                            let source_plan = ImportSourcePlan::seal(
                                ImportFormat::SketchupScene,
                                path.clone(),
                                source,
                                (),
                                &self.document.current(),
                            );
                            let plan =
                                self.prepare_sketchup_scene_import_preview_plan(source_plan)?;
                            Ok(PendingSketchupSceneImport {
                                plan,
                                invalidated: false,
                            })
                        });
                    match result {
                        Ok(pending) => self.modal.open(pending),
                        Err(reason) => {
                            self.modal.close::<PendingSketchupSceneImport>();
                            self.report_import_failure(&path, &reason);
                        }
                    }
                }
            }
            AppCommand::ImportBlenderGlb => self.begin_glb_import(),
            AppCommand::ImportDrawingDxf => {
                if let Some(path) = self.choose_dxf_import_path() {
                    let result = Self::read_dxf_source(&path).and_then(|source| {
                        let initial_review = Self::inspect_dxf_for_review(&source)?;
                        let unit = initial_review.units().source_unit();
                        let unit_confirmed =
                            initial_review.units().authority() == ImportUnitAuthority::FileDeclared;
                        let source_plan = ImportSourcePlan::seal(
                            ImportFormat::Dxf,
                            path.clone(),
                            source,
                            unit,
                            &self.document.current(),
                        );
                        let plan = self.prepare_dxf_import_preview_plan(source_plan)?;
                        Ok(PendingDxfImport {
                            plan,
                            unit_confirmed,
                            review_error: None,
                            invalidated: false,
                        })
                    });
                    match result {
                        Ok(pending) => self.modal.open(pending),
                        Err(reason) => {
                            self.report_import_failure(&path, &reason);
                        }
                    }
                }
            }
            AppCommand::ExportDrawingDxf => {
                if let Some(path) = self.choose_export_path("dxf") {
                    self.export_current_profiles_dxf_to(&path);
                }
            }
            AppCommand::ExportExactStep => {
                if let Some(path) = self.choose_export_path("step") {
                    self.export_current_model_step_to(&path);
                }
            }
            AppCommand::ExportExactIges => {
                if let Some(path) = self.choose_export_path("iges") {
                    self.export_current_model_iges_to(&path);
                }
            }
            AppCommand::ExportMeshStl => {
                if let Some(path) = self.choose_export_path("stl") {
                    self.export_current_model_stl_to(&path);
                }
            }
            AppCommand::ExportPrintThreeMf => {
                if let Some(path) = self.choose_export_path("3mf") {
                    self.export_current_model_three_mf_to(&path);
                }
            }
            AppCommand::ExportBlenderGlb => {
                if let Some(path) = self.choose_export_path("glb") {
                    self.export_current_model_glb_to(&path);
                }
            }
            AppCommand::ExportGeneralFabrication => {
                if let Some(path) = self.choose_export_path("csv") {
                    self.export_current_general_fabrication_to(&path);
                }
            }
            AppCommand::ExportWeldmentCutList => {
                if let Some(path) = self.choose_export_path("csv") {
                    self.export_current_weldment_cut_list_to(&path);
                }
            }
            AppCommand::ExportSheetMetalManufacturing => {
                match self.sole_exportable_sheet_metal_feature_id() {
                    Ok(feature_id) => {
                        if let Some(path) = self.choose_export_path("dxf") {
                            self.export_current_sheet_metal_manufacturing_to(feature_id, &path);
                        }
                    }
                    Err(error) => {
                        self.digest = self.catalog.format(
                            "error-export-general-fabrication",
                            &BTreeMap::from([
                                ("path", "sheet-metal flat pattern".to_owned()),
                                ("reason", error.to_string()),
                            ]),
                        );
                    }
                }
            }
            AppCommand::ReviewCamExport => {
                self.reviews.cam_export_dialog =
                    self.document
                        .current()
                        .cam_plans()
                        .next()
                        .map(|plan| CamExportDialog {
                            plan_id: plan.id(),
                            review: None,
                        });
                if self.reviews.cam_export_dialog.is_none() {
                    self.digest = self.catalog.text("cam-review-no-plan");
                }
            }
            AppCommand::ReviewStaticFea => {
                let result = (|| {
                    let selected = self
                        .selected_root_occurrence_ids()
                        .map_err(|error| self.root_occurrence_selection_error(&error))?;
                    if selected.len() != 1 {
                        return Err(self.catalog.refusal("fea-review-select-one"));
                    }
                    let occurrence_id = *selected.iter().next().expect("one selected occurrence");
                    let snapshot = self.document.current();
                    let occurrence = snapshot
                        .occurrence(occurrence_id)
                        .ok_or_else(|| self.catalog.refusal("fea-review-select-one"))?;
                    let terminals =
                        exact_body_terminal_features(&snapshot, occurrence.definition_id())
                            .map_err(|error| failed("fea_review.terminal_features", error))?;
                    let terminal_features = terminals.values().copied().collect::<Vec<_>>();
                    let [feature_id] = terminal_features.as_slice() else {
                        return Err(self.catalog.refusal("fea-review-one-body"));
                    };
                    Ok(FeaReviewDialog {
                        definition_id: occurrence.definition_id(),
                        feature_id: *feature_id,
                        occurrence_id,
                        case_id: "linear-static-case".to_owned(),
                        youngs_modulus_mpa: "200000".to_owned(),
                        poisson_ratio: "0.3".to_owned(),
                        yield_strength_mpa: "250".to_owned(),
                        constrained_face_ordinals: "0".to_owned(),
                        loaded_face_ordinal: "1".to_owned(),
                        traction_x_n_per_mm2: "0".to_owned(),
                        traction_y_n_per_mm2: "0".to_owned(),
                        traction_z_n_per_mm2: "-1".to_owned(),
                        coarse_deflection_mm: "0.3".to_owned(),
                        fine_deflection_mm: "0.12".to_owned(),
                        review: None,
                    })
                })();
                match result {
                    Ok(dialog) => self.reviews.fea_review_dialog = Some(dialog),
                    Err(error) => self.digest = error.reason_text().to_owned(),
                }
            }
            AppCommand::ReviewLocalPdm => {
                let snapshot = self.document.current();
                let repository = self
                    .file
                    .path
                    .as_deref()
                    .and_then(Path::parent)
                    .map_or_else(
                        || PathBuf::from(".ketchup-pdm"),
                        |parent| parent.join(".ketchup-pdm"),
                    );
                self.reviews.pdm_review_dialog = Some(PdmReviewDialog {
                    source: PdmSourceIdentity::observed(&snapshot, self.document.mutation_epoch()),
                    repository: repository.display().to_string(),
                    parent_release_id: String::new(),
                    release_id: String::new(),
                    compare_release_id: String::new(),
                    dependencies: String::new(),
                    actor: "local-user".to_owned(),
                    note: String::new(),
                    catalog: Vec::new(),
                    opened: None,
                    comparison: None,
                });
            }
            AppCommand::ExportHomagMpr => {
                if let Some(path) = self.choose_export_path("mpr") {
                    self.export_current_model_homag_mpr_to(&path);
                }
            }
            AppCommand::ExportHundeggerBtlx => {
                if let Some(path) = self.choose_export_path("btlx") {
                    self.export_current_model_btlx_to(&path);
                }
            }
            AppCommand::New | AppCommand::Open => {}
            _ => unreachable!("only file commands are routed here"),
        }
    }

    #[must_use]
    pub fn document_revision(&self) -> u64 {
        self.document.current().revision_id()
    }

    #[must_use]
    pub fn document_snapshot(&self) -> Snapshot {
        self.document.current()
    }

    #[must_use]
    pub fn revision_catalog(&self) -> Vec<ketchup_model::document::RevisionCatalogEntry> {
        self.document.revision_catalog()
    }

    /// Canonical identity of the active document: schema, units, IDs,
    /// hierarchy, parameters, transforms, and sharing folded into one value.
    #[must_use]
    pub fn canonical_digest(&self) -> String {
        self.document.current().canonical_digest()
    }

    #[must_use]
    pub fn last_side_effect_receipt(&self) -> Option<&SideEffectAuthorizationReceipt> {
        self.side_effect_receipts.last()
    }

    #[must_use]
    pub fn side_effect_receipt_count(&self) -> usize {
        self.side_effect_receipts.len()
    }

    pub fn is_dirty(&self) -> bool {
        self.file.recovery_open.is_some()
            || self.document.history_digest() != self.file.saved_digest
            || assistant_conversation_digest(&self.assistant.messages)
                != self.assistant.saved_conversation_digest
    }

    /// Opens a native Kečup document from a caller-provided path.
    pub fn open_document_path(&mut self, path: &Path) -> bool {
        self.open_document_from(path)
    }

    /// Path the active document is bound to, if it has been saved or opened.
    #[must_use]
    pub fn document_path(&self) -> Option<&Path> {
        self.file.path.as_deref()
    }

    /// Corrupt-primary path whose backup is active until an explicit Save As succeeds.
    #[must_use]
    pub fn recovery_requested_path(&self) -> Option<&Path> {
        self.file
            .recovery_open
            .as_ref()
            .map(|recovery| recovery.requested_path.as_path())
    }

    /// Actual backup source supplying the active recovered document.
    #[must_use]
    pub fn recovery_source_path(&self) -> Option<&Path> {
        self.file
            .recovery_open
            .as_ref()
            .map(|recovery| recovery.source_path.as_path())
    }

    /// The action digest the shell is currently reporting to the user.
    #[must_use]
    pub fn action_digest(&self) -> &str {
        &self.digest
    }

    /// The localization catalog the shell paints with.
    ///
    /// Acceptance tests resolve expected labels through this catalog instead of
    /// hard-coding English, so a translation change cannot break them.
    #[must_use]
    pub fn catalog(&self) -> &LocaleCatalog {
        &self.catalog
    }

    #[must_use]
    pub fn document_height_mm(&self) -> f64 {
        self.box_height_mm(INITIAL_BOX_DEFINITION)
            .expect("the initial box definition exists")
    }

    #[must_use]
    pub fn can_undo(&self) -> bool {
        self.document.visible_undo_steps() > 0
    }

    #[must_use]
    pub fn can_redo(&self) -> bool {
        self.document.visible_redo_steps() > 0
    }

    pub(crate) fn mutate_document_with_work_recovery<T, E>(
        &mut self,
        mutate: impl FnOnce(&mut DocumentStore) -> Result<T, E>,
    ) -> Result<
        (T, Option<ketchup_model::persistence::FilePersistenceError>),
        WorkRecoveryMutationError<E>,
    > {
        if let Err(error) = self.acquire_work_recovery_lock() {
            let path = self.file.path.clone().unwrap_or_default();
            self.report_file_persistence_error("error-save-document", &path, &error);
            return Err(WorkRecoveryMutationError::Recovery(error));
        }
        self.store_assistant_conversation();
        self.store_assistant_memory();
        let document_path = self.file.path.clone();
        let file_identity = self.file.identity;
        let recovery_open = self.file.recovery_open.is_some();
        let saved_digest = self.file.saved_digest.clone();
        let conversation_digest = assistant_conversation_digest(&self.assistant.messages);
        let saved_conversation_digest = self.assistant.saved_conversation_digest.clone();
        let container_data = &self.file.container_data;
        let work_recovery_identity = self.file.work_recovery_identity;
        let mut next_work_recovery_identity = work_recovery_identity;
        let mut publication_error = None;
        let result = self.document.try_canonical_transaction(
            |document| mutate(document).map_err(WorkRecoveryMutationError::Mutation),
            |document| {
                let dirty = recovery_open
                    || document.history_digest() != saved_digest
                    || conversation_digest != saved_conversation_digest;
                if !dirty {
                    if let Some(path) = document_path.as_deref() {
                        ketchup_model::persistence::clear_work_recovery(
                            path,
                            work_recovery_identity,
                        )
                        .map_err(WorkRecoveryMutationError::Recovery)?;
                    }
                    next_work_recovery_identity = None;
                    return Ok(());
                }
                let (Some(path), Some(identity)) = (document_path.as_deref(), file_identity) else {
                    return Ok(());
                };
                match ketchup_model::persistence::save_work_recovery_document_store_with_container_if_unchanged(
                    path, document, container_data, identity, work_recovery_identity,
                ) {
                    Ok(identity) => next_work_recovery_identity = Some(identity),
                    Err(error) => {
                        if let Some(identity) = error.published_identity(&ketchup_model::persistence::work_recovery_path(path)) {
                            next_work_recovery_identity = Some(identity);
                            publication_error = Some(error);
                        } else {
                            return Err(WorkRecoveryMutationError::Recovery(error));
                        }
                    }
                }
                Ok(())
            },
        );
        if (result.is_ok() && next_work_recovery_identity.is_none())
            || (result.is_err() && work_recovery_identity.is_none())
        {
            self.file.work_recovery_lock = None;
        }
        match result {
            Ok(value) => {
                self.file.work_recovery_identity = next_work_recovery_identity;
                self.file.work_recovery_digest = (self.is_dirty()
                    && document_path.is_some()
                    && file_identity.is_some())
                .then(|| format!("{}:{conversation_digest}", self.document.history_digest()));
                if let Some(error) = &publication_error {
                    let path = document_path.as_deref().unwrap_or_else(|| Path::new(""));
                    self.report_file_persistence_error("error-save-document", path, error);
                }
                Ok((value, publication_error))
            }
            Err(error @ WorkRecoveryMutationError::Mutation(_)) => Err(error),
            Err(WorkRecoveryMutationError::Recovery(error)) => {
                let path = document_path
                    .as_deref()
                    .map(ketchup_model::persistence::work_recovery_path)
                    .unwrap_or_default();
                self.digest = self.catalog.format(
                    "error-save-document",
                    &BTreeMap::from([
                        ("path", path.display().to_string()),
                        ("reason", error.to_string()),
                    ]),
                );
                Err(WorkRecoveryMutationError::Recovery(error))
            }
        }
    }

    pub(crate) fn complete_mutation_with_work_recovery<T, E>(
        &mut self,
        mutate: impl FnOnce(&mut DocumentStore) -> Result<T, E>,
    ) -> Result<T, WorkRecoveryMutationError<E>> {
        self.complete_mutation_with_publication(
            |document| mutate(document).map(|value| (value, ())),
            |_, ()| {},
        )
    }

    pub(crate) fn apply_batch_with_work_recovery(
        &mut self,
        batch: &CommandBatch,
    ) -> Result<(), WorkRecoveryMutationError<CanonicalError>> {
        self.complete_mutation_with_work_recovery(|document| {
            document.apply_batch(batch).map(|_| ())
        })
    }

    pub(crate) fn commit_proposal_with_work_recovery(
        &mut self,
        proposal: &Proposal,
    ) -> Result<(), WorkRecoveryMutationError<ProposalCommitError>> {
        self.complete_mutation_with_work_recovery(|document| {
            document.commit_proposal(proposal).map(|_| ())
        })
    }

    pub(crate) fn commit_verified_proposal_with_work_recovery(
        &mut self,
        proposal: &Proposal,
    ) -> Result<
        ketchup_model::document::VerifiedProposalCommit,
        WorkRecoveryMutationError<ProposalCommitError>,
    > {
        self.complete_mutation_with_work_recovery(|document| {
            document.commit_verified_proposal(proposal)
        })
    }

    pub(crate) fn commit_verified_proposal_with_container_work_recovery(
        &mut self,
        proposal: &Proposal,
        staged_container: ContainerData,
    ) -> Result<
        ketchup_model::document::VerifiedProposalCommit,
        WorkRecoveryMutationError<ProposalCommitError>,
    > {
        let previous_container = std::mem::replace(&mut self.file.container_data, staged_container);
        let result = self.commit_verified_proposal_with_work_recovery(proposal);
        if result.as_ref().is_err_and(|error| {
            !matches!(
                error,
                WorkRecoveryMutationError::Recovery(
                    ketchup_model::persistence::FilePersistenceError::Published { .. }
                )
            )
        }) {
            self.file.container_data = previous_container;
        }
        result
    }

    pub(crate) fn mutate_history_with_work_recovery(
        &mut self,
        mutate: impl FnOnce(&mut DocumentStore) -> Option<Snapshot>,
    ) -> bool {
        self.complete_mutation_with_work_recovery(|document| {
            Ok::<_, std::convert::Infallible>(mutate(document))
        })
        .is_ok_and(|snapshot| snapshot.is_some())
    }

    pub fn undo(&mut self) -> bool {
        if self.cancel_ephemeral_edit_for_history() {
            return false;
        }
        let undoing_assistant_change = self.assistant_change_can_undo();
        if !self.mutate_history_with_work_recovery(DocumentStore::undo) {
            return false;
        }
        self.invalidate_pending_import_reviews();
        if undoing_assistant_change {
            self.assistant.verification = None;
        }
        self.clear_ephemeral_edit_state();
        self.end_transform_correction();
        self.cancel_rectangle_sketch();
        self.parameter.editor_node = None;
        self.parameter.provenance = None;
        self.parameter.last_recomputed_nodes.clear();
        self.reconcile_selection();
        self.status_key = "status-undo";
        true
    }

    pub fn redo(&mut self) -> bool {
        if self.cancel_ephemeral_edit_for_history() {
            return false;
        }
        if !self.mutate_history_with_work_recovery(DocumentStore::redo) {
            return false;
        }
        self.invalidate_pending_import_reviews();
        self.clear_ephemeral_edit_state();
        self.end_transform_correction();
        self.cancel_rectangle_sketch();
        self.parameter.editor_node = None;
        self.parameter.provenance = None;
        self.parameter.last_recomputed_nodes.clear();
        self.reconcile_selection();
        self.status_key = "status-redo";
        true
    }

    pub(crate) fn show_pdm_review_window(&mut self, context: &egui::Context) {
        let Some(mut pending) = self.reviews.pdm_review_dialog.take() else {
            return;
        };
        let mut open = true;
        let mut refresh = false;
        let mut open_release = false;
        let mut compare = false;
        let mut create = false;
        let mut cancel = false;
        egui::Window::new(self.catalog.text("pdm-review-title"))
            .id(egui::Id::new("local-pdm-review"))
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .show(context, |ui| {
                ui.label(format!(
                    "Revision {} · Epoch {} · SHA-256 {}",
                    pending.source.revision,
                    pending.source.mutation_epoch,
                    pending.source.canonical_digest
                ));
                for (label, value) in [
                    ("pdm-repository", &mut pending.repository),
                    ("pdm-parent-release", &mut pending.parent_release_id),
                    ("pdm-release-id", &mut pending.release_id),
                    ("pdm-compare-release", &mut pending.compare_release_id),
                    ("pdm-audit-actor", &mut pending.actor),
                    ("pdm-audit-note", &mut pending.note),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(self.catalog.text(label));
                        ui.text_edit_singleline(value);
                    });
                }
                ui.label(self.catalog.text("pdm-dependencies"));
                ui.text_edit_multiline(&mut pending.dependencies);
                ui.label(self.catalog.text("pdm-dependencies-help"));
                if !pending.catalog.is_empty() {
                    ui.separator();
                    ui.label(self.catalog.format(
                        "pdm-catalog-count",
                        &BTreeMap::from([("count", pending.catalog.len().to_string())]),
                    ));
                    for entry in pending.catalog.iter().rev().take(4) {
                        ui.monospace(format!(
                            "{} · r{} · {} · {} dependencies",
                            entry.release_id,
                            entry.revision,
                            entry.audit.actor,
                            entry.dependency_count
                        ));
                    }
                }
                if let Some(manifest) = &pending.opened {
                    ui.separator();
                    ui.label(self.catalog.text("pdm-open-verified"));
                    ui.monospace(format!(
                        "{} · r{} · SHA-256 {}",
                        manifest.release_id,
                        manifest.document.revision,
                        manifest.document.canonical_digest
                    ));
                }
                if let Some(comparison) = &pending.comparison {
                    ui.separator();
                    let verdict = match comparison.conflict_verdict {
                        ReleaseConflictVerdict::AlreadyCurrent => "already current",
                        ReleaseConflictVerdict::FastForward => "fast forward",
                        ReleaseConflictVerdict::IncomingBehind => "incoming behind",
                        ReleaseConflictVerdict::DivergedConflict => "diverged conflict",
                        ReleaseConflictVerdict::UnrelatedConflict => "unrelated conflict",
                    };
                    ui.label(format!(
                        "{}: {verdict} · document changed: {} · dependency changes: {}",
                        self.catalog.text("pdm-comparison"),
                        comparison.document_changed,
                        comparison.dependency_changes.len()
                    ));
                }
                ui.horizontal(|ui| {
                    if ui
                        .button(self.catalog.text("pdm-catalog-refresh"))
                        .clicked()
                    {
                        refresh = true;
                    }
                    if ui.button(self.catalog.text("pdm-open-release")).clicked() {
                        open_release = true;
                    }
                    if ui
                        .button(self.catalog.text("pdm-compare-releases"))
                        .clicked()
                    {
                        compare = true;
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(self.catalog.text("pdm-confirm-create")).clicked() {
                        create = true;
                    }
                    if ui.button(self.catalog.text("dialog-cancel")).clicked() {
                        cancel = true;
                    }
                });
            });

        let cancelled = AtomicBool::new(false);
        let current =
            PdmDocumentState::observed(self.document.current(), self.document.mutation_epoch());
        if refresh {
            match self.reviews.pdm.catalog(
                &current,
                &pending.source,
                &pending.repository,
                &cancelled,
            ) {
                Ok(catalog) => {
                    if let Some(latest) = catalog
                        .iter()
                        .rev()
                        .find(|entry| entry.document_id == self.document.current().document_id().0)
                    {
                        if pending.parent_release_id.is_empty() {
                            pending.parent_release_id.clone_from(&latest.release_id);
                        }
                        if pending.release_id.is_empty() {
                            pending.release_id.clone_from(&latest.release_id);
                        }
                    }
                    pending.catalog = catalog;
                    self.digest = self.catalog.text("pdm-catalog-ready");
                }
                Err(error) => self.digest = error.to_string(),
            }
        }
        if open_release {
            match self.reviews.pdm.open(
                &current,
                &pending.source,
                &pending.repository,
                pending.release_id.trim(),
                &cancelled,
            ) {
                Ok(release) => {
                    pending.opened = Some(release.manifest);
                    self.digest = self.catalog.text("pdm-open-verified");
                }
                Err(error) => self.digest = error.to_string(),
            }
        }
        if compare {
            match self.reviews.pdm.compare(
                &current,
                &pending.source,
                &pending.repository,
                pending.release_id.trim(),
                pending.compare_release_id.trim(),
                &cancelled,
            ) {
                Ok(comparison) => {
                    pending.comparison = Some(comparison);
                    self.digest = self.catalog.text("pdm-comparison-ready");
                }
                Err(error) => self.digest = error.to_string(),
            }
        }
        if create {
            let result = (|| {
                let dependencies = pending
                    .dependencies
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(|line| {
                        line.split_once('=')
                            .map(|(logical, source)| (logical.trim(), source.trim()))
                            .filter(|(logical, source)| !logical.is_empty() && !source.is_empty())
                            .map(|(logical, source)| ReleaseDependencyInput::new(logical, source))
                            .ok_or_else(|| {
                                Rejection::new("pdm.dependency_line", RejectionPhase::Validation)
                                    .target(line.to_owned())
                                    .reason(format!(
                                        "PDM dependency line {line:?} must use logical/path=source/path"
                                    ))
                            })
                    })
                    .collect::<Result<Vec<_>, Rejection>>()?;
                let request = PdmCreateReleaseRequest {
                    repository: PathBuf::from(pending.repository.trim()),
                    parent_release_id: (!pending.parent_release_id.trim().is_empty())
                        .then(|| pending.parent_release_id.trim().to_owned()),
                    dependencies,
                    audit: ReleaseAudit::new(
                        pending.actor.trim(),
                        current_unix_time_ms()?,
                        pending.note.trim(),
                    ),
                };
                self.reviews
                    .pdm
                    .create(
                        &current,
                        &self.file.container_data,
                        &pending.source,
                        &request,
                        true,
                        &cancelled,
                    )
                    .map_err(|error| failed("pdm.create", error))
            })();
            match result {
                Ok(manifest) => {
                    pending.release_id.clone_from(&manifest.release_id);
                    pending.parent_release_id.clone_from(&manifest.release_id);
                    pending.opened = Some(manifest);
                    self.digest = self.catalog.text("pdm-release-created");
                }
                Err(error) => self.digest = error.reason_text().to_owned(),
            }
        }
        if open && !cancel {
            self.reviews.pdm_review_dialog = Some(pending);
        }
    }
}

fn current_unix_time_ms() -> Result<u64, Rejection> {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| failed("clock", error))?
        .as_millis();
    u64::try_from(millis).map_err(|error| failed("clock", error))
}
