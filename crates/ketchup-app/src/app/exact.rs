//! Exact worker connection, exact products and topology results, and solid tools.

use crate::*;

impl KetchupApp {
    pub(crate) fn validate_exact_exchange_extension(
        path: &Path,
        format: &str,
        allowed_extensions: &[&str],
    ) -> Result<(), Rejection> {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default();
        if allowed_extensions
            .iter()
            .any(|allowed| extension.eq_ignore_ascii_case(allowed))
        {
            return Ok(());
        }
        let refusal = |code| {
            Rejection::new(code, RejectionPhase::Validation).target(path.display().to_string())
        };
        Err(match extension.to_ascii_lowercase().as_str() {
            "sldprt" | "sldasm" => refusal("exchange.native_format").reason(
                "native SolidWorks part and assembly files are unsupported; use an audited STEP or IGES exchange file",
            ),
            "x_t" | "x_b" => refusal("exchange.native_format").reason(
                "native Parasolid files are unsupported; use an audited STEP or IGES exchange file",
            ),
            _ => refusal("exchange.extension").reason(format!(
                "{format} requires an explicit .{} file; content sniffing and extension substitution are refused",
                allowed_extensions.join(" or .")
            )),
        })
    }

    pub(crate) fn exact_worker_executable(&mut self) -> Result<PathBuf, Rejection> {
        if !self.exact.worker_attempted {
            self.exact.worker_attempted = true;
            self.exact.worker_path = exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file());
        }
        self.exact
            .worker_path
            .clone()
            .ok_or_else(exact_worker_unavailable)
    }

    pub(crate) fn current_visible_exact_scene(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<(ExactBodyPackage, SceneOccurrence)>, ExportError> {
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
        if let Some(occurrence) = occurrences
            .iter()
            .find(|occurrence| definition_mesh_body(snapshot, occurrence.definition_id).is_some())
        {
            return Err(ExportError::OccurrenceNotExportable {
                occurrence: occurrence.instance_path.clone(),
                reason: "is a mesh body without verified exact geometry; exact-derived export is unavailable until explicit exact conversion",
            });
        }
        let mut scene = Vec::new();
        for occurrence in occurrences {
            let packages = self
                .exact
                .results
                .render_values(snapshot)
                .filter(|package| package.definition_id() == occurrence.definition_id)
                .collect::<Vec<_>>();
            if packages.is_empty() {
                return Err(ExportError::OccurrenceNotExportable {
                    occurrence: occurrence.instance_path.clone(),
                    reason: "has no current accepted exact result",
                });
            }
            scene.extend(
                packages
                    .into_iter()
                    .map(|package| ((**package).clone(), occurrence.clone())),
            );
        }
        Ok(scene)
    }

    #[must_use]
    pub fn native_options() -> eframe::NativeOptions {
        Self::native_options_for_adapter(None, Arc::new(Mutex::new(None)))
    }

    #[must_use]
    pub fn native_options_for_adapter(
        requirement: Option<AdapterRequirement>,
        selected_info: SelectedAdapterInfo,
    ) -> eframe::NativeOptions {
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::default();
        setup.instance_descriptor.backends = eframe::wgpu::Backends::DX12;
        setup.native_adapter_selector = Some(Arc::new(move |adapters, surface| {
            let mut matching = adapters.iter().filter(|adapter| {
                let info = adapter.get_info();
                info.backend == eframe::wgpu::Backend::Dx12
                    && !matches!(
                        info.device_type,
                        eframe::wgpu::DeviceType::Cpu | eframe::wgpu::DeviceType::VirtualGpu
                    )
                    && requirement.as_ref().is_none_or(|required| {
                        info.name == required.name && info.device_type == required.device_type
                    })
                    && surface.is_none_or(|surface| adapter.is_surface_supported(surface))
            });
            let selected = matching.next().ok_or_else(|| {
                "no Direct3D 12 physical adapter matched the frozen requirement".to_owned()
            })?;
            if matching.next().is_some() {
                return Err(
                    "multiple Direct3D 12 physical adapters matched the frozen requirement"
                        .to_owned(),
                );
            }
            let info = selected.get_info();
            *selected_info
                .lock()
                .map_err(|_: std::sync::PoisonError<_>| {
                    "selected-adapter evidence lock is unavailable".to_owned()
                })? = Some(info);
            Ok(selected.clone())
        }));

        eframe::NativeOptions {
            renderer: eframe::Renderer::Wgpu,
            wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
                present_mode: eframe::wgpu::PresentMode::AutoNoVsync,
                wgpu_setup: setup.into(),
                ..Default::default()
            },
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([1_100.0, 720.0])
                .with_min_inner_size([1_100.0, 600.0]),
            ..Default::default()
        }
    }

    /// Carry the exact products of the previous revision over to `snapshot`.
    ///
    /// A viewport drag commits its revision in the same frame that paints it,
    /// so products still bound to the revision before it would leave that one
    /// frame with nothing to draw and nothing to pick. Rebinding here, before
    /// the frame reads them, keeps the frame whole. Nothing is loosened:
    /// carrying forward re-checks every product against `snapshot` and drops
    /// whatever it no longer carries the evidence for.
    pub(crate) fn rebind_exact_results(&mut self, snapshot: &Snapshot) {
        Self::archive_exact_registry(&self.exact.results, &mut self.exact.result_history);
        Self::archive_exact_registry(
            &self.exact.topology_results,
            &mut self.exact.topology_result_history,
        );
        let source = ketchup_application::evaluation::exact_source(snapshot);
        if let Some(saved) = self.exact.result_history.get(&source) {
            self.exact.results = saved.clone();
        }
        if let Some(saved) = self.exact.topology_result_history.get(&source) {
            self.exact.topology_results = saved.clone();
        }
        ketchup_application::evaluation::rebind_exact_results(
            snapshot,
            &mut self.exact.results,
            &mut self.exact.topology_results,
        );
    }

    pub(crate) fn archive_exact_registry(
        registry: &ExactResultRegistry,
        history: &mut BTreeMap<ExactSource, ExactResultRegistry>,
    ) {
        let mut packages = registry.values();
        let Some(first) = packages.next() else {
            return;
        };
        let key = first.result_key();
        let source = ExactSource::from_source(
            key.document_id,
            key.source_revision,
            key.source_digest.clone(),
        );
        if packages.all(|package| {
            let key = package.result_key();
            key.document_id == source.document_id()
                && key.source_revision == source.source_revision()
                && key.source_digest == source.source_digest()
        }) {
            history.insert(source, registry.clone());
        }
    }

    pub(crate) fn exact_results_for_snapshot(
        &self,
        snapshot: &Snapshot,
    ) -> Option<&ExactResultRegistry> {
        if !self.exact.results.is_empty() && self.exact.results.is_bound_to(snapshot) {
            return Some(&self.exact.results);
        }
        self.exact
            .result_history
            .get(&ketchup_application::evaluation::exact_source(snapshot))
    }

    pub(crate) fn topology_results_for_snapshot(
        &self,
        snapshot: &Snapshot,
    ) -> Option<&ExactResultRegistry> {
        if !self.exact.topology_results.is_empty()
            && self.exact.topology_results.is_bound_to(snapshot)
        {
            return Some(&self.exact.topology_results);
        }
        self.exact
            .topology_result_history
            .get(&ketchup_application::evaluation::exact_source(snapshot))
    }

    pub(crate) fn refresh_exact_products(&mut self, context: &egui::Context) {
        self.poll_face_offset_evaluation(context);
        let snapshot = self.document.current();
        let source = ketchup_application::evaluation::exact_source(&snapshot);
        if self
            .exact
            .source
            .as_ref()
            .is_some_and(|known| known != &source)
        {
            self.exact.source = None;
        }
        self.rebind_exact_results(&snapshot);
        if self
            .exact
            .task
            .as_ref()
            .is_some_and(|task| task.source != source)
        {
            self.exact.task.take();
        }
        if let Some(task) = self.exact.task.as_ref() {
            match task.poll() {
                Ok(result) => {
                    let task = self.exact.task.take().expect("completed task exists");
                    let published = match result {
                        Ok(products) => self
                            .complete_mutation_and_exact_results_with_work_recovery(
                                |document, exact_results, topology_results| {
                                    ketchup_application::evaluation::publish_exact_products(
                                        document,
                                        exact_results,
                                        topology_results,
                                        &task,
                                        products,
                                    )
                                },
                            ),
                        Err(error) => Err(WorkRecoveryMutationError::Mutation(error)),
                    };
                    match published {
                        Ok(report) => {
                            self.render.plan = Some(Arc::new(InstancedRenderPlan::from_snapshot(
                                &snapshot,
                                &self.exact.results,
                                &mut self.render.cache,
                            )));
                            self.hover.projection_cache.get_mut().take();
                            self.exact.source = (!report.needs_retry()).then(|| source.clone());
                            self.exact.retry_at = report
                                .needs_retry()
                                .then(|| Instant::now() + Duration::from_secs(1));
                            // Continue through the shared retry wake-up below.
                        }
                        Err(error) => {
                            eprintln!("exact evaluation rejected: {error}");
                            self.exact.retry_at = Some(Instant::now() + Duration::from_secs(1));
                        }
                    }
                }
                Err(TryRecvError::Empty) => return,
                Err(TryRecvError::Disconnected) => {
                    self.exact.task.take();
                    self.exact.retry_at = Some(Instant::now() + Duration::from_secs(1));
                }
            }
        }
        if self.exact.source.as_ref() == Some(&source) {
            return;
        }
        if let Some(retry) = self.exact.retry_at {
            let remaining = retry.saturating_duration_since(Instant::now());
            if !remaining.is_zero() {
                context.request_repaint_after(remaining);
                return;
            }
        }
        if !self.exact.worker_attempted {
            self.exact.worker_attempted = true;
            self.exact.worker_path = exact_worker_candidates()
                .into_iter()
                .find(|path| path.is_file());
        }
        let repaint = context.clone();
        self.exact.task = Some(ketchup_application::evaluation::start_exact_evaluation(
            snapshot,
            &self.file.container_data,
            &self.exact.results,
            &self.exact.topology_results,
            self.exact.worker_path.clone(),
            move || repaint.request_repaint(),
        ));
    }

    #[doc(hidden)]
    pub fn headless_force_exact_worker_path(&mut self, executable: impl AsRef<Path>) {
        self.exact.worker_path = Some(executable.as_ref().to_owned());
        self.exact.worker_attempted = true;
    }

    #[doc(hidden)]
    pub fn headless_install_exact_package(&mut self, package: ExactBodyPackage) -> bool {
        if let Some(task) = self.exact.task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        let snapshot = self.document.current();
        let source = ketchup_application::evaluation::exact_source(&snapshot);
        let package = Arc::new(package);
        let mut exact_results = ExactResultRegistry::default();
        let mut topology_results = ExactResultRegistry::default();
        let inserted = exact_results
            .insert_current(&snapshot, Arc::clone(&package))
            .is_ok()
            && (package.topological_references().is_empty()
                || topology_results.insert_current(&snapshot, package).is_ok());
        if inserted {
            self.exact.results = exact_results;
            self.exact.topology_results = topology_results;
            self.exact.source = Some(source);
            self.exact.retry_at = None;
        }
        inserted
    }

    pub fn connect_exact_worker(
        &mut self,
        executable: impl AsRef<Path>,
    ) -> Result<(), std::io::Error> {
        let executable = executable.as_ref();
        if !executable.is_file() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!(
                    "exact worker executable {} was not found",
                    executable.display()
                ),
            ));
        }
        if let Some(task) = self.exact.task.take() {
            task.cancelled.store(true, Ordering::Release);
        }
        self.exact.worker_path = Some(executable.to_owned());
        self.exact.worker_attempted = true;
        self.exact.results.clear();
        self.exact.topology_results.clear();
        self.exact.result_history.clear();
        self.exact.topology_result_history.clear();
        self.exact.source = None;
        self.exact.retry_at = None;
        Ok(())
    }

    pub(crate) fn visible_exact_packages<'a>(
        &'a self,
        snapshot: &'a Snapshot,
    ) -> Vec<&'a Arc<ExactBodyPackage>> {
        let visible_definitions = snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .map(|occurrence| occurrence.definition_id)
            .collect::<BTreeSet<_>>();
        self.exact
            .results
            .render_values(snapshot)
            .filter(|package| visible_definitions.contains(&package.definition_id()))
            .collect()
    }

    #[must_use]
    pub fn exact_render_body_count(&self) -> usize {
        let snapshot = self.document.current();
        self.visible_exact_packages(&snapshot).len()
    }

    #[doc(hidden)]
    #[must_use]
    pub fn exact_current_producer_ids(&self) -> Vec<FeatureId> {
        let snapshot = self.document.current();
        self.visible_exact_packages(&snapshot)
            .into_iter()
            .map(|package| package.producer_feature_id())
            .collect()
    }

    #[must_use]
    pub fn exact_render_bounds(&self) -> Vec<[[f64; 3]; 2]> {
        let snapshot = self.document.current();
        self.visible_exact_packages(&snapshot)
            .into_iter()
            .map(|package| package.bounds_mm())
            .collect()
    }

    /// How many triangles the current exact render products actually carry.
    ///
    /// An exact body with no triangles is invisible and unpickable, which is
    /// indistinguishable from a failed import, so tests assert on this.
    #[must_use]
    pub fn exact_render_triangle_count(&self) -> usize {
        let snapshot = self.document.current();
        self.visible_exact_packages(&snapshot)
            .into_iter()
            .map(|package| package.triangles().len())
            .sum()
    }

    #[must_use]
    pub fn exact_stable_reference_count(&self) -> usize {
        let snapshot = self.document.current();
        self.visible_exact_packages(&snapshot)
            .into_iter()
            .map(|package| package.references().len())
            .sum()
    }

    #[must_use]
    pub fn exact_pick_durable(&self, ray: Ray) -> Option<AssemblySelectionTarget> {
        let snapshot = self.document.current();
        self.exact_projection(&snapshot)
            .exact_pick(ray)
            .map(|hit| hit.target)
    }

    #[must_use]
    pub fn latest_topology_shell_parameters(
        &self,
    ) -> Option<(FeatureId, TopologicalElementRef, f64)> {
        let (feature, references, thickness) = self.latest_topology_shell_set_parameters()?;
        Some((feature, references.into_iter().next()?, thickness))
    }

    #[must_use]
    pub fn latest_topology_shell_set_parameters(
        &self,
    ) -> Option<(FeatureId, Vec<TopologicalElementRef>, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::Shell {
                    removed_faces,
                    thickness,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((
                    feature.id(),
                    removed_faces
                        .iter()
                        .filter_map(FaceRef::topological)
                        .cloned()
                        .collect(),
                    thickness.millimetres(),
                ))
            })
            .last()
    }

    #[must_use]
    pub fn latest_topology_edge_finish_parameters(
        &self,
    ) -> Option<(FeatureId, TopologicalElementRef, EdgeFinishKind, f64)> {
        let (feature, references, kind, amount) =
            self.latest_topology_edge_finish_set_parameters()?;
        Some((feature, references.into_iter().next()?, kind, amount))
    }

    #[must_use]
    pub fn latest_topology_edge_finish_set_parameters(
        &self,
    ) -> Option<(FeatureId, Vec<TopologicalElementRef>, EdgeFinishKind, f64)> {
        self.document
            .current()
            .features()
            .filter_map(|feature| {
                let FeatureKind::EdgeFinish {
                    edges,
                    kind,
                    amount,
                    ..
                } = feature.kind()
                else {
                    return None;
                };
                Some((
                    feature.id(),
                    edges
                        .iter()
                        .filter_map(EdgeRef::topological)
                        .cloned()
                        .collect(),
                    *kind,
                    amount.millimetres(),
                ))
            })
            .last()
    }

    pub(crate) fn active_solid_tool_operation(&self) -> Option<BooleanOperation> {
        match self.active_tool {
            ActiveTool::SolidSubtract | ActiveTool::SolidTrim => Some(BooleanOperation::Cut),
            ActiveTool::SolidUnion => Some(BooleanOperation::Union),
            ActiveTool::SolidIntersect => Some(BooleanOperation::Intersect),
            ActiveTool::SolidSplit => Some(BooleanOperation::Split),
            _ => None,
        }
    }

    pub(crate) fn solid_tool_candidate(
        &self,
        selection: &SelectionId,
    ) -> Option<(RenderBox, FeatureId)> {
        if !selection.instance_path.is_root()
            || !self.occurrence_in_active_context(&selection.instance_path)
        {
            return None;
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        let snapshot = self.document.current();
        let body_feature_id = exact_solid_tool_feature_id(&snapshot, selection.definition_id)?;
        let occurrence = snapshot.occurrence(selection.instance_path.root_occurrence())?;
        snapshot
            .world_transform_for_occurrence(occurrence.id())?
            .rigid_inverse()?;
        let feature = snapshot.feature(body_feature_id)?;
        if !matches!(
            feature.kind(),
            FeatureKind::Pad(PadSpec {
                profile: PadProfile::Feature(_),
                extent: FeatureExtent::Blind(_),
                operation: PadOperation::NewBody,
                ..
            }) | FeatureKind::ImportedExactBody(_)
        ) && self
            .exact
            .results
            .get_render(&snapshot, selection.definition_id)
            .is_none()
        {
            return None;
        }
        if occurrence.definition_id() != selection.definition_id
            || feature.definition_id() != selection.definition_id
        {
            return None;
        }
        Some((item, body_feature_id))
    }

    pub(crate) fn solid_tool_source_plan(
        &self,
        target_selection: SelectionId,
        tool_selection: SelectionId,
        keep_tool: bool,
    ) -> Option<SolidToolSourcePlan> {
        let operation = self.active_solid_tool_operation()?;
        let keep_tool = operation == BooleanOperation::Split || keep_tool;
        if target_selection.instance_path == tool_selection.instance_path {
            return None;
        }
        let (target_box, target_feature_id) = self.solid_tool_candidate(&target_selection)?;
        let (tool_box, tool_feature_id) = self.solid_tool_candidate(&tool_selection)?;
        let snapshot = self.document.current();
        let target = snapshot.occurrence(target_selection.instance_path.root_occurrence())?;
        let tool = snapshot.occurrence(tool_selection.instance_path.root_occurrence())?;
        let result_definition_id = DefinitionId(
            snapshot
                .definitions()
                .map(|definition| definition.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let first_feature_value = snapshot
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap_or(0)
            .checked_add(1)?;
        let result_feature_count = snapshot
            .solid_tool_result_feature_count(target_feature_id, tool_feature_id)
            .ok()?;
        let result_feature_ids = (0..result_feature_count)
            .map(|offset| {
                u64::try_from(offset)
                    .ok()
                    .and_then(|offset| first_feature_value.checked_add(offset))
                    .map(FeatureId)
            })
            .collect::<Option<Vec<_>>>()?;
        Some(SolidToolSourcePlan {
            source_document_id: snapshot.document_id(),
            source_revision: snapshot.revision_id(),
            source_digest: snapshot.canonical_digest(),
            source_primary: self.selection.primary.clone(),
            source_selected_group: self.selection.selected_group,
            edit_context: self.selection.edit_context.clone(),
            operation,
            keep_tool,
            target_selection,
            target_name: target.name().to_owned(),
            target_transform: target.transform(),
            target_parent: target.parent(),
            target_tag: target.tag(),
            target_visible: target.visible(),
            target_box,
            target_feature_id,
            tool_selection,
            tool_name: tool.name().to_owned(),
            tool_transform: tool.transform(),
            tool_parent: tool.parent(),
            tool_tag: tool.tag(),
            tool_visible: tool.visible(),
            tool_box,
            tool_feature_id,
            result_definition_id,
            result_feature_ids,
        })
    }

    pub(crate) fn solid_tool_preview_plan(
        &self,
        source: &SolidToolSourcePlan,
    ) -> Option<SolidToolPreviewPlan> {
        let current = self.solid_tool_source_plan(
            source.target_selection.clone(),
            source.tool_selection.clone(),
            source.keep_tool,
        )?;
        if !source.same_canonical_identity(&current) {
            return None;
        }
        let operation_label = self
            .catalog
            .text(match (source.operation, source.keep_tool) {
                (BooleanOperation::Cut, true) => "solid-tool-trim",
                (BooleanOperation::Cut, false) => "solid-tool-subtract",
                (BooleanOperation::Union, _) => "solid-tool-union",
                (BooleanOperation::Intersect, _) => "solid-tool-intersect",
                (BooleanOperation::Split, _) => "solid-tool-split",
            });
        let command = CanonicalCommand::ApplySolidTool(SolidToolPlan {
            operation: source.operation,
            target_occurrence_id: source.target_selection.instance_path.root_occurrence(),
            target_feature_id: source.target_feature_id,
            tool_occurrence_id: source.tool_selection.instance_path.root_occurrence(),
            tool_feature_id: source.tool_feature_id,
            result_definition_id: source.result_definition_id,
            result_feature_ids: source.result_feature_ids.clone(),
            result_definition_name: self.catalog.format(
                "solid-tool-result-definition",
                &BTreeMap::from([("operation", operation_label.clone())]),
            ),
            result_feature_name: self.catalog.format(
                "solid-tool-result-feature",
                &BTreeMap::from([("operation", operation_label)]),
            ),
            keep_tool: source.keep_tool,
        });
        let batch = CommandBatch::new(vec![command.clone()]);
        self.document.validate_batch(&batch).ok()?;

        let mut preview_box = source.target_box.clone();
        preview_box.definition_id = source.result_definition_id;
        if source.result_feature_ids.len() == 5 && source.target_box.extrusion_feature_id.is_some()
        {
            preview_box.profile_feature_id = source.result_feature_ids[0];
            preview_box.extrusion_feature_id = Some(source.result_feature_ids[1]);
        } else {
            preview_box.profile_feature_id = *source.result_feature_ids.last()?;
            preview_box.extrusion_feature_id = None;
        }
        if source.operation == BooleanOperation::Union {
            let minimum = Vec3::new(
                source
                    .target_box
                    .origin_mm
                    .x
                    .min(source.tool_box.origin_mm.x),
                source
                    .target_box
                    .origin_mm
                    .y
                    .min(source.tool_box.origin_mm.y),
                source
                    .target_box
                    .origin_mm
                    .z
                    .min(source.tool_box.origin_mm.z),
            );
            let target_maximum = source.target_box.origin_mm + source.target_box.size_mm;
            let tool_maximum = source.tool_box.origin_mm + source.tool_box.size_mm;
            let maximum = Vec3::new(
                target_maximum.x.max(tool_maximum.x),
                target_maximum.y.max(tool_maximum.y),
                target_maximum.z.max(tool_maximum.z),
            );
            preview_box.origin_mm = minimum;
            preview_box.size_mm = maximum - minimum;
        } else if matches!(
            source.operation,
            BooleanOperation::Intersect | BooleanOperation::Split
        ) {
            let minimum = Vec3::new(
                source
                    .target_box
                    .origin_mm
                    .x
                    .max(source.tool_box.origin_mm.x),
                source
                    .target_box
                    .origin_mm
                    .y
                    .max(source.tool_box.origin_mm.y),
                source
                    .target_box
                    .origin_mm
                    .z
                    .max(source.tool_box.origin_mm.z),
            );
            let target_maximum = source.target_box.origin_mm + source.target_box.size_mm;
            let tool_maximum = source.tool_box.origin_mm + source.tool_box.size_mm;
            let maximum = Vec3::new(
                target_maximum.x.min(tool_maximum.x),
                target_maximum.y.min(tool_maximum.y),
                target_maximum.z.min(tool_maximum.z),
            );
            if maximum.x <= minimum.x || maximum.y <= minimum.y || maximum.z <= minimum.z {
                return None;
            }
            if source.operation == BooleanOperation::Intersect {
                preview_box.origin_mm = minimum;
                preview_box.size_mm = maximum - minimum;
            }
        }
        let tool_occurrence_id = source.tool_selection.instance_path.root_occurrence();
        Some(SolidToolPreviewPlan {
            source: source.clone(),
            command,
            preview_box,
            hidden_occurrences: (!source.keep_tool)
                .then_some(tool_occurrence_id)
                .into_iter()
                .collect(),
            selection_after: SelectionId {
                definition_id: source.result_definition_id,
                instance_path: source.target_selection.instance_path.clone(),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            committed_digest_key: match (source.operation, source.keep_tool) {
                (BooleanOperation::Cut, false) => "digest-solid-subtract-committed",
                (BooleanOperation::Cut, true) => "digest-solid-trim-committed",
                (BooleanOperation::Union, false) => "digest-solid-union-committed",
                (BooleanOperation::Union, true) => "digest-solid-union-kept-committed",
                (BooleanOperation::Intersect, false) => "digest-solid-intersect-committed",
                (BooleanOperation::Intersect, true) => "digest-solid-intersect-kept-committed",
                (BooleanOperation::Split, _) => "digest-solid-split-committed",
            },
        })
    }

    pub(crate) fn prepare_solid_tool_preview(
        &mut self,
        tool_selection: SelectionId,
        keep_tool: bool,
    ) -> bool {
        let Some(target_selection) = self.solid_tools.target.clone() else {
            return false;
        };
        if target_selection.instance_path == tool_selection.instance_path {
            self.digest = self.catalog.text("digest-solid-tool-distinct");
            return false;
        }
        let Some(source) = self.solid_tool_source_plan(target_selection, tool_selection, keep_tool)
        else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return false;
        };
        let Some(plan) = self.solid_tool_preview_plan(&source) else {
            self.digest = self.catalog.text("digest-solid-tool-invalid");
            return false;
        };
        let operation_label = self
            .catalog
            .text(match (source.operation, source.keep_tool) {
                (BooleanOperation::Cut, true) => "solid-tool-trim",
                (BooleanOperation::Cut, false) => "solid-tool-subtract",
                (BooleanOperation::Union, _) => "solid-tool-union",
                (BooleanOperation::Intersect, _) => "solid-tool-intersect",
                (BooleanOperation::Split, _) => "solid-tool-split",
            });
        let batch = CommandBatch::new(vec![plan.command.clone()]);
        let target_occurrence_id = source.target_selection.instance_path.root_occurrence();
        self.tool_preview.open(OccurrenceOperationPreview {
            source_revision: source.source_revision,
            command_digest: batch.digest(),
            batch,
            boxes: BTreeMap::from([(target_occurrence_id, plan.preview_box.clone())]),
            hidden_occurrences: plan.hidden_occurrences.clone(),
            selection_after: Some(plan.selection_after.clone()),
            committed_digest_key: plan.committed_digest_key,
            canonical_plan: None,
            solid_tool_plan: Some(plan),
        });
        self.status_key = "status-solid-tool-preview";
        self.digest = self.catalog.format(
            "digest-solid-tool-live",
            &BTreeMap::from([
                ("operation", operation_label),
                (
                    "tool",
                    self.catalog.text(if source.keep_tool {
                        "solid-tool-keep-enabled"
                    } else {
                        "solid-tool-keep-disabled"
                    }),
                ),
            ]),
        );
        true
    }

    pub(crate) fn complete_mutation_and_exact_results_with_work_recovery<T, E>(
        &mut self,
        mutate: impl FnOnce(
            &mut DocumentStore,
            &mut ExactResultRegistry,
            &mut ExactResultRegistry,
        ) -> Result<T, E>,
    ) -> Result<T, WorkRecoveryMutationError<E>> {
        let mut exact_results = self.exact.results.clone();
        let mut topology_results = self.exact.topology_results.clone();
        self.complete_mutation_with_publication(
            move |document| {
                let value = mutate(document, &mut exact_results, &mut topology_results)?;
                Ok((value, (exact_results, topology_results)))
            },
            |app, (exact_results, topology_results)| {
                app.exact.results = exact_results;
                app.exact.topology_results = topology_results;
            },
        )
    }

    pub(crate) fn complete_exact_line(&mut self) -> bool {
        let Some(start) = self.gesture.sketch.start else {
            return false;
        };
        let Some(length_mm) =
            parse_distance_mm(&self.value_box.input).filter(|value| *value > 0.01)
        else {
            return false;
        };
        let direction = self
            .gesture
            .sketch
            .cursor
            .map(|cursor| cursor - start)
            .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        let direction_length = length(direction);
        let unit = if direction_length > 0.01 {
            Vec3::new(
                direction.x / direction_length,
                direction.y / direction_length,
                direction.z / direction_length,
            )
        } else {
            Vec3::new(1.0, 0.0, 0.0)
        };
        self.complete_line_sketch(start, start + unit * length_mm)
    }

    pub(crate) fn complete_exact_circle(&mut self) -> bool {
        let Some(center) = self.gesture.sketch.start else {
            return false;
        };
        let Some(radius_mm) =
            parse_distance_mm(&self.value_box.input).filter(|radius| *radius > 0.01)
        else {
            return false;
        };
        let direction = self
            .gesture
            .sketch
            .cursor
            .map(|cursor| cursor - center)
            .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
        self.complete_circle(center, radius_mm, direction)
    }

    pub(crate) fn complete_exact_polygon(&mut self) -> bool {
        let Some(center) = self.gesture.sketch.start else {
            return false;
        };
        let Some(radius_mm) = parse_distance_mm(&self.value_box.input)
            .filter(|radius| *radius > limits::MIN_LENGTH_MM)
        else {
            return false;
        };
        let direction = self
            .gesture
            .sketch
            .cursor
            .map_or(Vec3::new(1.0, 0.0, 0.0), |cursor| cursor - center);
        self.complete_polygon(center, radius_mm, direction)
    }

    /// The typed value is the first half-axis until its end is placed, then
    /// the second one.
    pub(crate) fn complete_exact_ellipse(&mut self) -> bool {
        let Some(center) = self.gesture.sketch.start else {
            return false;
        };
        let Some(radius_mm) = parse_distance_mm(&self.value_box.input)
            .filter(|radius| *radius > limits::MIN_LENGTH_MM)
        else {
            return false;
        };
        let Some(major_end) = self.gesture.sketch.end else {
            let direction = self
                .gesture
                .sketch
                .cursor
                .map(|cursor| self.drawing_local_delta(center, cursor))
                .filter(|direction| length(*direction) > limits::MIN_LENGTH_MM)
                .unwrap_or(Vec3::new(1.0, 0.0, 0.0));
            let unit = direction * (1.0 / length(direction));
            self.place_sketch_point(self.drawing_world_delta(center, unit * radius_mm));
            return true;
        };
        let major = self.drawing_local_delta(center, major_end);
        self.complete_ellipse(center, length(major), radius_mm, major.y.atan2(major.x))
    }

    pub(crate) fn complete_exact_arc(&mut self) -> bool {
        let (Some(start), Some(end)) = (self.gesture.sketch.start, self.gesture.sketch.end) else {
            return false;
        };
        let Some(bulge_mm) =
            parse_distance_mm(&self.value_box.input).filter(|value| value.abs() > 0.01)
        else {
            return false;
        };
        let chord = end - start;
        let local_chord = self.drawing_local_delta(start, end);
        let chord_length = length(local_chord);
        if chord_length <= 0.01 {
            return false;
        }
        let midpoint = (start + end) * 0.5;
        let normal = self.drawing_world_delta(
            Vec3::ZERO,
            Vec3::new(
                -local_chord.y / chord_length,
                local_chord.x / chord_length,
                0.0,
            ),
        );
        let cursor_side = self.gesture.sketch.cursor.map_or(1.0, |cursor| {
            if self.drawing_bulge(start, end, cursor) < 0.0 {
                -1.0
            } else {
                1.0
            }
        });
        self.complete_arc_sketch(
            midpoint - chord * 0.5,
            end,
            midpoint + normal * bulge_mm * cursor_side,
        )
    }

    pub(crate) fn complete_exact_rectangle(&mut self) -> bool {
        let Some(start) = self.gesture.sketch.start else {
            return false;
        };
        let Some([width, depth]) = parse_rectangle_dimensions(&self.value_box.input) else {
            return false;
        };
        let frame = self.drawing_frame(Some(start));
        let frame_x = Vec3::new(frame.x_axis[0], frame.x_axis[1], frame.x_axis[2]);
        let frame_y = Vec3::new(frame.y_axis[0], frame.y_axis[1], frame.y_axis[2]);
        let cursor = self
            .gesture
            .sketch
            .cursor
            .unwrap_or(start + frame_x + frame_y);
        let x_direction = if dot(cursor - start, frame_x) < 0.0 {
            -1.0
        } else {
            1.0
        };
        let y_direction = if dot(cursor - start, frame_y) < 0.0 {
            -1.0
        } else {
            1.0
        };
        self.complete_rectangle_sketch(
            start,
            start + frame_x * width * x_direction + frame_y * depth * y_direction,
        )
    }
}

/// The one refusal for a missing exact worker executable.
pub(crate) fn exact_worker_unavailable() -> Rejection {
    Rejection::new("exact_worker.unavailable", RejectionPhase::Io)
        .reason("exact worker is unavailable")
        .fix_hint("Install or rebuild ketchup-exact-worker next to the application.")
}
