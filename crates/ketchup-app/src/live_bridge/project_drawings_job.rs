//! `file action=export_drawings` of the project set as a job (review 2026-11-01
//! UI-100): the projection, hidden-line removal and file bytes, and the exact
//! worker making the cosmetic body of each part with modelled threads, run on a
//! thread while the window keeps drawing and names the export as the job it is
//! busy with. A cancelled request stops the worker, writes no file and puts back
//! the settings the export kept.
//!
//! Sent while the exact evaluation runs, an export of the set or of one sheet
//! waits in the window for it and is then drawn from its solids, in the same
//! request (Měchenice MECH-4); it was refused, and the client had to poll.

use super::drawings::{StartedDrawings, export_failed, finish_export, start_export};
use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::project_drawings::ProjectDrawingsError;
use ketchup_manufacturing::project_drawings::DrawingSet;

type Made = Result<(DrawingSet, Vec<Vec<u8>>), ProjectDrawingsError>;

/// No export waits longer for the exact evaluation: the drawing itself still
/// needs time within what the connection waits for its answer.
const EXACT_WAIT_MOST: Duration = Duration::from_secs(5 * 60);

pub(super) struct ProjectDrawingsExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    pub(super) cancelled: Arc<AtomicBool>,
    stage: Stage,
    /// The document the set is drawn from and keeps its settings in.
    epoch: u64,
}

enum Stage {
    /// Held until the exact evaluation ends, then started as it was sent.
    AwaitingExact {
        request: Request,
        ui_busy: bool,
        until: Instant,
    },
    Drawing {
        started: StartedDrawings,
        receiver: mpsc::Receiver<Made>,
    },
}

impl ProjectDrawingsExportJob {
    pub(super) const fn awaits_exact(&self) -> bool {
        matches!(self.stage, Stage::AwaitingExact { .. })
    }
}

/// Whether `request` exports the project set, which runs as a job.
pub(super) fn exports_project_drawings(request: &Request) -> bool {
    matches!(request, Request::ExportDrawings { sheet_id: None, .. })
}

/// Whether `request` exports drawings while the exact evaluation runs, so it
/// waits for the evaluation as a job.
pub(super) fn waits_for_exact(app: &KetchupApp, request: &Request) -> bool {
    app.exact.task.is_some() && matches!(request, Request::ExportDrawings { .. })
}

impl LiveBridge {
    pub(super) fn start_project_drawings_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        queued: QueuedReply,
        mut request: Request,
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
        if waits_for_exact(app, &request) {
            if let Request::ExportDrawings { expected, .. } = &mut request {
                if let Err(code) = Self::guard(app, expected) {
                    Self::reply(app, id, &reply, Err(code));
                    return;
                }
                // The client saw the document the evaluation runs for;
                // publishing its solids is the next mutation, so the stamp is
                // checked now.
                *expected = None;
            }
            self.program_check_job =
                Some(ProgramCheckJob::ProjectDrawings(ProjectDrawingsExportJob {
                    id,
                    reply,
                    cancelled,
                    stage: Stage::AwaitingExact {
                        request,
                        ui_busy,
                        until: Instant::now() + EXACT_WAIT_MOST,
                    },
                    epoch: self.document_epoch,
                }));
            context.request_repaint_after(Duration::from_millis(50));
            return;
        }
        if !exports_project_drawings(&request) {
            // One sheet is drawn in the frame, as when nothing was evaluated.
            let result = self.execute_authorized(app, request, ui_busy, &cancelled);
            Self::reply(app, id, &reply, result);
            context.request_repaint();
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
            stage: Stage::Drawing { started, receiver },
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
        let ProjectDrawingsExportJob {
            id,
            reply,
            cancelled,
            stage,
            epoch,
        } = job;
        let (started, receiver) = match stage {
            Stage::AwaitingExact {
                request,
                ui_busy,
                until,
            } => {
                if cancelled.load(Ordering::Acquire) {
                    Self::reply(app, id, &reply, Err("request_cancelled"));
                    return;
                }
                if app.exact.task.is_none() {
                    let queued = QueuedReply {
                        id,
                        reply,
                        cancelled,
                    };
                    self.start_project_drawings_export(app, context, queued, request, ui_busy);
                    return;
                }
                if Instant::now() >= until {
                    let waited = failure(
                        "drawings_unavailable",
                        format!(
                            "The exact evaluation still runs after {} minutes, so nothing was drawn or written. Export again when inspect action=status reports it complete.",
                            EXACT_WAIT_MOST.as_secs() / 60
                        ),
                        json!({"exact_evaluation_pending": true}),
                    );
                    Self::reply(app, id, &reply, Err(waited));
                    return;
                }
                self.program_check_job =
                    Some(ProgramCheckJob::ProjectDrawings(ProjectDrawingsExportJob {
                        id,
                        reply,
                        cancelled,
                        stage: Stage::AwaitingExact {
                            request,
                            ui_busy,
                            until,
                        },
                        epoch,
                    }));
                context.request_repaint_after(Duration::from_millis(50));
                return;
            }
            Stage::Drawing { started, receiver } => (started, receiver),
        };
        // The worker sees the same flag and stops; nothing is written.
        if cancelled.load(Ordering::Acquire) {
            started.undo(app);
            Self::reply(app, id, &reply, Err("request_cancelled"));
            return;
        }
        let made = match receiver.try_recv() {
            Ok(made) => made,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job =
                    Some(ProgramCheckJob::ProjectDrawings(ProjectDrawingsExportJob {
                        id,
                        reply,
                        cancelled,
                        stage: Stage::Drawing { started, receiver },
                        epoch,
                    }));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => Err(ProjectDrawingsError::Write(
                std::io::Error::other("the thread that draws the sheets stopped"),
            )),
        };
        take_error_details();
        let result = finish_export(app, started, made).map_err(export_failed);
        Self::reply(app, id, &reply, result);
        context.request_repaint();
    }
}
