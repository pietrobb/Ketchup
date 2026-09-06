//! Opt-in live GUI bridge v1. No independent document/session or scripting engine.
//! Wire: u32 big-endian byte length, then UTF-8 JSON `Envelope`; responses use
//! the same framing. Both directions are capped at 32 KiB. One active client,
//! one in-flight request, queue capacity 8, at most 4 requests per UI frame.
//! Authentication is required on EVERY request. Never publish credentials in
//! logs, command lines, environment variables, documents, or ordinary UI output.
//! Only a trusted embedding host should use `live_bridge_credentials` and carry
//! its result over an independently trusted out-of-band channel.
//! Proposals/receipts are connection-local; disconnect drops pending authority.
//! On a lost response, do not retry mutations blindly. Re-observe the document.
//! Images are callback-correlated CAD-only PNG thumbnails; geometry completeness is not claimed.

use crate::{AppCommand, KetchupApp, SelectionId};
use eframe::egui;
use ketchup_application::model_query::{EntityKind, ModelQuery, PageRequest};
use ketchup_core::{
    assistant_sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    },
    document::{OccurrenceId, Proposal},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::{BTreeSet, VecDeque},
    io,
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::JoinHandle,
};

pub mod bootstrap;
mod image;
#[cfg(test)]
mod tests;
mod transport;
pub const MAX_FRAME_BYTES: usize = 32 * 1024;
pub const QUEUE_CAPACITY: usize = 8;
pub const MAX_SELECTION: usize = 100;
pub const MAX_RECEIPTS: usize = 32;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Stamp {
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
    pub mutation_epoch: u64,
}

/// Intentionally no Debug or Serialize implementation: trusted API only.
pub struct Credentials {
    pub address: SocketAddr,
    pub token: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Envelope {
    pub version: u32,
    pub id: u64,
    pub token: String,
    pub request: Request,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "method", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    Status {},
    Summary {},
    Query {
        expected: Stamp,
        query: PageRequest,
    },
    Detail {
        expected: Stamp,
        kind: EntityKind,
        entity_id: u64,
    },
    WorksetCreate {
        expected: Stamp,
        query: PageRequest,
    },
    WorksetStatus {
        expected: Stamp,
        handle: String,
    },
    Propose {
        expected: Stamp,
        selection: Vec<u64>,
        program: AssistantCadEditProgram,
    },
    Commit {
        expected: Stamp,
        proposal_id: u64,
    },
    Undo {
        expected: Stamp,
    },
    Redo {
        expected: Stamp,
    },
    Selection {
        expected: Stamp,
        occurrence_ids: Vec<u64>,
    },
    View {
        expected: Stamp,
        view: View,
    },
    Image {
        expected: Stamp,
        #[serde(default)]
        capture_mode: CaptureMode,
    },
    Disconnect {},
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    Offscreen,
    VisibleViewport,
}

impl CaptureMode {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Offscreen => "offscreen",
            Self::VisibleViewport => "visible_viewport",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum View {
    Iso,
    Top,
    Front,
    ZoomFit,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub version: u32,
    pub id: u64,
    pub ok: bool,
    pub stamp: Option<Stamp>,
    pub result: Option<Value>,
    pub error: Option<String>,
}
impl Response {
    fn error(id: u64, code: &str) -> Self {
        Self {
            version: 1,
            id,
            ok: false,
            stamp: None,
            result: None,
            error: Some(code.into()),
        }
    }
}

struct Queued {
    session: u64,
    id: u64,
    request: Request,
    cancelled: Arc<AtomicBool>,
    reply: mpsc::SyncSender<Response>,
}
struct Pending {
    id: u64,
    stamp: Stamp,
    selection: Vec<u64>,
    primary: Option<SelectionId>,
    proposal: Proposal,
}
struct Receipt {
    id: u64,
    expected: Stamp,
    value: Value,
}
pub(crate) struct LiveBridge {
    address: SocketAddr,
    token: String,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    queue: mpsc::Receiver<Queued>,
    query: ModelQuery,
    observed: Option<Stamp>,
    session: u64,
    pending: Option<Pending>,
    next_proposal: u64,
    receipts: VecDeque<Receipt>,
    image: image::ImageState,
}
impl Drop for LiveBridge {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // Idle/frame reads poll stop; writes have a total two-second deadline. Never join on the UI.
        self.worker.take();
    }
}

impl KetchupApp {
    /// Explicit opt-in by a trusted UI-thread host; binds ONLY 127.0.0.1:0.
    /// Generates fresh credentials; launcher-supplied tokens use the separate bootstrap API.
    pub fn enable_live_bridge(&mut self, context: &egui::Context) -> io::Result<SocketAddr> {
        if self.live_bridge.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "bridge already enabled",
            ));
        }
        let bridge = transport::start(context.clone())?;
        let address = bridge.address;
        self.live_bridge = Some(bridge);
        Ok(address)
    }

    /// Secret-bearing out-of-band accessor for trusted host code only.
    pub fn live_bridge_credentials(&self) -> Option<Credentials> {
        self.live_bridge.as_ref().map(|b| Credentials {
            address: b.address,
            token: b.token.clone(),
        })
    }

    pub fn disable_live_bridge(&mut self) {
        self.live_bridge = None;
    }

    pub fn live_bridge_stamp(&self) -> Stamp {
        let snapshot = self.document.current();
        Stamp {
            document_id: snapshot.document_id().0,
            revision: snapshot.revision_id(),
            canonical_digest: snapshot.canonical_digest(),
            mutation_epoch: self.document.mutation_epoch(),
        }
    }

    pub(crate) fn poll_live_bridge(&mut self, context: &egui::Context) {
        let Some(mut bridge) = self.live_bridge.take() else {
            return;
        };
        for _ in 0..4 {
            let Ok(queued) = bridge.queue.try_recv() else {
                break;
            };
            if queued.cancelled.load(Ordering::Acquire) {
                continue;
            }
            if bridge.session != queued.session {
                bridge.session = queued.session;
                bridge.pending = None;
                bridge.image.revoke();
                bridge.receipts.clear();
                bridge.query.invalidate();
            }
            let stamp = self.live_bridge_stamp();
            if bridge.observed.as_ref() != Some(&stamp) {
                bridge.query.invalidate();
                bridge.observed = Some(stamp);
            }
            if matches!(queued.request, Request::Image { .. }) {
                bridge.request_image(self, context, queued);
                continue;
            }
            let result = bridge.execute(
                self,
                queued.request,
                context.wants_keyboard_input() || context.is_using_pointer(),
            );
            let mut response = match result {
                Ok(value) => Response {
                    version: 1,
                    id: queued.id,
                    ok: true,
                    stamp: None,
                    result: Some(value),
                    error: None,
                },
                Err(code) => Response::error(queued.id, code),
            };
            response.stamp = Some(self.live_bridge_stamp());
            let _ = queued.reply.try_send(response);
            context.request_repaint();
        }
        self.live_bridge = Some(bridge);
    }
}

impl LiveBridge {
    // Deliberately inspect raw state: validity-filtered preview helpers can hide stale human work.
    fn busy(app: &KetchupApp) -> bool {
        app.preview.is_some()
            || app.preview_box.is_some()
            || app.preview_definition_id.is_some()
            || app.smart_push_pull_proposal.is_some()
            || app.smart_push_pull_planning.is_some()
            || app.smart_push_pull_chooser.is_some()
            || app.occurrence_operation_preview.is_some()
            || app.solid_tool_target.is_some()
            || app.revolve_tool.is_some()
            || app.revolve_preview.is_some()
            || app.planar_offset_preview.is_some()
            || app.sweep_preview.is_some()
            || app.loft_input_sections.is_some()
            || app.loft_preview.is_some()
            || app.general_finish_preview.is_some()
            || app.pocket_preview.is_some()
            || app.pocket_editor_feature.is_some()
            || app.parameter_editor_node.is_some()
            || app.parameter_provenance.is_some()
            || app.assistant_proposal.is_some()
            || app.assistant_pending_execution.is_some()
            || app.assistant_chat_task.is_some()
            || app.push_pull_drag.is_some()
            || app.push_pull_anchor.is_some()
            || app.move_drag.is_some()
            || app.move_anchor.is_some()
            || app.rotate_drag.is_some()
            || app.rotate_anchor.is_some()
            || app.camera_drag_active
            || app.camera_wheel_active
            || app.zoom_window_start.is_some()
            || app.zoom_window_cursor.is_some()
            || app.sketch_mode
            || app.sketch_start.is_some()
            || app.sketch_end.is_some()
            || app.sketch_cursor.is_some()
            || app.line_chain_origin.is_some()
            || !app.line_chain_points.is_empty()
            || !app.line_chain_items.is_empty()
            || app.focus_value_box
            || app.measure_start.is_some()
            || app.measure_cursor.is_some()
            || app.measure_end.is_some()
            || app.pending_definition_rename.is_some()
            || app.pending_occurrence_rename.is_some()
            || app.pending_component_replacement.is_some()
            || app.pending_tag_creation.is_some()
            || app.pending_tag_deletion.is_some()
            || app.pending_tag_clear.is_some()
            || app.pending_tag_rename.is_some()
            || app.pending_tag_assignment.is_some()
            || app.pending_occurrence_align.is_some()
            || app.pending_occurrence_distribution.is_some()
            || app.pending_linear_pattern.is_some()
            || app.pending_rectangular_pattern.is_some()
            || app.pending_circular_pattern.is_some()
            || app.pending_stl_import.is_some()
            || app.pending_dxf_import.is_some()
            || app.pending_step_import.is_some()
            || app.pending_sketchup_scene_import.is_some()
            || app.migration_review_plan.is_some()
            || app.assembly_preview_pending()
            || app.body_preview_pending()
            || app.feature_history_preview_pending()
            || app.face_workflow.xray_preview()
            || Self::fixture_busy(app)
    }
    #[cfg(feature = "named-product-fixtures")]
    fn fixture_busy(app: &KetchupApp) -> bool {
        app.bottle_direct_drag.is_some()
            || app.bottle_editor.is_some()
            || app.part_authoring_preview_pending()
    }
    #[cfg(not(feature = "named-product-fixtures"))]
    fn fixture_busy(_app: &KetchupApp) -> bool {
        false
    }
    fn available(app: &KetchupApp, ui_busy: bool) -> Result<(), &'static str> {
        if app.review_candidate.is_some() {
            return Err("read_only_document");
        }
        if ui_busy || Self::busy(app) {
            return Err("busy");
        }
        Ok(())
    }
    fn guard(app: &KetchupApp, expected: &Stamp) -> Result<(), &'static str> {
        if &app.live_bridge_stamp() != expected {
            return Err("stale_document");
        }
        Ok(())
    }

    fn selection(app: &KetchupApp) -> Result<Vec<u64>, &'static str> {
        if !app.selection.edit_context.is_empty()
            || app.selection.selected_group.is_some()
            || !app.selection.topological.is_empty()
            || app.selected_instance_paths().iter().any(|p| !p.is_root())
        {
            return Err("unsupported_selection_scope");
        }
        let selected = app.selected_occurrence_ids();
        if selected.len() > MAX_SELECTION {
            return Err("selection_limit");
        }
        let ids: Vec<_> = selected.iter().map(|id| id.0).collect();
        Self::root_ids(app, &ids)?;
        Ok(ids)
    }

    fn root_ids(app: &KetchupApp, ids: &[u64]) -> Result<(), &'static str> {
        if ids.is_empty() {
            return Ok(());
        }
        let snapshot = app.document.current();
        for id in ids {
            let occurrence = snapshot
                .occurrence(OccurrenceId(*id))
                .ok_or("entity_not_found")?;
            if occurrence.parent().is_some() {
                return Err("unsupported_selection_scope");
            }
        }
        let selectable: BTreeSet<_> = app
            .active_scene_query()
            .into_iter()
            .filter(|o| o.visible && o.parent.is_none() && o.instance_path.is_root())
            .map(|o| o.instance_path.root_occurrence().0)
            .collect();
        if ids.iter().any(|id| !selectable.contains(id)) {
            return Err("unsupported_selection_scope");
        }
        Ok(())
    }
    fn program_scope(
        app: &KetchupApp,
        program: &AssistantCadEditProgram,
    ) -> Result<(), &'static str> {
        for operation in &program.operations {
            let selector = match operation {
                AssistantCadEditOperation::Delete { selector, .. }
                | AssistantCadEditOperation::Transform { selector, .. }
                | AssistantCadEditOperation::SetColor { selector, .. }
                | AssistantCadEditOperation::Copy { selector, .. }
                | AssistantCadEditOperation::LinearPattern { selector, .. }
                | AssistantCadEditOperation::Mirror { selector, .. } => Some(selector),
                AssistantCadEditOperation::CreateSketch { .. }
                | AssistantCadEditOperation::CreatePart { .. }
                | AssistantCadEditOperation::AppendFeature { .. }
                | AssistantCadEditOperation::SetDimension { .. } => None,
            };
            if let Some(AssistantCadEntitySelector::Occurrences { occurrence_ids }) = selector {
                Self::root_ids(app, occurrence_ids)?;
            }
        }
        Ok(())
    }
    fn validate_ids(ids: &[u64]) -> Result<Vec<u64>, &'static str> {
        if ids.len() > MAX_SELECTION || ids.contains(&0) {
            return Err("invalid_selection");
        }
        let sorted: BTreeSet<_> = ids.iter().copied().collect();
        if sorted.len() != ids.len() {
            return Err("invalid_selection");
        }
        Ok(sorted.into_iter().collect())
    }

    fn execute(
        &mut self,
        app: &mut KetchupApp,
        request: Request,
        ui_busy: bool,
    ) -> Result<Value, &'static str> {
        match request {
            Request::Status {} => Ok(
                json!({"connected":true,"protocol":1,"image":"cad_viewport_png_thumbnail","image_capture_modes":["offscreen","visible_viewport"],"default_image_capture_mode":"offscreen","busy":ui_busy || Self::busy(app),"read_only":app.review_candidate.is_some(),
                "selection":Self::selection(app).ok(),"selection_scope":"root_occurrences_only",
                "undo_steps":app.undo_step_count(),"redo_steps":app.redo_step_count(),
                "pending_proposal_id":self.pending.as_ref().map(|p|p.id),
                "limits":{"frame_bytes":MAX_FRAME_BYTES,"queue":QUEUE_CAPACITY,"receipts":MAX_RECEIPTS,"selection":MAX_SELECTION},
                "methods":["status","summary","query","detail","workset_create","workset_status","propose","commit","undo","redo","selection","view","image","disconnect"]}),
            ),
            Request::Summary {} => Ok(self.query.summary(&app.document.current())),
            Request::Query { expected, query } => {
                Self::guard(app, &expected)?;
                self.query
                    .page(&app.document.current(), &query)
                    .map_err(|e| e.code())
            }
            Request::Detail {
                expected,
                kind,
                entity_id,
            } => {
                Self::guard(app, &expected)?;
                self.query
                    .detail(&app.document.current(), kind, entity_id)
                    .map_err(|e| e.code())
            }
            Request::WorksetCreate { expected, query } => {
                Self::guard(app, &expected)?;
                self.query
                    .create_workset(&app.document.current(), &query)
                    .map_err(|e| e.code())
            }
            Request::WorksetStatus { expected, handle } => {
                Self::guard(app, &expected)?;
                self.query
                    .workset_status(&app.document.current(), &handle)
                    .map_err(|e| e.code())
            }
            Request::Propose {
                expected,
                selection,
                program,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                // No GUI-owned edit state is cleared while unavailable.
                // Receipt replay above remains observational.
                let selection = Self::validate_ids(&selection)?;
                if selection != Self::selection(app)? {
                    return Err("selection_changed");
                }
                // Existing typed CAD validation bounds operations, selectors and geometry.
                program.validate().map_err(|_| "invalid_program")?;
                Self::program_scope(app, &program)?;
                let proposal = app
                    .derive_assistant_cad_edit_proposal(&program)
                    .map_err(|_| "planning_rejected")?;
                let id = self.next_proposal;
                self.next_proposal = id.checked_add(1).ok_or("proposal_ids_exhausted")?;
                let value = json!({"proposal_id":id,"observational":true,"selection":selection,
                    "command_digest":proposal.command_digest(),"result_digest":proposal.intended_result_digest(),
                    "write_count":proposal.authoritative_writes().len(),"image":"not_requested"});
                self.pending = Some(Pending {
                    id,
                    stamp: expected,
                    selection,
                    primary: app.selection.primary.clone(),
                    proposal,
                });
                Ok(value)
            }
            Request::Commit {
                expected,
                proposal_id,
            } => {
                // A retained receipt is returned even after later human edits, with its
                // original commit stamp. Never execute the proposal for a second time.
                if let Some(receipt) = self.receipts.iter().find(|r| r.id == proposal_id) {
                    if receipt.expected != expected {
                        return Err("receipt_guard_mismatch");
                    }
                    return Ok(receipt.value.clone());
                }
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                // No GUI-owned edit state is cleared while unavailable.
                // Receipt replay above remains observational.
                let pending = self.pending.as_ref().ok_or("proposal_not_found")?;
                if pending.id != proposal_id {
                    return Err("proposal_not_found");
                }
                if pending.stamp != expected {
                    return Err("stale_document");
                }
                if pending.selection != Self::selection(app)?
                    || pending.primary != app.selection.primary
                {
                    return Err("selection_changed");
                }
                let pending = self.pending.take().expect("checked pending proposal");
                let committed = app
                    .document
                    .commit_verified_proposal(&pending.proposal)
                    .map_err(|_| "commit_rejected")?;
                let value = json!({"proposal_id":proposal_id,"committed":true,"verified":true,
                    "before":expected,"after":app.live_bridge_stamp(),"command_digest":committed.command_digest(),
                    "result_digest":committed.result_digest(),"write_count":committed.verified_writes().len(),
                    "undo_steps":app.undo_step_count(),"geometry_evaluated":false,"image":"not_requested"});
                app.invalidate_pending_import_reviews();
                app.clear_ephemeral_edit_state();
                app.reconcile_selection();
                app.status_key = "status-ready";
                if self.receipts.len() == MAX_RECEIPTS {
                    self.receipts.pop_front();
                }
                self.receipts.push_back(Receipt {
                    id: proposal_id,
                    expected,
                    value: value.clone(),
                });
                Ok(value)
            }
            Request::Undo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Undo) {
                    return Err("undo_unavailable");
                }
                Ok(json!({"changed":app.undo()}))
            }
            Request::Redo { expected } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                if !app.command_enabled(AppCommand::Redo) {
                    return Err("redo_unavailable");
                }
                Ok(json!({"changed":app.redo()}))
            }
            Request::Selection {
                expected,
                occurrence_ids,
            } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                Self::selection(app)?;
                let ids = Self::validate_ids(&occurrence_ids)?;
                Self::root_ids(app, &ids)?;
                let snapshot = app.document.current();
                if ids
                    .iter()
                    .any(|id| snapshot.occurrence(OccurrenceId(*id)).is_none())
                {
                    return Err("entity_not_found");
                }
                app.selection.clear();
                for id in &ids {
                    app.selection.select_occurrence(OccurrenceId(*id), true);
                }
                // An explicit selection command revokes proposals even if it reselects
                // the same IDs; never silently retarget an immutable proposal.
                self.pending = None;
                Ok(json!({"occurrence_ids":ids,"canonical_mutation":false}))
            }
            Request::View { expected, view } => {
                Self::guard(app, &expected)?;
                Self::available(app, ui_busy)?;
                let command = match view {
                    View::Iso => AppCommand::ViewIso,
                    View::Top => AppCommand::ViewTop,
                    View::Front => AppCommand::ViewFront,
                    View::ZoomFit => AppCommand::ZoomFit,
                };
                if !app.command_enabled(command) {
                    return Err("view_unavailable");
                }
                app.dispatch_command(command);
                Ok(json!({"view":view,"canonical_mutation":false,"image":"not_requested"}))
            }
            Request::Image { expected, .. } => {
                Self::guard(app, &expected)?;
                Err("image_requires_frame_callback")
            }
            Request::Disconnect {} => {
                self.pending = None;
                self.receipts.clear();
                self.query.invalidate();
                Ok(json!({"disconnected":true}))
            }
        }
    }
}
