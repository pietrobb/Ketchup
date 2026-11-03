//! `file action=export_drawings` of the project set as a job (review 2026-11-01
//! UI-100): the projection, hidden-line removal and file bytes, and the exact
//! worker making the cosmetic body of each part with modelled threads, run on a
//! thread while the window keeps drawing and names the export as the job it is
//! busy with. A cancelled request stops the worker, writes no file and puts back
//! the settings the export kept.

use super::drawings::{StartedDrawings, export_failed, finish_export, start_export};
use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::project_drawings::ProjectDrawingsError;
use ketchup_manufacturing::project_drawings::DrawingSet;

type Made = Result<(DrawingSet, Vec<Vec<u8>>), ProjectDrawingsError>;

pub(super) struct ProjectDrawingsExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    pub(super) cancelled: Arc<AtomicBool>,
    started: StartedDrawings,
    receiver: mpsc::Receiver<Made>,
    /// The document the set is drawn from and keeps its settings in.
    epoch: u64,
}

/// Whether `request` exports the project set, which runs as a job.
pub(super) fn exports_project_drawings(request: &Request) -> bool {
    matches!(request, Request::ExportDrawings { sheet_id: None, .. })
}

impl LiveBridge {
    pub(super) fn start_project_drawings_export(
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
        if let Err(code) = Self::require_request_authority(&cancelled) {
            Self::reply(app, id, &reply, Err(code));
            return;
        }
        let (started, job) = match start_export(app, request, ui_busy) {
            Ok(started) => started,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let (format, flag, repaint) =
            (started.file_format, Arc::clone(&cancelled), context.clone());
        let spawn = std::thread::Builder::new()
            .name("ketchup-drawings-export".into())
            .spawn(move || {
                let _ = sender.try_send(job.render(format, &flag));
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            started.undo(app);
            let error = failed_because("job_worker_unavailable", error);
            Self::reply(app, id, &reply, Err(error));
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::ProjectDrawings(ProjectDrawingsExportJob {
            id,
            reply,
            cancelled,
            started,
            receiver,
            epoch: self.document_epoch,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_project_drawings_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: ProjectDrawingsExportJob,
    ) {
        // The settings it kept went with that document: putting them back
        // would write them into the one open now (review 2026-11-02 MCP-122).
        if job.epoch != self.document_epoch {
            job.cancelled.store(true, Ordering::Release);
            Self::reply(app, job.id, &job.reply, Err(Self::document_replaced()));
            return;
        }
        // The worker sees the same flag and stops; nothing is written.
        if job.cancelled.load(Ordering::Acquire) {
            job.started.undo(app);
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            return;
        }
        let made = match job.receiver.try_recv() {
            Ok(made) => made,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::ProjectDrawings(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(ProjectDrawingsError::Write(
                std::io::Error::other("the thread that draws the sheets stopped"),
            )),
        };
        take_error_details();
        let result = finish_export(app, job.started, made).map_err(export_failed);
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}
