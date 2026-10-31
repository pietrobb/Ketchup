//! The shell state of [`crate::KetchupApp`] grouped by what it serves, so each
//! part of the window owns one small struct instead of loose fields.

use super::*;

/// The open file: where it lives, what was saved, and the recovery and
/// migration reviews around opening it.
pub(crate) struct FileState {
    pub(crate) activity: crate::app::file_activity::FileActivity,
    pub(crate) container_data: ketchup_model::persistence::ContainerData,
    pub(crate) review_candidate: Option<ketchup_model::persistence::LoadOutcome>,
    pub(crate) migration_review_plan: Option<MigrationReviewPlan>,
    pub(crate) recovery_open: Option<RecoveryOpenState>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) identity: Option<ketchup_model::persistence::FileIdentity>,
    pub(crate) work_recovery_identity: Option<ketchup_model::persistence::FileIdentity>,
    pub(crate) work_recovery_lock: Option<ketchup_model::persistence::WorkRecoveryLock>,
    /// The file's folder is read-only, so it was opened without a work-recovery
    /// checkpoint and edits are not checkpointed until it is saved elsewhere.
    pub(crate) work_recovery_off: bool,
    pub(crate) pending_work_recovery_cleanup:
        Option<(PathBuf, ketchup_model::persistence::FileIdentity)>,
    pub(crate) work_recovery_digest: Option<String>,
    pub(crate) saved_digest: String,
}

/// A file opened from a recovery copy in place of the one the user asked for.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecoveryOpenState {
    pub(crate) requested_path: PathBuf,
    pub(crate) source_path: PathBuf,
    /// The file a work-recovery checkpoint opened in its place was made from;
    /// the window's own checkpoints are made from it too. None for a backup.
    pub(crate) work_recovery_base: Option<ketchup_model::persistence::FileIdentity>,
}

/// The live AI bridge and the consent that attaches it.
pub(crate) struct LiveState {
    pub(crate) bridge: Option<live_bridge::LiveBridge>,
    pub(crate) consent_broker: Option<live_bridge::consent::ConsentBroker>,
    pub(crate) consent_attached: bool,
    /// The client that started this window (`open_window`) holds it until its
    /// connection ends; another client's attach then takes the window over.
    pub(crate) launch_attached: bool,
    /// An export a client asked for is running: its loss report goes back to the
    /// client instead of a lossy-conversion prompt, and it never replaces a file.
    /// Holds the request's cancel flag, set once the client's connection ends.
    pub(crate) client_export: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Plugins the user allowed in this window, each bound to its files' hashes
    /// and the permissions shown; nothing is kept after the window closes.
    pub(crate) plugin_consents: Vec<live_bridge::plugin_consent::PluginConsent>,
    pub(crate) plugin_menu: live_bridge::plugin_menu::PluginMenu,
    pub(crate) variants: live_bridge::variant_window::VariantDock,
    /// A pattern confirmed in the window, sent as one MCP CAD edit.
    pub(crate) edits: live_bridge::window_edit::WindowEdits,
    /// What Claude Code and Codex start as `ketchup`, read when Connect AI opens.
    pub(crate) harnesses: Option<Vec<crate::live_bridge::mcp_setup::HarnessRow>>,
}

/// Push/Pull input, its exact face-offset evaluation and the last committed
/// push/pull.
pub(crate) struct PushPullState {
    pub(crate) distance_input: String,
    pub(crate) face_offset_evaluation: Option<face_offset_preview::FaceOffsetEvaluation>,
    pub(crate) face_offset_preview_due: Option<Instant>,
    pub(crate) smart_proposal: Option<SmartPushPullProposal>,
    pub(crate) smart_planning: Option<SmartPushPullPlanning>,
    pub(crate) last: Option<LastPushPull>,
    /// The last re-planned preview and the command digest it re-planned to,
    /// `None` when the preview no longer re-plans to itself.
    pub(crate) preview_check: std::cell::RefCell<Option<(PreviewCheckKey, Option<String>)>>,
}

/// Targets and typed inputs of the solid tools (revolve, loft, pocket editor).
pub(crate) struct SolidToolInputs {
    pub(crate) target: Option<SelectionId>,
    pub(crate) revolve: Option<RevolveToolState>,
    pub(crate) loft_input_sections: Option<(DefinitionId, Vec<LoftSection>)>,
    pub(crate) pocket_editor_feature: Option<FeatureId>,
    pub(crate) pocket_depth_input: String,
    /// The depth the input was loaded from; the input differs only after typing.
    pub(crate) pocket_depth_source: String,
    /// A click of Pull area that waits for the regions of the drawings to be found off
    /// the UI thread (review 2026-10-26 UO-40).
    pub(crate) area_click_awaits_regions: Option<crate::app::underlay_extrude::AreaClick>,
    /// A 2D edit of a drawing that waits for the regions of the areas that follow it to be
    /// found off the UI thread (review 2026-10-29 UO-40).
    pub(crate) drawing_edit_awaits_regions: Option<crate::app::underlay_edit::AwaitedDrawingEdit>,
}

/// The running Move/Rotate/Scale session, its typed input and a correction of
/// the last transform.
pub(crate) struct TransformToolState {
    pub(crate) session: Option<ToolSession>,
    pub(crate) input: TransformInputInterpreter,
    pub(crate) correction: Option<CorrectionSession>,
}

/// The typed value box under the viewport.
pub(crate) struct ValueBox {
    pub(crate) input: String,
    pub(crate) focus: bool,
    /// How every field of the window reads a decimal number: the system's
    /// separator, read once at startup.
    pub(crate) decimal: crate::length_input::DecimalSeparator,
}

/// The parameter editor and the provenance of the last recompute.
pub(crate) struct ParameterEditor {
    pub(crate) editor_node: Option<NodeId>,
    pub(crate) expression_input: String,
    pub(crate) canonical_source: String,
    pub(crate) provenance: Option<(DocumentId, u64, String)>,
    pub(crate) last_recomputed_nodes: BTreeSet<NodeId>,
}

/// Which validators the panel runs and its last report.
pub(crate) struct ValidatorPanel {
    pub(crate) selection: BTreeSet<&'static str>,
    pub(crate) report: Option<ValidatorPanelReport>,
    pub(crate) state: validator_ui::ValidatorPanelState,
}

/// Where the camera looks from, the viewport it draws into and a pending Zoom
/// Fit.
pub(crate) struct CameraState {
    pub(crate) projection_mode: ProjectionMode,
    pub(crate) distance_mm: f64,
    pub(crate) yaw: f32,
    pub(crate) pitch: f32,
    pub(crate) target_z: f64,
    pub(crate) zoom: f32,
    pub(crate) pan: Vec2,
    pub(crate) previous_view: Option<CameraViewState>,
    pub(crate) drag_active: bool,
    pub(crate) wheel_active: bool,
    pub(crate) viewport_rect: Option<Rect>,
    /// Zoom Fit was requested before the viewport was laid out or had anything
    /// to frame (e.g. right after launch); it is applied on the next frame that can.
    pub(crate) zoom_fit_pending: bool,
    /// The pending fit frames a just-opened document and must not replace its
    /// open/recovery status message.
    pub(crate) zoom_fit_pending_quiet: bool,
}

/// What the pointer is over and what it would snap to.
pub(crate) struct HoverState {
    pub(crate) target: Option<SelectionId>,
    /// The profile a drawing tool just created, with the revision it created;
    /// Push/Pull targets it next while that revision is current.
    pub(crate) drawn_profile: Option<(u64, SelectionId)>,
    pub(crate) pick: Option<PickResult>,
    pub(crate) snap: Option<SnapResult>,
    pub(crate) overlap_index: usize,
    /// The non-front choice was made with Alt: it lasts while Alt is held or
    /// the pointer stays put, so a stray Alt never leaves a hidden part chosen.
    pub(crate) alt_choice: bool,
    pub(crate) pointer: Option<Pos2>,
    pub(crate) projection_cache: RefCell<Option<InteractionProjectionCache>>,
    /// The parts the last two Select presses hit, the latest last (none for a
    /// press that picked an annotation): a double click opens a part only when
    /// both of its presses hit it.
    pub(crate) presses: [Option<InstancePath>; 2],
    /// Where on screen the last two Select presses were: a double click selects a whole
    /// drawing only when both were on the same spot, not two quick picks of two objects.
    pub(crate) press_points: [Option<Pos2>; 2],
}

/// Which side panels and windows are open.
pub(crate) struct Panels {
    pub(crate) outliner_visible: bool,
    pub(crate) tags_visible: bool,
    pub(crate) dimensions_visible: bool,
    pub(crate) manual_cad_panels_visible: bool,
    pub(crate) shortcuts_open: bool,
    pub(crate) about_open: bool,
}

/// The command search field: typed text, highlighted result, focus request.
#[derive(Default)]
pub(crate) struct CommandSearch {
    pub(crate) query: String,
    /// The result Enter runs; arrows move it.
    pub(crate) highlight: usize,
    /// Ctrl+K asked for keyboard focus in the search field.
    pub(crate) focus: bool,
}

/// Typed names in the classification panel.
pub(crate) struct ClassificationInputs {
    pub(crate) dimension_name_input: String,
    pub(crate) category_name_input: String,
    pub(crate) selected_dimension: Option<ClassificationDimensionId>,
}

/// What a copy or cut puts on the system clipboard. Ctrl+V pastes the parts
/// only while the system clipboard still holds it: text copied elsewhere since
/// then is not a request to paste old parts.
pub(crate) const SYSTEM_CLIPBOARD_TEXT: &str = "Ketchup object selection";

/// Copied and cut occurrences waiting for Paste.
pub(crate) struct Clipboard {
    pub(crate) occurrences: Vec<OccurrenceId>,
    pub(crate) cut_occurrences: Vec<CutClipboardOccurrence>,
    /// A copy or cut not yet written to the system clipboard.
    pub(crate) announce: bool,
}

/// CAM, FEA and PDM review workflows and their open dialogs.
pub(crate) struct ReviewWorkflows {
    pub(crate) cam_reviews: CamReviewWorkflow,
    pub(crate) cam_export_dialog: Option<CamExportDialog>,
    pub(crate) fea_reviews: FeaReviewWorkflow,
    pub(crate) fea_review_dialog: Option<FeaReviewDialog>,
    pub(crate) pdm: LocalPdmWorkflow,
    pub(crate) pdm_review_dialog: Option<PdmReviewDialog>,
}

/// The exact worker, the running evaluation and the exact and topology results
/// by source.
pub(crate) struct ExactState {
    pub(crate) worker_path: Option<PathBuf>,
    pub(crate) worker_attempted: bool,
    pub(crate) task: Option<ExactEvaluationTask>,
    /// When the running evaluation started; progress is shown only for long ones.
    pub(crate) task_started: Option<Instant>,
    /// How many producers the started evaluations sent to the exact worker;
    /// producers whose exact results are current are reused, not counted.
    pub(crate) producers_requested: usize,
    pub(crate) mutation_readiness: MutationReadiness,
    pub(crate) results: ExactResultRegistry,
    pub(crate) topology_results: ExactResultRegistry,
    pub(crate) result_history: BTreeMap<ExactSource, ExactResultRegistry>,
    pub(crate) topology_result_history: BTreeMap<ExactSource, ExactResultRegistry>,
    pub(crate) source: Option<ExactSource>,
    pub(crate) retry_at: Option<Instant>,
    /// Status-bar body and reference counts for (document, revision, exact
    /// contents): counting checks every package's freshness, too slow per frame.
    pub(crate) status_counts: std::cell::Cell<Option<(ExactCacheKey, usize, usize)>>,
    /// The unique current topology package per definition for (document,
    /// revision, topology contents): hover resolves the face of every body
    /// under the pointer, and each lookup checked every package's freshness.
    pub(crate) topology_by_definition: std::cell::RefCell<Option<TopologyByDefinition>>,
    /// A box face the user clicked in the viewport before that body's exact
    /// result was shown; the selection moves to the exact face once it is.
    pub(crate) clicked_box_face: Option<ketchup_interaction::SelectionId>,
}

/// The material takeoff window and the takeoff it shows.
#[derive(Default)]
pub(crate) struct TakeoffState {
    pub(crate) open: bool,
    /// The takeoff for (document, revision, exact contents); the program model
    /// it counts is kept while the program text is unchanged.
    pub(crate) cache: std::cell::RefCell<Option<TakeoffCache>>,
}

/// The project drawings window: the sheet settings being edited there.
#[derive(Default)]
pub(crate) struct DrawingsState {
    pub(crate) open: bool,
    pub(crate) editing: ketchup_manufacturing::title_block::SheetSettings,
    /// The defaults the title block fields showed in the last frame.
    pub(crate) shown_defaults: BTreeMap<ketchup_manufacturing::title_block::TitleField, String>,
    /// The document's stored settings as the window last read or wrote them:
    /// their bytes, what the window's fields keep of them, and why they do
    /// not read. The window reads them again only when the bytes change.
    pub(crate) seen_bytes: Option<Vec<u8>>,
    pub(crate) kept: ketchup_manufacturing::title_block::SheetSettings,
    pub(crate) unreadable: Option<String>,
    /// The document's sheet settings changed since it was opened or saved.
    pub(crate) unsaved: bool,
}

/// Viewer sharing preparation kept as project companion data, not CAD.
#[derive(Default)]
pub(crate) struct ViewerPrepState {
    /// Notes or sharing choices changed since the document was opened or saved.
    pub(crate) unsaved: bool,
    /// Customer comments returned on an export of this document.
    pub(crate) received: crate::app::received_comments::ReceivedComments,
}

pub(crate) struct TakeoffCache {
    pub(crate) program: (String, String, BTreeMap<String, f64>),
    pub(crate) model: Result<Arc<ketchup_program::ProgramModel>, ketchup_program::ProgramError>,
    pub(crate) key: ExactCacheKey,
    pub(crate) takeoff: Result<Arc<ketchup_program::takeoff::Takeoff>, TakeoffError>,
}

/// Why there is no material takeoff, or why it could not be written.
#[derive(Clone, Debug)]
pub(crate) enum TakeoffError {
    NoProgram,
    /// The program is still being evaluated in the background.
    Planning,
    /// A quote was asked of a program that declares no `price()`.
    NoPrices,
    Program(ketchup_program::ProgramError),
    Write(Arc<std::io::Error>),
}

/// Document, revision and exact registry contents stamp that derived exact
/// data was computed for.
pub(crate) type ExactCacheKey = (DocumentId, u64, u64);

pub(crate) struct TopologyByDefinition {
    pub(crate) key: ExactCacheKey,
    pub(crate) packages: BTreeMap<DefinitionId, Arc<ExactBodyPackage>>,
}

/// Derived render data and the GPU handles it draws with.
pub(crate) struct RenderState {
    pub(crate) cache: DerivedRenderCache,
    pub(crate) plan: Option<Arc<InstancedRenderPlan>>,
    /// `plan` was built while the program was still being evaluated, so it
    /// lacks the threads of program parts until it is built again.
    pub(crate) plan_awaits_program: bool,
    /// The last Move/Rotate/Scale preview plan: its base plan, the placements
    /// it shows and the plan itself.
    pub(crate) moved_plan: Option<MovedPlan>,
    pub(crate) overlay_edge_cache: RefCell<BTreeMap<DefinitionId, OverlayEdges>>,
    /// The GPU vertices of the underlays' lines, by the SHA-256 of their blob.
    pub(crate) underlays: BTreeMap<[u8; 32], Arc<crate::renderer::underlay::UnderlayGeometry>>,
    /// The snap indexes of the underlays' lines, by the SHA-256 of their blob.
    pub(crate) underlay_snaps:
        RefCell<BTreeMap<[u8; 32], crate::app::underlay_snapping::UnderlaySnapSlot>>,
    /// The bounds of each layer of the underlays' lines, by the SHA-256 of their blob: what
    /// framing the view takes of them.
    pub(crate) underlay_extents: RefCell<BTreeMap<[u8; 32], crate::app::underlay::UnderlayExtent>>,
    /// The lines and objects of the underlays with their edits, by
    /// [`ketchup_model::underlay::Underlay::lines_key`].
    pub(crate) underlay_lines:
        RefCell<BTreeMap<[u8; 32], Arc<ketchup_model::underlay::UnderlayLines>>>,
    /// The underlays' blobs as decoded, before their edits, by the SHA-256 of the blob and
    /// the underlay's layer count, so an edit does not decode the blob again (UI-62).
    pub(crate) underlay_decoded:
        RefCell<BTreeMap<DecodedBlobKey, Arc<ketchup_model::underlay::UnderlayLines>>>,
    /// The children and bounds of the objects of `underlay_lines`, by the same key, so
    /// picking and painting picked objects does not work them out each frame (UI-62).
    pub(crate) underlay_object_trees:
        RefCell<BTreeMap<[u8; 32], Arc<ketchup_model::underlay::object_query::ObjectTree>>>,
    /// What the viewport paints of them this frame.
    pub(crate) underlay_frame: Vec<crate::renderer::underlay::UnderlayDraw>,
    pub(crate) wgpu_target_format: Option<eframe::wgpu::TextureFormat>,
    pub(crate) wgpu_device: Option<eframe::wgpu::Device>,
    pub(crate) wgpu_queue: Option<eframe::wgpu::Queue>,
}

/// An underlay's blob by its SHA-256 and the layer count it is decoded with.
pub(crate) type DecodedBlobKey = ([u8; 32], usize);

pub(crate) type MovedPlan = (
    Arc<InstancedRenderPlan>,
    BTreeMap<InstancePath, Transform>,
    Arc<InstancedRenderPlan>,
);

impl RenderState {
    /// `plan` with a Move/Rotate/Scale preview's instances placed. A frame
    /// whose placements did not change (the camera turned, the pointer
    /// rested) gets the same plan back, so the GPU keeps its upload.
    pub(crate) fn moved_plan(
        &mut self,
        plan: &Arc<InstancedRenderPlan>,
        overrides: BTreeMap<InstancePath, Transform>,
    ) -> Arc<InstancedRenderPlan> {
        if let Some((base, placed, moved)) = &self.moved_plan
            && Arc::ptr_eq(base, plan)
            && *placed == overrides
        {
            return Arc::clone(moved);
        }
        let moved = Arc::new(plan.with_transform_overrides(&overrides));
        self.moved_plan = Some((Arc::clone(plan), overrides, Arc::clone(&moved)));
        moved
    }
}
