//! Read-only face queries share the existing background-check slot and document guard.
use super::program_check::{PROGRAM_EXACT_TIMEOUT, ProgramCheckJob};
use super::*;
use ketchup_application::measurement::{FaceTarget, SelectedFaceMeasurement};
use ketchup_rejection::Rejection;
#[cfg(test)]
#[path = "measurement_tests.rs"]
pub(super) mod tests;

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MeasurementMode {
    Minimum,
    SupportingPlanes,
}

pub(super) struct MeasurementJob {
    id: u64,
    reply: mpsc::SyncSender<Response>,
    cancelled: Arc<AtomicBool>,
    before: Stamp,
    started: Instant,
    receiver: mpsc::Receiver<Result<Value, Rejection>>,
}

fn code(error: Rejection) -> &'static str {
    let code = match error.code() {
        "entity_not_found" => "entity_not_found",
        "stale_document" => "stale_document",
        "job_worker_unavailable" => "job_worker_unavailable",
        "invalid_params" => "invalid_params",
        _ => return failed_because("invalid_params", error),
    };
    record_rejection(&error, json!({"canonical_mutation": false}));
    code
}

fn evaluate(
    measurement: SelectedFaceMeasurement,
    snapshot: Snapshot,
    mode: MeasurementMode,
    direction: Option<[f64; 3]>,
    path: Option<PathBuf>,
    container: ketchup_model::persistence::ContainerData,
    cancelled: Arc<AtomicBool>,
) -> Result<Value, Rejection> {
    match mode {
        MeasurementMode::SupportingPlanes => measurement.supporting_plane_clearance(
            &snapshot,
            direction.ok_or_else(|| {
                rejected("invalid_params")
                    .target("direction")
                    .reason("Supporting-plane mode requires a world-space direction")
                    .fix_hint("Supply a nonzero [x,y,z] direction.")
            })?,
        ),
        MeasurementMode::Minimum => {
            if direction.is_some() {
                return Err(rejected("invalid_params")
                    .target("direction")
                    .reason("Minimum distance has no prescribed direction")
                    .fix_hint("Omit direction, or choose supporting_planes."));
            }
            measurement.minimum_distance_with_worker(
                &snapshot,
                path.as_deref(),
                container.blobs(),
                &cancelled,
            )
        }
    }
}

impl LiveBridge {
    /// Mass properties of the visible parts or of the named root occurrences,
    /// with the densities of the program evaluated now (this answer waits for it).
    pub(super) fn mass(
        app: &KetchupApp,
        expected: &Option<Stamp>,
        occurrence_ids: Option<Vec<u64>>,
    ) -> Result<Value, &'static str> {
        Self::guard(app, expected)?;
        let snapshot = app.document.current();
        let scope = occurrence_ids
            .map(|ids| {
                ids.into_iter()
                    .map(|id| {
                        let id = OccurrenceId(id);
                        snapshot
                            .occurrence(id)
                            .map(|_| ketchup_model::document::InstancePath::root(id))
                            .ok_or_else(|| {
                                super::program_access::invalid(
                                    "occurrence_ids",
                                    format!("No root occurrence {}.", id.0),
                                    "Give root occurrence IDs from inspect query kind=occurrences.",
                                )
                            })
                    })
                    .collect::<Result<BTreeSet<_>, _>>()
            })
            .transpose()?;
        // A bridge request answers in the same call.
        app.wait_for_program_evaluation();
        let evaluation = app.document.current_rule_program().and_then(|program| {
            match app.program_evaluations.try_get(program) {
                crate::program_evaluation::Lookup::Ready(evaluation) => Some(evaluation),
                crate::program_evaluation::Lookup::Pending => None,
            }
        });
        let mut report = ketchup_application::mass_properties::mass_report(
            &snapshot,
            app.exact_results_for_snapshot(&snapshot),
            evaluation
                .as_ref()
                .and_then(|evaluation| evaluation.as_ref().ok())
                .map(|evaluated| &evaluated.model),
            ketchup_application::validation_rules::ValidationRules::library(),
            scope.as_ref(),
        );
        if let Some(Err(error)) = &evaluation {
            report["program_error"] = json!(error.to_string());
        }
        Ok(report)
    }

    fn prepare_measurement(
        &self,
        app: &KetchupApp,
        expected: &Stamp,
        faces: [FaceTarget; 2],
    ) -> Result<SelectedFaceMeasurement, &'static str> {
        Self::guard(app, &Some(expected.clone()))?;
        SelectedFaceMeasurement::prepare(
            &app.document.current(),
            &app.exact.topology_results,
            &self.query,
            faces,
        )
        .map_err(code)
    }

    pub(super) fn measure_now(
        &self,
        app: &KetchupApp,
        expected: Stamp,
        faces: [FaceTarget; 2],
        mode: MeasurementMode,
        direction: Option<[f64; 3]>,
        cancelled: Arc<AtomicBool>,
    ) -> Result<Value, &'static str> {
        Self::require_request_authority(&cancelled)?;
        let measurement = self.prepare_measurement(app, &expected, faces)?;
        evaluate(
            measurement,
            app.document.current(),
            mode,
            direction,
            Self::worker_path(app),
            app.file.container_data.clone(),
            cancelled,
        )
        .map_err(code)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn start_measurement(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        id: u64,
        reply: mpsc::SyncSender<Response>,
        cancelled: Arc<AtomicBool>,
        expected: Stamp,
        faces: [FaceTarget; 2],
        mode: MeasurementMode,
        direction: Option<[f64; 3]>,
    ) {
        take_error_details();
        let prepared = Self::require_request_authority(&cancelled).and_then(|()| {
            if self.program_check_job.is_some() {
                return Err(failure(
                    "busy",
                    "Another geometry check is running.",
                    json!({"canonical_mutation":false}),
                ));
            }
            self.prepare_measurement(app, &expected, faces)
        });
        let measurement = match prepared {
            Ok(measurement) => measurement,
            Err(error) => {
                Self::reply(app, id, &reply, Err(error));
                return;
            }
        };
        let snapshot = app.document.current();
        let container = app.file.container_data.clone();
        let path = Self::worker_path(app);
        let worker_cancelled = cancelled.clone();
        let (sender, receiver) = mpsc::sync_channel(1);
        let repaint = context.clone();
        let spawn = std::thread::Builder::new()
            .name("ketchup-measure-faces".into())
            .spawn(move || {
                let result = evaluate(
                    measurement,
                    snapshot,
                    mode,
                    direction,
                    path,
                    container,
                    worker_cancelled,
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
        self.program_check_job = Some(ProgramCheckJob::Measuring(MeasurementJob {
            id,
            reply,
            cancelled,
            before: expected,
            started: Instant::now(),
            receiver,
        }));
        context.request_repaint_after(Duration::from_millis(10));
    }

    pub(super) fn poll_measurement(
        &mut self,
        app: &KetchupApp,
        context: &egui::Context,
        job: MeasurementJob,
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
                "Face measurement timed out; the document was not changed.",
                json!({"canonical_mutation": false}),
            ))
        } else {
            match job.receiver.try_recv() {
                Ok(result) => result.map_err(code),
                Err(mpsc::TryRecvError::Empty) => {
                    self.program_check_job = Some(ProgramCheckJob::Measuring(job));
                    context.request_repaint_after(Duration::from_millis(10));
                    return;
                }
                Err(error) => Err(failed_because("program_worker_disconnected", error)),
            }
        };
        Self::reply(app, job.id, &job.reply, result);
    }
}
