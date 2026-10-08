//! Construction of the application, the per-frame entry point `ui`, theme, top bar, menu bar, status bar and About window.

use crate::*;

impl KetchupApp {
    #[must_use]
    pub fn new() -> Self {
        Self::with_catalog(LocaleCatalog::english())
    }

    #[must_use]
    pub fn with_catalog(catalog: LocaleCatalog) -> Self {
        Self::with_catalog_and_initial_box(catalog, true)
    }

    pub(crate) fn with_catalog_and_initial_box(
        catalog: LocaleCatalog,
        include_initial_box: bool,
    ) -> Self {
        catalog
            .validate_complete_against(&LocaleCatalog::english())
            .expect("the active locale must match the complete English key set");
        let mut confirmation_key = [0; 32];
        getrandom::fill(&mut confirmation_key)
            .expect("the operating system must provide confirmation-key entropy");
        let confirmation_surface = TrustedConfirmationSurface::new(confirmation_key, 1)
            .expect("the built-in non-zero confirmation policy is valid");
        let mut document = DocumentStore::new();
        document
            .configure_human_confirmation_policy(confirmation_surface.verifying_key(), 1)
            .expect("a fresh document accepts the application confirmation policy");
        if include_initial_box {
            let box_name = catalog.format(
                "model-default-box",
                &BTreeMap::from([("number", "1".to_owned())]),
            );
            let occurrence_name = catalog.format(
                "model-default-occurrence",
                &BTreeMap::from([("name", box_name.clone())]),
            );
            document
                .apply_batch(&create_box_batch(
                    DefinitionId(1),
                    [FeatureId(1), FeatureId(2)],
                    OccurrenceId(1),
                    [
                        &box_name,
                        &catalog.text("model-default-profile"),
                        &catalog.text("model-default-extrusion"),
                        &occurrence_name,
                    ],
                    Vec3::ZERO,
                    Vec3::new(BOX_WIDTH_MM, BOX_DEPTH_MM, 20.0),
                ))
                .expect("the built-in initial document is valid");
            document.discard_history_before_current();
        }
        let assistant_memory = AssistantProjectMemory::empty(document.current().document_id().0);
        let saved_digest = document.history_digest();
        let digest = catalog.text("status-ready");
        Self {
            document,
            live: app_state::LiveState {
                bridge: None,
                consent_broker: None,
                consent_attached: false,
            },
            close_guard: close_guard::CloseGuard::default(),
            file: app_state::FileState {
                container_data: ketchup_model::persistence::ContainerData::default(),
                review_candidate: None,
                migration_review_plan: None,
                recovery_open: None,
                path: None,
                identity: None,
                work_recovery_identity: None,
                work_recovery_lock: None,
                pending_work_recovery_cleanup: None,
                work_recovery_digest: None,
                saved_digest,
            },
            confirmation_surface,
            side_effect_receipts: Vec::new(),
            btlx_profile_strategy: BtlxProfileStrategy::EdgeSawCutsThenMillContour,
            btlx_intermediate_saw_cuts: 0,
            catalog,
            assembly_editor: assembly_ui::AssemblyEditorState::default(),
            body_editor: body_ui::BodyEditorState::default(),
            face_workflow: face_workflow_ui::FaceWorkflowUiState::default(),
            feature_history: feature_history_ui::FeatureHistoryUiState::default(),
            push_pull: app_state::PushPullState {
                distance_input: String::new(),
                face_offset_evaluation: None,
                face_offset_preview_due: None,
                smart_proposal: None,
                smart_planning: None,
                last: None,
                preview_check: std::cell::RefCell::new(None),
            },
            program_evaluations: Default::default(),
            solid_tools: app_state::SolidToolInputs {
                target: None,
                revolve: None,
                loft_input_sections: None,
                pocket_editor_feature: None,
                pocket_depth_input: String::new(),
            },
            helix_tool: helix_ui::HelixUiState::default(),
            parameter: app_state::ParameterEditor {
                editor_node: None,
                expression_input: String::new(),
                canonical_source: String::new(),
                provenance: None,
                last_recomputed_nodes: BTreeSet::new(),
            },
            validator_panel: app_state::ValidatorPanel {
                selection: ASSISTANT_VALIDATOR_IDS.into_iter().collect(),
                report: None,
                state: validator_ui::ValidatorPanelState::default(),
            },
            status_key: "status-ready",
            theme: ThemeKind::default(),
            camera: app_state::CameraState {
                projection_mode: ProjectionMode::Parallel,
                distance_mm: 420.0 / 2.8,
                yaw: -0.65,
                pitch: -0.5,
                target_z: 10.0,
                zoom: 2.8,
                pan: Vec2::ZERO,
                previous_view: None,
                drag_active: false,
                wheel_active: false,
                viewport_rect: None,
                zoom_fit_pending: false,
                zoom_fit_pending_quiet: false,
            },
            view: ViewSettings::default(),
            selection: SelectionState::default(),
            hover: app_state::HoverState {
                target: None,
                drawn_profile: None,
                pick: None,
                snap: None,
                overlap_index: 0,
                alt_choice: false,
                pointer: None,
                projection_cache: RefCell::new(None),
            },
            active_tool: ActiveTool::Select,
            panels: app_state::Panels {
                outliner_visible: true,
                tags_visible: true,
                dimensions_visible: true,
                manual_cad_panels_visible: false,
                shortcuts_open: false,
                about_open: false,
                command_search: String::new(),
            },
            takeoff: app_state::TakeoffState::default(),
            drawings: app_state::DrawingsState::default(),
            saved_views_ui: Default::default(),
            section: None,
            digest,
            assistant: app_state::AssistantState {
                provider: AssistantProvider::initial(),
                model: AssistantProvider::initial().default_model().to_owned(),
                workspace_mode: AssistantWorkspaceMode::Dock,
                input: String::new(),
                messages: Vec::new(),
                memory: assistant_memory,
                diagnostics_enabled: false,
                api_logs: Vec::new(),
                selected_api_log: None,
                inspector_tab: AssistantInspectorTab::default(),
                memory_search: String::new(),
                transport: Arc::new(ProcessAssistantTransport),
                #[cfg(feature = "testing")]
                context_preparation_delay: Duration::ZERO,
                chat_task: None,
                pending_execution: None,
                request_sequence: 0,
                saved_conversation_digest: assistant_conversation_digest(&[]),
                intent_kind: AssistantIntentKind::FeatureDimension,
                target_input: "2".to_owned(),
                value_input: "35".to_owned(),
                proposal: None,
                verification: None,
            },
            classification: app_state::ClassificationInputs {
                dimension_name_input: String::new(),
                category_name_input: String::new(),
                selected_dimension: None,
            },
            transform_tool: app_state::TransformToolState {
                session: None,
                input: TransformInputInterpreter::default(),
                correction: None,
            },
            value_box: app_state::ValueBox {
                input: String::new(),
                focus: false,
            },
            clipboard: app_state::Clipboard {
                occurrences: Vec::new(),
                cut_occurrences: Vec::new(),
            },
            modal: None,
            tool_preview: None,
            gesture: Gesture::default(),
            mesh_conversion_state: mesh_conversion_ui::MeshConversionUiState::default(),
            dialogs: Box::new(NativeFileDialogs::default()),
            reviews: app_state::ReviewWorkflows {
                cam_reviews: CamReviewWorkflow::new(None),
                cam_export_dialog: None,
                fea_reviews: FeaReviewWorkflow::new(None),
                fea_review_dialog: None,
                pdm: LocalPdmWorkflow::new(),
                pdm_review_dialog: None,
            },
            exact: app_state::ExactState {
                worker_path: None,
                worker_attempted: false,
                task: None,
                mutation_readiness: MutationReadiness::Ready,
                results: ExactResultRegistry::default(),
                topology_results: ExactResultRegistry::default(),
                result_history: BTreeMap::new(),
                topology_result_history: BTreeMap::new(),
                source: None,
                retry_at: None,
                status_counts: std::cell::Cell::new(None),
                topology_by_definition: std::cell::RefCell::new(None),
            },
            render: app_state::RenderState {
                cache: DerivedRenderCache::default(),
                plan: None,
                overlay_edge_cache: RefCell::new(BTreeMap::new()),
                wgpu_target_format: None,
                wgpu_device: None,
                wgpu_queue: None,
            },
        }
    }

    #[must_use]
    pub fn from_creation_context(context: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::with_catalog_and_initial_box(LocaleCatalog::english(), false);
        app.dialogs = Box::new(NativeFileDialogs::with_parent(
            DialogParentWindow::from_creation_context(context),
        ));
        if let Some(render_state) = context.wgpu_render_state.as_ref() {
            render_state
                .renderer
                .write()
                .callback_resources
                .insert(GpuInstancedRenderer::new(
                    &render_state.device,
                    render_state.target_format,
                ));
            app.render.wgpu_target_format = Some(render_state.target_format);
            app.render.wgpu_device = Some(render_state.device.clone());
            app.render.wgpu_queue = Some(render_state.queue.clone());
        }
        app
    }

    /// Answer file dialogs from `dialogs` instead of the operating system.
    #[must_use]
    pub fn with_dialogs(mut self, dialogs: Box<dyn FileDialogs>) -> Self {
        self.dialogs = dialogs;
        self
    }

    #[must_use]
    pub fn with_assistant_transport(mut self, transport: Arc<dyn AssistantTransport>) -> Self {
        self.assistant.transport = transport;
        self
    }

    pub(crate) fn choose_open_path(&mut self) -> Option<PathBuf> {
        let filter_label = self.catalog.text("file-filter-ketchup");
        self.dialogs.pick_open_path(&filter_label)
    }

    pub(crate) fn current_visible_mesh_scene(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<CurrentVisibleMesh>, ExportError> {
        let occurrences = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter(|occurrence| {
                snapshot
                    .definition(occurrence.definition_id)
                    .is_some_and(|definition| {
                        definition.feature_ids().iter().any(|feature_id| {
                            snapshot
                                .feature(*feature_id)
                                .is_some_and(|feature| feature.kind().produces_body())
                        })
                    })
            })
            .collect::<Vec<_>>();
        if occurrences.is_empty() {
            return Err(ExportError::NothingToExport {
                subject: "visible body",
            });
        }

        let mut scene = Vec::new();
        for occurrence in occurrences {
            let terminals = exact_body_terminal_features(snapshot, occurrence.definition_id)
                .map_err(ExportError::failed)?;
            if terminals.is_empty() {
                return Err(ExportError::OccurrenceNotExportable {
                    occurrence: occurrence.instance_path.clone(),
                    reason: "has no unambiguous terminal body",
                });
            }
            for producer_feature_id in terminals.values() {
                let feature = snapshot.feature(*producer_feature_id).ok_or_else(|| {
                    ExportError::OccurrenceNotExportable {
                        occurrence: occurrence.instance_path.clone(),
                        reason: "has a missing terminal body feature",
                    }
                })?;
                let source = if let FeatureKind::MeshBody(mesh) = feature.kind() {
                    CurrentVisibleMeshSource::Canonical {
                        definition_id: occurrence.definition_id,
                        producer_feature_id: *producer_feature_id,
                        mesh: Box::new(mesh.clone()),
                    }
                } else {
                    let package = self
                        .exact.results
                        .render_values(snapshot)
                        .find(|package| {
                            package.definition_id() == occurrence.definition_id
                                && package.producer_feature_id() == *producer_feature_id
                        })
                        .ok_or_else(|| ExportError::OccurrenceNotExportable {
                            occurrence: occurrence.instance_path.clone(),
                            reason: "has no current accepted exact result for a terminal body feature",
                        })?;
                    CurrentVisibleMeshSource::Exact(Box::new((**package).clone()))
                };
                scene.push(CurrentVisibleMesh {
                    source,
                    occurrence: occurrence.clone(),
                });
            }
        }
        Ok(scene)
    }

    /// Whether the active document carries unsaved changes.
    #[must_use]
    pub fn has_review_candidate(&self) -> bool {
        self.file.review_candidate.is_some()
    }

    /// The cached feature edges of `definition_id`, rebuilt only when
    /// `identity` changes.
    ///
    /// Deriving them costs seconds on an imported mesh, so recomputing them
    /// every frame freezes the viewport while a body is hovered or selected.
    pub(crate) fn overlay_feature_edges<G: Copy + Ord>(
        &self,
        definition_id: DefinitionId,
        identity: &str,
        build: impl FnOnce() -> (Vec<[f32; 3]>, Vec<[u32; 3]>, Vec<Option<G>>),
    ) -> Arc<Vec<([u32; 2], Vec<u32>)>> {
        let mut cache = self.render.overlay_edge_cache.borrow_mut();
        if let Some(cached) = cache.get(&definition_id)
            && cached.identity == identity
        {
            return Arc::clone(&cached.edges);
        }
        let (positions, triangles, face_groups) = build();
        let edges = Arc::new(renderer::feature_edge_triangles(
            &positions,
            &triangles,
            &face_groups,
        ));
        cache.insert(
            definition_id,
            OverlayEdges {
                identity: identity.to_owned(),
                edges: Arc::clone(&edges),
            },
        );
        edges
    }

    #[must_use]
    pub fn feature_count(&self) -> usize {
        self.document.current().features().count()
    }

    #[must_use]
    pub fn mesh_body_count(&self) -> usize {
        self.document
            .current()
            .features()
            .filter(|feature| matches!(feature.kind(), FeatureKind::MeshBody(_)))
            .count()
    }

    /// How deep the shell is inside group or component edit contexts.
    #[must_use]
    pub fn edit_context_depth(&self) -> usize {
        self.selection.edit_context.len()
    }

    pub(crate) fn active_scene_query(&self) -> Vec<SceneOccurrence> {
        self.active_scene_query_for_snapshot(&self.document.current())
    }

    pub(crate) fn active_scene_query_for_snapshot(
        &self,
        snapshot: &Snapshot,
    ) -> Vec<SceneOccurrence> {
        let Some(context) = self.selection.edit_context.last() else {
            return snapshot
                .scene_query()
                .into_iter()
                .filter(|occurrence| occurrence.visible && occurrence.instance_path.is_root())
                .collect();
        };
        let context = match context {
            EditContext::Group(group_id) => SceneQueryContext::Group(*group_id),
            EditContext::Definition {
                definition_id,
                instance_path,
            } => SceneQueryContext::Definition {
                definition_id: *definition_id,
                instance_path: instance_path.clone(),
            },
        };
        snapshot
            .bind_scene_query(context)
            .and_then(|query| snapshot.scene_query_in(&query))
            .unwrap_or_default()
    }

    pub(crate) fn correction_session_is_current(&self, session: &CorrectionSession) -> bool {
        let current = self.document.current();
        session.revision == current.revision_id()
            && session.canonical_digest == current.canonical_digest()
            && self.correction_selection_is_current(&session.selection)
    }

    pub(crate) fn exit_edit_context(&mut self) -> bool {
        if self.selection.edit_context.is_empty() {
            return false;
        }
        self.cancel_transform_session();
        self.end_transform_correction();
        self.selection.edit_context.pop();
        self.invalidate_pending_import_reviews();
        self.selection.clear();
        self.digest = self.catalog.text("digest-exited-edit-context");
        true
    }

    pub(crate) fn ephemeral_edit_active(&self) -> bool {
        self.has_preview()
            || self.has_drawn_shape_preview()
            || self.has_occurrence_operation_preview()
            || self.solid_tools.target.is_some()
            || self.solid_tools.revolve.is_some()
            || self.tool_preview.get::<RevolvePreview>().is_some()
            || self.tool_preview.get::<PlanarOffsetPreview>().is_some()
            || self.active_tool == ActiveTool::Helix
            || self.tool_preview.get::<SweepPreview>().is_some()
            || self.tool_preview.get::<LoftPreview>().is_some()
            || self.tool_preview.get::<GeneralFinishPreview>().is_some()
            || self
                .gesture
                .drag
                .get::<PushPullDrag>()
                .is_some_and(|drag| self.push_pull_gesture_is_current(drag))
            || self
                .gesture
                .drag
                .get::<PushPullAnchor>()
                .map(|anchor| &anchor.0)
                .is_some_and(|anchor| self.push_pull_gesture_is_current(anchor))
            || self.transform_gesture_active()
    }

    /// Whether the About window is on screen.
    #[must_use]
    pub const fn about_visible(&self) -> bool {
        self.panels.about_open
    }

    #[must_use]
    pub const fn manual_cad_panels_visible(&self) -> bool {
        self.panels.manual_cad_panels_visible
    }

    pub fn set_manual_cad_panels_visible(&mut self, visible: bool) {
        self.panels.manual_cad_panels_visible = visible;
    }

    /// Restore ordinary material and tonal face painting without changing independent overlays.
    pub fn restore_shaded(&mut self) {
        self.view.set(ViewFlag::Wireframe, false);
        self.view.set(ViewFlag::Monochrome, false);
        self.view.set(ViewFlag::HiddenLine, false);
        self.digest = self.catalog.text("digest-shaded-restored");
    }

    pub(crate) fn clear_ephemeral_edit_state(&mut self) {
        self.clear_push_pull_preview();
        self.tool_preview = None;
        self.solid_tools.target = None;
        self.solid_tools.revolve = None;
        self.clear_helix_preview();
        self.gesture.drag.close::<PushPullDrag>();
        self.gesture.drag.close::<PushPullAnchor>();
        self.reset_transform_interaction();
        self.gesture.drag.close::<ZoomWindowDrag>();
        self.gesture.drag.close::<SelectionWindowDrag>();
        self.clear_measurement();
        self.gesture.mirror = None;
        self.gesture.planar_offset_mm = None;
    }

    pub(crate) fn value_label_key(&self) -> &'static str {
        match self.active_tool {
            ActiveTool::Line => "value-label-distance",
            ActiveTool::Rectangle => "value-label-width-depth",
            ActiveTool::Circle => "value-label-radius",
            ActiveTool::Polygon if self.gesture.sketch.start.is_none() => "value-label-sides",
            ActiveTool::Polygon => "value-label-radius",
            ActiveTool::Ellipse if self.gesture.sketch.end.is_none() => "value-label-radius",
            ActiveTool::Ellipse => "value-label-second-radius",
            ActiveTool::Spline => "value-label-distance",
            ActiveTool::Arc => "value-label-bulge",
            ActiveTool::Revolve => "value-label-angle",
            ActiveTool::Shell => "value-label-thickness",
            ActiveTool::Fillet | ActiveTool::Chamfer => "value-label-radius-distance",
            ActiveTool::Rotate => "value-label-angle",
            ActiveTool::Mirror => "value-label-mirror-offset",
            ActiveTool::Scale => "value-label-scale-factor",
            ActiveTool::PushPull | ActiveTool::Move | ActiveTool::Measure => "value-label-distance",
            _ => "value-label-dimensions",
        }
    }

    /// Colours every surface of the shell paints with.
    #[must_use]
    pub const fn palette(&self) -> Palette {
        Palette::of(self.theme)
    }

    /// Which of the four appearances is showing.
    #[must_use]
    pub const fn theme(&self) -> ThemeKind {
        self.theme
    }

    /// Switch appearance. Purely presentational — the document never changes.
    pub fn set_theme(&mut self, theme: ThemeKind) {
        self.theme = theme;
        self.digest = self.catalog.text(theme.label_key());
    }

    pub(crate) fn show_top_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;

            let (logo, _) = ui.allocate_exact_size(Vec2::splat(24.0), Sense::hover());
            ui.painter()
                .rect_filled(logo, egui::CornerRadius::same(7), palette.accent);
            theme::paint_icon(
                ui.painter(),
                logo,
                Icon::Logo,
                palette.accent_ink,
                palette.accent_ink,
                2.1,
            );
            ui.label(
                egui::RichText::new(self.catalog.text("app-title"))
                    .size(14.0)
                    .strong(),
            );

            vertical_rule(ui, palette);

            // The extension is set in the tertiary tone so the eye lands on the
            // model's name, and unsaved work is an accent dot instead of a `*`.
            let title = self.document_title();
            let (stem, extension) = title
                .trim_end_matches(" *")
                .rsplit_once('.')
                .map_or((title.trim_end_matches(" *"), ""), |(stem, extension)| {
                    (stem, extension)
                });
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(egui::RichText::new(stem).color(palette.dim));
            if !extension.is_empty() {
                ui.label(egui::RichText::new(format!(".{extension}")).color(palette.faint));
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            if title.ends_with(" *") {
                let (dot, response) = ui.allocate_exact_size(Vec2::splat(9.0), Sense::hover());
                ui.painter()
                    .circle_filled(dot.center(), 2.5, palette.accent);
                response.on_hover_text(self.catalog.text("status-unsaved"));
            }

            ui.spacing_mut().item_spacing.x = 2.0;
            for (id, icon) in [
                (AppCommand::Undo, Icon::Undo),
                (AppCommand::Redo, Icon::Redo),
            ] {
                if self.icon_button(ui, id, icon, Vec2::new(28.0, 26.0)) {
                    self.dispatch_command(id);
                }
            }

            let views = [
                AppCommand::ViewIso,
                AppCommand::ViewTop,
                AppCommand::ViewBottom,
                AppCommand::ViewFront,
                AppCommand::ViewBack,
                AppCommand::ViewRight,
                AppCommand::ViewLeft,
                AppCommand::ZoomFit,
            ];
            let chips = ThemeKind::ALL;
            // Both clusters are laid out from the right so the centre one keeps
            // its place as the document name grows.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                segmented(ui, palette, |ui| {
                    for kind in chips {
                        self.theme_chip(ui, palette, kind);
                    }
                });
                ui.add_space(ui.available_width() * 0.5 - 150.0);
                segmented(ui, palette, |ui| {
                    for id in views {
                        if self.segment_button(ui, palette, id, false) {
                            self.dispatch_command(id);
                        }
                    }
                    vertical_rule(ui, palette);
                    if self.segment_button(ui, palette, AppCommand::ViewProjection, true) {
                        self.dispatch_command(AppCommand::ViewProjection);
                    }
                });
            });
        });
    }

    /// A frameless glyph button in the chrome, e.g. Undo.
    pub(crate) fn icon_button(
        &self,
        ui: &mut egui::Ui,
        id: AppCommand,
        icon: Icon,
        size: Vec2,
    ) -> bool {
        let palette = self.palette();
        let enabled = self.command_enabled(id);
        let label = self.command_label(id);
        let response = ui.add_enabled(enabled, egui::Button::new("").min_size(size).frame(false));
        let ink = if !enabled {
            palette.faint
        } else if response.hovered() {
            palette.text
        } else {
            palette.dim
        };
        if enabled && response.hovered() {
            ui.painter()
                .rect_filled(response.rect, egui::CornerRadius::same(6), palette.panel2);
        }
        theme::paint_icon(
            ui.painter(),
            shrink_to_icon(response.rect, 15.0),
            icon,
            ink,
            if enabled { palette.accent } else { ink },
            1.7,
        );
        name_widget(&response, enabled, &label);
        response
            .on_hover_text(keymap::shortcut_text(&self.catalog, id))
            .clicked()
    }

    /// One pill inside a segmented control. `accent_when_on` fills the pill.
    pub(crate) fn segment_button(
        &self,
        ui: &mut egui::Ui,
        palette: Palette,
        id: AppCommand,
        accent_when_on: bool,
    ) -> bool {
        let label = if id == AppCommand::ViewProjection {
            self.catalog.text(self.camera.projection_mode.label_key())
        } else {
            self.command_label(id)
        };
        let enabled = self.command_enabled(id);
        let response =
            ui.add_enabled(
                enabled,
                egui::Button::new(egui::RichText::new(&label).size(12.0).color(
                    if accent_when_on {
                        palette.accent_ink
                    } else {
                        palette.dim
                    },
                ))
                .fill(if accent_when_on {
                    palette.accent
                } else {
                    Color32::TRANSPARENT
                })
                .stroke(Stroke::NONE)
                .corner_radius(egui::CornerRadius::same(6))
                .min_size(Vec2::new(0.0, 26.0)),
            );
        name_widget(&response, enabled, &label);
        response.clicked()
    }

    /// A theme chip: colour swatch plus name, outlined while it is the active one.
    ///
    /// Drawn by hand rather than as a `Button`, because the swatch has to sit
    /// inside the chip's own padding and still belong to the same hit target.
    pub(crate) fn theme_chip(&mut self, ui: &mut egui::Ui, palette: Palette, kind: ThemeKind) {
        let selected = self.theme == kind;
        let label = self.catalog.text(kind.label_key());
        let font = egui::FontId::proportional(11.5);
        let galley = ui.painter().layout_no_wrap(
            label.clone(),
            font,
            if selected { palette.text } else { palette.dim },
        );
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(galley.size().x + 30.0, 24.0), Sense::click());
        let painter = ui.painter();
        let corner = egui::CornerRadius::same(6);
        if selected {
            painter.rect_filled(rect, corner, palette.panel2);
            painter.rect_stroke(
                rect,
                corner,
                Stroke::new(1.0_f32, palette.line),
                egui::StrokeKind::Inside,
            );
        } else if response.hovered() {
            painter.rect_filled(rect, corner, palette.panel);
        }
        painter.rect_filled(
            Rect::from_center_size(
                Pos2::new(rect.left() + 12.0, rect.center().y),
                Vec2::splat(12.0),
            ),
            egui::CornerRadius::same(4),
            Palette::of(kind).accent,
        );
        painter.galley(
            Pos2::new(rect.left() + 24.0, rect.center().y - galley.size().y * 0.5),
            galley,
            palette.text,
        );
        name_widget(&response, true, &label);
        if response.clicked() {
            self.set_theme(kind);
        }
    }

    pub(crate) fn show_menu_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            // A menu bar is not a row of buttons: the top-level entries stay
            // frameless until they are hovered, the way the platform draws them.
            let widgets = &mut ui.visuals_mut().widgets;
            widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
            widgets.inactive.bg_stroke = Stroke::NONE;
            widgets.hovered.bg_stroke = Stroke::NONE;
            widgets.active.bg_stroke = Stroke::NONE;
            widgets.open.bg_stroke = Stroke::NONE;
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.spacing_mut().button_padding = Vec2::new(9.0, 3.0);
            ui.menu_button(self.catalog.text("menu-file"), |ui| {
                self.menu_command(ui, AppCommand::New);
                self.menu_command(ui, AppCommand::Open);
                self.menu_command(ui, AppCommand::Save);
                self.menu_command(ui, AppCommand::SaveAs);
                ui.separator();
                self.menu_command(ui, AppCommand::ImportMeshStl);
                self.menu_command(ui, AppCommand::ImportDrawingDxf);
                self.menu_command(ui, AppCommand::ImportExactStep);
                self.menu_command(ui, AppCommand::ImportExactIges);
                self.menu_command(ui, AppCommand::ImportSketchupScene);
                self.menu_command(ui, AppCommand::ImportBlenderGlb);
                ui.separator();
                self.menu_command(ui, AppCommand::ExportDrawingDxf);
                self.menu_command(ui, AppCommand::ExportExactStep);
                self.menu_command(ui, AppCommand::ExportExactIges);
                self.menu_command(ui, AppCommand::ExportMeshStl);
                self.menu_command(ui, AppCommand::ExportPrintThreeMf);
                self.menu_command(ui, AppCommand::ExportBlenderGlb);
                self.menu_command(ui, AppCommand::ExportGeneralFabrication);
                self.menu_command(ui, AppCommand::ExportWeldmentCutList);
                self.menu_command(ui, AppCommand::ExportProjectDrawings);
                self.menu_command(ui, AppCommand::ExportSheetMetalManufacturing);
                self.menu_command(ui, AppCommand::ExportHomagMpr);
                self.menu_command(ui, AppCommand::ReviewCamExport);
                self.menu_command(ui, AppCommand::ReviewStaticFea);
                self.menu_command(ui, AppCommand::ReviewLocalPdm);
                ui.menu_button(self.catalog.text("file-export-btlx-options"), |ui| {
                    ui.selectable_value(
                        &mut self.btlx_profile_strategy,
                        BtlxProfileStrategy::PortableFreeContour,
                        self.catalog.text("file-export-btlx-portable-contour"),
                    );
                    ui.selectable_value(
                        &mut self.btlx_profile_strategy,
                        BtlxProfileStrategy::EdgeSawCutsThenMillContour,
                        self.catalog.text("file-export-btlx-saw-then-mill"),
                    );
                    ui.add_enabled(
                        self.btlx_profile_strategy
                            == BtlxProfileStrategy::EdgeSawCutsThenMillContour,
                        egui::Slider::new(&mut self.btlx_intermediate_saw_cuts, 0..=32)
                            .text(self.catalog.text("file-export-btlx-intermediate-saw-cuts")),
                    );
                });
                self.menu_command(ui, AppCommand::ExportHundeggerBtlx);
            });
            ui.menu_button(self.catalog.text("menu-edit"), |ui| {
                self.menu_command(ui, AppCommand::Undo);
                self.menu_command(ui, AppCommand::Redo);
                ui.separator();
                self.menu_command(ui, AppCommand::Copy);
                self.menu_command(ui, AppCommand::Cut);
                self.menu_command(ui, AppCommand::Paste);
                self.menu_command(ui, AppCommand::Duplicate);
                self.menu_command(ui, AppCommand::Delete);
                self.menu_command(ui, AppCommand::Group);
                self.menu_command(ui, AppCommand::Ungroup);
                self.menu_command(ui, AppCommand::MakeUnique);
                self.menu_command(ui, AppCommand::SelectAll);
                self.menu_command(ui, AppCommand::InvertSelection);
                self.menu_command(ui, AppCommand::Deselect);
            });
            ui.menu_button(self.catalog.text("menu-view"), |ui| {
                self.menu_command(ui, AppCommand::PreviousView);
                self.menu_command(ui, AppCommand::HomeView);
                self.menu_command(ui, AppCommand::ViewIso);
                self.menu_command(ui, AppCommand::ViewTop);
                self.menu_command(ui, AppCommand::ViewBottom);
                self.menu_command(ui, AppCommand::ViewFront);
                self.menu_command(ui, AppCommand::ViewBack);
                self.menu_command(ui, AppCommand::ViewRight);
                self.menu_command(ui, AppCommand::ViewLeft);
                self.menu_command(ui, AppCommand::ZoomFit);
                self.menu_command(ui, AppCommand::ZoomSelection);
                self.menu_command(ui, AppCommand::CenterSelection);
                self.menu_command(ui, AppCommand::ZoomWindow);
                self.menu_command(ui, AppCommand::ZoomIn);
                self.menu_command(ui, AppCommand::ZoomOut);
                ui.separator();
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::ViewProjection);
                    self.menu_command(ui, AppCommand::View(ViewFlag::GridAxes));
                    self.menu_command(ui, AppCommand::View(ViewFlag::WhiteBackground));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Shadows));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Fog));
                    self.menu_command(ui, AppCommand::View(ViewFlag::HiddenObjects));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Xray));
                    self.menu_command(ui, AppCommand::ViewShaded);
                    self.menu_command(ui, AppCommand::View(ViewFlag::Wireframe));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Monochrome));
                    self.menu_command(ui, AppCommand::View(ViewFlag::HiddenLine));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Edges));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Profiles));
                    self.menu_command(ui, AppCommand::View(ViewFlag::DepthCue));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Endpoints));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::FadeDistantEdges));
                    self.menu_command(ui, AppCommand::View(ViewFlag::HighContrastEdges));
                    self.menu_command(ui, AppCommand::View(ViewFlag::SelectionHalo));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Midpoints));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Extensions));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Jitter));
                });
                ui.horizontal(|ui| {
                    self.menu_command(ui, AppCommand::View(ViewFlag::Dashes));
                    self.menu_command(ui, AppCommand::View(ViewFlag::ColorByAxis));
                    self.menu_command(ui, AppCommand::View(ViewFlag::Halos));
                });
                ui.separator();
                self.menu_command(ui, AppCommand::Hide);
                self.menu_command(ui, AppCommand::Unhide);
            });
            ui.menu_button(self.catalog.text("menu-draw"), |ui| {
                self.menu_command(ui, AppCommand::Line);
                self.menu_command(ui, AppCommand::Rectangle);
                self.menu_command(ui, AppCommand::Circle);
                self.menu_command(ui, AppCommand::Arc);
                self.menu_command(ui, AppCommand::Polygon);
                self.menu_command(ui, AppCommand::Ellipse);
                self.menu_command(ui, AppCommand::Spline);
            });
            ui.menu_button(self.catalog.text("menu-tools"), |ui| {
                self.menu_command(ui, AppCommand::Select);
                self.menu_command(ui, AppCommand::PushPull);
                self.menu_command(ui, AppCommand::Move);
                self.menu_command(ui, AppCommand::Rotate);
                self.menu_command(ui, AppCommand::Mirror);
                self.menu_command(ui, AppCommand::Scale);
                self.menu_command(ui, AppCommand::Measure);
                self.menu_command(ui, AppCommand::Orbit);
                self.menu_command(ui, AppCommand::Pan);
            });
            ui.menu_button(self.catalog.text("menu-model"), |ui| {
                self.menu_command(ui, AppCommand::PlanarOffset);
                self.menu_command(ui, AppCommand::Helix);
                self.menu_command(ui, AppCommand::Sweep);
                self.menu_command(ui, AppCommand::Loft);
                self.menu_command(ui, AppCommand::Revolve);
                self.menu_command(ui, AppCommand::Shell);
                self.menu_command(ui, AppCommand::Fillet);
                self.menu_command(ui, AppCommand::Chamfer);
                ui.separator();
                self.menu_command(ui, AppCommand::LinearPattern);
                self.menu_command(ui, AppCommand::RectangularPattern);
                self.menu_command(ui, AppCommand::CircularPattern);
                ui.separator();
                self.menu_command(ui, AppCommand::SolidSubtract);
                self.menu_command(ui, AppCommand::SolidTrim);
                self.menu_command(ui, AppCommand::SolidUnion);
                self.menu_command(ui, AppCommand::SolidIntersect);
                self.menu_command(ui, AppCommand::SolidSplit);
                ui.separator();
                self.menu_command(ui, AppCommand::Group);
                self.menu_command(ui, AppCommand::Ungroup);
                self.menu_command(ui, AppCommand::MakeComponent);
                self.menu_command(ui, AppCommand::MakeUnique);
                self.menu_command(ui, AppCommand::HideOthers);
                self.menu_command(ui, AppCommand::UnhideAll);
                self.menu_command(ui, AppCommand::PurgeUnused);
                self.menu_command(ui, AppCommand::ConvertSelectedMeshToExact);
                self.menu_command(ui, AppCommand::ReplaceComponent);
                self.menu_command(ui, AppCommand::SelectAllInstances);
                self.menu_command(ui, AppCommand::AssignTag);
                self.menu_command(ui, AppCommand::AlignOccurrences);
                self.menu_command(ui, AppCommand::DistributeOccurrences);
                self.menu_command(ui, AppCommand::GroundOccurrence);
                self.menu_command(ui, AppCommand::UngroundOccurrence);
                self.menu_command(ui, AppCommand::RenameOccurrence);
                self.menu_command(ui, AppCommand::RenameDefinition);
                ui.separator();
            });
            ui.menu_button(self.catalog.text("menu-window"), |ui| {
                ui.checkbox(
                    &mut self.panels.outliner_visible,
                    self.catalog.text("dock-outliner"),
                );
                ui.checkbox(
                    &mut self.panels.tags_visible,
                    self.catalog.text("dock-tags"),
                );
                ui.checkbox(
                    &mut self.panels.dimensions_visible,
                    self.catalog.text("dock-dimensions"),
                );
                ui.checkbox(
                    &mut self.panels.manual_cad_panels_visible,
                    self.catalog.text("dock-manual-cad"),
                );
                ui.separator();
                self.menu_command(ui, AppCommand::MaterialTakeoff);
            });
            ui.menu_button(self.catalog.text("menu-help"), |ui| {
                self.menu_command(ui, AppCommand::Shortcuts);
                self.menu_command(ui, AppCommand::About);
            });
            ui.separator();
            self.show_command_search(ui);
        });
    }

    /// Feature history, bodies and assembly joints serve manual CAD editing only;
    /// they stay hidden unless Window > Manual CAD panels turns them on.
    pub(crate) fn show_manual_cad_panels(&mut self, ui: &mut egui::Ui) {
        if !self.panels.manual_cad_panels_visible {
            return;
        }
        self.show_feature_history(ui);
        self.show_body_editor(ui);
        self.show_assembly_editor(ui);
    }

    pub(crate) fn show_about_window(&mut self, context: &egui::Context) {
        if !self.panels.about_open {
            return;
        }
        let mut open = true;
        egui::Window::new(self.catalog.text("help-about"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .show(context, |ui| {
                ui.heading(self.catalog.text("app-title"));
                ui.label(self.catalog.text("about-description"));
                ui.label(self.catalog.format(
                    "about-version",
                    &BTreeMap::from([("version", Self::build_version().to_owned())]),
                ));
                ui.label(self.catalog.format(
                    "about-license",
                    &BTreeMap::from([("license", env!("CARGO_PKG_LICENSE").to_owned())]),
                ));
                ui.separator();
                if ui.button(self.catalog.text("about-close")).clicked() {
                    self.panels.about_open = false;
                }
            });
        if !open {
            self.panels.about_open = false;
        }
    }

    pub(crate) fn show_status_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette();
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 7.0;
            let (dot, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
            ui.painter()
                .circle_filled(dot.center(), 3.0, palette.accent);
            ui.label(
                egui::RichText::new(self.catalog.text(self.active_tool.label_key()))
                    .strong()
                    .color(palette.text),
            );

            // Everything after the tool name is running commentary, so it is set
            // in the tertiary tone and truncated rather than allowed to push the
            // measured chips off the right edge.
            let mut context = vec![self.catalog.format(
                "status-selected",
                &BTreeMap::from([("count", self.selection_count().to_string())]),
            )];
            if let Some(edit_context) = self.selection.edit_context.last() {
                let (key, id) = match edit_context {
                    EditContext::Group(id) => ("status-editing-group", id.0),
                    EditContext::Definition { definition_id, .. } => {
                        ("status-editing-component", definition_id.0)
                    }
                };
                context.push(
                    self.catalog
                        .format(key, &BTreeMap::from([("id", id.to_string())])),
                );
            }
            context.push(self.digest.clone());
            ui.add(
                egui::Label::new(
                    egui::RichText::new(context.join("  \u{b7}  "))
                        .size(11.5)
                        .color(palette.faint),
                )
                .truncate(),
            );

            // The measured facts are pinned right, in the same mono pill the
            // viewport readouts use, so they line up down the whole session.
            let mut chips = vec![
                self.catalog.text(if self.face_workflow.snaps_enabled() {
                    "status-snap-on"
                } else {
                    "status-snap-off"
                }),
                if self.view.contains(ViewFlag::GridAxes) {
                    self.catalog
                        .format("status-grid", &BTreeMap::from([("step", "10".to_owned())]))
                } else {
                    self.catalog.text("status-grid-hidden")
                },
                self.catalog.text("status-refs-guaranteed"),
            ];
            chips.push(if self.exact.results.is_empty() {
                self.catalog.text("status-exact-unavailable")
            } else {
                let (bodies, refs) = self.exact_status_counts();
                self.catalog.format(
                    "status-exact-current",
                    &BTreeMap::from([("bodies", bodies.to_string()), ("refs", refs.to_string())]),
                )
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_space(16.0);
                ui.label(
                    egui::RichText::new(format!("v{}", Self::build_version()))
                        .size(9.5)
                        .color(palette.faint),
                );
                for text in chips.into_iter().rev() {
                    status_chip(ui, palette, &text);
                }
            });
        });
    }

    /// Draw the whole designed shell into an `egui` context.
    ///
    /// This is the single entry point used both by the windowed `eframe`
    /// integration and by the offscreen [`crate::testing::HeadlessShell`].
    pub fn ui(&mut self, context: &egui::Context) {
        self.begin_live_image_frame();
        self.poll_live_consent(context);
        self.poll_live_bridge(context);
        self.poll_validator_panel(context);
        self.poll_mesh_conversion(context);
        if let Some(program) = self.document.current_rule_program() {
            self.program_evaluations.warm(program, context);
        }
        self.refresh_exact_products(context);
        let palette = self.palette();
        apply_shell_style(context, palette);
        self.handle_shortcuts(context);

        // Chrome surfaces carry a hairline on the edge they meet the viewport
        // on, so the shell reads as panels around a canvas rather than as one
        // flat sheet.
        let hairline = Stroke::new(1.0_f32, palette.line);
        let chrome = |bottom: bool, top: bool| {
            egui::Frame::new()
                .fill(palette.chrome)
                .inner_margin(egui::Margin::symmetric(12, 0))
                .stroke(Stroke::NONE)
                .outer_margin(egui::Margin::ZERO)
                .shadow(egui::epaint::Shadow::NONE)
                .stroke(if bottom || top {
                    hairline
                } else {
                    Stroke::NONE
                })
        };
        egui::TopBottomPanel::top("top-bar")
            .exact_height(46.0)
            .frame(chrome(true, false))
            .show(context, |ui| self.show_top_bar(ui));
        egui::TopBottomPanel::top("menu-bar")
            .exact_height(32.0)
            .frame(chrome(true, false))
            .show(context, |ui| self.show_menu_bar(ui));
        egui::TopBottomPanel::bottom("status-bar")
            .exact_height(32.0)
            .frame(chrome(false, true))
            .show(context, |ui| self.show_status_bar(ui));
        egui::SidePanel::left("tool-rail")
            .resizable(false)
            .exact_width(TOOL_RAIL_WIDTH)
            .frame(
                egui::Frame::new()
                    .fill(palette.chrome)
                    .inner_margin(egui::Margin::symmetric(0, 8))
                    .stroke(hairline),
            )
            .show(context, |ui| self.show_tool_rail(ui));
        if self.assistant.workspace_mode == AssistantWorkspaceMode::Dock {
            egui::SidePanel::right("right-dock")
                .resizable(true)
                .default_width(440.0)
                .width_range(380.0..=720.0)
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .stroke(hairline),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    dock_scroll_area().show(ui, |ui| {
                        let layers_first = self.layers_lead_dock();
                        self.show_face_workflow_ui(ui);
                        if layers_first {
                            self.show_layers_section(ui, true);
                        }
                        self.show_program_source(ui);
                        self.show_manual_cad_panels(ui);
                        self.show_occurrence_color_editor(ui);
                        self.show_helix_tool(ui);
                        self.show_parameter_editor(ui);
                        if !Self::is_manual_alpha_build() {
                            self.show_assistant(ui);
                        }
                        self.show_validator_panel(ui);
                        // Below the docked assistant so its input stays in view.
                        self.show_outliner_without_assistant(ui);
                        if !layers_first {
                            self.show_layers_section(ui, false);
                        }
                    });
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(palette.viewport_outer))
                .show(context, |ui| self.viewport(ui));
        } else {
            egui::SidePanel::right("right-dock")
                .resizable(true)
                .default_width(340.0)
                .width_range(280.0..=520.0)
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::symmetric(14, 8))
                        .stroke(hairline),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    dock_scroll_area().show(ui, |ui| {
                        let layers_first = self.layers_lead_dock();
                        self.show_face_workflow_ui(ui);
                        if layers_first {
                            self.show_layers_section(ui, true);
                        }
                        self.show_program_source(ui);
                        self.show_outliner_without_assistant(ui);
                        if !layers_first {
                            self.show_layers_section(ui, false);
                        }
                        self.show_manual_cad_panels(ui);
                        self.show_occurrence_color_editor(ui);
                        self.show_helix_tool(ui);
                        self.show_parameter_editor(ui);
                        self.show_validator_panel(ui);
                    });
                });
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::new()
                        .fill(palette.chrome)
                        .inner_margin(egui::Margin::same(16)),
                )
                .show(context, |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                    if self.live_offscreen_image_pending() {
                        let mut hidden_viewport = ui.new_child(
                            egui::UiBuilder::new()
                                .id_salt("live-offscreen-viewport")
                                .max_rect(ui.max_rect())
                                .invisible(),
                        );
                        self.viewport(&mut hidden_viewport);
                    }
                    if !Self::is_manual_alpha_build() {
                        self.show_assistant(ui);
                    }
                });
        }
        self.show_occurrence_rename_window(context);
        self.show_definition_rename_window(context);
        self.show_component_replacement_window(context);
        self.show_tag_creation_window(context);
        self.show_tag_deletion_window(context);
        self.show_tag_clear_window(context);
        self.show_tag_rename_window(context);
        self.show_tag_assignment_window(context);
        self.show_occurrence_align_window(context);
        self.show_occurrence_distribution_window(context);
        self.show_linear_pattern_window(context);
        self.show_rectangular_pattern_window(context);
        self.show_circular_pattern_window(context);
        self.show_cam_export_window(context);
        self.show_fea_review_window(context);
        self.show_pdm_review_window(context);
        self.show_stl_import_window(context);
        self.show_dxf_import_window(context);
        self.show_step_import_window(context);
        self.show_iges_import_window(context);
        self.show_sketchup_scene_import_window(context);
        self.show_glb_import_window(context);
        self.show_mesh_conversion_window(context);
        self.show_shortcuts_window(context);
        self.show_about_window(context);
        self.show_material_takeoff_window(context);
        self.show_project_drawings_window(context);
        self.show_live_consent(context);
        self.poll_assistant_chat(context);
        self.finish_live_image_frame(context);
        self.show_close_guard(context);
        self.refresh_work_recovery_checkpoint();
    }
}

/// One shared type scale and spacing rhythm for every surface of the shell.
///
/// Before this existed each panel picked its own font size, so a section title
/// was nearly twice the size of the text under it. The whole shell now derives
/// from four sizes: title, body, small and monospace.
fn apply_shell_style(context: &egui::Context, palette: Palette) {
    let mut visuals = if palette.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    visuals.override_text_color = Some(palette.text);
    visuals.panel_fill = palette.chrome;
    visuals.window_fill = palette.panel;
    visuals.window_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.extreme_bg_color = palette.bg;
    visuals.selection.bg_fill = palette.accent_wash(90);
    visuals.selection.stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.hyperlink_color = palette.accent;
    visuals.widgets.noninteractive.bg_fill = palette.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, palette.dim);
    visuals.widgets.inactive.weak_bg_fill = palette.panel;
    visuals.widgets.inactive.bg_fill = palette.panel;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.hovered.weak_bg_fill = palette.panel2;
    visuals.widgets.hovered.bg_fill = palette.panel2;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, palette.line);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.active.weak_bg_fill = palette.panel2;
    visuals.widgets.active.bg_fill = palette.panel2;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, palette.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, palette.text);
    visuals.widgets.open.weak_bg_fill = palette.panel2;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, palette.line);
    for widget in [
        &mut visuals.widgets.noninteractive,
        &mut visuals.widgets.inactive,
        &mut visuals.widgets.hovered,
        &mut visuals.widgets.active,
        &mut visuals.widgets.open,
    ] {
        widget.corner_radius = egui::CornerRadius::same(6);
    }

    let mut style = (*context.style()).clone();
    style.text_styles = [
        (
            egui::TextStyle::Heading,
            egui::FontId::proportional(SHELL_TITLE_SIZE),
        ),
        (
            egui::TextStyle::Body,
            egui::FontId::proportional(SHELL_BODY_SIZE),
        ),
        (
            egui::TextStyle::Button,
            egui::FontId::proportional(SHELL_BODY_SIZE),
        ),
        (
            egui::TextStyle::Small,
            egui::FontId::proportional(SHELL_SMALL_SIZE),
        ),
        (
            egui::TextStyle::Monospace,
            egui::FontId::monospace(SHELL_MONO_SIZE),
        ),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(6.0, 5.0);
    style.spacing.button_padding = Vec2::new(8.0, 4.0);
    style.spacing.interact_size.y = 22.0;
    style.spacing.combo_width = 180.0;
    style.spacing.text_edit_width = 160.0;
    style.visuals = visuals;
    context.set_style(style);
}

/// Draw a dock section title so every section reads at the same weight.
/// The right dock stacks every editor panel, so its content is routinely
/// taller than the window. Without a scroll area the overflow is simply
/// unreachable: the panel below the fold is painted outside the dock and no
/// click ever lands on it, which reads as a dead button rather than as
/// content that is merely out of view.
fn dock_scroll_area() -> egui::ScrollArea {
    egui::ScrollArea::vertical()
        .id_salt("right-dock-scroll")
        .auto_shrink([false, false])
}

pub(crate) fn section_header(ui: &mut egui::Ui, palette: Palette, title: &str) {
    ui.add_space(9.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .size(SHELL_SECTION_SIZE)
                .strong()
                .color(palette.dim),
        );
    });
    ui.add_space(4.0);
}

/// One measured fact in the status bar, as a dotted monospace pill.
fn status_chip(ui: &mut egui::Ui, palette: Palette, text: &str) {
    let galley =
        ui.painter()
            .layout_no_wrap(text.to_owned(), egui::FontId::monospace(10.5), palette.dim);
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(galley.size().x + 26.0, galley.size().y + 6.0),
        Sense::hover(),
    );
    let painter = ui.painter();
    let corner = egui::CornerRadius::same(5);
    painter.rect_filled(rect, corner, palette.panel);
    painter.rect_stroke(
        rect,
        corner,
        Stroke::new(1.0_f32, palette.line),
        egui::StrokeKind::Inside,
    );
    painter.circle_filled(
        Pos2::new(rect.left() + 9.0, rect.center().y),
        2.5,
        palette.accent,
    );
    painter.galley(
        Pos2::new(rect.left() + 17.0, rect.center().y - galley.size().y * 0.5),
        galley,
        palette.dim,
    );
}

/// A hairline divider between two clusters inside a horizontal bar.
fn vertical_rule(ui: &mut egui::Ui, palette: Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(1.0, 20.0), Sense::hover());
    ui.painter().rect_filled(rect, 0.0, palette.line);
}

/// Wrap `content` in the shell's segmented-control shell: a raised, outlined,
/// rounded tray whose children are flush pills.
pub(crate) fn segmented(ui: &mut egui::Ui, palette: Palette, content: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(palette.panel)
        .stroke(Stroke::new(1.0_f32, palette.line))
        .corner_radius(egui::CornerRadius::same(9))
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().button_padding.x = 10.0;
            ui.horizontal(content);
        });
}

/// The square a glyph of `size` occupies inside a larger hit target.
pub(crate) fn shrink_to_icon(button: Rect, size: f32) -> Rect {
    Rect::from_center_size(button.center(), Vec2::splat(size))
}

/// Give a widget an accessible name.
///
/// Icon-only controls paint a glyph, which is useless both to a screen reader
/// and to an acceptance test. This publishes the command's localized name to
/// the accessibility tree instead.
pub(crate) fn name_widget(response: &egui::Response, enabled: bool, name: &str) {
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, name));
}

/// A button that shows only a glyph. `name` is its accessible name and its
/// hover hint, shown also while the button is disabled.
pub(crate) fn icon_button(
    ui: &mut egui::Ui,
    enabled: bool,
    glyph: &str,
    name: &str,
) -> egui::Response {
    let response = ui.add_enabled(enabled, egui::Button::new(glyph));
    name_widget(&response, enabled, name);
    response.on_hover_text(name).on_disabled_hover_text(name)
}
