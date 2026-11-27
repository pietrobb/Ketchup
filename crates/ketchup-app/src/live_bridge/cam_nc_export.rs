//! `file action=export` of a CAM program (`cam_nc`) as a job (review 2026-11-27 R1127-CAM-2):
//! the window checks the request and what the program needs, a thread plans the toolpath and
//! the exact worker simulates it while the window keeps drawing and names the export as busy,
//! and the window then writes the program as it was simulated. A cancelled request stops the
//! exact worker and writes nothing.

use super::*;
use crate::app::CamProgramRun;
use crate::delivery::Deliver;
use crate::export_bundle::ExportError;
use crate::live_bridge::interchange::ExportRequest;
use ketchup_application::cam_workflow::{CamComputedReview, CamReviewError};

type Computed = (CamProgramRun, Result<CamComputedReview, CamReviewError>);

pub(in crate::live_bridge) struct CamExportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    pub(in crate::live_bridge) cancelled: Arc<AtomicBool>,
    path: String,
    export_format: String,
    overwrite: bool,
    receiver: mpsc::Receiver<Computed>,
    /// The client was told `request_cancelled`; the job keeps its place until its thread ends.
    answered: bool,
}

/// Whether `request` writes a CAM program, which runs as a job.
pub(super) fn exports_cam_program(request: &Request) -> bool {
    matches!(request, Request::Export(ExportRequest {
        export_format,
        drawing_scale: None,
        ..
    }) if export_format == "cam_nc")
}

impl LiveBridge {
    pub(super) fn start_cam_export(
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
        let target = Self::export_target(app, request, ui_busy).and_then(|target| {
            Self::require_request_authority(&cancelled)?;
            Ok(target)
        });
        let ExportTarget {
            path,
            export_format,
            overwrite,
            cam_dialect,
            cam_operations,
            ..
        } = match target {
            Ok(target) => target,
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let Some(dialect) = cam_dialect else {
            Self::reply(app, id, &reply, Err("invalid_request"));
            return;
        };
        let prepared = as_client(app, &cancelled, overwrite, |app| {
            app.prepare_cam_program(Path::new(&path), dialect, cam_operations)
        });
        let run = match prepared {
            Ok(run) => run,
            Err(error) => {
                let written = app.report_export_outcome("cam-nc", Path::new(&path), Err(error));
                Self::reply(
                    app,
                    id,
                    &reply,
                    Self::exported(&path, &export_format, written),
                );
                return;
            }
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let (thread_cancelled, repaint) = (Arc::clone(&cancelled), context.clone());
        let spawn = std::thread::Builder::new()
            .name("ketchup-cam-program".into())
            .spawn(move || {
                let computed = run.compute(&thread_cancelled);
                sender.deliver((run, computed));
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            let error = failed_because("job_worker_unavailable", error);
            Self::reply(app, id, &reply, Err(error));
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::CamExport(CamExportJob {
            id,
            reply,
            cancelled,
            path,
            export_format,
            overwrite,
            receiver,
            answered: false,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(in crate::live_bridge) fn poll_cam_export(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        mut job: CamExportJob,
    ) {
        let received = job.receiver.try_recv();
        // The exact worker sees the same flag and stops; nothing is written. The client is
        // answered at once, but the job keeps its place until the thread has ended, so no
        // second program is made beside it.
        if !job.answered && job.cancelled.load(Ordering::Acquire) {
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            job.answered = true;
        }
        let computed = match received {
            Ok(computed) => Some(computed),
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::CamExport(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => None,
        };
        if job.answered {
            context.request_repaint();
            return;
        }
        take_error_details();
        let written = match computed {
            Some((run, computed)) => as_client(app, &job.cancelled, job.overwrite, |app| {
                app.finish_cam_program(Path::new(&job.path), run, computed)
            }),
            None => Err(ExportError::failed(
                "the thread that plans and simulates the CAM program stopped",
            )),
        };
        let written = app.report_export_outcome("cam-nc", Path::new(&job.path), written);
        let answer = Self::exported(&job.path, &job.export_format, written);
        Self::reply(app, job.id, &job.reply, answer);
        context.request_repaint();
    }
}

/// Runs `step` as the export of the client whose request `cancelled` revokes, which
/// replaces a file only with `overwrite`.
fn as_client<T>(
    app: &mut KetchupApp,
    cancelled: &Arc<AtomicBool>,
    overwrite: bool,
    step: impl FnOnce(&mut KetchupApp) -> T,
) -> T {
    app.live.client_export = Some(Arc::clone(cancelled));
    app.live.client_overwrite = overwrite;
    let result = step(app);
    app.live.client_export = None;
    app.live.client_overwrite = false;
    result
}
