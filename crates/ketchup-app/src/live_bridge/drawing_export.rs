//! `drawing action=export` as a job (review 2026-10-31 MCP-80): the program worker
//! writes the drawing back on a thread for up to its budget while the window keeps
//! drawing and names the export as the job it is busy with. A cancelled request
//! stops the worker and no file takes the place of the one named.

use super::drawing_edit::DrawingExportTarget;
use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::underlay_export::{ClientUnderlayExport, UnderlayExportError, WrittenDrawing};
use ketchup_program::CancelToken;

pub(super) struct DrawingExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    target: DrawingExportTarget,
    cancel: CancelToken,
    receiver: mpsc::Receiver<Result<WrittenDrawing, UnderlayExportError>>,
}

impl LiveBridge {
    pub(super) fn start_drawing_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        queued: QueuedReply,
        request: &Request,
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
        let started =
            Self::drawing_export_target(app, request, ui_busy, &cancelled).and_then(|target| {
                match target.start(app, context) {
                    Ok(export) => Ok((target, export)),
                    Err(error) => Err(target.refuse(error)),
                }
            });
        match started {
            Ok((target, ClientUnderlayExport { cancel, receiver })) => {
                self.program_check_job = Some(ProgramCheckJob::DrawingExport(DrawingExportJob {
                    id,
                    reply,
                    cancelled,
                    target,
                    cancel,
                    receiver,
                }));
                context.request_repaint_after(Duration::from_millis(10));
            }
            Err(code) => Self::reply(app, id, &reply, Err(code)),
        }
    }

    pub(super) fn poll_drawing_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: DrawingExportJob,
    ) {
        // The worker is killed; the drawing it may have written replaces nothing.
        if job.cancelled.load(Ordering::Acquire) {
            job.cancel.cancel();
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            return;
        }
        let written = match job.receiver.try_recv() {
            Ok(written) => written,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::DrawingExport(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(UnderlayExportError::Io(
                std::io::Error::other("the thread that writes the drawing stopped"),
            )),
        };
        take_error_details();
        let result = ClientUnderlayExport::publish(written)
            .map(|written| job.target.written(written))
            .map_err(|error| job.target.refuse(error));
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}
