//! `file action=import` of a `.dwg` file as a job (review DR-3): the DWG reader
//! runs in the program worker for up to its deadline, and the window does not
//! wait for it. A thread reads the file and its conversion while the window
//! keeps drawing and names the import as the job it is busy with; a cancelled
//! request kills the worker. The import itself then runs on the window's thread
//! from the bytes that thread read, while the job holds their conversion.

use super::program_check::ProgramCheckJob;
use super::program_dry_run::QueuedReply;
use super::*;
use crate::app::dwg_import::{DwgRead, is_dwg_path, read_dwg_file};
use ketchup_program::CancelToken;

pub(super) struct DwgImportJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    cancel: CancelToken,
    request: Request,
    receiver: mpsc::Receiver<DwgRead>,
}

/// Whether `request` imports a DWG file, which runs as a job.
pub(super) fn imports_dwg(request: &Request) -> bool {
    matches!(request, Request::Import { path, .. } if is_dwg_path(Path::new(path)))
}

impl LiveBridge {
    pub(super) fn start_dwg_import(
        &mut self,
        app: &KetchupApp,
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
        let path = match Self::import_target(app, &request, ui_busy) {
            Ok((path, ..)) => PathBuf::from(path),
            Err(code) => {
                Self::reply(app, id, &reply, Err(code));
                return;
            }
        };
        let cancel = CancelToken::default();
        let worker_cancel = cancel.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-dwg-import".into())
            .spawn(move || {
                let _ = sender.try_send(read_dwg_file(&path, &worker_cancel));
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
            Err(mpsc::TryRecvError::Disconnected) => Err(crate::ImportError::failed(
                crate::ImportFormat::Dxf,
                "the thread that reads the DWG file stopped",
            )),
        };
        take_error_details();
        let result = match read {
            // `_held` keeps the conversion found while the import plans and
            // publishes it.
            Ok((source, _held)) => Self::import_file(
                app,
                &job.request,
                ui_busy(context),
                &job.cancelled,
                Some(source),
            ),
            Err(error) => match &job.request {
                Request::Import { path, .. } => Err(interchange::import_failed(path, error)),
                _ => Err("invalid_request"),
            },
        };
        Self::reply(app, job.id, &job.reply, result);
        context.request_repaint();
    }
}
