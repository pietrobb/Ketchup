//! Background orchestration only; all validation rules live in ketchup-application.

use super::*;

#[derive(Clone)]
pub(super) struct ValidatorSnapshot {
    document_id: DocumentId,
    revision: u64,
    canonical_digest: String,
}

impl ValidatorSnapshot {
    pub(super) fn new(snapshot: &Snapshot) -> Self {
        Self {
            document_id: snapshot.document_id(),
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
        }
    }

    pub(super) fn matches(&self, snapshot: &Snapshot) -> bool {
        self.document_id == snapshot.document_id()
            && self.revision == snapshot.revision_id()
            && self.canonical_digest == snapshot.canonical_digest()
    }
}

struct ValidatorPanelTask {
    source: ValidatorSnapshot,
    receiver: Receiver<ValidatorPanelReport>,
    stale: bool,
}

#[derive(Default)]
pub(super) struct ValidatorPanelState {
    task: Option<ValidatorPanelTask>,
    pub(super) report_source: Option<ValidatorSnapshot>,
    pub(super) notice: Option<&'static str>,
}

impl KetchupApp {
    /// Whether a manual validation is still running, including a stale run that
    /// must finish before another expensive worker can be started.
    #[must_use]
    pub fn validator_panel_pending(&self) -> bool {
        self.validator_panel_state.task.is_some()
    }

    /// Starts read-only validation in the background. Returns false for an empty
    /// selection, a duplicate request, or failure to start the background thread.
    pub fn start_validator_panel(&mut self, context: &egui::Context) -> bool {
        if self.validator_panel_pending() || self.validator_panel_selection.is_empty() {
            return false;
        }
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let source = ValidatorSnapshot::new(&snapshot);
        let exact_results = self.exact_results.clone();
        let container = self.container_data.clone();
        let executable = self.validator_worker_path();
        let selection = ketchup_application::validation::AssistantValidationSelection {
            mode: "only",
            requested: self.validator_panel_selection.clone(),
            unknown: Vec::new(),
        };
        let (sender, receiver) = mpsc::channel();
        let repaint = context.clone();
        let started = std::thread::Builder::new()
            .name("manual-validator".to_owned())
            .spawn(move || {
                let validation =
                    ketchup_application::validation::assistant_validation_context_with_worker(
                        &snapshot,
                        &exact_results,
                        &selection,
                        &container,
                        executable,
                        Duration::from_secs(30),
                    );
                let _ = sender.send(validator_panel_report(&validation));
                repaint.request_repaint();
            });
        self.validator_panel_report = None;
        self.validator_panel_state.report_source = None;
        if started.is_err() {
            self.validator_panel_state.notice = Some("validators-run-unavailable");
            return false;
        }
        self.validator_panel_state.notice = None;
        self.validator_panel_state.task = Some(ValidatorPanelTask {
            source,
            receiver,
            stale: false,
        });
        context.request_repaint();
        true
    }

    pub(super) fn validator_worker_path(&self) -> Option<PathBuf> {
        self.exact_worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file())
        })
    }

    pub(super) fn poll_validator_panel(&mut self, context: &egui::Context) {
        let snapshot = self.document.current();
        if self
            .validator_panel_state
            .report_source
            .as_ref()
            .is_some_and(|source| !source.matches(&snapshot))
        {
            self.validator_panel_report = None;
            self.validator_panel_state.report_source = None;
            self.validator_panel_state.notice = Some("validators-stale");
        }
        let Some(task) = self.validator_panel_state.task.as_mut() else {
            return;
        };
        task.stale |= !task.source.matches(&snapshot);
        if task.stale {
            self.validator_panel_state.notice = Some("validators-stale");
        }
        let received = task.receiver.try_recv();
        match received {
            Ok(report) => {
                let task = self
                    .validator_panel_state
                    .task
                    .take()
                    .expect("pending validation");
                if task.stale
                    || report.revision != task.source.revision
                    || report.canonical_digest != task.source.canonical_digest
                {
                    self.validator_panel_state.notice = Some("validators-stale");
                } else {
                    self.validator_panel_state.report_source = Some(task.source);
                    self.validator_panel_state.notice = None;
                    self.validator_panel_report = Some(report);
                }
            }
            Err(TryRecvError::Empty) => {
                // Also polls a disconnected/panicked thread, even with the panel closed.
                context.request_repaint_after(Duration::from_millis(50));
            }
            Err(TryRecvError::Disconnected) => {
                self.validator_panel_state.task = None;
                self.validator_panel_state.notice = Some("validators-run-unavailable");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};

    fn pending(app: &mut KetchupApp) -> (mpsc::Sender<ValidatorPanelReport>, ValidatorPanelReport) {
        let snapshot = app.document.current();
        let report = validator_panel_report(&serde_json::json!({
            "revision": snapshot.revision_id(),
            "canonical_digest": snapshot.canonical_digest(),
            "state": "not_evaluated", "complete": false,
            "executed": ["collision"], "issue_count": 0,
            "not_evaluated": [{"validator": "collision", "reason": "worker_unavailable"}]
        }));
        let (sender, receiver) = mpsc::channel();
        app.validator_panel_state.task = Some(ValidatorPanelTask {
            source: ValidatorSnapshot::new(&snapshot),
            receiver,
            stale: false,
        });
        (sender, report)
    }

    #[test]
    fn pending_indicator_disables_duplicate_runs_without_desktop_input() {
        let mut app = KetchupApp::new();
        let (sender, report) = pending(&mut app);
        let context = egui::Context::default();
        assert!(!app.start_validator_panel(&context));
        app.run_validator_panel();
        assert!(app.validator_panel_report().is_none());
        let run = app.catalog.text("validators-run");
        let pending_label = app.catalog.text("validators-pending");
        let mut harness = Harness::new_state(
            |context, app: &mut KetchupApp| {
                app.poll_validator_panel(context);
                egui::CentralPanel::default()
                    .show(context, |ui| app.show_validator_panel_content(ui));
            },
            app,
        );
        harness.step();
        harness.step();
        assert!(harness.query_by_label(&pending_label).is_some());
        assert!(harness.get_by_label(&run).accesskit_node().is_disabled());
        sender.send(report).unwrap();
        harness.step();
        harness.step();
        assert!(!harness.state().validator_panel_pending());
        let report = harness.state().validator_panel_report().unwrap();
        assert!(!report.complete);
        assert!(report.findings.is_empty());
        assert_eq!(report.not_evaluated[0].1, "worker_unavailable");
        assert!(
            harness
                .query_by_label(
                    &harness
                        .state()
                        .catalog
                        .text("validators-no-confirmed-findings")
                )
                .is_some()
        );
    }

    #[test]
    fn stale_snapshot_revision_digest_and_document_are_rejected() {
        for field in ["document", "revision", "digest"] {
            let mut app = KetchupApp::new();
            let (sender, report) = pending(&mut app);
            let source = &mut app.validator_panel_state.task.as_mut().unwrap().source;
            match field {
                "document" => source.document_id = DocumentId(source.document_id.0 + 1),
                "revision" => source.revision += 1,
                _ => source.canonical_digest.push_str("stale"),
            }
            app.poll_validator_panel(&egui::Context::default());
            assert!(app.validator_panel_pending());
            assert!(!app.start_validator_panel(&egui::Context::default()));
            sender.send(report).unwrap();
            app.poll_validator_panel(&egui::Context::default());
            assert!(!app.validator_panel_pending());
            assert!(app.validator_panel_report().is_none());
            assert_eq!(app.validator_panel_state.notice, Some("validators-stale"));
        }
    }

    #[test]
    fn disconnected_validation_clears_pending_without_confirming_collisions() {
        let mut app = KetchupApp::new();
        let (sender, _) = pending(&mut app);
        drop(sender);
        app.poll_validator_panel(&egui::Context::default());
        assert!(!app.validator_panel_pending());
        assert!(app.validator_panel_report().is_none());
        assert_eq!(
            app.validator_panel_state.notice,
            Some("validators-run-unavailable")
        );
    }

    #[test]
    fn completed_report_is_hidden_when_document_changes() {
        let mut app = KetchupApp::new();
        let (sender, report) = pending(&mut app);
        sender.send(report).unwrap();
        app.poll_validator_panel(&egui::Context::default());
        assert!(app.validator_panel_report().is_some());
        app.validator_panel_state
            .report_source
            .as_mut()
            .unwrap()
            .revision += 1;
        assert!(app.validator_panel_report().is_none());
        app.poll_validator_panel(&egui::Context::default());
        assert!(app.validator_panel_report.is_none());
        assert_eq!(app.validator_panel_state.notice, Some("validators-stale"));
    }
}
