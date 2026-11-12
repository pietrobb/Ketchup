//! `file action=export` to STEP or IGES as a job (review 2026-10-18 EX-10): the
//! exact worker writes and reads back the file on a thread for up to its budget,
//! while the window keeps drawing and names the export as the job it is busy
//! with. A cancelled request stops the worker; the window then publishes the
//! file as a client's export. A presentation image runs as such a job too (review 2026-11-12
//! R1112-IMG-502): it is made on a thread, as from the window, and a cancelled request stops
//! it within a row of samples.

use super::interchange::{ExportTarget, InvalidImage, presentation_request};
use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::exchange_export::{ClientExchangeExport, ExchangeFormat};
use crate::app::presentation_export::MadePresentation;

pub(super) struct ExchangeExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    pub(super) cancelled: Arc<AtomicBool>,
    path: String,
    export_format: String,
    overwrite: bool,
    receiver: mpsc::Receiver<ClientExchangeExport>,
}

/// Whether `request` is an export that runs as a job.
pub(super) fn runs_as_job(request: &Request) -> bool {
    exchange_format(request).is_some() || presents(request)
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
        if presents(&request) {
            let queued = QueuedReply {
                id,
                reply,
                cancelled,
            };
            self.start_presentation(app, context, queued, request, ui_busy);
            return;
        }
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
        let started = Self::export_target(app, request, ui_busy).and_then(|target| {
            Self::require_request_authority(&cancelled)?;
            let receiver = app
                .start_client_exchange_export(
                    format,
                    Path::new(&target.path),
                    Arc::clone(&cancelled),
                    context,
                )
                .map_err(|error| Self::export_failed(&target.path, &target.export_format, error))?;
            Ok((target, receiver))
        });
        match started {
            Ok((target, receiver)) => {
                self.program_check_job = Some(ProgramCheckJob::ExchangeExport(ExchangeExportJob {
                    id,
                    reply,
                    cancelled,
                    path: target.path,
                    export_format: target.export_format,
                    overwrite: target.overwrite,
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
        let written = app.finish_client_exchange_export(export, &job.cancelled, job.overwrite);
        let result = Self::exported(&job.path, &job.export_format, written);
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}

pub(super) struct PresentationJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    pub(super) cancelled: Arc<AtomicBool>,
    path: String,
    export_format: String,
    overwrite: bool,
    receiver: mpsc::Receiver<MadePresentation>,
}

/// Whether `request` is a presentation image, which runs as a job.
pub(super) fn presents(request: &Request) -> bool {
    matches!(request, Request::Export { export_format, .. } if export_format == "presentation_png")
}

impl LiveBridge {
    fn start_presentation(
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
        let image = match &request {
            Request::Export { image, .. } => image.clone(),
            _ => None,
        };
        let started = Self::export_target(app, request, ui_busy).and_then(|target| {
            let request = presentation_request(image).map_err(InvalidImage::refused)?;
            Self::require_request_authority(&cancelled)?;
            let receiver = app
                .start_client_presentation(
                    Path::new(&target.path),
                    &request,
                    Arc::clone(&cancelled),
                    context,
                )
                .map_err(|error| Self::export_failed(&target.path, &target.export_format, error))?;
            Ok((target, receiver))
        });
        match started {
            Ok((
                ExportTarget {
                    path,
                    export_format,
                    overwrite,
                    ..
                },
                receiver,
            )) => {
                self.program_check_job = Some(ProgramCheckJob::Presentation(PresentationJob {
                    id,
                    reply,
                    cancelled,
                    path,
                    export_format,
                    overwrite,
                    receiver,
                }));
                context.request_repaint_after(Duration::from_millis(10));
            }
            Err(code) => Self::reply(app, id, &reply, Err(code)),
        }
    }

    pub(super) fn poll_presentation(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: PresentationJob,
    ) {
        // The thread sees the same flag and stops; nothing is written.
        if job.cancelled.load(Ordering::Acquire) {
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            return;
        }
        let made = match job.receiver.try_recv() {
            Ok(made) => made,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::Presentation(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                Err(crate::export_bundle::ExportError::failed(
                    "the thread that makes the presentation image stopped",
                ))
            }
        };
        take_error_details();
        let written = app.finish_client_presentation(
            Path::new(&job.path),
            made,
            &job.cancelled,
            job.overwrite,
        );
        let result = Self::exported(&job.path, &job.export_format, written);
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}
