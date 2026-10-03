//! Observational busy diagnostics. Never finish or cancel human work here.
use super::*;
use std::cell::Cell;

#[derive(Clone, Copy, Default)]
pub(super) struct InputBusy {
    pointer: bool,
    keyboard: bool,
}

// The bridge dispatches synchronously on the UI thread. Keep the sampled input
// detail alongside the existing bool passed through job/publication guards.
// A false guard always ignores this sample; each context guard resamples it.
thread_local! {
    static INPUT: Cell<InputBusy> = Cell::new(InputBusy::default());
}

impl InputBusy {
    pub(super) fn is_busy(self) -> bool {
        self.pointer || self.keyboard
    }
}

pub(super) fn sample_input(context: &egui::Context) -> InputBusy {
    let input = InputBusy {
        pointer: context.is_using_pointer(),
        keyboard: context.wants_keyboard_input() && context.input(|input| input.focused),
    };
    INPUT.set(input);
    input
}

#[derive(Serialize)]
pub(super) struct Blocker {
    kind: &'static str,
    target: &'static str,
    reason: String,
    fix_hint: &'static str,
}

impl Blocker {
    fn new(kind: &'static str, target: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind,
            target,
            reason: reason.into(),
            fix_hint: match kind {
                "job" => "Wait for this job to finish, then read status before retrying.",
                "pointer" => {
                    "Wait for the user to release the pointer, then read status before retrying."
                }
                "keyboard" => {
                    "Wait for the user to leave the focused input, then read status before retrying."
                }
                _ => {
                    "Ask the user to finish this interaction; do not cancel their work automatically. Read status before retrying."
                }
            },
        }
    }
}

// Inspect raw state: validity-filtered helpers can hide unfinished human work.
// Selection is deliberately absent; choosing an object is not an edit session.
pub(super) fn blockers(app: &KetchupApp, ui_busy: bool) -> Vec<Blocker> {
    let mut out = Vec::new();
    if ui_busy {
        let input = INPUT.get();
        if input.pointer {
            out.push(Blocker::new(
                "pointer",
                "pointer",
                "The pointer is captured by a UI interaction.",
            ));
        }
        if input.keyboard {
            out.push(Blocker::new(
                "keyboard",
                "keyboard",
                "A keyboard input has focus in the active window.",
            ));
        }
        if !input.is_busy() {
            out.push(Blocker::new(
                "input",
                "window",
                "A UI input interaction is active.",
            ));
        }
    }
    let sketch = &app.gesture.sketch;
    let measure = &app.gesture.measure;
    #[rustfmt::skip]
    let conditions = [
        (app.tool_preview.is_some(), "preview", "tool_preview", "An unfinished tool preview is retained."),
        (app.push_pull.smart_proposal.is_some(), "preview", "push_pull_proposal", "A Push/Pull proposal is awaiting review."),
        (app.push_pull.smart_planning.is_some(), "job", "push_pull_planning", "Push/Pull planning is running."),
        (app.solid_tools.target.is_some(), "tool", "solid_tool_target", "A solid tool has an unfinished target selection."),
        (app.solid_tools.revolve.is_some(), "tool", "revolve", "A Revolve session is unfinished."),
        (app.active_tool == ActiveTool::Helix, "tool", "helix", "The Helix tool session is active."),
        (app.solid_tools.loft_input_sections.is_some(), "tool", "loft", "Loft input sections are retained."),
        (app.solid_tools.pocket_editor_feature.is_some(), "editor", "pocket_editor", "The pocket depth editor is open."),
        (app.parameter.editor_node.is_some(), "editor", "parameter_editor", "The parameter expression editor is open."),
        (app.parameter.provenance.is_some(), "editor", "parameter_provenance", "A parameter provenance edit is retained."),
        (app.assistant.proposal.is_some(), "preview", "assistant_proposal", "The built-in Assistant has a proposal awaiting review."),
        (app.assistant.pending_execution.is_some(), "job", "assistant_execution", "The built-in Assistant has a pending execution."),
        (app.assistant.chat_task.is_some(), "job", "assistant_chat", "The built-in Assistant is processing a request."),
        (app.gesture.drag.is_some(), "pointer", "viewport_drag", "A viewport drag is active."),
        (app.transform_gesture_active(), "tool", "transform", "A transform gesture or anchor is retained."),
        (app.camera.drag_active, "pointer", "camera_drag", "A camera drag is active."),
        (app.camera.wheel_active, "pointer", "camera_wheel", "A camera wheel interaction is active."),
        (sketch.armed || sketch.start.is_some() || sketch.end.is_some() || sketch.cursor.is_some() || sketch.chain_origin.is_some() || !sketch.chain_points.is_empty() || !sketch.chain_items.is_empty(), "tool", "sketch", "An unfinished drawing gesture or line chain is retained."),
        (app.value_box.focus, "editor", "value_box", "The tool value input is focused."),
        (measure.start.is_some() || measure.cursor.is_some() || measure.end.is_some(), "tool", "measurement", "A measurement session is retained."),
        (app.modal.is_some(), "dialog", "modal", "A modal dialog is open."),
        (app.mesh_conversion_active(), "preview", "mesh_conversion", "Mesh conversion is running or awaiting review."),
        (app.file.migration_review_plan.is_some(), "preview", "migration_review", "A document migration is awaiting review."),
        (app.assembly_preview_pending(), "preview", "assembly_preview", "An assembly preview is pending."),
        (app.body_preview_pending(), "preview", "body_preview", "A body preview is pending."),
        (app.feature_history_preview_pending(), "preview", "feature_history_preview", "A feature history preview is pending."),
        (app.face_workflow.xray_preview(), "preview", "face_xray_preview", "A face workflow X-ray preview is active."),
    ];
    for (active, kind, target, reason) in conditions {
        if active {
            let reason = if matches!(kind, "tool" | "preview") {
                format!("{reason} Current tool: {:?}.", app.active_tool)
            } else {
                reason.to_owned()
            };
            out.push(Blocker::new(kind, target, reason));
        }
    }
    out
}

pub(super) fn reject_if_busy(blockers: &[Blocker]) -> Result<(), &'static str> {
    let Some(first) = blockers.first() else {
        return Ok(());
    };
    record_rejection(
        &rejected("busy")
            .target(first.target)
            .reason(&first.reason)
            .fix_hint(first.fix_hint),
        json!({"busy_reasons": blockers}),
    );
    Err("busy")
}

impl LiveBridge {
    pub(super) fn busy_diagnostics(&self, app: &KetchupApp, ui_busy: bool) -> Vec<Blocker> {
        let mut out = blockers(app, ui_busy);
        out.extend(self.busy_jobs());
        out
    }

    pub(super) fn busy_jobs(&self) -> Vec<Blocker> {
        let mut out = Vec::new();
        for (active, target, reason) in [
            (
                self.image.is_pending(),
                "image",
                "An image request is waiting for its frame.",
            ),
            (
                self.apply_and_verify_job.is_some(),
                "apply_and_verify",
                "An apply-and-verify job is running.",
            ),
            (
                self.program_check_job.is_some(),
                if matches!(
                    self.program_check_job,
                    Some(program_check::ProgramCheckJob::Validating(_))
                ) {
                    "program_validate"
                } else {
                    if matches!(
                        self.program_check_job,
                        Some(program_check::ProgramCheckJob::Measuring(_))
                    ) {
                        "measure_faces"
                    } else {
                        "program_apply"
                    }
                },
                "A program evaluation or geometry validation is running.",
            ),
        ] {
            if active {
                out.push(Blocker::new("job", target, reason));
            }
        }
        out
    }
}
