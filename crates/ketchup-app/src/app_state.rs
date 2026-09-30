//! The shell state of [`crate::KetchupApp`] grouped by what it serves, so each
//! part of the window owns one small struct instead of loose fields.

use super::*;

/// The open file: where it lives, what was saved, and the recovery and
/// migration reviews around opening it.
pub(crate) struct FileState {
    pub(crate) container_data: ketchup_model::persistence::ContainerData,
    pub(crate) review_candidate: Option<ketchup_model::persistence::LoadOutcome>,
    pub(crate) migration_review_plan: Option<MigrationReviewPlan>,
    pub(crate) recovery_open: Option<RecoveryOpenState>,
    pub(crate) path: Option<PathBuf>,
    pub(crate) identity: Option<ketchup_model::persistence::FileIdentity>,
    pub(crate) work_recovery_identity: Option<ketchup_model::persistence::FileIdentity>,
    pub(crate) pending_work_recovery_cleanup:
        Option<(PathBuf, ketchup_model::persistence::FileIdentity)>,
    pub(crate) work_recovery_digest: Option<String>,
    pub(crate) saved_digest: String,
}

/// The live AI bridge and the consent that attaches it.
pub(crate) struct LiveState {
    pub(crate) bridge: Option<live_bridge::LiveBridge>,
    pub(crate) consent_broker: Option<live_bridge::consent::ConsentBroker>,
    pub(crate) consent_attached: bool,
}

/// Push/Pull input, its exact face-offset evaluation and the last committed
/// push/pull.
pub(crate) struct PushPullState {
    pub(crate) distance_input: String,
    pub(crate) face_offset_evaluation: Option<planar_push_pull::FaceOffsetEvaluation>,
    pub(crate) face_offset_preview_due: Option<Instant>,
    pub(crate) smart_proposal: Option<SmartPushPullProposal>,
    pub(crate) smart_planning: Option<SmartPushPullPlanning>,
    pub(crate) last: Option<LastPushPull>,
}

/// Targets and typed inputs of the solid tools (revolve, loft, pocket editor).
pub(crate) struct SolidToolInputs {
    pub(crate) target: Option<SelectionId>,
    pub(crate) revolve: Option<RevolveToolState>,
    pub(crate) loft_input_sections: Option<(DefinitionId, Vec<LoftSection>)>,
    pub(crate) pocket_editor_feature: Option<FeatureId>,
    pub(crate) pocket_depth_input: String,
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
    pub(crate) pointer: Option<Pos2>,
    pub(crate) projection_cache: RefCell<Option<InteractionProjectionCache>>,
}

/// Which side panels and windows are open, and the command search text.
pub(crate) struct Panels {
    pub(crate) outliner_visible: bool,
    pub(crate) tags_visible: bool,
    pub(crate) dimensions_visible: bool,
    pub(crate) manual_cad_panels_visible: bool,
    pub(crate) shortcuts_open: bool,
    pub(crate) about_open: bool,
    pub(crate) command_search: String,
}

/// Typed names in the classification panel.
pub(crate) struct ClassificationInputs {
    pub(crate) dimension_name_input: String,
    pub(crate) category_name_input: String,
    pub(crate) selected_dimension: Option<ClassificationDimensionId>,
}

/// The AI assistant: provider, conversation, memory, diagnostics and the
/// proposal under review.
pub(crate) struct AssistantState {
    pub(crate) provider: AssistantProvider,
    pub(crate) model: String,
    pub(crate) workspace_mode: AssistantWorkspaceMode,
    pub(crate) input: String,
    pub(crate) messages: Vec<AssistantChatMessage>,
    pub(crate) memory: AssistantProjectMemory,
    pub(crate) diagnostics_enabled: bool,
    pub(crate) api_logs: Vec<AssistantApiLogEntry>,
    pub(crate) selected_api_log: Option<usize>,
    pub(crate) inspector_tab: AssistantInspectorTab,
    pub(crate) memory_search: String,
    pub(crate) transport: Arc<dyn AssistantTransport>,
    pub(crate) context_preparation_delay: Duration,
    pub(crate) chat_task: Option<AssistantChatTask>,
    pub(crate) pending_execution: Option<AssistantPendingExecution>,
    pub(crate) request_sequence: u64,
    pub(crate) saved_conversation_digest: String,
    pub(crate) intent_kind: AssistantIntentKind,
    pub(crate) target_input: String,
    pub(crate) value_input: String,
    pub(crate) proposal: Option<AssistantPreviewPlan>,
    pub(crate) verification: Option<AssistantVerification>,
}

/// Copied and cut occurrences waiting for Paste.
pub(crate) struct Clipboard {
    pub(crate) occurrences: Vec<OccurrenceId>,
    pub(crate) cut_occurrences: Vec<CutClipboardOccurrence>,
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
    pub(crate) mutation_readiness: MutationReadiness,
    pub(crate) results: ExactResultRegistry,
    pub(crate) topology_results: ExactResultRegistry,
    pub(crate) result_history: BTreeMap<ExactSource, ExactResultRegistry>,
    pub(crate) topology_result_history: BTreeMap<ExactSource, ExactResultRegistry>,
    pub(crate) source: Option<ExactSource>,
    pub(crate) retry_at: Option<Instant>,
}

/// Derived render data and the GPU handles it draws with.
pub(crate) struct RenderState {
    pub(crate) cache: DerivedRenderCache,
    pub(crate) plan: Option<Arc<InstancedRenderPlan>>,
    pub(crate) overlay_edge_cache: RefCell<BTreeMap<DefinitionId, OverlayEdges>>,
    pub(crate) wgpu_target_format: Option<eframe::wgpu::TextureFormat>,
    pub(crate) wgpu_device: Option<eframe::wgpu::Device>,
    pub(crate) wgpu_queue: Option<eframe::wgpu::Queue>,
}
