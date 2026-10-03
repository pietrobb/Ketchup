//! Explicit program validation reads an immutable snapshot without publishing an edit.
use super::program_check::{PROGRAM_EXACT_TIMEOUT, ProgramCheckJob};
use super::*;
#[cfg(test)]
#[path = "program_validation_job_tests.rs"]
mod job_tests;
#[cfg(test)]
#[path = "program_validation_tests.rs"]
mod tests;
use ketchup_application::verify_rule_program_all;
use ketchup_model::document::RuleProgramSource;
use ketchup_model::persistence::ContainerData;

pub(super) struct ProgramValidationJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    before: Stamp,
    started: Instant,
    receiver:
        mpsc::Receiver<Result<(Value, ketchup_program::Report), ketchup_program::ProgramError>>,
}

fn validate(
    source: RuleProgramSource,
    snapshot: Snapshot,
    container: ContainerData,
    worker_path: Option<PathBuf>,
    cancelled: Arc<AtomicBool>,
    stamp: Stamp,
    undo_steps: usize,
) -> Result<(Value, ketchup_program::Report), ketchup_program::ProgramError> {
    let (evaluated, mut report) =
        ketchup_program::run(&source.file_name, &source.source, &source.overrides)?;
    let exact = verify_rule_program_all(
        &snapshot,
        &evaluated.model,
        &mut report,
        &container,
        worker_path,
        PROGRAM_EXACT_TIMEOUT,
        cancelled,
    );
    let mut result = program_edit_result(
        crate::program_edit::ProgramEdit::Unchanged,
        (&report, &evaluated.model),
        &snapshot,
        &snapshot,
        &stamp,
        undo_steps,
        exact,
    );
    result["validation_only"] = json!(true);
    result["canonical_mutation"] = json!(false);
    result["validation_scope"] = json!("all_visible_program_parts");
    Ok((result, report))
}

impl LiveBridge {
    pub(super) fn validate_program(
        &mut self,
        app: &KetchupApp,
        expected: Option<Stamp>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Value, &'static str> {
        Self::guard(app, &expected)?;
        Self::require_request_authority(&cancelled)?;
        let source = app
            .document
            .current_rule_program()
            .cloned()
            .ok_or_else(|| {
                failure(
                    "program_rejected",
                    "No Starlark program owns this document to validate.",
                    json!({}),
                )
            })?;
        validate(
            source,
            app.document.current(),
            app.file.container_data.clone(),
            Self::worker_path(app),
            cancelled,
            app.live_bridge_stamp(),
            app.undo_step_count(),
        )
        .map(|(value, report)| {
            self.program_report = Some((app.live_bridge_stamp(), report, "explicit_validation"));
            value
        })
        .map_err(|error| failed_because("program_rejected", error))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_queued_validate_program(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        expected: Option<Stamp>,
    ) {
        take_error_details();
        let prepared = (|| {
            Self::guard(app, &expected)?;
            Self::require_request_authority(&cancelled)?;
            if self.program_check_job.is_some() {
                return Err(failure(
                    "busy",
                    "A program is already planning or checking geometry.",
                    json!({}),
                ));
            }
            app.document.current_rule_program().cloned().ok_or_else(|| {
                failure(
                    "program_rejected",
                    "No Starlark program owns this document to validate.",
                    json!({}),
                )
            })
        })();
        let source = match prepared {
            Ok(source) => source,
            Err(error) => {
                Self::reply(app, id, &reply, Err(error));
                return;
            }
        };
        let snapshot = app.document.current();
        let container = app.file.container_data.clone();
        let worker_path = Self::worker_path(app);
        let before = app.live_bridge_stamp();
        let stamp = before.clone();
        let undo_steps = app.undo_step_count();
        let worker_cancelled = Arc::clone(&cancelled);
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-program-validate".into())
            .spawn(move || {
                let result = validate(
                    source,
                    snapshot,
                    container,
                    worker_path,
                    worker_cancelled,
                    stamp,
                    undo_steps,
                );
                let _ = sender.try_send(result);
                repaint.request_repaint();
            });
        if let Err(error) = spawn {
            Self::reply(
                app,
                id,
                &reply,
                Err(failed_because("job_worker_unavailable", error)),
            );
            return;
        }
        self.program_check_job = Some(ProgramCheckJob::Validating(ProgramValidationJob {
            id,
            reply,
            cancelled,
            before,
            started: Instant::now(),
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_program_validation(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        job: ProgramValidationJob,
    ) {
        take_error_details();
        let result = if let Err(error) = Self::guard(app, &Some(job.before.clone())) {
            job.cancelled.store(true, Ordering::Release);
            Err(error)
        } else if let Err(error) = Self::require_request_authority(&job.cancelled) {
            Err(error)
        } else if job.started.elapsed() > PROGRAM_EXACT_TIMEOUT + Duration::from_secs(5) {
            job.cancelled.store(true, Ordering::Release);
            Err(failure(
                "job_worker_unavailable",
                "Program validation timed out; no model changes were made.",
                json!({}),
            ))
        } else {
            match job.receiver.try_recv() {
                Ok(result) => result
                    .map(|(value, report)| {
                        self.program_report =
                            Some((job.before.clone(), report, "explicit_validation"));
                        value
                    })
                    .map_err(|error| failed_because("program_rejected", error)),
                Err(mpsc::TryRecvError::Empty) => {
                    self.program_check_job = Some(ProgramCheckJob::Validating(job));
                    context.request_repaint_after(Duration::from_millis(10));
                    return;
                }
                Err(error) => Err(failed_because("program_worker_disconnected", error)),
            }
        };
        Self::reply(app, job.id, &job.reply, result);
    }
}
