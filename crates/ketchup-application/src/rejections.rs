//! Every error code a Kečup host answers a client with, as the shared
//! [`Rejection`]: what the code means, which request field it is about and what
//! the client should change. This table is the one source of the codes; the
//! Python SDK's accepted codes are checked against it.

use ketchup_rejection::{Rejection, RejectionPhase};

/// One host error code with its meaning and repair.
pub struct HostRejection {
    pub code: &'static str,
    pub phase: RejectionPhase,
    pub target: &'static str,
    pub reason: &'static str,
    pub fix_hint: &'static str,
}

impl HostRejection {
    #[must_use]
    pub fn rejection(&self) -> Rejection {
        Rejection::new(self.code, self.phase)
            .target(self.target)
            .reason(self.reason)
            .fix_hint(self.fix_hint)
    }
}

const fn entry(
    code: &'static str,
    phase: RejectionPhase,
    target: &'static str,
    reason: &'static str,
    fix_hint: &'static str,
) -> HostRejection {
    HostRejection {
        code,
        phase,
        target,
        reason,
        fix_hint,
    }
}

use RejectionPhase::{Commit, Evaluation, Io, Planning, Request, Validation, Verification};

/// Every host error code, sorted by code.
pub const HOST_REJECTIONS: &[HostRejection] = &[
    entry(
        "apply_and_verify_busy",
        Validation,
        "window",
        "Another apply_and_verify job is still running in this window.",
        "Wait for its answer, then send the edit again.",
    ),
    entry(
        "apply_and_verify_worker_disconnected",
        Io,
        "window",
        "The apply_and_verify job stopped before it answered.",
        "Re-read status to see whether the edit was published before sending it again.",
    ),
    entry(
        "batch_cancelled",
        Commit,
        "job_handle",
        "The batch job was cancelled; its remaining steps did not run.",
        "Start a new batch job for the occurrences that still need the change.",
    ),
    entry(
        "batch_job_ids_exhausted",
        Planning,
        "job_handle",
        "No more batch job handles are available on this connection.",
        "Disconnect and attach again.",
    ),
    entry(
        "batch_job_limit",
        Validation,
        "job_handle",
        "Too many batch jobs are open at once.",
        "Finish or cancel an open batch job, then start the new one.",
    ),
    entry(
        "batch_job_not_found",
        Validation,
        "job_handle",
        "No open batch job has that handle.",
        "Use the handle returned by batch_job_start on this connection.",
    ),
    entry(
        "batch_transaction_failed",
        Commit,
        "job_handle",
        "The batch step could not be published to the document.",
        "Re-read status and start the batch job again.",
    ),
    entry(
        "busy",
        Validation,
        "window",
        "The window is busy with an earlier request or a user interaction.",
        "Wait until it finishes, then send the request again.",
    ),
    entry(
        "candidate_rejected",
        Verification,
        "program",
        "The planned edit would leave the document invalid; nothing was published.",
        "Change the program so every reference, dimension and placement stays valid.",
    ),
    entry(
        "capability_gap",
        Planning,
        "program",
        "The program asks for a result the modeler cannot build yet; nothing was published.",
        "Express the change with other operations, or report the missing capability.",
    ),
    entry(
        "commit_rejected",
        Commit,
        "proposal_id",
        "The proposal could not be published to the document.",
        "Re-read status, propose again and commit the new proposal.",
    ),
    entry(
        "cross_query_cursor",
        Request,
        "cursor",
        "The cursor belongs to a different query.",
        "Send the cursor with the unchanged query that returned it, or drop the cursor.",
    ),
    entry(
        "drawings_target_exists",
        Request,
        "path",
        "A file already exists at the drawing path; it is never replaced without the user's confirmation.",
        "Export to a new file name, or let the user export from the window, which asks before replacing.",
    ),
    entry(
        "drawings_unavailable",
        Validation,
        "document",
        "There are no visible exact solids to draw yet.",
        "Show at least one layer and wait until inspect action=status reports the exact evaluation complete, then export again.",
    ),
    entry(
        "drawings_write_failed",
        Io,
        "path",
        "The PDF could not be written.",
        "Choose a writable absolute path ending in .pdf and export again.",
    ),
    entry(
        "entity_not_found",
        Validation,
        "entity_id",
        "No entity with that id exists in the current document.",
        "Query the current document for ids and send the request again.",
    ),
    entry(
        "exact_evaluation_incomplete",
        Evaluation,
        "program",
        "Some changed parts were not evaluated exactly before the deadline.",
        "Send the edit again with a larger timeout_ms.",
    ),
    entry(
        "exact_evaluation_rejected",
        Evaluation,
        "program",
        "The exact solids of the edit could not be built; nothing was published.",
        "Give the changed parts valid solids: positive sizes and closed, non-crossing profiles.",
    ),
    entry(
        "exact_reference_rejected",
        Commit,
        "program",
        "The exact evidence of an edited part could not be recorded.",
        "Re-read status and send the edit again.",
    ),
    entry(
        "exact_worker_disconnected",
        Io,
        "exact_worker",
        "The exact geometry worker stopped during the job.",
        "Send the request again; simplify the changed parts if the worker keeps stopping.",
    ),
    entry(
        "exact_worker_unavailable",
        Io,
        "exact_worker",
        "The exact geometry worker could not be started.",
        "Check that the worker executable is installed beside the app, then retry.",
    ),
    entry(
        "hidden_viewport",
        Validation,
        "capture_mode",
        "The viewport is hidden, minimized or off screen, so it cannot be captured visibly.",
        "Show the window on screen, or request capture_mode offscreen.",
    ),
    entry(
        "image_requires_frame_callback",
        Request,
        "method",
        "Images are captured during the window's paint cycle, not by this call.",
        "Request the image with the image method.",
    ),
    entry(
        "image_timeout",
        Evaluation,
        "image",
        "The window did not paint the requested image in time.",
        "Keep the window open and not minimized, then request the image again.",
    ),
    entry(
        "image_unavailable",
        Io,
        "image",
        "The window could not prepare an image capture.",
        "Request the image again; restart the window if it keeps failing.",
    ),
    entry(
        "incomplete_image",
        Evaluation,
        "image",
        "The rendered image has pixels the renderer did not cover.",
        "Request the image again after the window finishes painting.",
    ),
    entry(
        "incomplete_workset",
        Validation,
        "workset_handle",
        "The workset does not hold every occurrence its query matched.",
        "Create the workset again with a narrower query.",
    ),
    entry(
        "invalid_cursor",
        Request,
        "cursor",
        "The cursor is not one this host issued.",
        "Send the next_cursor from the previous page unchanged, or drop the cursor.",
    ),
    entry(
        "invalid_image_callback",
        Evaluation,
        "image",
        "The renderer answered with a frame that does not match the request.",
        "Request the image again.",
    ),
    entry(
        "invalid_image_dimensions",
        Request,
        "max_side_px",
        "The requested image size is outside the supported range.",
        "Choose max_side_px inside the range the image method documents.",
    ),
    entry(
        "invalid_image_framing",
        Request,
        "framing",
        "The framing has nothing to frame: the selection is empty or the detail is unknown.",
        "Select occurrences for selection framing, or pass a detail from a topology query for detail_selection.",
    ),
    entry(
        "invalid_job_timeout",
        Request,
        "timeout_ms",
        "The job timeout is zero or above the supported maximum.",
        "Pass a positive timeout_ms inside the documented maximum.",
    ),
    entry(
        "invalid_params",
        Request,
        "params",
        "The request parameters do not match the method's schema.",
        "Fix the parameter named in the reason and send the request again.",
    ),
    entry(
        "invalid_path",
        Request,
        "path",
        "The path is empty or not absolute.",
        "Pass an explicit absolute path.",
    ),
    entry(
        "invalid_program",
        Validation,
        "program",
        "The edit program breaks a rule of one of its operations.",
        "Fix the operation named in the reason; the operations method lists every field.",
    ),
    entry(
        "invalid_request",
        Request,
        "request",
        "The frame is not a valid bridge request envelope.",
        "Send one JSON envelope with version, id, token, method and params per frame.",
    ),
    entry(
        "invalid_selection",
        Request,
        "selection",
        "The selection lists a zero id or more ids than one request may carry.",
        "List existing root occurrence ids from status or query.",
    ),
    entry(
        "invalid_sheet_format",
        Request,
        "format",
        "The sheet format is not one of auto, A3, A2, A1 or A0.",
        "Pass format as auto, A3, A2, A1 or A0, or omit it to keep the document's format.",
    ),
    entry(
        "job_timeout",
        Evaluation,
        "timeout_ms",
        "The job did not finish before its deadline.",
        "Re-read status to see what was published, then retry with a larger timeout_ms or a smaller change.",
    ),
    entry(
        "job_worker_unavailable",
        Io,
        "window",
        "The window could not start a background job.",
        "Send the request again; restart the window if it keeps failing.",
    ),
    entry(
        "missing_workset_identity",
        Validation,
        "workset_handle",
        "An occurrence of the workset no longer has the identity it was collected with.",
        "Create the workset again from the current document.",
    ),
    entry(
        "open_rejected",
        Validation,
        "path",
        "The window did not open the file.",
        "Pass an existing Kečup document and confirm discarding unsaved changes when asked.",
    ),
    entry(
        "output_too_large",
        Io,
        "query",
        "The answer is larger than one response may be.",
        "Lower the limit or narrow the query, then send it again.",
    ),
    entry(
        "planning_rejected",
        Planning,
        "program",
        "No edit could be planned for the program; nothing was published.",
        "Change the operation named in the reason as its hint says.",
    ),
    entry(
        "program_owned_document",
        Planning,
        "program",
        "A typed edit would detach the Starlark program that owns this document; nothing was published.",
        "Change the parts through program action=apply, or repeat without strict to detach the program.",
    ),
    entry(
        "program_rejected",
        Validation,
        "program",
        "The program was rejected; nothing was published.",
        "Fix the line named in the reason and send the whole program again.",
    ),
    entry(
        "program_worker_disconnected",
        Io,
        "program",
        "The program planner stopped before returning a result; nothing was published.",
        "Send the request again; simplify the program if the planner keeps stopping.",
    ),
    entry(
        "proposal_ids_exhausted",
        Planning,
        "proposal_id",
        "No more proposal ids are available on this connection.",
        "Disconnect and attach again.",
    ),
    entry(
        "proposal_not_found",
        Validation,
        "proposal_id",
        "No pending proposal has that id.",
        "Propose again and commit the proposal_id it returns.",
    ),
    entry(
        "queue_unavailable",
        Io,
        "window",
        "The window stopped accepting requests.",
        "Attach again; the window may be closing.",
    ),
    entry(
        "read_only_document",
        Validation,
        "document",
        "A recovered document is open for review and cannot be edited.",
        "Accept or discard the recovered document in the window first.",
    ),
    entry(
        "recovery_rejected",
        Commit,
        "document",
        "The change could not be recorded for crash recovery, so it was not published.",
        "Free disk space or fix the permissions of the recovery folder, then send the change again.",
    ),
    entry(
        "redo_unavailable",
        Validation,
        "document",
        "There is nothing to redo.",
        "Check status before redoing.",
    ),
    entry(
        "request_cancelled",
        Request,
        "request",
        "The request was cancelled or its connection closed before it ran.",
        "Send the request again on an open connection.",
    ),
    entry(
        "response_limit",
        Io,
        "response",
        "The answer does not fit in one response frame.",
        "Narrow the request (smaller limit, fewer targets or a smaller image) and send it again.",
    ),
    entry(
        "response_timeout",
        Io,
        "window",
        "The window did not answer in time: it is still loading or evaluating, or a dialog in the window is waiting for the user. The request was revoked unless it had already started.",
        "The connection stays open: read inspect action=status (it answers once the window is free) before sending the request again.",
    ),
    entry(
        "save_path_required",
        Validation,
        "save",
        "The window has no file bound to this document.",
        "Save with save_as and an explicit absolute path.",
    ),
    entry(
        "save_rejected",
        Io,
        "save",
        "The document could not be written.",
        "Choose a writable path and save again.",
    ),
    entry(
        "selection_changed",
        Validation,
        "selection",
        "The window selection differs from the asserted selection.",
        "Re-read status and assert the current selection, or omit selection.",
    ),
    entry(
        "selection_hidden",
        Validation,
        "occurrence_ids",
        "An occurrence lies on a hidden layer (tag) or is hidden, so it cannot be selected.",
        "Show its layer with view action=tag_visibility first, or leave it out.",
    ),
    entry(
        "selection_limit",
        Validation,
        "selection",
        "The window selection holds more occurrences than one request may carry.",
        "Select fewer occurrences in the window, or omit selection.",
    ),
    entry(
        "stale_batch_task",
        Validation,
        "job_handle",
        "The document changed since the batch job started.",
        "Cancel the job and start a new one from the current document.",
    ),
    entry(
        "stale_cursor",
        Validation,
        "cursor",
        "The document changed since the cursor was issued.",
        "Send the query again without a cursor.",
    ),
    entry(
        "stale_document",
        Validation,
        "expected",
        "The document changed since that stamp.",
        "Re-read status and send its stamp as expected.",
    ),
    entry(
        "stale_image",
        Validation,
        "expected",
        "The model or view changed while the image was being rendered.",
        "Re-read status and request the image again.",
    ),
    entry(
        "stale_workset",
        Validation,
        "workset_handle",
        "The document changed since the workset was created.",
        "Create the workset again from the current document.",
    ),
    entry(
        "unauthorized",
        Request,
        "token",
        "The session token does not belong to this window.",
        "Attach again to receive a token for this window.",
    ),
    entry(
        "undo_unavailable",
        Validation,
        "document",
        "There is nothing to undo.",
        "Check status before undoing.",
    ),
    entry(
        "unknown_operation",
        Request,
        "operation",
        "No operation has that name.",
        "List the operations with the operations method and use one of their names.",
    ),
    entry(
        "unknown_validator",
        Request,
        "validators",
        "A validator id is not known.",
        "Use validator ids from the validators discovery.",
    ),
    entry(
        "unsupported_image_protocol",
        Request,
        "image_protocol_version",
        "The image protocol version is not the one this window speaks.",
        "Send the image protocol version reported by status.",
    ),
    entry(
        "unsupported_image_renderer",
        Evaluation,
        "image",
        "The renderer painted the frame more than once, so the capture is ambiguous.",
        "Request the image again.",
    ),
    entry(
        "unsupported_selection_scope",
        Validation,
        "selection",
        "The selection contains nested instances; only root occurrences can be asserted.",
        "Select root occurrences only, or omit selection.",
    ),
    entry(
        "unsupported_version",
        Request,
        "version",
        "The request protocol version is not supported.",
        "Send protocol version 1.",
    ),
    entry(
        "unsupported_workset_scope",
        Validation,
        "workset_handle",
        "Worksets hold root occurrences or instances only.",
        "Create the workset from an occurrences or instances query.",
    ),
    entry(
        "validation_failed",
        Verification,
        "validators",
        "A validator reported issues; with strict the edit was not published.",
        "Fix the issues listed in details and send the edit again.",
    ),
    entry(
        "validation_incomplete",
        Verification,
        "validators",
        "Not every validator could finish; with strict the edit was not published.",
        "Send the edit again with a larger timeout_ms, or without strict.",
    ),
    entry(
        "view_unavailable",
        Validation,
        "view",
        "The view command is not available right now.",
        "Wait until the window shows a document with no dialog open, then retry.",
    ),
    entry(
        "workset_not_found",
        Validation,
        "workset_handle",
        "No workset has that handle.",
        "Use the handle returned by workset_create on this connection.",
    ),
];

/// The catalog entry of `code`, if the host knows it.
#[must_use]
pub fn host_rejection(code: &str) -> Option<&'static HostRejection> {
    HOST_REJECTIONS
        .binary_search_by(|entry| entry.code.cmp(code))
        .ok()
        .map(|index| &HOST_REJECTIONS[index])
}

/// The rejection for `code`. A code missing from the catalog still names
/// itself and tells the client how to recover.
#[must_use]
pub fn rejected(code: &str) -> Rejection {
    host_rejection(code).map_or_else(
        || {
            Rejection::new(code.to_owned(), Request)
                .target("request")
                .fix_hint("Re-read status and send the request again.")
        },
        HostRejection::rejection,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_sorted_unique_and_complete() {
        for pair in HOST_REJECTIONS.windows(2) {
            assert!(
                pair[0].code < pair[1].code,
                "{} / {}",
                pair[0].code,
                pair[1].code
            );
        }
        for entry in HOST_REJECTIONS {
            assert!(
                !entry.target.is_empty() && !entry.reason.is_empty() && !entry.fix_hint.is_empty(),
                "{} lacks a target, reason or fix hint",
                entry.code
            );
            let rejection = rejected(entry.code);
            assert_eq!(rejection.code(), entry.code);
            assert_eq!(rejection.phase(), entry.phase);
        }
    }

    #[test]
    fn an_unknown_code_still_names_a_target_and_a_fix() {
        let rejection = rejected("not_in_catalog");
        assert_eq!(rejection.code(), "not_in_catalog");
        assert!(!rejection.target_name().is_empty());
        assert!(!rejection.fix_hint_text().is_empty());
    }
}
