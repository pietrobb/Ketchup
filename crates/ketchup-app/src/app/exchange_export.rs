//! STEP and IGES export of the visible exact model. The exact worker writes the
//! file and reads it back on a thread, so the window keeps painting and the file
//! activity indication can cancel it (review 2026-10-17 EX-6); the window then
//! asks for consent and publishes the file with its loss report.

use crate::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ExchangeFormat {
    Step,
    Iges,
}

impl ExchangeFormat {
    pub(crate) fn key(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Iges => "iges",
        }
    }

    pub(crate) fn command(self) -> AppCommand {
        match self {
            Self::Step => AppCommand::ExportExactStep,
            Self::Iges => AppCommand::ExportExactIges,
        }
    }
}

/// What the worker needs, taken from the window before the export starts.
struct ExchangeExportJob {
    format: ExchangeFormat,
    path: PathBuf,
    snapshot: Snapshot,
    model: Arc<Vec<(ExactBodyPackage, SceneOccurrence)>>,
    executable: PathBuf,
    blobs: ketchup_model::persistence::Blobs,
}

impl ExchangeExportJob {
    /// The checked file bytes, prepared in [`prepared_directory`].
    fn run(&self, cancelled: &AtomicBool) -> Result<Vec<u8>, ExportError> {
        let prepared_directory = prepared_directory().map_err(ExportError::failed)?;
        let prepared_step = prepared_directory.path().join("model.step");
        let mut worker =
            ExactWorkerSupervisor::spawn_with_cancellation(&self.executable, cancelled)
                .map_err(ExportError::failed)?;
        let verified_step = worker
            .export_current_model_step_scene_with_imported_sources(
                &self.snapshot,
                &self.model,
                &prepared_step,
                &self.blobs,
                cancelled,
            )
            .map_err(ExportError::failed)?;
        match self.format {
            ExchangeFormat::Step => canonical_step_header(verified_step),
            ExchangeFormat::Iges => {
                let prepared_iges = prepared_directory.path().join("model.iges");
                let iges = worker
                    .convert_step_to_iges_with_cancellation(
                        &prepared_step,
                        &prepared_iges,
                        self.model.len(),
                        cancelled,
                    )
                    .map_err(ExportError::failed)?;
                let iges_sha256 = ketchup_model::graph::sha256_hex(&iges);
                let exported_evidence = worker
                    .inspect_iges_xde_import_with_cancellation(
                        &prepared_iges,
                        &iges_sha256,
                        cancelled,
                    )
                    .map_err(ExportError::failed)?;
                if exported_evidence.parts.len() != self.model.len()
                    || exported_evidence
                        .parts
                        .iter()
                        .any(|part| part.exact.source_unit != ImportLengthUnit::Millimetre)
                {
                    return Err(ExportError::ArtifactUnverified {
                        check: "every exported root keeps millimetre units",
                    });
                }
                Ok(iges)
            }
        }
    }
}

/// Where an export is prepared: the system temporary folder, as on Windows a
/// cancelled worker may still hold its files, and what it leaves must not land
/// next to the user's file (review 2026-10-18 EX-12).
fn prepared_directory() -> std::io::Result<tempfile::TempDir> {
    tempfile::Builder::new()
        .prefix("ketchup-prepared-export-")
        .tempdir()
}

/// Replaces the writing time in the STEP header, so the same model gives the same file.
fn canonical_step_header(verified_step: Vec<u8>) -> Result<Vec<u8>, ExportError> {
    let mut step = String::from_utf8(verified_step).map_err(ExportError::failed)?;
    let timestamp_marker = "FILE_NAME('Open CASCADE Shape Model','";
    let timestamp_start = step
        .find(timestamp_marker)
        .map(|index| index + timestamp_marker.len())
        .ok_or(ExportError::ArtifactUnverified {
            check: "the canonical FILE_NAME header is present",
        })?;
    let timestamp_end = step[timestamp_start..]
        .find("',(")
        .map(|index| timestamp_start + index)
        .ok_or(ExportError::ArtifactUnverified {
            check: "the FILE_NAME header is well formed",
        })?;
    step.replace_range(timestamp_start..timestamp_end, "1970-01-01T00:00:00");
    Ok(step.into_bytes())
}

/// What the thread of a client's export sends back: the job and its file.
pub(crate) struct ClientExchangeExport {
    job: ExchangeExportJob,
    bytes: Result<Vec<u8>, ExportError>,
}

/// A STEP or IGES export whose worker runs on its own thread.
pub(crate) struct ExchangeExportTask {
    format: ExchangeFormat,
    path: PathBuf,
    snapshot: Snapshot,
    model: Arc<Vec<(ExactBodyPackage, SceneOccurrence)>>,
    cancelled: Arc<AtomicBool>,
    receiver: Receiver<Result<Vec<u8>, ExportError>>,
}

impl ExchangeExportTask {
    pub(crate) fn format(&self) -> ExchangeFormat {
        self.format
    }
}

impl KetchupApp {
    fn exchange_export_job(
        &mut self,
        format: ExchangeFormat,
        path: &Path,
    ) -> Result<ExchangeExportJob, ExportError> {
        match format {
            ExchangeFormat::Step => {
                Self::validate_exact_exchange_extension(path, "STEP export", &["step", "stp"])?
            }
            ExchangeFormat::Iges => {
                Self::validate_exact_exchange_extension(path, "IGES export", &["iges", "igs"])?
            }
        }
        let snapshot = self.document.current();
        let model = self.current_visible_exact_scene(&snapshot)?;
        let executable = self
            .exact
            .worker_path
            .clone()
            .or_else(|| {
                exact_worker_candidates()
                    .into_iter()
                    .find(|candidate| candidate.is_file())
            })
            .ok_or_else(super::exact::exact_worker_unavailable)?;
        Ok(ExchangeExportJob {
            format,
            path: path.to_owned(),
            snapshot,
            model: Arc::new(model),
            executable,
            blobs: self.file.container_data.blobs().clone(),
        })
    }

    /// Exports and waits; the live bridge and tests take this path.
    pub(crate) fn export_exchange_to(
        &mut self,
        format: ExchangeFormat,
        path: &Path,
    ) -> Result<Written, ExportError> {
        self.export_exchange_with(format, path, ExchangeExportJob::run)
    }

    /// A client that stops waiting for its export stops the worker, and
    /// nothing is written for it afterwards (review 2026-10-21 MCP-6).
    fn export_exchange_with(
        &mut self,
        format: ExchangeFormat,
        path: &Path,
        run: impl FnOnce(&ExchangeExportJob, &AtomicBool) -> Result<Vec<u8>, ExportError>,
    ) -> Result<Written, ExportError> {
        self.side_effect_receipts.clear();
        let cancelled = self.live.client_export.clone().unwrap_or_default();
        let result = self.exchange_export_job(format, path).and_then(|job| {
            let bytes = run(&job, &cancelled);
            self.publish_unless_client_gone(&job, bytes, &cancelled)
        });
        self.report_export_outcome(format.key(), path, result)
    }

    fn publish_unless_client_gone(
        &mut self,
        job: &ExchangeExportJob,
        bytes: Result<Vec<u8>, ExportError>,
        cancelled: &AtomicBool,
    ) -> Result<Written, ExportError> {
        let bytes = bytes?;
        if cancelled.load(Ordering::Acquire) {
            return Err(
                Rejection::new("side_effect.client_gone", RejectionPhase::Validation)
                    .reason(
                        "the client that asked for this export disconnected before it was written",
                    )
                    .into(),
            );
        }
        self.publish_exchange_export(job.format, &job.path, &job.snapshot, &job.model, bytes)
    }

    /// Starts the export a client asked for on a thread, so the window keeps
    /// drawing while the worker runs (review 2026-10-18 EX-10); the client's
    /// cancel flag stops the worker. [`Self::finish_client_exchange_export`]
    /// publishes what the thread sends.
    pub(crate) fn start_client_exchange_export(
        &mut self,
        format: ExchangeFormat,
        path: &Path,
        cancelled: Arc<AtomicBool>,
        context: &egui::Context,
    ) -> Result<Receiver<ClientExchangeExport>, ExportError> {
        self.side_effect_receipts.clear();
        let job = self.exchange_export_job(format, path)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        std::thread::Builder::new()
            .name("ketchup-exchange-export".into())
            .spawn(move || {
                let bytes = job.run(&cancelled);
                if sender.send(ClientExchangeExport { job, bytes }).is_ok() {
                    repaint.request_repaint();
                }
            })
            .map_err(ExportError::failed)?;
        Ok(receiver)
    }

    /// Publishes a client's export the thread finished, as a client's: its
    /// loss report goes back instead of a prompt, and a client that left gets
    /// nothing written.
    pub(crate) fn finish_client_exchange_export(
        &mut self,
        export: ClientExchangeExport,
        cancelled: &Arc<AtomicBool>,
    ) -> Result<Written, ExportError> {
        let ClientExchangeExport { job, bytes } = export;
        self.live.client_export = Some(Arc::clone(cancelled));
        let result = self.publish_unless_client_gone(&job, bytes, cancelled);
        self.live.client_export = None;
        self.report_export_outcome(job.format.key(), &job.path, result)
    }

    /// Starts the export on a thread; [`Self::poll_exchange_export`] finishes it.
    pub(super) fn start_exchange_export(
        &mut self,
        format: ExchangeFormat,
        path: PathBuf,
        context: &egui::Context,
    ) {
        self.side_effect_receipts.clear();
        match self.exchange_export_job(format, &path) {
            Ok(job) => self.queue_exchange_export(job, context, ExchangeExportJob::run),
            Err(error) => {
                self.report_export_outcome(format.key(), &path, Err(error))
                    .ok();
            }
        }
    }

    fn queue_exchange_export(
        &mut self,
        job: ExchangeExportJob,
        context: &egui::Context,
        run: impl FnOnce(&ExchangeExportJob, &AtomicBool) -> Result<Vec<u8>, ExportError>
        + Send
        + 'static,
    ) {
        let (format, path) = (job.format, job.path.clone());
        let cancelled = Arc::new(AtomicBool::new(false));
        let (sender, receiver) = mpsc::channel();
        let snapshot = job.snapshot.clone();
        let model = Arc::clone(&job.model);
        let flag = Arc::clone(&cancelled);
        let repaint = context.clone();
        std::thread::spawn(move || {
            if sender.send(run(&job, &flag)).is_ok() {
                repaint.request_repaint();
            }
        });
        self.file.activity.exchange_export = Some(ExchangeExportTask {
            format,
            path,
            snapshot,
            model,
            cancelled,
            receiver,
        });
        context.request_repaint();
    }

    /// Stops the worker; nothing has been written to the chosen path yet.
    pub(super) fn cancel_exchange_export(&mut self) {
        if let Some(task) = self.file.activity.exchange_export.take() {
            task.cancelled.store(true, Ordering::Release);
            self.digest = self.catalog.text("digest-cancelled");
        }
    }

    pub(super) fn poll_exchange_export(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.exchange_export.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(error @ TryRecvError::Disconnected) => Err(ExportError::failed(error)),
        };
        let task = self
            .file
            .activity
            .exchange_export
            .take()
            .expect("polled export");
        let result = result.and_then(|bytes| {
            self.publish_exchange_export(
                task.format,
                &task.path,
                &task.snapshot,
                &task.model,
                bytes,
            )
        });
        self.report_export_outcome(task.format.key(), &task.path, result)
            .ok();
        context.request_repaint();
    }

    /// Asks for consent and writes the file with its loss report.
    fn publish_exchange_export(
        &mut self,
        format: ExchangeFormat,
        path: &Path,
        snapshot: &Snapshot,
        model: &[(ExactBodyPackage, SceneOccurrence)],
        bytes: Vec<u8>,
    ) -> Result<Written, ExportError> {
        let (report, domain, consent) = match format {
            ExchangeFormat::Step => (
                exact_model_step_loss_report(snapshot, model),
                &b"ketchup.current-model-step-export.v1"[..],
                ExportConsent {
                    class: HighRiskClass::LossyConversion,
                    operation: "export-current-model-step-with-loss-report",
                    title_key: "dialog-export-lossy-title",
                    risk_key: "dialog-export-lossy-risk",
                    overwrite_title_key: "dialog-export-overwrite-title",
                    overwrite_operations: [
                        "overwrite-current-model-step-export",
                        "overwrite-current-model-step-loss-report",
                    ],
                },
            ),
            ExchangeFormat::Iges => (
                exact_model_iges_loss_report(snapshot, model),
                &b"ketchup.current-model-iges-export.v1"[..],
                ExportConsent {
                    class: HighRiskClass::LossyConversion,
                    operation: "export-current-model-iges-with-loss-report",
                    title_key: "dialog-export-iges-title",
                    risk_key: "dialog-export-iges-risk",
                    overwrite_title_key: "dialog-export-iges-overwrite-title",
                    overwrite_operations: [
                        "overwrite-current-model-iges-export",
                        "overwrite-current-model-iges-loss-report",
                    ],
                },
            ),
        };
        let report_path = path.with_extension(format!("{}.loss.txt", format.key()));
        let evidence = super::export::export_bundle_evidence(
            domain,
            path,
            &bytes,
            &report_path,
            report.as_bytes(),
        );
        let precondition = ExportBundlePrecondition::capture(path, &report_path)?;
        self.authorize_export_bundle(consent, path, &report_path, &precondition, &evidence)?;
        write_export_bundle(path, &bytes, &report_path, report.as_bytes(), &precondition)?;
        Ok(Written::with_loss_report(path, &report_path))
    }
}

#[cfg(test)]
mod tests;

fn exact_model_step_loss_report(
    snapshot: &Snapshot,
    model: &[(ExactBodyPackage, SceneOccurrence)],
) -> String {
    let fingerprints = model
        .iter()
        .map(|(package, _)| package.result_key().result_fingerprint)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "authority=accepted exact OCCT B-Rep and canonical scene hierarchy\nformat=ISO 10303 STEP with XDE assembly metadata\nconversion=current-visible-exact-model-to-xde-assembly\nassembly=global groups, nested occurrences, repeated part definitions, local rigid transforms, names, and occurrence sRGB colors are preserved\neditability_loss=canonical Ketchup features, rules, dimensions, and Undo history are not preserved\ntopology_loss=exact B-Rep topology is preserved, but durable Ketchup subshape, group, occurrence, and feature IDs are not preserved\ntolerance_loss=no tessellation loss; receiving systems may apply a different modeling tolerance\nsource_digest={}\noccurrence_body_count={}\nresult_fingerprints={fingerprints}\n",
        snapshot.canonical_digest(),
        model.len(),
    )
}

fn exact_model_iges_loss_report(
    snapshot: &Snapshot,
    model: &[(ExactBodyPackage, SceneOccurrence)],
) -> String {
    let fingerprints = model
        .iter()
        .map(|(package, _)| package.result_key().result_fingerprint)
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "authority=accepted exact OCCT B-Rep and canonical scene metadata\nformat=IGES 5.3\nunits=millimetre\nconversion=current-visible-exact-scene-to-flat-iges-brep-roots\npreserved=one exact root per visible occurrence, occurrence names, sRGB colors, and world placement baked into geometry\neditability_loss=canonical Ketchup features, rules, dimensions, and Undo history are not preserved\ntopology_loss=exact B-Rep geometry is transferred, but durable Ketchup subshape and occurrence identity are not preserved\nassembly_loss=hierarchy, local transforms, and repeated shared definitions are unavailable; roots are duplicated and world transforms are baked into exact geometry\ntolerance_loss=no tessellation loss; receiving systems may apply a different modeling tolerance\nsource_digest={}\noccurrence_count={}\nresult_fingerprints={fingerprints}\n",
        snapshot.canonical_digest(),
        model.len(),
    )
}
