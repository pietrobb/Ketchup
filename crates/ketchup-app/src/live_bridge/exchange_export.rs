//! `file action=export` to STEP or IGES as a job (review 2026-10-18 EX-10): the
//! exact worker writes and reads back the file on a thread for up to its budget,
//! while the window keeps drawing and names the export as the job it is busy
//! with. A cancelled request stops the worker; the window then publishes the
//! file as a client's export.

use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::exchange_export::{ClientExchangeExport, ExchangeFormat};

pub(super) struct ExchangeExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    path: String,
    export_format: String,
    receiver: mpsc::Receiver<ClientExchangeExport>,
}

/// The exact format `request` exports to, which runs as a job.
pub(super) fn exchange_format(request: &Request) -> Option<ExchangeFormat> {
    match request {
        Request::Export { export_format, .. } if export_format == "step" => {
            Some(ExchangeFormat::Step)
        }
        Request::Export { export_format, .. } if export_format == "iges" => {
            Some(ExchangeFormat::Iges)
        }
        _ => None,
    }
}

impl LiveBridge {
    pub(super) fn start_exchange_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        queued: QueuedReply,
        request: Request,
        ui_busy: bool,
    ) {
        let QueuedReply {
            id,
            reply,
            cancelled,
        } = queued;
        take_error_details();
        if self.program_check_job.is_some() {
            let busy = failure(
                "busy",
                "A program, geometry check, import or export is running.",
                json!({}),
            );
            Self::reply(app, id, &reply, Err(busy));
            return;
        }
        let Some(format) = exchange_format(&request) else {
            Self::reply(app, id, &reply, Err("invalid_request"));
            return;
        };
        let started =
            Self::export_target(app, request, ui_busy).and_then(|(path, export_format, _)| {
                Self::require_request_authority(&cancelled)?;
                let receiver = app
                    .start_client_exchange_export(
                        format,
                        Path::new(&path),
                        Arc::clone(&cancelled),
                        context,
                    )
                    .map_err(|error| Self::export_failed(&path, &export_format, error))?;
                Ok((path, export_format, receiver))
            });
        match started {
            Ok((path, export_format, receiver)) => {
                self.program_check_job = Some(ProgramCheckJob::ExchangeExport(ExchangeExportJob {
                    id,
                    reply,
                    cancelled,
                    path,
                    export_format,
                    receiver,
                }));
                context.request_repaint_after(Duration::from_millis(10));
            }
            Err(code) => Self::reply(app, id, &reply, Err(code)),
        }
    }

    pub(super) fn poll_exchange_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: ExchangeExportJob,
    ) {
        // The worker sees the same flag and stops; nothing is written.
        if job.cancelled.load(Ordering::Acquire) {
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            return;
        }
        let export = match job.receiver.try_recv() {
            Ok(export) => export,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::ExchangeExport(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                let error = crate::export_bundle::ExportError::failed(
                    "the thread that exports the model stopped",
                );
                let code = Self::export_failed(&job.path, &job.export_format, error);
                Self::reply(app, job.id, &job.reply, Err(code));
                return;
            }
        };
        take_error_details();
        let written = app.finish_client_exchange_export(export, &job.cancelled);
        let result = Self::exported(&job.path, &job.export_format, written);
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}
