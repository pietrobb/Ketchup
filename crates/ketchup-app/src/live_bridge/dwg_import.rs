//! `file action=import` of a `.dwg` file, or of a drawing `as_underlay`, as a job
//! (review DR-3): the program worker reads it for up to its deadline, and the window
//! does not wait for it. A thread reads the file while the window keeps drawing and
//! names the import as the job it is busy with; a cancelled request kills the worker.
//! The import itself then runs on the window's thread from what that thread read,
//! while the job holds a DWG's conversion.

use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::dwg_import::{DwgRead, is_dwg_path, read_dwg_file};
use crate::app::underlay::{UnderlayReadResult, read_underlay_file};
use ketchup_program::CancelToken;

/// What the job's thread read.
enum JobRead {
    Dwg(DwgRead),
    Underlay(Box<UnderlayReadResult>),
}

pub(super) struct DwgImportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    cancel: CancelToken,
    request: Request,
    receiver: mpsc::Receiver<JobRead>,
}

/// Whether `request` imports a DWG file or an underlay, which run as a job.
pub(super) fn imports_dwg(request: &Request) -> bool {
    matches!(request, Request::Import { path, as_underlay, .. }
        if *as_underlay || is_dwg_path(Path::new(path)))
}

impl LiveBridge {
    pub(super) fn start_dwg_import(
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
                "A program, geometry check or another import is running.",
                json!({}),
            );
            Self::reply(app, id, &reply, Err(busy));
            return;
        }
        let (path, unit) = match Self::import_target(app, &request, ui_busy) {
            Ok((path, _, unit)) => (PathBuf::from(path), unit),
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let as_underlay = matches!(
            request,
            Request::Import {
                as_underlay: true,
                ..
            }
        );
        let cancel = CancelToken::default();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-dwg-import".into())
            .spawn(move || {
                let read = if as_underlay {
                    JobRead::Underlay(Box::new(read_underlay_file(&path, unit, &worker_cancel)))
                } else {
                    JobRead::Dwg(read_dwg_file(&path, &worker_cancel))
                };
                let _ = sender.try_send(read);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            let error = failed_because("job_worker_unavailable", error);
            Self::reply(app, id, &reply, Err(error));
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::DwgImport(DwgImportJob {
            id,
            reply,
            cancelled,
            cancel,
            request,
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_dwg_import(
        &mut self,
        app: &mut KetchupApp,
        context: &egui::Context,
        job: DwgImportJob,
    ) {
        if job.cancelled.load(Ordering::Acquire) {
            job.cancel.cancel();
            Self::reply(app, job.id, &job.reply, Err("request_cancelled"));
            return;
        }
        let read = match job.receiver.try_recv() {
            Ok(read) => read,
            Err(mpsc::TryRecvError::Empty) => {
                self.program_check_job = Some(ProgramCheckJob::DwgImport(job));
                context.request_repaint_after(Duration::from_millis(10));
                return;
            }
            Err(mpsc::TryRecvError::Disconnected) => JobRead::Dwg(Err(crate::ImportError::failed(
                crate::ImportFormat::Dxf,
                "the thread that reads the drawing stopped",
            ))),
        };
        take_error_details();
        let Request::Import { path, .. } = &job.request else {
            Self::reply(app, job.id, &job.reply, Err("invalid_request"));
            return;
        };
        let result = match read {
            // `_held` keeps the conversion found while the import plans and
            // publishes it.
            JobRead::Dwg(Ok((source, _held))) => Self::import_file(
                app,
                &job.request,
                ui_busy(context),
                &job.cancelled,
                Some(source),
            ),
            JobRead::Dwg(Err(error)) => Err(interchange::import_failed(path, error)),
            JobRead::Underlay(read) => match *read {
                Ok(read) => {
                    Self::add_underlay(app, &job.request, ui_busy(context), &job.cancelled, read)
                }
                Err(error) => Err(interchange::import_failed(path, error)),
            },
        };
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }

    /// `remove_underlay`: removes the underlay and the tags only it held, as one Undo step.
    pub(super) fn remove_underlay(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::RemoveUnderlay {
            expected,
            underlay_id,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let id = ketchup_model::underlay::UnderlayId(*underlay_id);
        if app.document.current().underlay(id).is_none() {
            return Err("entity_not_found");
        }
        Self::require_request_authority(cancelled)?;
        app.remove_underlay(id)
            .map_err(|error| failed_because("commit_rejected", error))?;
        self.query.invalidate();
        Ok(json!({"removed": true, "underlay_id": underlay_id, "dirty": app.is_dirty()}))
    }

    /// `fit_underlay`: places the underlay so two points of its lines land on two model
    /// points, as one Undo step; two picks at one point are refused with the reason.
    pub(super) fn fit_underlay(
        &mut self,
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
    ) -> Result<Value, &'static str> {
        let Request::FitUnderlay {
            expected,
            underlay_id,
            from_mm,
            to_mm,
            scale,
        } = request
        else {
            return Err("invalid_request");
        };
        Self::guard(app, expected)?;
        Self::available(app, ui_busy)?;
        let id = ketchup_model::underlay::UnderlayId(*underlay_id);
        if app.document.current().underlay(id).is_none() {
            return Err("entity_not_found");
        }
        Self::require_request_authority(cancelled)?;
        let fit = app
            .fit_underlay(id, *from_mm, *to_mm, *scale)
            .map_err(|error| match error {
                crate::app::underlay::UnderlayEditError::Commit(_) => {
                    failed_because("commit_rejected", error)
                }
                _ => failed_because("invalid_params", error),
            })?;
        self.query.invalidate();
        Ok(json!({
            "fitted": true,
            "underlay_id": underlay_id,
            "turn_deg": fit.turn_deg,
            "scale": fit.scale,
            // How far the second point misses its target along the line from the first,
            // as the two pairs lie apart by different lengths and the fit did not scale.
            "length_mismatch_mm": fit.length_mismatch_mm,
            "dirty": app.is_dirty(),
        }))
    }

    /// Adds the drawing read for the import `request` as an underlay.
    pub(super) fn add_underlay(
        app: &mut KetchupApp,
        request: &Request,
        ui_busy: bool,
        cancelled: &AtomicBool,
        read: crate::app::underlay::ReadUnderlay,
    ) -> Result<Value, &'static str> {
        let (path, ..) = Self::import_target(app, request, ui_busy)?;
        Self::require_request_authority(cancelled)?;
        let summary = read.summary();
        let id = app
            .add_underlay(Path::new(&path), read)
            .map_err(|error| interchange::import_failed(&path, error))?;
        Ok(json!({"imported": true, "path": path, "underlay_id": id.0,
            "underlay": summary, "dirty": app.is_dirty()}))
    }
}
