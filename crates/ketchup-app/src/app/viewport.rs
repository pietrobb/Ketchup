//! The 3D viewport: drawing, picking at screen positions, projection, inference and measurement.

use crate::*;

impl KetchupApp {
    pub fn current_sheet_metal_manufacturing_projection(
        &self,
        feature_id: FeatureId,
    ) -> Result<
        SheetMetalManufacturingProjection,
        ketchup_model::sheet_metal::SheetMetalManufacturingExportError,
    > {
        project_sheet_metal_manufacturing(&self.document.current(), feature_id)
    }

    pub fn current_general_fabrication_projection(
        &mut self,
    ) -> Result<GeneralFabricationProjection, Rejection> {
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let tolerance = snapshot.tolerance();
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
        if let Some(occurrence) = occurrences
            .iter()
            .find(|occurrence| definition_mesh_body(&snapshot, occurrence.definition_id).is_some())
        {
            return Err(
                Rejection::new("fabrication.mesh_body", RejectionPhase::Validation)
                    .target(format!("{:?}", occurrence.instance_path))
                    .reason("a visible occurrence is a mesh body without verified exact geometry; general fabrication projection is unavailable until explicit exact conversion")
                    .fix_hint("Convert the mesh body to exact geometry first."),
            );
        }
        let participants = occurrences
            .into_iter()
            .map(|occurrence| {
                GeneralBodyParticipant::accept(
                    &snapshot,
                    &self.exact.results,
                    occurrence.instance_path,
                    tolerance,
                )
                .map_err(|error| failed("fabrication.participant", error))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let collision_validation =
            ketchup_application::validation::fabrication_collision_validation_with_worker(
                &snapshot,
                &participants,
                &self.file.container_data,
                self.validator_worker_path(),
                Duration::from_secs(30),
            )
            .map_err(|error| failed("fabrication.collision_validation", error))?;
        project_general_fabrication(
            &snapshot,
            &self.exact.results,
            &collision_validation.cases,
            &collision_validation.report,
            tolerance,
        )
        .map_err(|error| failed("fabrication.projection", error))
    }

    /// Screen rectangle of the 3D viewport, or `None` before the first frame
    /// has laid the shell out.
    #[must_use]
    pub fn viewport_rect(&self) -> Option<Rect> {
        self.camera.viewport_rect
    }

    #[must_use]
    pub fn viewport_position(&self, point_mm: Vec3) -> Option<Pos2> {
        self.camera
            .viewport_rect
            .map(|rect| self.project(point_mm, rect))
    }

    #[must_use]
    pub fn hovered_selection(&self) -> Option<&SelectionId> {
        self.hover.target.as_ref()
    }

    #[must_use]
    pub fn hovered_snap_kind(&self) -> Option<SnapKind> {
        self.hover.snap.as_ref().map(|snap| snap.kind)
    }

    #[must_use]
    pub fn hovered_snap_position(&self) -> Option<Vec3> {
        self.hover.snap.as_ref().map(|snap| snap.position_mm)
    }

    #[must_use]
    pub fn hovered_overlap_choice(&self) -> Option<(usize, usize)> {
        self.hover
            .pick
            .as_ref()
            .map(|pick| (self.hover.overlap_index, pick.overlapping.len()))
    }

    #[doc(hidden)]
    pub fn enable_headless_instanced_scene(&mut self) {
        self.render.wgpu_target_format = Some(eframe::wgpu::TextureFormat::Bgra8UnormSrgb);
    }

    /// How many triangles the instanced scene would actually paint.
    ///
    /// This reads the plan the last painted frame used, so a body that exists
    /// as an exact product but never reached the scene reports zero here.
    #[must_use]
    pub fn instanced_scene_triangle_count(&self) -> usize {
        self.render.plan.as_ref().map_or(0, |plan| {
            plan.batches()
                .iter()
                .map(|batch| batch.geometry.index_count() / 3 * batch.instances.len())
                .sum()
        })
    }

    /// Whether holding Alt turns the whole scene translucent. Translucency is
    /// painted face by face on the CPU, which a house of thousands of parts
    /// cannot do every frame; there the GPU scene stays and only the chosen
    /// target is highlighted on top of it.
    pub(crate) fn alt_xray_paints_translucent(&self) -> bool {
        const TRANSLUCENT_TRIANGLE_LIMIT: usize = 20_000;
        self.face_workflow.xray_preview()
            && self.instanced_scene_triangle_count() <= TRANSLUCENT_TRIANGLE_LIMIT
    }

    pub(crate) fn exact_projection(&self, snapshot: &Snapshot) -> ExactInteractionProjection {
        ExactInteractionProjection::from_snapshot(
            snapshot,
            self.exact_results_for_snapshot(snapshot)
                .unwrap_or(&self.exact.results),
        )
    }

    pub(crate) fn topology_projection(&self, snapshot: &Snapshot) -> ExactInteractionProjection {
        ExactInteractionProjection::from_snapshot(
            snapshot,
            self.topology_results_for_snapshot(snapshot)
                .unwrap_or(&self.exact.topology_results),
        )
    }

    pub(crate) fn refresh_interaction_projection_cache(&self, snapshot: &Snapshot) {
        let Ok(cache) = self.hover.projection_cache.try_borrow() else {
            return;
        };
        let current = self.document.current();
        let exact_results = if snapshot.document_id() == current.document_id()
            && snapshot.revision_id() == current.revision_id()
            && snapshot.canonical_digest() == current.canonical_digest()
        {
            Some(&self.exact.results)
        } else {
            self.exact_results_for_snapshot(snapshot)
        };
        let exact_results_stamp = (
            exact_results.map_or(0, ExactResultRegistry::contents_stamp),
            self.topology_results_for_snapshot(snapshot)
                .map_or(0, ExactResultRegistry::contents_stamp),
        );
        let rebuild = cache.as_ref().is_none_or(|cache| {
            cache.document_id != snapshot.document_id()
                || cache.revision_id != snapshot.revision_id()
                || cache.canonical_digest != snapshot.canonical_digest()
                || cache.edit_context != self.selection.edit_context
                || cache.exact_results_stamp != exact_results_stamp
        });
        drop(cache);
        if rebuild {
            let active_context_paths = self
                .active_scene_query_for_snapshot(snapshot)
                .into_iter()
                .map(|occurrence| occurrence.instance_path)
                .collect::<BTreeSet<_>>();
            let canonical = CanonicalInteractionProjection::from_snapshot(snapshot);
            let combined = self.interaction_exact_registry(snapshot);
            let exact_results = &combined;
            let exact =
                ExactInteractionProjection::from_snapshot_where(snapshot, exact_results, |path| {
                    active_context_paths.contains(path)
                });
            let mesh = MeshInteractionProjection::from_snapshot_where(snapshot, |path| {
                active_context_paths.contains(path) && !exact.contains_occurrence(path)
            });
            let boxes = canonical
                .scene_where(|occurrence| {
                    active_context_paths.contains(&occurrence.instance_path)
                        && !exact.contains_occurrence(&occurrence.instance_path)
                        && !mesh.contains_occurrence(&occurrence.instance_path)
                        && !Self::definition_is_imported_exact_body(
                            snapshot,
                            occurrence.body.definition_id,
                        )
                        && occurrence.local_box.is_some()
                })
                .expect("canonical visible box projections are valid");
            let proxies = canonical
                .scene_where(|occurrence| {
                    active_context_paths.contains(&occurrence.instance_path)
                        && occurrence.local_box.is_some()
                })
                .expect("canonical visible proxy projections are valid");
            if let Ok(mut cache) = self.hover.projection_cache.try_borrow_mut() {
                *cache = Some(InteractionProjectionCache {
                    document_id: snapshot.document_id(),
                    revision_id: snapshot.revision_id(),
                    canonical_digest: snapshot.canonical_digest(),
                    edit_context: self.selection.edit_context.clone(),
                    exact_results_stamp,
                    canonical,
                    exact,
                    mesh,
                    boxes,
                    proxies,
                    render_boxes: std::cell::OnceCell::new(),
                    frame_bounds: std::cell::OnceCell::new(),
                    snap_geometry: std::cell::OnceCell::new(),
                });
            }
        }
    }

    pub(crate) fn hidden_ghost_corners(
        &self,
        snapshot: &Snapshot,
        projection: &InteractionProjection,
    ) -> Vec<[Vec3; 8]> {
        projection
            .occurrences()
            .iter()
            .filter(|occurrence| !occurrence.visible)
            .filter_map(|occurrence| {
                let [minimum, maximum] = self.definition_local_bounds(
                    snapshot,
                    occurrence.body.definition_id,
                    occurrence.local_box,
                    true,
                )?;
                let size = maximum - minimum;
                Some(box_corners(size.x, size.y, size.z).map(|corner| {
                    transform_model_point(occurrence.canonical_world_transform, corner + minimum)
                }))
            })
            .collect()
    }

    pub(crate) fn active_frame_bounds(&self) -> Vec<(Vec3, Vec3)> {
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        if let Some(bounds) = self
            .hover
            .projection_cache
            .borrow()
            .as_ref()
            .and_then(|cache| cache.frame_bounds.get())
        {
            return bounds.clone();
        }
        let mut bounds = self
            .active_boxes()
            .into_iter()
            .map(|item| (item.origin_mm, item.size_mm))
            .collect::<Vec<_>>();
        for occurrence in self.active_scene_query() {
            let Some(definition) = snapshot.definition(occurrence.definition_id) else {
                continue;
            };
            let matrix = occurrence.transform.matrix();
            let mut mesh_vertices = definition
                .feature_ids()
                .iter()
                .filter_map(|feature_id| snapshot.feature(*feature_id))
                .filter_map(|feature| match feature.kind() {
                    FeatureKind::MeshBody(mesh) => Some(mesh.vertices_mm.iter()),
                    _ => None,
                })
                .flatten()
                .map(|vertex| {
                    Vec3::new(
                        matrix[0] * vertex[0]
                            + matrix[1] * vertex[1]
                            + matrix[2] * vertex[2]
                            + matrix[3],
                        matrix[4] * vertex[0]
                            + matrix[5] * vertex[1]
                            + matrix[6] * vertex[2]
                            + matrix[7],
                        matrix[8] * vertex[0]
                            + matrix[9] * vertex[1]
                            + matrix[10] * vertex[2]
                            + matrix[11],
                    )
                });
            let Some(first) = mesh_vertices.next() else {
                continue;
            };
            let (minimum, maximum) =
                mesh_vertices.fold((first, first), |(minimum, maximum), point| {
                    (
                        Vec3::new(
                            minimum.x.min(point.x),
                            minimum.y.min(point.y),
                            minimum.z.min(point.z),
                        ),
                        Vec3::new(
                            maximum.x.max(point.x),
                            maximum.y.max(point.y),
                            maximum.z.max(point.z),
                        ),
                    )
                });
            bounds.push((minimum, maximum - minimum));
        }
        if let Some(cache) = self.hover.projection_cache.borrow().as_ref() {
            let _ = cache.frame_bounds.set(bounds.clone());
        }
        bounds
    }

    pub(crate) fn render_boxes_from_projection(
        &self,
        snapshot: &Snapshot,
        projection: &InteractionProjection,
        use_exact_bounds: bool,
    ) -> Vec<RenderBox> {
        if !projection.is_current(snapshot) {
            return Vec::new();
        }
        let exact_packages = use_exact_bounds
            .then(|| self.exact_results_for_snapshot(snapshot))
            .flatten()
            .map(|results| results.render_by_definition(snapshot));
        let mut exact_solid_tool_features = BTreeMap::new();
        projection
            .occurrences()
            .iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                if occurrence.box_proxy.is_none() {
                    let is_imported_exact_body = Self::definition_is_imported_exact_body(
                        snapshot,
                        occurrence.body.definition_id,
                    );
                    if !is_imported_exact_body
                        && definition_requires_evaluated_geometry(
                            snapshot,
                            occurrence.body.definition_id,
                        )
                        && exact_packages
                            .as_ref()
                            .and_then(|packages| packages.get(&occurrence.body.definition_id))
                            .is_none()
                    {
                        return None;
                    }
                    // The packages were matched once above; asking per occurrence
                    // rescans every package, quadratic in the part count.
                    let [minimum, maximum] = match exact_packages
                        .as_ref()
                        .and_then(|packages| packages.get(&occurrence.body.definition_id))
                    {
                        Some(package) => {
                            let [minimum, maximum] = package.bounds_mm();
                            [Vec3::from(minimum), Vec3::from(maximum)]
                        }
                        None => self.definition_local_bounds(
                            snapshot,
                            occurrence.body.definition_id,
                            occurrence.local_box,
                            false,
                        )?,
                    };
                    let size = maximum - minimum;
                    let [world_minimum, world_maximum] = bounds_of(
                        box_corners(size.x, size.y, size.z)
                            .into_iter()
                            .map(|corner| {
                                transform_model_point(
                                    occurrence.canonical_world_transform,
                                    corner + minimum,
                                )
                            }),
                    )?;
                    let profile_feature_id = occurrence
                        .body
                        .profile_feature_id
                        .or_else(|| {
                            *exact_solid_tool_features
                                .entry(occurrence.body.definition_id)
                                .or_insert_with(|| {
                                    exact_solid_tool_feature_id(
                                        snapshot,
                                        occurrence.body.definition_id,
                                    )
                                })
                        })
                        .or_else(|| {
                            snapshot
                                .definition(occurrence.body.definition_id)?
                                .feature_ids()
                                .iter()
                                .find_map(|feature_id| {
                                    matches!(
                                        snapshot.feature(*feature_id)?.kind(),
                                        FeatureKind::Profile { .. } | FeatureKind::Sketch(_)
                                    )
                                    .then_some(*feature_id)
                                })
                        })?;
                    return Some(RenderBox {
                        definition_id: occurrence.body.definition_id,
                        profile_feature_id,
                        extrusion_feature_id: occurrence.body.extrusion_feature_id,
                        instance_path: occurrence.instance_path.clone(),
                        origin_mm: world_minimum,
                        size_mm: world_maximum - world_minimum,
                    });
                }
                let box_proxy = occurrence.box_proxy?;
                let exact_bounds =
                    exact_packages
                        .as_ref()
                        .and_then(|packages| packages.get(&occurrence.body.definition_id))
                        .and_then(|package| {
                            let [minimum, maximum] = package.bounds_mm();
                            let minimum = Vec3::new(minimum[0], minimum[1], minimum[2]);
                            let maximum = Vec3::new(maximum[0], maximum[1], maximum[2]);
                            let size = maximum - minimum;
                            bounds_of(box_corners(size.x, size.y, size.z).into_iter().map(
                                |corner| {
                                    transform_model_point(
                                        occurrence.canonical_world_transform,
                                        corner + minimum,
                                    )
                                },
                            ))
                        })
                        .map(|[minimum, maximum]| (minimum, maximum - minimum));
                let (origin_mm, size_mm) =
                    exact_bounds.unwrap_or((box_proxy.origin_mm, box_proxy.size_mm));
                Some(RenderBox {
                    definition_id: occurrence.body.definition_id,
                    profile_feature_id: occurrence.body.profile_feature_id?,
                    extrusion_feature_id: occurrence.body.extrusion_feature_id,
                    instance_path: occurrence.instance_path.clone(),
                    origin_mm,
                    size_mm,
                })
            })
            .collect()
    }

    /// Where a world point currently lands inside the viewport.
    ///
    /// Callers that need to aim at real geometry must ask the camera instead of
    /// assuming a screen offset: under a converging projection the pixels per
    /// millimetre depend on how far the point is from the eye.
    #[must_use]
    pub fn project_to_screen(&self, point: Vec3, rect: Rect) -> Pos2 {
        self.project(point, rect)
    }

    pub(crate) fn select_from_viewport(&mut self, target: Option<SelectionId>, additive: bool) {
        let Some(target) = target else {
            if !additive {
                self.clear_selection();
            }
            return;
        };
        let occurrence_id = target.instance_path.root_occurrence();
        if !self.occurrence_in_active_context(&target.instance_path) {
            return;
        }
        self.end_transform_correction();
        let snapshot = self.document.current();
        if self.selection.edit_context.is_empty()
            && target.instance_path.is_root()
            && let Some(group_id) = snapshot
                .occurrence(occurrence_id)
                .and_then(|occurrence| occurrence.parent())
        {
            self.select_group(group_id);
            return;
        }
        self.select_part_from_viewport(target, additive);
    }

    /// Selects the clicked part itself even inside a group: a click on an exact
    /// face picks that part, so Shift on another face adds or removes that part.
    fn select_part_from_viewport(&mut self, target: SelectionId, additive: bool) {
        if !self.occurrence_in_active_context(&target.instance_path) {
            return;
        }
        self.end_transform_correction();
        let snapshot = self.document.current();
        self.selection.select_exact(target.clone(), additive);
        if let Some(item) = snapshot
            .scene_query()
            .into_iter()
            .find(|item| item.instance_path == target.instance_path)
        {
            self.digest = self.catalog.format(
                "digest-selected-viewport",
                &BTreeMap::from([
                    ("name", item.definition_name),
                    ("count", item.shared_occurrence_count.to_string()),
                ]),
            );
        }
    }

    /// Current camera magnification, as changed by zoom commands and the wheel.
    #[must_use]
    pub const fn camera_zoom(&self) -> f32 {
        self.camera.zoom
    }

    /// Current camera yaw and pitch in radians.
    #[must_use]
    pub const fn camera_orientation(&self) -> (f32, f32) {
        (self.camera.yaw, self.camera.pitch)
    }

    /// Whether the viewport presentation switch `flag` is on.
    #[must_use]
    pub const fn view_visible(&self, flag: ViewFlag) -> bool {
        self.view.contains(flag)
    }

    /// Number of hidden occurrence bounds currently exposed by the ghost overlay.
    #[must_use]
    pub fn hidden_ghost_count(&self) -> usize {
        if !self.view.contains(ViewFlag::HiddenObjects) {
            return 0;
        }
        let snapshot = self.document.current();
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        self.hidden_ghost_corners(&snapshot, &projection).len()
    }

    pub(crate) fn camera_view_state(&self) -> CameraViewState {
        CameraViewState {
            projection_mode: self.camera.projection_mode,
            yaw: self.camera.yaw,
            pitch: self.camera.pitch,
            target_z: self.camera.target_z,
            zoom: self.camera.zoom,
            pan: self.camera.pan,
            view: self.view,
        }
    }

    pub(crate) fn remember_camera_change(&mut self, before: CameraViewState) -> bool {
        if self.camera_view_state() == before {
            return false;
        }
        self.camera.previous_view = Some(before);
        true
    }

    pub fn previous_view(&mut self) {
        let Some(previous) = self.camera.previous_view.take() else {
            return;
        };
        let current = self.camera_view_state();
        self.camera.projection_mode = previous.projection_mode;
        self.camera.yaw = previous.yaw;
        self.camera.pitch = previous.pitch;
        self.camera.target_z = previous.target_z;
        self.camera.zoom = previous.zoom;
        self.camera.pan = previous.pan;
        self.view = previous.view;
        self.refresh_camera_distance();
        self.camera.previous_view = Some(current);
        self.digest = self.catalog.text("digest-previous-view");
    }

    /// Flip a viewport presentation switch; the document and what can be
    /// picked stay as they are.
    pub fn toggle_view(&mut self, flag: ViewFlag) {
        let shown = self.view.toggle(flag);
        let name = CommandRegistry::spec(AppCommand::View(flag))
            .label_key
            .trim_start_matches("view-");
        self.digest = self.catalog.text(&format!(
            "digest-{name}-{}",
            if shown { "shown" } else { "hidden" }
        ));
    }

    /// Restore the isometric home orientation and frame every visible occurrence.
    pub fn home_view(&mut self) {
        self.look_isometric();
        let bounds = self.active_frame_bounds();
        let count = bounds.len();
        if !self.frame_bounds(&bounds) {
            self.camera.target_z = 10.0;
            self.camera.pan = Vec2::ZERO;
            self.camera.zoom = 2.8;
            self.refresh_camera_distance();
        }
        self.digest = self.catalog.format(
            "digest-home-view",
            &BTreeMap::from([("count", count.to_string())]),
        );
    }

    pub(crate) fn look_isometric(&mut self) {
        self.look_from(
            -std::f32::consts::FRAC_PI_4,
            -3.0_f32.recip().sqrt().acos(),
            "view-iso",
        );
    }

    pub(crate) fn look_from(&mut self, yaw: f32, pitch: f32, view_key: &str) {
        self.camera.zoom_fit_pending = false;
        self.camera.yaw = yaw;
        self.camera.pitch = pitch;
        self.digest = self.catalog.format(
            "digest-view-changed",
            &BTreeMap::from([("view", self.catalog.text(view_key))]),
        );
    }

    /// Frame every visible occurrence in the viewport laid out by the last frame.
    pub fn zoom_fit(&mut self) {
        self.camera.zoom_fit_pending_quiet = false;
        let bounds = self.active_frame_bounds();
        let count = bounds.len();
        self.camera.zoom_fit_pending = !self.frame_bounds(&bounds);
        if !self.camera.zoom_fit_pending {
            self.digest = self.catalog.format(
                "digest-zoom-fit",
                &BTreeMap::from([("count", count.to_string())]),
            );
        }
    }

    /// Frame only visible selected occurrences without changing the viewing direction.
    pub fn zoom_selection(&mut self) {
        let boxes = self.selected_active_boxes();
        let count = boxes.len();
        if self.frame_boxes(&boxes) {
            self.digest = self.catalog.format(
                "digest-zoom-selection",
                &BTreeMap::from([("count", count.to_string())]),
            );
        }
    }

    pub(crate) fn zoom_window(&mut self, start: Pos2, end: Pos2, viewport: Rect) -> bool {
        let window = Rect::from_two_pos(start, end).intersect(viewport);
        if window.width() < 8.0 || window.height() < 8.0 {
            return false;
        }
        let anchor = self
            .surface_point_at_screen(window.center(), viewport)
            .or_else(|| self.screen_to_plane(window.center(), viewport, self.camera.target_z));
        let Some(anchor) = anchor else {
            return false;
        };
        let before = self.camera_view_state();
        let factor = (viewport.width() / window.width()).min(viewport.height() / window.height());
        self.camera.zoom = (self.camera.zoom * factor).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        self.refresh_camera_distance();
        let anchor_position = self.project(anchor, viewport);
        self.camera.pan += viewport.center() - anchor_position;
        if !self.remember_camera_change(before) {
            return false;
        }
        self.digest = self.catalog.text("digest-zoom-window");
        true
    }

    pub(crate) fn zoom_by(&mut self, factor: f32, digest_key: &str) {
        let zoom = (self.camera.zoom * factor).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        if zoom != self.camera.zoom {
            self.camera.zoom = zoom;
            self.refresh_camera_distance();
            self.digest = self.catalog.text(digest_key);
        }
    }

    pub(crate) fn frame_boxes(&mut self, boxes: &[RenderBox]) -> bool {
        let bounds = boxes
            .iter()
            .map(|item| (item.origin_mm, item.size_mm))
            .collect::<Vec<_>>();
        self.frame_bounds(&bounds)
    }

    pub(crate) fn frame_bounds(&mut self, bounds: &[(Vec3, Vec3)]) -> bool {
        let Some(rect) = self.camera.viewport_rect else {
            return false;
        };
        let corners = bounds
            .iter()
            .flat_map(|(origin, size)| {
                box_corners(size.x, size.y, size.z).map(|point| point + *origin)
            })
            .collect::<Vec<_>>();
        let Some(first) = corners.first().copied() else {
            return false;
        };
        let (mut low, mut high) = (first.z, first.z);
        for corner in &corners {
            low = low.min(corner.z);
            high = high.max(corner.z);
        }
        self.camera.target_z = f64::midpoint(low, high);
        self.camera.pan = Vec2::ZERO;
        self.camera.zoom = 1.0;
        let flat = projected_bounds(&corners, |point| self.project(point, rect));
        let fit =
            (rect.width() / flat.width().max(1.0)).min(rect.height() / flat.height().max(1.0));
        self.camera.zoom = (fit * 0.82).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        let scaled = projected_bounds(&corners, |point| self.project(point, rect));
        self.camera.pan = rect.center() - scaled.center();
        true
    }

    pub(crate) fn push_pull_screen_projection(
        &self,
        selection: &SelectionId,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(Vec2, f32)> {
        if matches!(selection.element, ElementId::TopologicalFace(_)) {
            return self.planar_screen_projection(selection, pointer, rect);
        }
        let item = self
            .active_boxes()
            .into_iter()
            .find(|item| item.instance_path == selection.instance_path)?;
        let ElementId::Face { axis, side } = selection.element else {
            return None;
        };
        let sign = if side == Side::Maximum { 1.0 } else { -1.0 };
        let mut center = item.origin_mm + item.size_mm * 0.5;
        let normal = match axis {
            Axis::X => {
                center.x = if side == Side::Maximum {
                    item.origin_mm.x + item.size_mm.x
                } else {
                    item.origin_mm.x
                };
                Vec3::new(sign, 0.0, 0.0)
            }
            Axis::Y => {
                center.y = if side == Side::Maximum {
                    item.origin_mm.y + item.size_mm.y
                } else {
                    item.origin_mm.y
                };
                Vec3::new(0.0, sign, 0.0)
            }
            Axis::Z => {
                center.z = if side == Side::Maximum {
                    item.origin_mm.z + item.size_mm.z
                } else {
                    item.origin_mm.z
                };
                Vec3::new(0.0, 0.0, sign)
            }
        };
        let projected = self.project(center + normal, rect) - self.project(center, rect);
        let pixels_per_mm = projected.length();
        if pixels_per_mm > SCREEN_ROUNDING_PX {
            Some((projected / pixels_per_mm, pixels_per_mm))
        } else {
            let fallback_scale = self.camera.zoom * rect.width().min(rect.height()) / 420.0;
            Some((Vec2::new(0.0, -1.0), fallback_scale.max(SCREEN_ROUNDING_PX)))
        }
    }

    pub(crate) fn render_box(&self, item: RenderBox) -> RenderBox {
        if self.has_occurrence_operation_preview()
            && item.instance_path.is_root()
            && let Some(preview) = self
                .tool_preview
                .get::<OccurrenceOperationPreview>()
                .and_then(|operation| operation.boxes.get(&item.instance_path.root_occurrence()))
        {
            return preview.clone();
        }
        if !self.has_preview() {
            return item;
        }
        let Some(ephemeral) = self.tool_preview.get::<EphemeralBoxPreview>() else {
            return item;
        };
        let preview = &ephemeral.plan.preview_box;
        let selection = &ephemeral.plan.source.target;
        if preview.definition_id != item.definition_id {
            return item;
        }
        if item.instance_path == selection.instance_path {
            return preview.clone();
        }
        let preview_element = match &selection.element {
            ElementId::Face {
                axis,
                side: Side::Minimum,
            } if item.instance_path != selection.instance_path => ElementId::Face {
                axis: *axis,
                side: Side::Maximum,
            },
            element => element.clone(),
        };
        face_extent(preview, Some(&selection.element))
            .and_then(|extent| resize_box_from_face(&item, &preview_element, extent))
            .unwrap_or(item)
    }

    pub(crate) fn circular_cut_profile_target_at_screen(
        &self,
        snapshot: &Snapshot,
        pointer: Pos2,
        rect: Rect,
    ) -> Option<(SelectionId, MoveProfileTarget, Vec3)> {
        let mut candidates = Vec::new();
        for occurrence in snapshot
            .scene_query()
            .into_iter()
            .filter(|item| item.visible)
        {
            let definition = snapshot.definition(occurrence.definition_id)?;
            for producer_id in definition.feature_ids() {
                let Some(producer) = snapshot.feature(*producer_id) else {
                    continue;
                };
                let FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    tool,
                    ..
                } = producer.kind()
                else {
                    continue;
                };
                let Some((extrusion_id, tool_transform)) =
                    snapshot
                        .feature(*tool)
                        .and_then(|feature| match feature.kind() {
                            FeatureKind::RigidTransform { target, transform } => {
                                Some((*target, *transform))
                            }
                            FeatureKind::Pad(PadSpec {
                                profile: PadProfile::Feature(_),
                                extent: FeatureExtent::Blind(_),
                                operation: PadOperation::NewBody,
                                ..
                            }) => Some((*tool, Transform::identity())),
                            _ => None,
                        })
                else {
                    continue;
                };
                let Some(profile_id) = snapshot.feature(extrusion_id).and_then(|feature| {
                    let FeatureKind::Pad(PadSpec {
                        profile: PadProfile::Feature(profile),
                        extent: FeatureExtent::Blind(_),
                        operation: PadOperation::NewBody,
                        ..
                    }) = feature.kind()
                    else {
                        return None;
                    };
                    Some(*profile)
                }) else {
                    continue;
                };
                let Some((center_mm, radius_mm)) =
                    snapshot.feature(profile_id).and_then(|feature| {
                        let FeatureKind::Profile { segments, closed } = feature.kind() else {
                            return None;
                        };
                        exact_circle_geometry(segments, *closed)
                    })
                else {
                    continue;
                };
                // A drawn-shape pocket keeps its profile in the tool's own body.
                let Some(body_id) = [extrusion_id, *producer_id].into_iter().find_map(|id| {
                    definition
                        .feature_body_ownership(id)
                        .and_then(|ownership| ownership.output_body_id())
                }) else {
                    continue;
                };
                let tool_origin = transform_model_point(tool_transform, Vec3::ZERO);
                let tool_x =
                    transform_model_point(tool_transform, Vec3::new(1.0, 0.0, 0.0)) - tool_origin;
                let tool_y =
                    transform_model_point(tool_transform, Vec3::new(0.0, 1.0, 0.0)) - tool_origin;
                let mut world_origin = transform_model_point(occurrence.transform, tool_origin);
                let world_x = transform_model_point(occurrence.transform, tool_origin + tool_x)
                    - world_origin;
                let world_y = transform_model_point(occurrence.transform, tool_origin + tool_y)
                    - world_origin;
                let x_length = length(world_x);
                let y_length = length(world_y);
                if x_length <= ROUNDING || y_length <= ROUNDING {
                    continue;
                }
                let world_x_axis = world_x * (1.0 / x_length);
                let world_y_axis = world_y * (1.0 / y_length);
                let Some(box_item) = self
                    .active_boxes_for_snapshot(snapshot)
                    .into_iter()
                    .find(|item| item.instance_path == occurrence.instance_path)
                else {
                    continue;
                };
                world_origin.z = box_item.origin_mm.z + box_item.size_mm.z;
                let world_center = world_origin
                    + world_x_axis * (center_mm[0] * x_length)
                    + world_y_axis * (center_mm[1] * y_length);
                let points = (0..=64)
                    .map(|step| {
                        let angle = std::f64::consts::TAU * f64::from(step) / 64.0;
                        world_center
                            + world_x_axis * (radius_mm * x_length * angle.cos())
                            + world_y_axis * (radius_mm * y_length * angle.sin())
                    })
                    .collect::<Vec<_>>();
                for segment in points.windows(2) {
                    let from = self.project(segment[0], rect);
                    let to = self.project(segment[1], rect);
                    let screen = to - from;
                    let length_squared = screen.length_sq();
                    if length_squared <= f32::EPSILON {
                        continue;
                    }
                    let factor = ((pointer - from).dot(screen) / length_squared).clamp(0.0, 1.0);
                    let distance = from.lerp(to, factor).distance(pointer);
                    if distance <= 10.0 {
                        let selection = SelectionId {
                            definition_id: occurrence.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element: ElementId::Face {
                                axis: Axis::Z,
                                side: Side::Maximum,
                            },
                        };
                        candidates.push((
                            distance,
                            selection,
                            MoveProfileTarget {
                                definition_id: occurrence.definition_id,
                                body_id,
                                profile_id,
                                world_origin,
                                world_x_axis,
                                world_y_axis,
                            },
                            segment[0] + (segment[1] - segment[0]) * f64::from(factor),
                        ));
                    }
                }
            }
        }
        candidates
            .into_iter()
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, selection, target, position)| (selection, target, position))
    }

    pub(crate) fn move_inference_target_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        destination: Vec3,
    ) -> Option<Vec3> {
        self.datum_snap_with_position(pointer, rect, None, Some(destination))
            .map(|(point, _)| point)
    }

    pub(crate) fn screen_to_rotation_plane(
        &self,
        pointer: Pos2,
        rect: Rect,
        centre_mm: Vec3,
        axis: Axis,
    ) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        let normal = axis_direction(axis);
        let denominator = dot(ray.direction, normal);
        if denominator.abs() <= ROUNDING {
            return None;
        }
        let distance = dot(centre_mm - ray.origin, normal) / denominator;
        (distance >= 0.0 && distance.is_finite()).then(|| ray.origin + ray.direction * distance)
    }

    pub(crate) fn canonical_profile_viewport_mesh(
        &self,
        snapshot: &Snapshot,
        item: &RenderBox,
    ) -> Option<renderer::PlanarProfileMesh> {
        let mesh = renderer::canonical_profile_feature_mesh(snapshot, item.profile_feature_id)
            .or_else(|| {
                if self.proxy_preview_is_active(item) {
                    return None;
                }
                let points_mm = snapshot
                    .feature(item.profile_feature_id)?
                    .kind()
                    .polygon_points()?;
                let segments = points_mm
                    .iter()
                    .enumerate()
                    .map(|(i, start)| ProfileSegment::Line {
                        start_mm: *start,
                        end_mm: points_mm[(i + 1) % points_mm.len()],
                    })
                    .collect::<Vec<_>>();
                ketchup_interaction::mesh_projection::segment_profile_mesh(&segments)
            })?;
        let extent = self
            .tool_preview
            .get::<EphemeralBoxPreview>()
            .filter(|preview| {
                preview.plan.source.target.instance_path == item.instance_path
                    && preview.plan.source.topological_reference.is_none()
            })
            .map(|preview| f64::from_bits(preview.plan.new_extent_mm_bits))
            .or_else(|| {
                let FeatureKind::Pad(PadSpec {
                    profile: PadProfile::Feature(_),
                    extent: FeatureExtent::Blind(height),
                    operation: PadOperation::NewBody,
                    ..
                }) = snapshot.feature(item.extrusion_feature_id?)?.kind()
                else {
                    return None;
                };
                Some(height.millimetres())
            });
        let Some(extent) = extent else {
            return Some(mesh);
        };
        match snapshot.feature(item.profile_feature_id)?.kind() {
            FeatureKind::Sketch(sketch) => {
                let FeatureKind::Workplane(workplane) = snapshot.feature(sketch.workplane)?.kind()
                else {
                    return None;
                };
                renderer::extrude_planar_profile_mesh_along(
                    mesh,
                    workplane.frame.normal.map(|component| component * extent),
                )
            }
            _ => renderer::extrude_planar_profile_mesh(mesh, extent),
        }
    }

    pub(crate) fn viewport_boxes(
        &self,
        snapshot: &Snapshot,
        exact_projection: &ExactInteractionProjection,
    ) -> Vec<RenderBox> {
        let mut boxes = self.active_boxes_for_snapshot(snapshot);
        if let Some((drag, _)) = self.move_session()
            && self.move_preview_is_current(drag)
            && (drag.copy || drag.profile_target.is_none())
        {
            let mut copies = Vec::new();
            for item in boxes
                .iter_mut()
                .filter(|item| self.move_drag_applies_to_path(drag, &item.instance_path))
            {
                let mut preview = item.clone();
                preview.origin_mm += drag.delta_mm;
                if drag.copy && drag.group_id.is_none() {
                    copies.push(preview);
                } else {
                    *item = preview;
                }
            }
            boxes.extend(copies);
        }
        if self.has_occurrence_operation_preview()
            && let Some(operation) = self.tool_preview.get::<OccurrenceOperationPreview>()
        {
            for (occurrence_id, preview_box) in &operation.boxes {
                if !boxes
                    .iter()
                    .any(|item| item.instance_path == InstancePath::root(*occurrence_id))
                {
                    boxes.push(preview_box.clone());
                }
            }
        }
        if self.has_occurrence_operation_preview()
            && let Some(operation) = self.tool_preview.get::<OccurrenceOperationPreview>()
        {
            boxes.retain(|item| {
                !item.instance_path.is_root()
                    || !operation
                        .hidden_occurrences
                        .contains(&item.instance_path.root_occurrence())
            });
        }
        boxes.retain(|item| {
            if item.extrusion_feature_id.is_none()
                && snapshot
                    .feature(item.profile_feature_id)
                    .is_some_and(|feature| {
                        matches!(feature.kind(), FeatureKind::Profile { closed: false, .. })
                    })
            {
                return false;
            }
            let proxy_preview = self.proxy_preview_is_active(item);
            (!Self::definition_is_imported_exact_body(snapshot, item.definition_id)
                || proxy_preview)
                && (!exact_projection.contains_occurrence(&item.instance_path) || proxy_preview)
        });
        boxes
    }

    pub(crate) fn orbit(&mut self, pointer_delta: Vec2) {
        self.camera.yaw += pointer_delta.x * 0.006;
        self.camera.pitch += pointer_delta.y * 0.006;
    }

    /// The first measured point while a measurement is being taken.
    pub(crate) const fn measure_anchor(&self) -> Option<Vec3> {
        match (self.gesture.measure.start, self.gesture.measure.end) {
            (start, None) => start,
            _ => None,
        }
    }

    /// The measured segment, either finished or following the pointer.
    pub(crate) fn measure_span(&self) -> Option<(Vec3, Vec3)> {
        let start = self.gesture.measure.start?;
        let end = self.gesture.measure.end.or(self.gesture.measure.cursor)?;
        Some((start, end))
    }

    /// Record a measured point. Measuring never changes the document.
    pub(crate) fn add_measured_point(&mut self, point: Vec3) {
        if let Some(start) = self.measure_anchor() {
            self.gesture.measure.end = Some(point);
            self.gesture.measure.cursor = Some(point);
            self.status_key = "status-ready";
            self.digest = self.measurement_text(start, point, "digest-measured");
        } else {
            self.gesture.measure.start = Some(point);
            self.gesture.measure.cursor = Some(point);
            self.gesture.measure.end = None;
            self.value_box.input.clear();
            self.status_key = "status-measure-second-point";
        }
    }

    pub(crate) fn measurement_text(&self, start: Vec3, end: Vec3, key: &str) -> String {
        let delta = Vec3::new(end.x - start.x, end.y - start.y, end.z - start.z);
        self.catalog.format(
            key,
            &BTreeMap::from([
                ("distance", format_height(length(delta))),
                ("vector", format_vector_mm(delta)),
            ]),
        )
    }

    pub(crate) fn clear_measurement(&mut self) {
        self.gesture.measure.start = None;
        self.gesture.measure.cursor = None;
        self.gesture.measure.end = None;
    }

    /// The measured distance in millimetres, once both points are placed.
    #[must_use]
    pub fn measured_points(&self) -> Option<(Vec3, Vec3)> {
        Some((self.gesture.measure.start?, self.gesture.measure.end?))
    }

    /// The measured distance in millimetres, once both points are placed.
    #[must_use]
    pub fn measured_distance_mm(&self) -> Option<f64> {
        let (start, end) = self.measured_points()?;
        Some(length(Vec3::new(
            end.x - start.x,
            end.y - start.y,
            end.z - start.z,
        )))
    }

    /// Camera axes in world space: screen right, screen up, and view direction.
    pub(crate) fn camera_basis(&self) -> (Vec3, Vec3, Vec3) {
        let yaw_sin = f64::from(self.camera.yaw.sin());
        let yaw_cos = f64::from(self.camera.yaw.cos());
        let pitch_sin = f64::from(self.camera.pitch.sin());
        let pitch_cos = f64::from(self.camera.pitch.cos());
        (
            Vec3::new(yaw_cos, -yaw_sin, 0.0),
            Vec3::new(yaw_sin * pitch_cos, yaw_cos * pitch_cos, -pitch_sin),
            Vec3::new(-yaw_sin * pitch_sin, -yaw_cos * pitch_sin, -pitch_cos),
        )
    }

    pub(crate) fn camera_target(&self) -> Vec3 {
        Vec3::new(BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5, self.camera.target_z)
    }

    /// Millimetres between the eye and the orbit target — the readout's `dist`.
    ///
    /// The nominal distance follows the zoom, but a converging projection is
    /// only meaningful while the eye is outside the model, so the cached value
    /// is pushed back to clear the scene. See [`Self::refresh_camera_distance`].
    pub(crate) fn camera_distance(&self) -> f64 {
        self.camera.distance_mm
    }

    /// Recompute the eye distance for the current scene. Once per frame.
    pub(crate) fn refresh_camera_distance(&mut self) {
        let nominal = 420.0 / f64::from(self.camera.zoom);
        let target = self.camera_target();
        let radius = self
            .active_frame_bounds()
            .into_iter()
            .flat_map(|(origin, size)| {
                box_corners(size.x, size.y, size.z).map(|corner| length(origin + corner - target))
            })
            .fold(0.0_f64, f64::max);
        self.camera.distance_mm = nominal
            .max(radius * CAMERA_CLEARANCE)
            .max(PERSPECTIVE_NEAR_MM);
    }

    /// Pixels per millimetre at the orbit target.
    ///
    /// Both projections agree here by construction, so switching between them
    /// keeps the model the same size and only changes how depth is treated.
    pub(crate) fn view_scale(&self, rect: Rect) -> f64 {
        f64::from(self.camera.zoom) * f64::from(rect.width().min(rect.height())) / 420.0
    }

    /// Focal length in pixels, chosen so the target plane matches `view_scale`.
    pub(crate) fn camera_focal(&self, rect: Rect) -> f64 {
        self.view_scale(rect) * self.camera_distance()
    }

    pub fn toggle_projection_mode(&mut self) {
        self.camera.projection_mode = self.camera.projection_mode.toggled();
        self.digest = self.catalog.text(self.camera.projection_mode.label_key());
    }

    /// Current viewport projection.
    #[must_use]
    pub const fn projection_mode(&self) -> ProjectionMode {
        self.camera.projection_mode
    }

    pub(crate) fn view_ray(&self, pointer: Pos2, rect: Rect) -> Option<Ray> {
        let (right, up, forward) = self.camera_basis();
        let target = self.camera_target();
        let horizontal = f64::from(pointer.x - rect.center().x - self.camera.pan.x);
        let vertical = f64::from(rect.center().y + self.camera.pan.y - pointer.y);
        match self.camera.projection_mode {
            ProjectionMode::Parallel => {
                let scale = self.view_scale(rect);
                let view_plane_point =
                    target + right * (horizontal / scale) + up * (vertical / scale);
                Ray::new(view_plane_point - forward * self.camera_distance(), forward).ok()
            }
            ProjectionMode::Perspective => {
                let focal = self.camera_focal(rect);
                let eye = target - forward * self.camera_distance();
                let direction = right * (horizontal / focal) + up * (vertical / focal) + forward;
                Ray::new(eye, direction).ok()
            }
        }
    }

    pub(crate) fn screen_to_plane(&self, pointer: Pos2, rect: Rect, plane_z: f64) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        if ray.direction.z.abs() <= ROUNDING {
            return None;
        }
        let distance = (plane_z - ray.origin.z) / ray.direction.z;
        (distance >= 0.0).then(|| ray.origin + ray.direction * distance)
    }

    pub(crate) fn screen_to_workplane(
        &self,
        pointer: Pos2,
        rect: Rect,
        frame: WorkplaneFrame,
    ) -> Option<Vec3> {
        let ray = self.view_ray(pointer, rect)?;
        let origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
        let normal = Vec3::new(frame.normal[0], frame.normal[1], frame.normal[2]);
        let denominator = dot(ray.direction, normal);
        if denominator.abs() <= ROUNDING {
            return None;
        }
        let distance = dot(origin - ray.origin, normal) / denominator;
        (distance >= 0.0).then(|| ray.origin + ray.direction * distance)
    }

    pub(crate) fn sketch_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        if self.active_tool == ActiveTool::Line
            && let (Some(start), Some(axis)) =
                (self.gesture.sketch.start, self.gesture.sketch.axis_lock)
        {
            let ray = self.view_ray(pointer, rect)?;
            let travel = axis_travel_along(&ray, start, axis)?;
            return self
                .scene_snap_at_screen(pointer, rect, 8.0, None)
                .map(|snap| snap.position_mm)
                .or(Some(start + axis_direction(axis) * travel));
        }
        self.viewport_point_at_screen(pointer, rect, plane_z)
    }

    pub fn zoom_at_screen(&mut self, pointer: Pos2, rect: Rect, scroll: f32) {
        let anchor = self
            .surface_point_at_screen(pointer, rect)
            .or_else(|| self.screen_to_plane(pointer, rect, self.camera.target_z));
        let Some(anchor) = anchor else {
            return;
        };
        let old_position = self.project(anchor, rect);
        let new_zoom =
            (self.camera.zoom * (scroll * 0.001).exp()).clamp(MIN_CAMERA_ZOOM, MAX_CAMERA_ZOOM);
        if new_zoom == self.camera.zoom {
            return;
        }
        self.camera.zoom = new_zoom;
        self.refresh_camera_distance();
        let new_position = self.project(anchor, rect);
        self.camera.pan += old_position - new_position;
    }

    /// The nearest exact or mesh surface under the pointer: the point hit and
    /// the outward normal there.
    pub(crate) fn surface_hit_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<(Vec3, Vec3)> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let exact = self
            .exact_projection(&snapshot)
            .exact_surface_pick(ray)
            .map(|hit| (hit.ray_distance_mm, hit.position_mm, hit.outward_normal));
        let mesh = MeshInteractionProjection::from_snapshot(&snapshot)
            .exact_surface_pick(ray)
            .map(|hit| (hit.ray_distance_mm, hit.position_mm, hit.outward_normal));
        [exact, mesh]
            .into_iter()
            .flatten()
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, point, normal)| (point, normal))
    }

    pub(crate) fn surface_point_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        self.surface_hit_at_screen(pointer, rect)
            .map(|(point, _)| point)
            .or_else(|| {
                self.pick_result_at_screen(pointer, rect, 8.0)
                    .map(|pick| pick.primary.position_mm)
            })
    }

    pub(crate) fn hover_readout(&self) -> String {
        let Some(hovered) = self
            .hover
            .target
            .as_ref()
            .or(self.selection.primary.as_ref())
        else {
            return self.catalog.text("hover-none");
        };
        let snapshot = self.document.current();
        let Some(occurrence) = snapshot.occurrence(hovered.instance_path.root_occurrence()) else {
            return self.catalog.text("hover-none");
        };
        let Some(definition) = snapshot.definition(occurrence.definition_id()) else {
            return self.catalog.text("hover-none");
        };
        let face = self.catalog.text(match hovered.element {
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            } => "face-top",
            ElementId::Face {
                axis: Axis::Z,
                side: Side::Minimum,
            } => "face-bottom",
            _ => "face-side",
        });
        let overlap_count = self
            .hover
            .pick
            .as_ref()
            .map_or(0, |pick| pick.overlapping.len());
        if let Some(snap) = self.hover.snap.as_ref()
            && (snap.kind != SnapKind::Face || overlap_count > 1)
        {
            return self.catalog.format(
                "hover-inference",
                &BTreeMap::from([
                    ("name", definition.name().to_owned()),
                    ("face", face),
                    (
                        "snap",
                        self.catalog.text(match snap.kind {
                            SnapKind::Endpoint => "snap-endpoint",
                            SnapKind::Intersection => "snap-intersection",
                            SnapKind::Midpoint => "snap-midpoint",
                            SnapKind::Center => "snap-center",
                            SnapKind::Tangent => "snap-tangent",
                            SnapKind::Edge => "snap-edge",
                            SnapKind::Face => "snap-face",
                        }),
                    ),
                    ("index", (self.hover.overlap_index + 1).to_string()),
                    ("count", overlap_count.to_string()),
                ]),
            );
        }
        self.catalog.format(
            "hover-face",
            &BTreeMap::from([("name", definition.name().to_owned()), ("face", face)]),
        )
    }

    pub(crate) fn viewport_overlays(&mut self, ui: &mut egui::Ui, rect: Rect) {
        let palette = self.palette();
        let painter = ui.painter();
        let glass = palette.glass();
        let line = palette.line;
        let text = palette.text;
        let dim = palette.dim;
        let mono = egui::FontId::monospace(11.0);

        // What is under the pointer, or selected, is the one thing the user
        // looks up most often, so it sits top-left where the eye starts.
        let hover_galley =
            painter.layout_no_wrap(self.hover_readout(), egui::FontId::proportional(12.0), text);
        let hover_rect = Rect::from_min_size(
            rect.left_top() + Vec2::new(14.0, 12.0),
            Vec2::new(hover_galley.size().x + 34.0, 28.0),
        );
        painter.rect_filled(hover_rect, 8.0, glass);
        painter.rect_stroke(
            hover_rect,
            8.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        painter.circle_filled(
            Pos2::new(hover_rect.left() + 13.0, hover_rect.center().y),
            3.0,
            palette.accent,
        );
        painter.galley(
            Pos2::new(
                hover_rect.left() + 24.0,
                hover_rect.center().y - hover_galley.size().y * 0.5,
            ),
            hover_galley,
            text,
        );
        if let Some((index, count)) = self
            .hovered_overlap_choice()
            .filter(|(_, count)| *count > 1)
        {
            if self.face_workflow.xray_preview() {
                painter.rect_stroke(
                    hover_rect.expand(3.0),
                    10.0,
                    Stroke::new(2.0_f32, palette.accent),
                    egui::StrokeKind::Outside,
                );
            }
            painter.text(
                hover_rect.left_bottom() + Vec2::new(0.0, 8.0),
                egui::Align2::LEFT_TOP,
                self.catalog.format(
                    if self.face_workflow.xray_preview() {
                        "face-workflow-xray"
                    } else {
                        "face-workflow-overlap-hint"
                    },
                    &BTreeMap::from([
                        ("index", (index + 1).to_string()),
                        ("count", count.to_string()),
                    ]),
                ),
                egui::FontId::proportional(SHELL_SMALL_SIZE),
                palette.accent,
            );
        }

        let camera = self.catalog.format(
            "camera-readout",
            &BTreeMap::from([
                ("distance", format_height(self.camera_distance())),
                (
                    "azimuth",
                    format_height(f64::from(self.camera.yaw.to_degrees())),
                ),
                (
                    "elevation",
                    format_height(f64::from(self.camera.pitch.to_degrees())),
                ),
            ]),
        );
        // The camera readout is laid out first so the plate is sized to the text
        // instead of to a guessed constant that the text then overflows.
        let camera_galley = painter.layout_no_wrap(camera, mono.clone(), dim);
        let readout_rect = Rect::from_min_size(
            Pos2::new(
                rect.right() - 14.0 - (camera_galley.size().x + 24.0),
                rect.top() + 12.0,
            ),
            Vec2::new(camera_galley.size().x + 24.0, 28.0),
        );
        painter.rect_filled(readout_rect, 7.0, glass);
        painter.galley(
            Pos2::new(
                readout_rect.center().x - camera_galley.size().x * 0.5,
                readout_rect.center().y - camera_galley.size().y * 0.5,
            ),
            camera_galley,
            dim,
        );

        // The value box is placed first; the hint then takes whatever width is
        // left, so the two can never paint over one another.
        let value_size = Vec2::new(248.0_f32.min(rect.width() - 28.0), 46.0);
        let value_rect = Rect::from_min_size(
            Pos2::new(
                rect.right() - 14.0 - value_size.x,
                rect.bottom() - 14.0 - value_size.y,
            ),
            value_size,
        );
        painter.rect_filled(value_rect, 8.0, glass);
        painter.rect_stroke(
            value_rect,
            8.0,
            Stroke::new(1.0_f32, line),
            egui::StrokeKind::Inside,
        );
        // Label and field share one row: the label names the quantity on the
        // left, the accent-coloured number and its unit read off the right.
        let value_label = self.catalog.text(self.value_label_key());
        painter.text(
            Pos2::new(value_rect.left() + 14.0, value_rect.center().y),
            egui::Align2::LEFT_CENTER,
            value_label.clone(),
            egui::FontId::proportional(SHELL_SMALL_SIZE),
            dim,
        );
        let value_label_response = ui.interact(
            Rect::from_min_max(
                Pos2::new(value_rect.left() + 14.0, value_rect.top() + 8.0),
                Pos2::new(value_rect.center().x - 10.0, value_rect.bottom() - 8.0),
            ),
            ui.id().with("value-box-label"),
            Sense::hover(),
        );
        value_label_response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Label, true, value_label.clone())
        });
        painter.text(
            Pos2::new(value_rect.right() - 12.0, value_rect.center().y),
            egui::Align2::RIGHT_CENTER,
            self.catalog.text("unit-mm"),
            egui::FontId::monospace(10.5),
            palette.faint,
        );
        let input_rect = Rect::from_min_max(
            Pos2::new(value_rect.center().x - 6.0, value_rect.top() + 8.0),
            Pos2::new(value_rect.right() - 34.0, value_rect.bottom() - 8.0),
        );

        // The hint card names the armed tool and then explains it, so the two
        // read as a title and a body rather than as one wall of grey text.
        let hint_width = (value_rect.left() - 12.0) - (rect.left() + 14.0);
        if hint_width >= 200.0 {
            let body_width = hint_width - 46.0;
            let title_galley = painter.layout_no_wrap(
                self.catalog.text(self.active_tool.label_key()),
                egui::FontId::proportional(12.5),
                text,
            );
            let hint_galley = painter.layout(
                self.catalog.text(self.active_tool.hint_key()),
                egui::FontId::proportional(SHELL_SMALL_SIZE),
                dim,
                body_width,
            );
            let height = title_galley.size().y + hint_galley.size().y + 24.0;
            let hint_rect = Rect::from_min_size(
                Pos2::new(rect.left() + 14.0, value_rect.bottom() - height),
                Vec2::new(
                    title_galley.size().x.max(hint_galley.size().x) + 46.0,
                    height,
                ),
            );
            painter.rect_filled(hint_rect, 8.0, glass);
            painter.rect_stroke(
                hint_rect,
                8.0,
                Stroke::new(1.0_f32, line),
                egui::StrokeKind::Inside,
            );
            let badge = Pos2::new(hint_rect.left() + 20.0, hint_rect.top() + 19.0);
            painter.circle_stroke(badge, 7.0, Stroke::new(1.4_f32, palette.accent));
            painter.text(
                badge,
                egui::Align2::CENTER_CENTER,
                "i",
                egui::FontId::proportional(10.0),
                palette.accent,
            );
            let title_top = hint_rect.top() + 12.0;
            painter.galley(
                Pos2::new(hint_rect.left() + 36.0, title_top),
                title_galley.clone(),
                text,
            );
            painter.galley(
                Pos2::new(
                    hint_rect.left() + 36.0,
                    title_top + title_galley.size().y + 3.0,
                ),
                hint_galley,
                dim,
            );
        }
        let response = ui
            .put(
                input_rect,
                egui::TextEdit::singleline(&mut self.value_box.input)
                    .id(egui::Id::new("value-box-input"))
                    .hint_text(self.catalog.text("value-placeholder"))
                    .font(egui::FontId::monospace(15.0))
                    .text_color(palette.accent)
                    .horizontal_align(egui::Align::Max)
                    .frame(false),
            )
            .labelled_by(value_label_response.id);
        if self.value_box.focus {
            response.request_focus();
            self.value_box.focus = false;
        }
        if response.changed() && self.active_tool == ActiveTool::PlanarOffset {
            self.refresh_planar_offset_preview();
        }
        // A single-line `TextEdit` surrenders focus on Enter, so the commit has
        // to be accepted on the frame the focus is lost as well.
        if (response.has_focus() || response.lost_focus())
            && ui.input(|input| input.key_pressed(egui::Key::Enter))
        {
            self.apply_value_input();
            response.surrender_focus();
        }
    }

    pub(crate) fn viewport(&mut self, ui: &mut egui::Ui) {
        self.show_scene_tabs(ui);
        let preserve_hover = self.show_edit_context_bar(ui);
        self.refresh_camera_distance();
        let desired = ui.available_size().max(Vec2::new(320.0, 280.0));
        let (response, painter) = ui.allocate_painter(desired, Sense::click_and_drag());
        self.camera.viewport_rect = Some(response.rect);
        if self.camera.zoom_fit_pending {
            if self.camera.zoom_fit_pending_quiet {
                let bounds = self.active_frame_bounds();
                self.camera.zoom_fit_pending = !self.frame_bounds(&bounds);
                self.camera.zoom_fit_pending_quiet = self.camera.zoom_fit_pending;
            } else {
                self.zoom_fit();
            }
            self.refresh_camera_distance();
        }
        let palette = self.palette();
        let (viewport_inner, viewport_outer) = if self.view.contains(ViewFlag::WhiteBackground) {
            (Color32::WHITE, Color32::WHITE)
        } else {
            (palette.viewport_inner, palette.viewport_outer)
        };
        theme::paint_vignette(&painter, response.rect, viewport_inner, viewport_outer);
        let camera_dragging = response.dragged_by(egui::PointerButton::Secondary)
            || response.dragged_by(egui::PointerButton::Middle)
            || (response.dragged_by(egui::PointerButton::Primary)
                && matches!(self.active_tool, ActiveTool::Orbit | ActiveTool::Pan));
        if camera_dragging {
            self.hover.pick = None;
            self.hover.target = None;
            self.hover.snap = None;
        } else if !preserve_hover {
            self.update_viewport_inference(response.hover_pos(), response.rect);
        }

        let primary_press = ui.input(|input| {
            input
                .pointer
                .button_pressed(egui::PointerButton::Primary)
                .then(|| input.pointer.press_origin())
                .flatten()
        });
        let primary_release =
            ui.input(|input| input.pointer.button_released(egui::PointerButton::Primary));
        if response.hovered()
            && let Some(pointer) = primary_press
        {
            self.viewport_primary_press(ui, &response, pointer);
        }
        self.viewport_tool_drags(ui, &response);
        let camera_before = self.camera_view_state();
        self.viewport_camera_drag(ui, &response, camera_dragging);
        self.viewport_release_and_hover(ui, &response, primary_release);
        if self.camera_view_state() != camera_before {
            // The hover above was picked with the old camera; without another
            // frame a wheel zoom would leave a part highlighted that is no
            // longer under the pointer.
            ui.ctx().request_repaint();
        }
        let scene = self.project_viewport_scene(response.rect, camera_dragging);
        self.paint_viewport_scene(ui, &response, &painter, scene);
    }

    /// A primary press in the viewport: starts a zoom window, places a sketch
    /// point, adds a revolve axis point or picks with the active tool.
    fn viewport_primary_press(&mut self, ui: &egui::Ui, response: &egui::Response, pointer: Pos2) {
        self.face_workflow.set_xray_preview(false);
        self.gesture.drag.close::<PushPullDrag>();
        self.take_move_session(Some(ToolSessionPhase::Gesture));
        if self.active_tool == ActiveTool::ZoomWindow {
            self.gesture.drag.open(ZoomWindowDrag {
                start: pointer,
                cursor: pointer,
            });
        } else if self.gesture.sketch.armed {
            let point = if self.uses_drawing_plane() {
                self.drawing_input_point(pointer, response.rect)
            } else {
                let plane_z = self.gesture.sketch.start.map_or_else(
                    || self.rectangle_plane_z(pointer, response.rect),
                    |start| start.z,
                );
                self.sketch_point_at_screen(pointer, response.rect, plane_z)
            };
            if let Some(point) = point {
                self.gesture.sketch.dragging_first_point = self.gesture.sketch.start.is_none();
                self.place_sketch_point(point);
            }
        } else if self.active_tool == ActiveTool::Revolve {
            if let Some(plane_z) = self
                .solid_tools
                .revolve
                .as_ref()
                .map(|tool| tool.source.plane_z)
                && let Some(point) = self.viewport_point_at_screen(pointer, response.rect, plane_z)
            {
                self.add_revolve_axis_point(point);
            }
        } else if self.active_tool == ActiveTool::Select {
            let additive = ui.input(|input| input.modifiers.shift);
            let target = self
                .hover
                .snap
                .as_ref()
                .filter(|snap| {
                    matches!(
                        snap.reference.element,
                        ElementId::Edge(_) | ElementId::EdgeMidpoint(_)
                    ) || (matches!(snap.reference.element, ElementId::TopologicalEdge { .. })
                        && matches!(
                            snap.kind,
                            SnapKind::Edge | SnapKind::Midpoint | SnapKind::Center
                        )
                        && self
                            .project(snap.position_mm, response.rect)
                            .distance(pointer)
                            <= 3.0
                        && (snap.kind != SnapKind::Center
                            || self.hover.target.as_ref().is_some_and(|hit| {
                                hit.instance_path == snap.reference.instance_path
                            })))
                        || (matches!(snap.reference.element, ElementId::Snap { .. })
                            && self.hover.target.as_ref().is_none_or(|hit| {
                                hit.instance_path != snap.reference.instance_path
                            }))
                })
                .map(|snap| snap.reference.clone())
                .or_else(|| self.hover.target.clone());
            if target.is_none() {
                self.gesture.drag.open(SelectionWindowDrag {
                    start: pointer,
                    cursor: pointer,
                    additive,
                });
            } else {
                let topological = target.as_ref().and_then(|selection| {
                    self.topological_selection_at_screen(pointer, response.rect, selection)
                });
                match (target, topological) {
                    (Some(target), Some(topological))
                        if !additive
                            || self.selection.occurrences.is_empty()
                            || !self.selection.topological.is_empty() =>
                    {
                        let same_topological_scope = self
                            .selection
                            .topological
                            .first()
                            .is_some_and(|(current, _)| {
                                current.definition_id == target.definition_id
                                    && current.instance_path == target.instance_path
                            });
                        if !self
                            .selection
                            .select_topological(target.clone(), topological, additive)
                            && !same_topological_scope
                        {
                            self.select_part_from_viewport(target, additive);
                        }
                    }
                    (Some(target), Some(_)) => self.select_part_from_viewport(target, additive),
                    (target, _) => self.select_from_viewport(target, additive),
                }
            }
        } else if matches!(
            self.active_tool,
            ActiveTool::SolidSubtract
                | ActiveTool::SolidTrim
                | ActiveTool::SolidUnion
                | ActiveTool::SolidIntersect
                | ActiveTool::SolidSplit
        ) {
            let selection = self.hover.target.clone();
            let keep_tool =
                self.active_tool == ActiveTool::SolidTrim || ui.input(|input| input.modifiers.ctrl);
            self.select_solid_tool_occurrence(selection, keep_tool);
        } else if self.active_tool == ActiveTool::PushPull {
            if let Some(anchor) = self
                .gesture
                .drag
                .remove::<PushPullAnchor>()
                .map(|anchor| anchor.0)
            {
                if self.update_push_pull_gesture(&anchor, pointer)
                    && (self.has_preview()
                        || self.has_occurrence_operation_preview()
                        || self.has_drawn_shape_preview())
                {
                    self.confirm_push_pull_preview();
                } else if self.push_pull_gesture_is_current(&anchor) {
                    self.gesture.drag.open(PushPullAnchor(anchor));
                }
            } else {
                if let Some(target) = self.push_pull_pointer_target() {
                    self.select_push_pull_target(target, pointer, response.rect);
                }
                if self.push_pull_face_selected()
                    && let Some(selection) = self.selection.primary.clone()
                    && let Some((screen_normal, pixels_per_mm)) =
                        self.push_pull_screen_projection(&selection, pointer, response.rect)
                    && let Some(extent_start_mm) = self.selected_face_extent_mm()
                {
                    let snapshot = self.document.current();
                    self.push_pull.distance_input = "0".to_owned();
                    self.value_box.input = "0".to_owned();
                    self.gesture.drag.open(PushPullDrag {
                        source_document_id: snapshot.document_id(),
                        source_revision: snapshot.revision_id(),
                        source_digest: snapshot.canonical_digest(),
                        selection,
                        pointer_start: pointer,
                        extent_start_mm,
                        screen_normal,
                        pixels_per_mm,
                    });
                }
            }
        } else if self.active_tool == ActiveTool::Move {
            if let Some(mut anchor) = self.take_move_session(Some(ToolSessionPhase::Anchor)) {
                if !self.move_preview_is_current(&anchor) {
                    self.commit_move_drag(&anchor);
                } else {
                    if TransformInputInterpreter::interpret_pointer_copy(
                        ui.input(|input| input.modifiers.command),
                    ) == Some(TransformInputEvent::CopyRequested)
                    {
                        anchor.copy = anchor.group_id.is_none();
                        self.gesture.transform.move_copy = anchor.copy;
                    }
                    self.advance_move(
                        &mut anchor,
                        pointer,
                        response.rect,
                        ui.input(|input| input.modifiers.shift),
                    );
                    if length(anchor.delta_mm) >= 0.01 {
                        self.commit_move_drag(&anchor);
                    } else {
                        self.set_move_session(ToolSessionPhase::Anchor, anchor);
                    }
                }
            } else {
                let copy_requested = TransformInputInterpreter::interpret_pointer_copy(
                    ui.input(|input| input.modifiers.command),
                ) == Some(TransformInputEvent::CopyRequested);
                self.begin_move_drag_at(
                    pointer,
                    response.rect,
                    self.gesture.transform.move_copy || copy_requested,
                );
            }
        } else if self.active_tool == ActiveTool::Rotate {
            if let Some(mut anchor) = self.take_rotate_session(Some(ToolSessionPhase::Anchor)) {
                if TransformInputInterpreter::interpret_pointer_copy(
                    ui.input(|input| input.modifiers.command),
                ) == Some(TransformInputEvent::CopyRequested)
                {
                    anchor.copy = anchor.group_id.is_none();
                    self.gesture.transform.rotate_copy = anchor.copy;
                }
                if !self.rotate_preview_is_current(&anchor) {
                    self.commit_rotate_drag(&anchor);
                } else if anchor.reference_mm.is_none() {
                    anchor.reference_mm = self
                        .screen_to_rotation_plane(
                            pointer,
                            response.rect,
                            anchor.centre_mm,
                            anchor.axis,
                        )
                        .map(|point| point - anchor.centre_mm)
                        .filter(|arm| length(*arm) >= ROTATION_MIN_ARM_MM);
                    self.set_rotate_session(ToolSessionPhase::Gesture, anchor);
                } else {
                    self.advance_rotation(
                        &mut anchor,
                        pointer,
                        response.rect,
                        ui.input(|input| input.modifiers.shift),
                    );
                    if rotation_is_meaningful(anchor.angle_degrees) {
                        self.commit_rotate_drag(&anchor);
                    } else {
                        self.set_rotate_session(ToolSessionPhase::Anchor, anchor);
                    }
                }
            } else {
                let copy_requested = TransformInputInterpreter::interpret_pointer_copy(
                    ui.input(|input| input.modifiers.command),
                ) == Some(TransformInputEvent::CopyRequested);
                self.begin_rotate_drag_at(pointer, response.rect, copy_requested);
            }
        } else if self.active_tool == ActiveTool::Scale {
            self.begin_scale_drag_at(pointer, response.rect);
        } else if self.active_tool == ActiveTool::Mirror {
            self.gesture.mirror = self.mirror_plane_at_screen(pointer, response.rect);
            self.commit_mirror();
        } else if self.active_tool == ActiveTool::PlanarOffset {
            if self.value_box.input.trim().is_empty() {
                self.gesture.planar_offset_mm =
                    self.planar_offset_distance_at_screen(pointer, response.rect);
            }
            if self.refresh_planar_offset_preview() {
                self.confirm_planar_offset_preview();
            }
        } else if self.active_tool == ActiveTool::Measure {
            let plane_z = self.measure_anchor().map_or_else(
                || self.rectangle_plane_z(pointer, response.rect),
                |start| start.z,
            );
            if let Some(point) = self.measurement_point_at_screen(pointer, response.rect, plane_z) {
                self.add_measured_point(point);
            }
        }
    }

    /// Drags of the Select, Move, Rotate and Push/Pull tools.
    fn viewport_tool_drags(&mut self, ui: &egui::Ui, response: &egui::Response) {
        if self.active_tool == ActiveTool::Select
            && response.double_clicked()
            && let Some(target) = self.hover.target.clone()
        {
            self.enter_occurrence_context(target.instance_path);
        }

        if self.active_tool == ActiveTool::Move
            && let Some(mut anchor) = self
                .move_session()
                .and_then(|(drag, phase)| (phase == ToolSessionPhase::Anchor).then(|| drag.clone()))
            && self.move_preview_is_current(&anchor)
            && let Some(pointer) = response.hover_pos()
        {
            self.advance_move(
                &mut anchor,
                pointer,
                response.rect,
                ui.input(|input| input.modifiers.shift),
            );
            let distance = length(anchor.delta_mm);
            let delta_mm = anchor.delta_mm;
            let copy = anchor.copy;
            self.set_move_session(ToolSessionPhase::Anchor, anchor);
            // While an anchor waits, the pointer only proposes a value. Typing
            // one is the stronger statement, so the live reading must not
            // overwrite what is being entered.
            if !self.value_box_is_being_typed_into(ui.ctx()) {
                self.value_box.input = format_height(distance);
            }
            self.digest = self.catalog.format(
                if copy {
                    "digest-copy-live"
                } else {
                    "digest-move-live"
                },
                &BTreeMap::from([
                    ("distance", format_height(distance)),
                    ("vector", format_vector_mm(delta_mm)),
                ]),
            );
        }

        if self.active_tool == ActiveTool::Rotate
            && let Some(mut anchor) = self
                .rotate_session()
                .and_then(|(drag, phase)| (phase == ToolSessionPhase::Anchor).then(|| drag.clone()))
            && self.rotate_preview_is_current(&anchor)
            && let Some(pointer) = response.hover_pos()
        {
            self.advance_rotation(
                &mut anchor,
                pointer,
                response.rect,
                ui.input(|input| input.modifiers.shift),
            );
            let angle = anchor.angle_degrees;
            let axis = anchor.axis;
            let copy = anchor.copy;
            self.set_rotate_session(ToolSessionPhase::Anchor, anchor);
            if !self.value_box_is_being_typed_into(ui.ctx()) {
                self.value_box.input = format_angle(angle);
            }
            self.digest = self.catalog.format(
                if self
                    .active_rotate_gesture()
                    .is_some_and(|drag| drag.reference_mm.is_none())
                {
                    "digest-rotate-anchor-set"
                } else if copy {
                    "digest-rotate-copy-live"
                } else {
                    "digest-rotate-live"
                },
                &BTreeMap::from([
                    ("angle", format_angle(angle)),
                    ("axis", self.catalog.text(axis_name_key(axis))),
                ]),
            );
        }

        if self.active_tool == ActiveTool::PushPull
            && self.gesture.drag.get::<PushPullDrag>().is_none()
            && !self.value_box_is_being_typed_into(ui.ctx())
            && let Some(anchor) = self
                .gesture
                .drag
                .get::<PushPullAnchor>()
                .map(|anchor| anchor.0.clone())
            && let Some(pointer) = response.hover_pos()
        {
            self.update_push_pull_gesture(&anchor, pointer);
        }
    }

    /// Orbit, pan and zoom from secondary, middle or Orbit/Pan tool drags.
    fn viewport_camera_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        camera_dragging: bool,
    ) {
        let pointer_delta = ui.input(|input| input.pointer.delta());
        let camera_before =
            (camera_dragging && !self.camera.drag_active && pointer_delta != Vec2::ZERO)
                .then(|| self.camera_view_state());
        if response.dragged_by(egui::PointerButton::Secondary) {
            self.orbit(pointer_delta);
        } else if response.dragged_by(egui::PointerButton::Middle) {
            if ui.input(|input| input.modifiers.shift) {
                self.camera.pan += pointer_delta;
            } else {
                self.orbit(pointer_delta);
            }
        } else if response.dragged_by(egui::PointerButton::Primary) {
            if self.active_tool == ActiveTool::Select {
                if let Some(pointer) = response.interact_pointer_pos()
                    && let Some(drag) = self.gesture.drag.get_mut::<SelectionWindowDrag>()
                {
                    drag.cursor = pointer;
                }
            } else if self.active_tool == ActiveTool::ZoomWindow {
                if let Some(pointer) = response.interact_pointer_pos()
                    && let Some(window) = self.gesture.drag.get_mut::<ZoomWindowDrag>()
                {
                    window.cursor = pointer;
                }
            } else if self.active_tool == ActiveTool::Orbit {
                self.orbit(pointer_delta);
            } else if self.active_tool == ActiveTool::Pan {
                self.camera.pan += pointer_delta;
            } else if self.gesture.sketch.armed {
                if let (Some(start), Some(pointer)) =
                    (self.gesture.sketch.start, response.interact_pointer_pos())
                {
                    self.gesture.sketch.cursor = if self.uses_drawing_plane() {
                        self.drawing_input_point(pointer, response.rect)
                    } else {
                        self.sketch_point_at_screen(pointer, response.rect, start.z)
                    };
                }
            } else if let (Some(mut drag), Some(pointer)) = (
                self.move_session().and_then(|(drag, phase)| {
                    (phase == ToolSessionPhase::Gesture).then(|| drag.clone())
                }),
                response.interact_pointer_pos(),
            ) {
                self.advance_move(
                    &mut drag,
                    pointer,
                    response.rect,
                    ui.input(|input| input.modifiers.shift),
                );
                let distance = length(drag.delta_mm);
                let delta_mm = drag.delta_mm;
                let copy = drag.copy;
                self.set_move_session(ToolSessionPhase::Gesture, drag);
                self.value_box.input = format_height(distance);
                self.digest = self.catalog.format(
                    if copy {
                        "digest-copy-live"
                    } else {
                        "digest-move-live"
                    },
                    &BTreeMap::from([
                        ("distance", format_height(distance)),
                        ("vector", format_vector_mm(delta_mm)),
                    ]),
                );
            } else if let (Some(mut drag), Some(pointer)) = (
                self.rotate_session().and_then(|(drag, phase)| {
                    (phase == ToolSessionPhase::Gesture).then(|| drag.clone())
                }),
                response.interact_pointer_pos(),
            ) {
                self.advance_rotation(
                    &mut drag,
                    pointer,
                    response.rect,
                    ui.input(|input| input.modifiers.shift),
                );
                let angle = drag.angle_degrees;
                let axis = drag.axis;
                let copy = drag.copy;
                self.set_rotate_session(ToolSessionPhase::Gesture, drag);
                self.value_box.input = format_angle(angle);
                self.digest = self.catalog.format(
                    if copy {
                        "digest-rotate-copy-live"
                    } else {
                        "digest-rotate-live"
                    },
                    &BTreeMap::from([
                        ("angle", format_angle(angle)),
                        ("axis", self.catalog.text(axis_name_key(axis))),
                    ]),
                );
            } else if let (Some(mut drag), Some(pointer)) = (
                self.scale_session().cloned(),
                response.interact_pointer_pos(),
            ) {
                Self::advance_scale(&mut drag, pointer);
                let factor = drag.factor;
                let axis = drag.axis;
                self.set_scale_session(drag);
                self.value_box.input = format_scale_factor(factor);
                self.digest = self.catalog.format(
                    "digest-scale-live",
                    &BTreeMap::from([
                        ("factor", format_scale_factor(factor)),
                        (
                            "axis",
                            self.catalog
                                .text(axis.map_or("axis-name-uniform", axis_name_key)),
                        ),
                    ]),
                );
            } else if let (Some(drag), Some(pointer)) = (
                self.gesture.drag.get::<PushPullDrag>().cloned(),
                response.interact_pointer_pos(),
            ) {
                self.update_push_pull_gesture(&drag, pointer);
            }
        }
        if let Some(before) = camera_before {
            self.remember_camera_change(before);
        }
        self.camera.drag_active =
            camera_dragging && (self.camera.drag_active || pointer_delta != Vec2::ZERO);
    }

    /// Primary release and drag end, sketch rubber band, measurement and hover.
    fn viewport_release_and_hover(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        primary_release: bool,
    ) {
        // Releasing the press that placed the first point after dragging places
        // the next point there, so a drag draws what two clicks draw.
        let dragged = response.drag_stopped_by(egui::PointerButton::Primary);
        if dragged || primary_release {
            let placed_first_point = std::mem::take(&mut self.gesture.sketch.dragging_first_point);
            if dragged
                && placed_first_point
                && self.gesture.sketch.armed
                && let (Some(start), Some(cursor)) =
                    (self.gesture.sketch.start, self.gesture.sketch.cursor)
                && length(cursor - start) > limits::MIN_LENGTH_MM
            {
                self.place_sketch_point(cursor);
            }
        }
        if response.drag_stopped_by(egui::PointerButton::Primary)
            || (response.hovered() && primary_release)
        {
            if self.active_tool == ActiveTool::Select
                && self.gesture.drag.get::<SelectionWindowDrag>().is_some()
            {
                if let Some(drag) = self.gesture.drag.remove::<SelectionWindowDrag>() {
                    self.complete_selection_window(drag, response.rect);
                }
            } else if self.active_tool == ActiveTool::ZoomWindow {
                if let Some(window) = self.gesture.drag.remove::<ZoomWindowDrag>()
                    && self.zoom_window(window.start, window.cursor, response.rect)
                {
                    self.active_tool = ActiveTool::Select;
                    self.status_key = "status-ready";
                }
            } else if let Some(mut drag) = self.take_move_session(Some(ToolSessionPhase::Gesture)) {
                if TransformInputInterpreter::interpret_pointer_copy(
                    ui.input(|input| input.modifiers.command),
                ) == Some(TransformInputEvent::CopyRequested)
                {
                    drag.copy = drag.group_id.is_none();
                    self.gesture.transform.move_copy = drag.copy;
                }
                if !self.move_preview_is_current(&drag) || length(drag.delta_mm) >= 0.01 {
                    self.commit_move_drag(&drag);
                } else {
                    self.set_move_session(ToolSessionPhase::Anchor, drag);
                    self.digest = self.catalog.text("digest-move-anchor-set");
                }
            } else if let Some(drag) = self.take_rotate_session(Some(ToolSessionPhase::Gesture)) {
                if !self.rotate_preview_is_current(&drag)
                    || rotation_is_meaningful(drag.angle_degrees)
                {
                    self.commit_rotate_drag(&drag);
                } else {
                    self.set_rotate_session(ToolSessionPhase::Anchor, drag);
                    self.digest = self.catalog.text("digest-rotate-anchor-set");
                }
            } else if let Some(drag) = self.take_scale_session() {
                self.commit_scale_drag(&drag);
            } else if let Some(drag) = self.gesture.drag.remove::<PushPullDrag>() {
                if self.has_preview()
                    || self.has_occurrence_operation_preview()
                    || self.has_drawn_shape_preview()
                {
                    self.confirm_push_pull_preview();
                } else if self.push_pull_gesture_is_current(&drag) {
                    self.gesture.drag.open(PushPullAnchor(drag));
                    self.digest = self.catalog.text("digest-push-pull-anchor-set");
                }
            }
        }
        if self.gesture.sketch.armed
            && self.gesture.sketch.start.is_some()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            self.gesture.sketch.cursor = if self.uses_drawing_plane() {
                self.drawing_input_point(pointer, response.rect)
            } else {
                let plane_z = self.gesture.sketch.start.map_or(0.0, |start| start.z);
                self.sketch_point_at_screen(pointer, response.rect, plane_z)
            };
            // Once the user starts typing, the value box owns the value: the
            // focus request only takes effect next frame, so the freshly typed
            // text would otherwise be overwritten by the hovered dimensions.
            if !ui.ctx().wants_keyboard_input()
                && !self.value_box.focus
                && let (Some(start), Some(cursor)) =
                    (self.gesture.sketch.start, self.gesture.sketch.cursor)
            {
                self.value_box.input = match self.active_tool {
                    ActiveTool::Line => format_height(length(Vec3::new(
                        cursor.x - start.x,
                        cursor.y - start.y,
                        cursor.z - start.z,
                    ))),
                    ActiveTool::Spline => self
                        .gesture
                        .sketch
                        .chain_points
                        .last()
                        .map_or_else(String::new, |last| format_height(length(cursor - *last))),
                    ActiveTool::Ellipse => self.gesture.sketch.end.map_or_else(
                        || format_height(length(cursor - start)),
                        |end| {
                            self.ellipse_axes(start, end, cursor)
                                .map_or_else(String::new, |(_, radius_y, _)| {
                                    format_height(radius_y)
                                })
                        },
                    ),
                    ActiveTool::Circle | ActiveTool::Polygon => format_height(length(Vec3::new(
                        cursor.x - start.x,
                        cursor.y - start.y,
                        cursor.z - start.z,
                    ))),
                    ActiveTool::Arc => self.gesture.sketch.end.map_or_else(
                        || format_height(length(cursor - start)),
                        |end| format_height(self.drawing_bulge(start, end, cursor).abs()),
                    ),
                    ActiveTool::Rectangle => {
                        let frame = self.drawing_frame(Some(start));
                        let frame_x = Vec3::new(frame.x_axis[0], frame.x_axis[1], frame.x_axis[2]);
                        let frame_y = Vec3::new(frame.y_axis[0], frame.y_axis[1], frame.y_axis[2]);
                        format!(
                            "{},{}",
                            format_height(dot(cursor - start, frame_x).abs()),
                            format_height(dot(cursor - start, frame_y).abs())
                        )
                    }
                    _ => format!(
                        "{},{}",
                        format_height((cursor.x - start.x).abs()),
                        format_height((cursor.y - start.y).abs())
                    ),
                };
            }
        }
        if self.active_tool == ActiveTool::Mirror
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            self.gesture.mirror = self.mirror_plane_at_screen(pointer, response.rect);
        }
        if self.active_tool == ActiveTool::PlanarOffset
            && self.value_box.input.trim().is_empty()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
        {
            let distance_mm = self.planar_offset_distance_at_screen(pointer, response.rect);
            if distance_mm != self.gesture.planar_offset_mm {
                self.gesture.planar_offset_mm = distance_mm;
                self.refresh_planar_offset_preview();
            }
        }
        if let Some(start) = self.measure_anchor()
            && response.hovered()
            && let Some(pointer) = ui.input(|input| input.pointer.hover_pos())
            && let Some(cursor) = self.measurement_point_at_screen(pointer, response.rect, start.z)
        {
            self.gesture.measure.cursor = Some(cursor);
            self.digest = self.measurement_text(start, cursor, "digest-measure-live");
        }
        if let Some((start, end)) = self.measure_span()
            && !ui.ctx().wants_keyboard_input()
            && !self.value_box.focus
        {
            self.value_box.input = format_height(length(Vec3::new(
                end.x - start.x,
                end.y - start.y,
                end.z - start.z,
            )));
        }
        if response.hovered() {
            let scroll = ui.input(|input| input.raw_scroll_delta.y);
            if scroll != 0.0
                && let Some(pointer) = response.hover_pos()
            {
                if self.camera.wheel_active {
                    self.zoom_at_screen(pointer, response.rect, scroll);
                } else {
                    let before = self.camera_view_state();
                    self.zoom_at_screen(pointer, response.rect, scroll);
                    self.camera.wheel_active = self.remember_camera_change(before);
                }
            } else {
                self.camera.wheel_active = false;
            }
        } else {
            self.camera.wheel_active = false;
        }
    }

    /// Project the scene for this frame: instanced plan, CPU faces and edges.
    /// Projected faces and edges of one CPU-painted frame, for tests of what is shown.
    #[cfg(test)]
    pub(crate) fn projected_faces_and_edges(
        &mut self,
        rect: Rect,
    ) -> (Vec<ProjectedFace>, Vec<ProjectedEdge>) {
        let scene = self.project_viewport_scene(rect, false);
        (scene.faces, scene.edges)
    }

    fn project_viewport_scene(
        &mut self,
        rect: Rect,
        camera_dragging: bool,
    ) -> ProjectedViewportScene {
        let forward = Vec3::new(
            -f64::from(self.camera.yaw.sin() * self.camera.pitch.sin()),
            -f64::from(self.camera.yaw.cos() * self.camera.pitch.sin()),
            -f64::from(self.camera.pitch.cos()),
        );
        let snapshot = self.document.current();
        self.rebind_exact_results(&snapshot);
        let move_transform_overrides = self.preview_transform_overrides();
        let rotate_copies = self.rotation_preview_transforms(true);
        let use_wgpu_scene = self.push_pull.face_offset_evaluation.is_none()
            && self.render.wgpu_target_format.is_some()
            && !self.has_occurrence_operation_preview()
            && !(self.view.contains(ViewFlag::Xray) || self.alt_xray_paints_translucent())
            && !self.view.contains(ViewFlag::Wireframe)
            && !self.view.contains(ViewFlag::Monochrome)
            && !self.view.contains(ViewFlag::HiddenLine);
        let scene_plan = if use_wgpu_scene {
            let plan = match self.render.plan.as_ref() {
                Some(plan)
                    if plan.is_same_revision(&snapshot)
                        && plan.matches_exact_results(&snapshot, &self.exact.results) =>
                {
                    Arc::clone(plan)
                }
                _ => {
                    let plan = Arc::new(InstancedRenderPlan::from_snapshot(
                        &snapshot,
                        &self.exact.results,
                        &mut self.render.cache,
                    ));
                    self.render.plan = Some(Arc::clone(&plan));
                    plan
                }
            };
            Some(if move_transform_overrides.is_empty() {
                plan
            } else {
                self.render
                    .moved_plan(&plan, move_transform_overrides.clone())
            })
        } else {
            None
        };
        self.refresh_interaction_projection_cache(&snapshot);
        let interaction_projection_cache = self.hover.projection_cache.borrow();
        let exact_projection = &interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .exact;
        let active_context_paths = self
            .active_scene_query_for_snapshot(&snapshot)
            .into_iter()
            .map(|occurrence| occurrence.instance_path)
            .collect::<BTreeSet<_>>();
        let occurrence_colors = snapshot
            .scene_query()
            .into_iter()
            .filter_map(|occurrence| {
                occurrence
                    .color()
                    .map(|[r, g, b]| (occurrence.instance_path, Color32::from_rgb(r, g, b)))
            })
            .collect::<BTreeMap<_, _>>();
        let mut faces = Vec::new();
        let mut feedback_faces = Vec::new();
        let mut edges = self
            .open_profile_line_segments()
            .into_iter()
            .filter_map(|(selection, points_mm)| {
                let points_mm = self.section_edge(points_mm)?;
                Some(ProjectedEdge {
                    selection,
                    points: points_mm.map(|point| self.project(point, rect)),
                    depth: points_mm
                        .into_iter()
                        .map(|point| point_depth(point, forward))
                        .sum::<f64>()
                        / 2.0,
                    dominant_axis: dominant_edge_axis(points_mm),
                })
            })
            .collect::<Vec<_>>();
        let viewport_boxes = self.viewport_boxes(&snapshot, exact_projection);
        let frame = ViewportProjectionFrame {
            rect,
            camera_dragging,
            forward,
            snapshot: &snapshot,
            interaction_projection_cache: &interaction_projection_cache,
            exact_projection,
            move_transform_overrides: &move_transform_overrides,
            rotate_copies: &rotate_copies,
            use_wgpu_scene,
            active_context_paths: &active_context_paths,
            occurrence_colors: &occurrence_colors,
        };
        self.project_viewport_boxes(
            &frame,
            &viewport_boxes,
            &mut faces,
            &mut feedback_faces,
            &mut edges,
        );
        self.project_viewport_exact_occurrences(
            &frame,
            &mut faces,
            &mut feedback_faces,
            &mut edges,
        );
        self.project_viewport_mesh_occurrences(&frame, &mut faces, &mut feedback_faces, &mut edges);
        let hidden_ghost_corners = if self.view.contains(ViewFlag::HiddenObjects) {
            self.hidden_ghost_corners(
                &snapshot,
                &interaction_projection_cache
                    .as_ref()
                    .expect("interaction cache was built")
                    .canonical,
            )
        } else {
            Vec::new()
        };
        drop(interaction_projection_cache);
        faces.sort_by(|left, right| right.depth.total_cmp(&left.depth));
        ProjectedViewportScene {
            scene_plan,
            viewport_boxes,
            faces,
            feedback_faces,
            edges,
            hidden_ghost_corners,
        }
    }

    /// Boxes (and their rotate-copy previews) projected to faces and edges.
    fn project_viewport_boxes(
        &self,
        frame: &ViewportProjectionFrame,
        viewport_boxes: &[RenderBox],
        faces: &mut Vec<ProjectedFace>,
        feedback_faces: &mut Vec<ProjectedFace>,
        edges: &mut Vec<ProjectedEdge>,
    ) {
        let ViewportProjectionFrame {
            rect,
            camera_dragging,
            forward,
            snapshot,
            interaction_projection_cache,
            move_transform_overrides,
            rotate_copies,
            use_wgpu_scene,
            active_context_paths,
            occurrence_colors,
            ..
        } = *frame;
        for (item, copy_transform) in viewport_boxes.iter().cloned().flat_map(|item| {
            let copy = rotate_copies.get(&item.instance_path).copied();
            std::iter::once((item.clone(), None))
                .chain(copy.map(|transform| (item, Some(transform))))
        }) {
            let item = self.render_box(item);
            let proxy_preview = self.proxy_preview_is_active(&item);
            let occurrence_color = occurrence_colors.get(&item.instance_path).copied();
            let out_of_context = !active_context_paths.contains(&item.instance_path);
            let needs_cpu_overlay = !camera_dragging
                && (self.selection.contains(&item.instance_path)
                    || self
                        .hover
                        .target
                        .as_ref()
                        .is_some_and(|hovered| hovered.instance_path == item.instance_path)
                    || proxy_preview
                    || out_of_context);
            let needs_cpu_fill = !use_wgpu_scene || proxy_preview || copy_transform.is_some();
            if use_wgpu_scene && !needs_cpu_overlay && !needs_cpu_fill {
                continue;
            }
            let profile_mesh = self.canonical_profile_viewport_mesh(snapshot, &item);
            if let Some((local_positions, triangles)) = profile_mesh {
                let Some(occurrence) = interaction_projection_cache
                    .as_ref()
                    .expect("interaction cache was built")
                    .canonical
                    .occurrences()
                    .iter()
                    .find(|occurrence| occurrence.instance_path == item.instance_path)
                else {
                    continue;
                };
                let transform = copy_transform
                    .or_else(|| move_transform_overrides.get(&item.instance_path).copied())
                    .unwrap_or(occurrence.canonical_world_transform);
                let positions_mm = local_positions
                    .iter()
                    .map(|point| {
                        transform_model_point(transform, Vec3::new(point[0], point[1], point[2]))
                    })
                    .collect::<Vec<_>>();
                let local_positions_f32 = local_positions
                    .iter()
                    .map(|point| point.map(|value| value as f32))
                    .collect::<Vec<_>>();
                let boundary_edges = renderer::feature_edge_triangles(
                    &local_positions_f32,
                    &triangles,
                    &vec![None::<u8>; triangles.len()],
                );
                let selection = SelectionId {
                    definition_id: item.definition_id,
                    instance_path: item.instance_path.clone(),
                    element: ElementId::Face {
                        axis: Axis::Z,
                        side: Side::Maximum,
                    },
                };
                for (edge, uses) in boundary_edges {
                    let Some(points_mm) =
                        self.section_edge(edge.map(|index| positions_mm[index as usize]))
                    else {
                        continue;
                    };
                    let elements = uses
                        .iter()
                        .map(|index| {
                            let points = triangles[*index as usize]
                                .map(|vertex| positions_mm[vertex as usize]);
                            face_element_from_normal(triangle_normal(points))
                        })
                        .collect::<BTreeSet<_>>();
                    for element in elements {
                        edges.push(ProjectedEdge {
                            selection: SelectionId {
                                element,
                                ..selection.clone()
                            },
                            points: points_mm.map(|point| self.project(point, rect)),
                            depth: points_mm
                                .into_iter()
                                .map(|point| point_depth(point, forward))
                                .sum::<f64>()
                                / 2.0,
                            dominant_axis: dominant_edge_axis(points_mm),
                        });
                    }
                }
                if needs_cpu_fill || needs_cpu_overlay {
                    for triangle in triangles {
                        let points_mm = triangle.map(|index| positions_mm[index as usize]);
                        let normal = triangle_normal(points_mm);
                        let selection = SelectionId {
                            element: face_element_from_normal(normal),
                            ..selection.clone()
                        };
                        let front = point_depth(normal, forward) < -ROUNDING;
                        if !front
                            && self.hover.target.as_ref() != Some(&selection)
                            && !self.paints_back_faces()
                        {
                            continue;
                        }
                        let Some(polygon) = self.section_polygon(&points_mm, rect) else {
                            continue;
                        };
                        (if needs_cpu_fill {
                            &mut *faces
                        } else {
                            &mut *feedback_faces
                        })
                        .push(ProjectedFace {
                            selection: selection.clone(),
                            polygon,
                            color: self.section_face_color(
                                front,
                                occurrence_color.unwrap_or_else(|| face_color_from_normal(normal)),
                            ),
                            depth: points_mm
                                .into_iter()
                                .map(|point| point_depth(point, forward))
                                .sum::<f64>()
                                / 3.0,
                            previewed: false,
                            out_of_context,
                        });
                    }
                }
                continue;
            }
            let corners = box_corners(item.size_mm.x, item.size_mm.y, item.size_mm.z)
                .map(|point| point + item.origin_mm);
            let projected = corners.map(|point| self.project(point, rect));
            for face in box_faces() {
                let front = face_is_visible(&face.element, forward);
                let outlined = front
                    || self.hover.target.as_ref().is_some_and(|hovered| {
                        hovered.definition_id == item.definition_id
                            && hovered.instance_path == item.instance_path
                            && hovered.element == face.element
                    });
                if !(outlined || self.paints_back_faces())
                    || !projected_face_has_area(face.corners, &projected)
                {
                    continue;
                }
                let face_mm = face.corners.map(|index| corners[index]);
                let Some(polygon) = self.section_polygon(&face_mm, rect) else {
                    continue;
                };
                let selection = SelectionId {
                    definition_id: item.definition_id,
                    instance_path: item.instance_path.clone(),
                    element: face.element.clone(),
                };
                for edge in (0..face_mm.len()).filter(|_| outlined) {
                    let Some(edge_corners) =
                        self.section_edge([face_mm[edge], face_mm[(edge + 1) % face_mm.len()]])
                    else {
                        continue;
                    };
                    edges.push(ProjectedEdge {
                        selection: selection.clone(),
                        points: edge_corners.map(|point| self.project(point, rect)),
                        depth: edge_corners
                            .into_iter()
                            .map(|point| point_depth(point, forward))
                            .sum::<f64>()
                            / 2.0,
                        dominant_axis: dominant_edge_axis(edge_corners),
                    });
                }
                let depth = face
                    .corners
                    .iter()
                    .map(|index| point_depth(corners[*index], forward))
                    .sum::<f64>()
                    / 4.0;
                if needs_cpu_fill || needs_cpu_overlay {
                    (if needs_cpu_fill {
                        &mut *faces
                    } else {
                        &mut *feedback_faces
                    })
                    .push(ProjectedFace {
                        selection,
                        polygon,
                        color: self
                            .section_face_color(front, occurrence_color.unwrap_or(face.color)),
                        depth,
                        previewed: (self.has_preview()
                            && self.push_pull_preview_definition() == Some(item.definition_id)
                            && matches!(
                                face.element,
                                ElementId::Face {
                                    axis: Axis::Z,
                                    side: Side::Maximum,
                                }
                            ))
                            || (self.has_occurrence_operation_preview()
                                && item.instance_path.is_root()
                                && self
                                    .tool_preview
                                    .get::<OccurrenceOperationPreview>()
                                    .is_some_and(|preview| {
                                        preview
                                            .boxes
                                            .contains_key(&item.instance_path.root_occurrence())
                                    })),
                        out_of_context,
                    });
                }
            }
        }
    }

    /// Exact bodies that need a CPU fill or hover/selection overlay this frame.
    fn project_viewport_exact_occurrences(
        &self,
        frame: &ViewportProjectionFrame,
        faces: &mut Vec<ProjectedFace>,
        feedback_faces: &mut Vec<ProjectedFace>,
        edges: &mut Vec<ProjectedEdge>,
    ) {
        let ViewportProjectionFrame {
            rect,
            camera_dragging,
            forward,
            snapshot,
            interaction_projection_cache,
            exact_projection,
            move_transform_overrides,
            rotate_copies,
            use_wgpu_scene,
            active_context_paths,
            occurrence_colors,
            ..
        } = *frame;
        for (occurrence, copy_transform) in interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical
            .occurrences()
            .iter()
            .filter(|occurrence| {
                occurrence.visible
                    && exact_projection.contains_occurrence(&occurrence.instance_path)
                    && !(self.has_occurrence_operation_preview()
                        && occurrence.instance_path.is_root()
                        && self
                            .tool_preview
                            .get::<OccurrenceOperationPreview>()
                            .is_some_and(|preview| {
                                let occurrence_id = occurrence.instance_path.root_occurrence();
                                preview.boxes.contains_key(&occurrence_id)
                                    || preview.hidden_occurrences.contains(&occurrence_id)
                            }))
            })
            .flat_map(|occurrence| {
                std::iter::once((occurrence, None)).chain(
                    rotate_copies
                        .get(&occurrence.instance_path)
                        .copied()
                        .map(|transform| (occurrence, Some(transform))),
                )
            })
        {
            let occurrence_color = occurrence_colors.get(&occurrence.instance_path).copied();
            let out_of_context = !active_context_paths.contains(&occurrence.instance_path);
            let needs_cpu_overlay =
                !camera_dragging
                    && (self.selection.contains(&occurrence.instance_path)
                        || self.hover.target.as_ref().is_some_and(|hovered| {
                            hovered.instance_path == occurrence.instance_path
                        })
                        || out_of_context);
            let needs_cpu_fill = !use_wgpu_scene || copy_transform.is_some();
            if use_wgpu_scene && !needs_cpu_overlay && !needs_cpu_fill {
                continue;
            }
            let preview_package = self.face_offset_preview_package(occurrence.body.definition_id);
            let previewed = preview_package.is_some();
            let Some(package) = preview_package.or_else(|| {
                self.interaction_render_package(snapshot, occurrence.body.definition_id)
            }) else {
                continue;
            };
            let transform = copy_transform
                .or_else(|| {
                    move_transform_overrides
                        .get(&occurrence.instance_path)
                        .copied()
                })
                .unwrap_or(occurrence.canonical_world_transform);
            let overlay_edges = self.overlay_feature_edges(
                occurrence.body.definition_id,
                &format!("exact:{}", package.result_fingerprint()),
                || {
                    (
                        package
                            .vertices()
                            .iter()
                            .map(|vertex| vertex.position_mm.map(|value| value as f32))
                            .collect(),
                        package
                            .triangles()
                            .iter()
                            .map(|triangle| triangle.vertex_indices)
                            .collect(),
                        package
                            .triangles()
                            .iter()
                            .map(|triangle| triangle.face_role)
                            .collect(),
                    )
                },
            );
            for (edge, uses) in overlay_edges.iter() {
                let Some(points_mm) = self.section_edge(edge.map(|index| {
                    let position = package.vertices()[index as usize].position_mm;
                    transform_model_point(
                        transform,
                        Vec3::new(position[0], position[1], position[2]),
                    )
                })) else {
                    continue;
                };
                let points = points_mm.map(|point| self.project(point, rect));
                let mut elements = Vec::new();
                for triangle in uses
                    .iter()
                    .map(|index| &package.triangles()[*index as usize])
                {
                    let element = triangle
                        .face_role
                        .and_then(exact_face_element)
                        .unwrap_or_else(|| {
                            let points_mm = triangle.vertex_indices.map(|index| {
                                let position = package.vertices()[index as usize].position_mm;
                                transform_model_point(
                                    transform,
                                    Vec3::new(position[0], position[1], position[2]),
                                )
                            });
                            face_element_from_normal(triangle_normal(points_mm))
                        });
                    if !elements.contains(&element) {
                        elements.push(element);
                    }
                }
                for element in elements {
                    edges.push(ProjectedEdge {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        points,
                        depth: points_mm
                            .into_iter()
                            .map(|point| point_depth(point, forward))
                            .sum::<f64>()
                            / 2.0,
                        dominant_axis: dominant_edge_axis(points_mm),
                    });
                }
            }
            let face_ordinals = planar_push_pull::canonical_face_ordinals(&package);
            for (triangle_index, triangle) in package.triangles().iter().enumerate() {
                let points_mm = triangle.vertex_indices.map(|index| {
                    let position = package.vertices()[index as usize].position_mm;
                    transform_model_point(
                        transform,
                        Vec3::new(position[0], position[1], position[2]),
                    )
                });
                let normal = triangle_normal(points_mm);
                let element = planar_push_pull::triangle_element(
                    &package,
                    &face_ordinals,
                    triangle_index,
                    normal,
                );
                let hovered = !previewed
                    && self.hover.target.as_ref().is_some_and(|hovered| {
                        hovered.definition_id == occurrence.body.definition_id
                            && hovered.instance_path == occurrence.instance_path
                            && hovered.element == element
                    });
                let front = point_depth(normal, forward) < -ROUNDING;
                if !front && !hovered && !self.paints_back_faces() {
                    continue;
                }
                let Some(polygon) = self.section_polygon(&points_mm, rect) else {
                    continue;
                };
                if needs_cpu_fill || needs_cpu_overlay {
                    (if needs_cpu_fill {
                        &mut *faces
                    } else {
                        &mut *feedback_faces
                    })
                    .push(ProjectedFace {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        polygon,
                        color: self.section_face_color(
                            front,
                            occurrence_color.unwrap_or_else(|| face_color_from_normal(normal)),
                        ),
                        depth: points_mm
                            .into_iter()
                            .map(|point| point_depth(point, forward))
                            .sum::<f64>()
                            / 3.0,
                        previewed,
                        out_of_context,
                    });
                }
            }
        }
    }

    /// Canonical mesh bodies that need a CPU fill or hover/selection overlay.
    fn project_viewport_mesh_occurrences(
        &self,
        frame: &ViewportProjectionFrame,
        faces: &mut Vec<ProjectedFace>,
        feedback_faces: &mut Vec<ProjectedFace>,
        edges: &mut Vec<ProjectedEdge>,
    ) {
        let ViewportProjectionFrame {
            rect,
            camera_dragging,
            forward,
            snapshot,
            interaction_projection_cache,
            move_transform_overrides,
            rotate_copies,
            use_wgpu_scene,
            active_context_paths,
            occurrence_colors,
            ..
        } = *frame;
        // Canonical mesh bodies are drawn by the instanced scene, but hover and
        // selection feedback is a CPU overlay: without this loop a grooved beam
        // is visible yet never highlights, so it reads as if it were not there.
        for (occurrence, copy_transform) in interaction_projection_cache
            .as_ref()
            .expect("interaction cache was built")
            .canonical
            .occurrences()
            .iter()
            .filter(|occurrence| {
                occurrence.visible
                    && interaction_projection_cache
                        .as_ref()
                        .expect("interaction cache was built")
                        .mesh
                        .contains_occurrence(&occurrence.instance_path)
            })
            .flat_map(|occurrence| {
                std::iter::once((occurrence, None)).chain(
                    rotate_copies
                        .get(&occurrence.instance_path)
                        .copied()
                        .map(|transform| (occurrence, Some(transform))),
                )
            })
        {
            let occurrence_color = occurrence_colors.get(&occurrence.instance_path).copied();
            let out_of_context = !active_context_paths.contains(&occurrence.instance_path);
            let needs_cpu_overlay =
                !camera_dragging
                    && (self.selection.contains(&occurrence.instance_path)
                        || self.hover.target.as_ref().is_some_and(|hovered| {
                            hovered.instance_path == occurrence.instance_path
                        })
                        || out_of_context);
            let needs_cpu_fill = !use_wgpu_scene || copy_transform.is_some();
            if use_wgpu_scene && !needs_cpu_overlay && !needs_cpu_fill {
                continue;
            }
            let Some(mesh) = definition_mesh_body(snapshot, occurrence.body.definition_id) else {
                continue;
            };
            let transform = copy_transform
                .or_else(|| {
                    move_transform_overrides
                        .get(&occurrence.instance_path)
                        .copied()
                })
                .unwrap_or(occurrence.canonical_world_transform);
            let points_mm = mesh
                .vertices_mm
                .iter()
                .map(|point| {
                    transform_model_point(transform, Vec3::new(point[0], point[1], point[2]))
                })
                .collect::<Vec<_>>();
            let overlay_edges = self.overlay_feature_edges(
                occurrence.body.definition_id,
                &format!("mesh:{}", snapshot.canonical_digest()),
                || {
                    (
                        mesh.vertices_mm
                            .iter()
                            .map(|point| point.map(|value| value as f32))
                            .collect(),
                        mesh.triangles.clone(),
                        vec![None::<u8>; mesh.triangles.len()],
                    )
                },
            );
            for (edge, uses) in overlay_edges.iter() {
                let Some(edge_points_mm) =
                    self.section_edge(edge.map(|index| points_mm[index as usize]))
                else {
                    continue;
                };
                let projected = edge_points_mm.map(|point| self.project(point, rect));
                let element = uses.first().map(|index| {
                    face_element_from_normal(triangle_normal(
                        mesh.triangles[*index as usize].map(|index| points_mm[index as usize]),
                    ))
                });
                if let Some(element) = element {
                    edges.push(ProjectedEdge {
                        selection: SelectionId {
                            definition_id: occurrence.body.definition_id,
                            instance_path: occurrence.instance_path.clone(),
                            element,
                        },
                        points: projected,
                        depth: edge
                            .map(|index| point_depth(points_mm[index as usize], forward))
                            .into_iter()
                            .sum::<f64>()
                            / 2.0,
                        dominant_axis: dominant_edge_axis(edge_points_mm),
                    });
                }
            }
            if !needs_cpu_fill && !needs_cpu_overlay {
                continue;
            }
            for triangle in &mesh.triangles {
                let corners = triangle.map(|index| points_mm[index as usize]);
                let normal = triangle_normal(corners);
                let front = point_depth(normal, forward) < -ROUNDING;
                if !front && !self.paints_back_faces() {
                    continue;
                }
                let Some(polygon) = self.section_polygon(&corners, rect) else {
                    continue;
                };
                (if needs_cpu_fill {
                    &mut *faces
                } else {
                    &mut *feedback_faces
                })
                .push(ProjectedFace {
                    selection: SelectionId {
                        definition_id: occurrence.body.definition_id,
                        instance_path: occurrence.instance_path.clone(),
                        element: face_element_from_normal(normal),
                    },
                    polygon,
                    color: self.section_face_color(
                        front,
                        occurrence_color.unwrap_or_else(|| face_color_from_normal(normal)),
                    ),
                    depth: corners
                        .into_iter()
                        .map(|point| point_depth(point, forward))
                        .sum::<f64>()
                        / 3.0,
                    previewed: false,
                    out_of_context,
                });
            }
        }
    }

    /// Paint the projected scene and every overlay on top of it.
    fn paint_viewport_scene(
        &mut self,
        ui: &mut egui::Ui,
        response: &egui::Response,
        painter: &egui::Painter,
        scene: ProjectedViewportScene,
    ) {
        let ProjectedViewportScene {
            scene_plan,
            viewport_boxes,
            faces,
            feedback_faces,
            edges,
            hidden_ghost_corners,
        } = scene;
        self.record_live_image_scene(
            ui,
            response.rect,
            &viewport_boxes,
            &faces,
            &edges,
            scene_plan.clone(),
        );
        self.paint_projected_shadows(painter, response.rect, &viewport_boxes);
        self.paint_scene_base_layers(painter, response.rect, scene_plan);

        self.paint_projected_faces(painter, &faces);

        self.paint_projected_edges(painter, &edges);
        self.paint_viewport_fog(painter, response.rect);

        let hidden_stroke = Stroke::new(1.25_f32, Color32::from_rgb(232, 158, 72));
        for corners in hidden_ghost_corners {
            let projected = corners.map(|point| self.project(point, response.rect));
            for [from, to] in [
                [0, 1],
                [0, 2],
                [1, 3],
                [2, 3],
                [4, 5],
                [4, 6],
                [5, 7],
                [6, 7],
                [0, 4],
                [1, 5],
                [2, 6],
                [3, 7],
            ] {
                paint_dashed_segment(painter, [projected[from], projected[to]], hidden_stroke);
            }
        }

        if let Some(&ZoomWindowDrag { start, cursor }) = self.gesture.drag.get::<ZoomWindowDrag>() {
            let window = Rect::from_two_pos(start, cursor).intersect(response.rect);
            painter.rect_filled(
                window,
                0.0,
                Color32::from_rgba_unmultiplied(65, 147, 214, 36),
            );
            painter.rect_stroke(
                window,
                0.0,
                Stroke::new(1.5_f32, Color32::from_rgb(94, 183, 235)),
                egui::StrokeKind::Inside,
            );
        }
        if let Some(&drag) = self
            .gesture
            .drag
            .get::<SelectionWindowDrag>()
            .filter(|drag| drag.start.distance(drag.cursor) >= 4.0)
        {
            let window = Rect::from_two_pos(drag.start, drag.cursor).intersect(response.rect);
            let crossing = drag.cursor.x < drag.start.x;
            let color = if crossing {
                Color32::from_rgb(90, 205, 125)
            } else {
                Color32::from_rgb(94, 183, 235)
            };
            painter.rect_filled(
                window,
                0.0,
                if crossing {
                    Color32::from_rgba_unmultiplied(70, 180, 105, 38)
                } else {
                    Color32::from_rgba_unmultiplied(65, 147, 214, 36)
                },
            );
            let stroke = Stroke::new(1.5_f32, color);
            if crossing {
                for points in [
                    [window.left_top(), window.right_top()],
                    [window.right_top(), window.right_bottom()],
                    [window.right_bottom(), window.left_bottom()],
                    [window.left_bottom(), window.left_top()],
                ] {
                    paint_dashed_segment(painter, points, stroke);
                }
            } else {
                painter.rect_stroke(window, 0.0, stroke, egui::StrokeKind::Inside);
            }
        }

        self.paint_face_feedback(painter, faces.iter().chain(&feedback_faces));
        self.paint_projected_selection(painter, &edges);
        self.paint_selected_topological_edges(painter, response.rect);

        let profile_move_stroke = Stroke::new(2.4_f32, Color32::from_rgb(255, 199, 68));
        for path in self.move_profile_preview_paths() {
            if path.len() >= 2 {
                painter.add(egui::Shape::line(
                    path.into_iter()
                        .map(|point| self.project(point, response.rect))
                        .collect(),
                    profile_move_stroke,
                ));
            }
        }

        if let (Some(start), Some(cursor)) = (self.gesture.sketch.start, self.gesture.sketch.cursor)
        {
            if self.active_tool == ActiveTool::Line {
                let from = self.project(start, response.rect);
                let to = self.project(cursor, response.rect);
                let color = match self.gesture.sketch.axis_lock {
                    Some(Axis::X) => Color32::from_rgb(230, 80, 80),
                    Some(Axis::Y) => Color32::from_rgb(80, 205, 120),
                    Some(Axis::Z) => Color32::from_rgb(80, 145, 255),
                    None => Color32::from_rgb(255, 199, 68),
                };
                painter.line_segment([from, to], Stroke::new(2.5_f32, color));
                let axis_label = self
                    .gesture
                    .sketch
                    .axis_lock
                    .map(|axis| format!("{} · ", self.catalog.text(axis_name_key(axis))))
                    .unwrap_or_default();
                painter.text(
                    from.lerp(to, 0.5) - Vec2::new(0.0, 14.0),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{axis_label}{} mm",
                        format_height(length(Vec3::new(
                            cursor.x - start.x,
                            cursor.y - start.y,
                            cursor.z - start.z,
                        )))
                    ),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            } else if self.active_tool == ActiveTool::Arc {
                if let Some(end) = self.gesture.sketch.end
                    && let Some(arc) = self.drawing_arc(start, end, cursor)
                {
                    let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                    let points = arc_polyline(arc, PREVIEW_CURVE_SEGMENTS)
                        .into_iter()
                        .map(|point| {
                            self.project(self.drawing_world_delta(start, point), response.rect)
                        })
                        .collect();
                    painter.add(egui::Shape::line(points, stroke));
                    painter.line_segment(
                        [
                            self.project(start, response.rect),
                            self.project(end, response.rect),
                        ],
                        Stroke::new(1.0_f32, Color32::from_rgb(160, 160, 170)),
                    );
                    painter.text(
                        self.project(cursor, response.rect),
                        egui::Align2::CENTER_CENTER,
                        format!(
                            "B {} mm",
                            format_height(self.drawing_bulge(start, end, cursor).abs())
                        ),
                        egui::FontId::proportional(14.0),
                        Color32::WHITE,
                    );
                }
            } else if let Some(outline) = self.closed_shape_preview_outline() {
                let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                let mut points: Vec<Pos2> = outline
                    .iter()
                    .map(|point| self.project(*point, response.rect))
                    .collect();
                points.extend(points.first().copied());
                painter.add(egui::Shape::line(points, stroke));
                if let Some(end) = self.gesture.sketch.end {
                    painter.line_segment(
                        [
                            self.project(start, response.rect),
                            self.project(end, response.rect),
                        ],
                        Stroke::new(1.0_f32, Color32::from_rgb(160, 160, 170)),
                    );
                }
                painter.text(
                    self.project(start, response.rect),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{} {} mm",
                        self.catalog.text(self.value_label_key()),
                        self.value_box.input
                    ),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            } else {
                let ground = if self.uses_drawing_plane() {
                    self.drawing_rectangle_corners(start, cursor)
                } else {
                    [
                        start,
                        Vec3::new(cursor.x, start.y, start.z),
                        cursor,
                        Vec3::new(start.x, cursor.y, start.z),
                    ]
                };
                let points = ground.map(|point| self.project(point, response.rect));
                let stroke = Stroke::new(2.0_f32, Color32::from_rgb(255, 199, 68));
                for edge in 0..points.len() {
                    painter.line_segment([points[edge], points[(edge + 1) % points.len()]], stroke);
                }
                painter.text(
                    Pos2::new(
                        points.iter().map(|point| point.x).sum::<f32>() / 4.0,
                        points.iter().map(|point| point.y).sum::<f32>() / 4.0,
                    ),
                    egui::Align2::CENTER_CENTER,
                    format!(
                        "{} × {} mm",
                        format_height(if self.uses_drawing_plane() {
                            self.drawing_local_delta(start, cursor).x.abs()
                        } else {
                            (cursor.x - start.x).abs()
                        }),
                        format_height(if self.uses_drawing_plane() {
                            self.drawing_local_delta(start, cursor).y.abs()
                        } else {
                            (cursor.y - start.y).abs()
                        })
                    ),
                    egui::FontId::proportional(14.0),
                    Color32::WHITE,
                );
            }
        }

        let helix_preview = self.helix_preview_points();
        if helix_preview.len() >= 2 {
            painter.add(egui::Shape::line(
                helix_preview
                    .into_iter()
                    .map(|point| self.project(point, response.rect))
                    .collect(),
                Stroke::new(2.4_f32, Color32::from_rgb(255, 199, 68)),
            ));
        }

        self.paint_origin_snap(painter, response.hover_pos(), response.rect);
        self.paint_rotation_guide(painter, response.rect);
        self.paint_mirror_preview(painter, response.rect);

        if let Some((start, end)) = self.measure_span() {
            let from = self.project(start, response.rect);
            let to = self.project(end, response.rect);
            let stroke = Stroke::new(2.0_f32, Color32::from_rgb(120, 205, 255));
            painter.line_segment([from, to], stroke);
            painter.circle_filled(from, 3.5, stroke.color);
            painter.circle_filled(to, 3.5, stroke.color);
            painter.text(
                from.lerp(to, 0.5) - Vec2::new(0.0, 14.0),
                egui::Align2::CENTER_CENTER,
                format!(
                    "{} mm",
                    format_height(length(Vec3::new(
                        end.x - start.x,
                        end.y - start.y,
                        end.z - start.z,
                    )))
                ),
                egui::FontId::proportional(14.0),
                Color32::WHITE,
            );
        }
        let measure_vertex = (self.active_tool == ActiveTool::Measure)
            .then(|| {
                ui.input(|input| input.pointer.hover_pos())
                    .and_then(|pointer| {
                        self.scene_snap_at_screen(pointer, response.rect, 8.0, None)
                            .map(|snap| snap.position_mm)
                    })
            })
            .flatten();
        if let Some(position) = measure_vertex.or_else(|| {
            self.hover
                .snap
                .as_ref()
                .filter(|snap| snap.kind != SnapKind::Face)
                .map(|snap| snap.position_mm)
        }) {
            let centre = self.project(position, response.rect);
            let stroke = Stroke::new(1.5_f32, Color32::from_rgb(80, 206, 190));
            painter.circle_stroke(centre, 6.0, stroke);
            painter.line_segment(
                [centre - Vec2::splat(4.0), centre + Vec2::splat(4.0)],
                stroke,
            );
            painter.line_segment(
                [centre + Vec2::new(-4.0, 4.0), centre + Vec2::new(4.0, -4.0)],
                stroke,
            );
        }
        if response.secondary_clicked()
            && let Some(target) = self.hover.target.clone()
            && !self.selection.contains(&target.instance_path)
        {
            self.select_from_viewport(Some(target), false);
        }
        response.context_menu(|ui| self.show_viewport_context_menu(ui));
        self.paint_face_offset_guide(painter, response.rect);
        self.viewport_overlays(ui, response.rect);
    }

    /// The viewport's right-click menu.
    ///
    /// Ordered the way SketchUp orders it: what the click landed on, then the
    /// edits that apply to it, then the commands that only move the camera.
    /// A right-click first selects whatever is under the pointer, so the menu
    /// always acts on the thing the user pointed at.
    pub(crate) fn show_viewport_context_menu(&mut self, ui: &mut egui::Ui) {
        if self.selection_count() > 0 {
            ui.label(
                egui::RichText::new(self.catalog.format(
                    "status-selected",
                    &BTreeMap::from([("count", self.selection_count().to_string())]),
                ))
                .color(self.palette().faint)
                .size(SHELL_SMALL_SIZE),
            );
            ui.separator();
            if ui.button(self.catalog.text("context-edit")).clicked() {
                if let Some(group_id) = self.selected_group_id() {
                    self.enter_group_context(group_id);
                } else if let Some(target) = self.selection.primary.clone() {
                    self.enter_occurrence_context(target.instance_path);
                }
                ui.close();
            }
            self.menu_command(ui, AppCommand::Delete);
            self.menu_command(ui, AppCommand::Hide);
            self.menu_command(ui, AppCommand::HideOthers);
            ui.separator();
            self.menu_command(ui, AppCommand::Fillet);
            self.menu_command(ui, AppCommand::Chamfer);
            ui.separator();
            self.menu_command(ui, AppCommand::Group);
            self.menu_command(ui, AppCommand::Ungroup);
            self.menu_command(ui, AppCommand::MakeComponent);
            self.menu_command(ui, AppCommand::MakeUnique);
            self.menu_command(ui, AppCommand::ReplaceComponent);
            self.menu_command(ui, AppCommand::SelectAllInstances);
            self.menu_command(ui, AppCommand::AssignTag);
            self.menu_command(ui, AppCommand::AlignOccurrences);
            self.menu_command(ui, AppCommand::DistributeOccurrences);
            self.menu_command(ui, AppCommand::LinearPattern);
            self.menu_command(ui, AppCommand::RectangularPattern);
            self.menu_command(ui, AppCommand::CircularPattern);
            self.menu_command(ui, AppCommand::GroundOccurrence);
            self.menu_command(ui, AppCommand::UngroundOccurrence);
            ui.separator();
            self.menu_command(ui, AppCommand::Copy);
            self.menu_command(ui, AppCommand::Cut);
            self.menu_command(ui, AppCommand::Duplicate);
        }
        self.menu_command(ui, AppCommand::Paste);
        self.menu_command(ui, AppCommand::SelectAll);
        self.menu_command(ui, AppCommand::InvertSelection);
        self.menu_command(ui, AppCommand::Deselect);
        self.menu_command(ui, AppCommand::Unhide);
        self.menu_command(ui, AppCommand::UnhideAll);
        if self.edit_context_depth() > 0 {
            ui.separator();
            if ui
                .button(self.catalog.text("context-close-context"))
                .clicked()
            {
                self.exit_edit_context();
                ui.close();
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            self.menu_command(ui, AppCommand::View(ViewFlag::GridAxes));
            self.menu_command(ui, AppCommand::View(ViewFlag::WhiteBackground));
            self.menu_command(ui, AppCommand::View(ViewFlag::Shadows));
            self.menu_command(ui, AppCommand::View(ViewFlag::Fog));
        });
        ui.horizontal(|ui| {
            self.menu_command(ui, AppCommand::View(ViewFlag::HiddenObjects));
            self.menu_command(ui, AppCommand::View(ViewFlag::Xray));
            self.menu_command(ui, AppCommand::ViewShaded);
            self.menu_command(ui, AppCommand::View(ViewFlag::Wireframe));
            self.menu_command(ui, AppCommand::View(ViewFlag::Monochrome));
            self.menu_command(ui, AppCommand::View(ViewFlag::HiddenLine));
            self.menu_command(ui, AppCommand::View(ViewFlag::Edges));
            self.menu_command(ui, AppCommand::View(ViewFlag::Profiles));
            self.menu_command(ui, AppCommand::View(ViewFlag::DepthCue));
        });
        ui.horizontal(|ui| {
            self.menu_command(ui, AppCommand::View(ViewFlag::FadeDistantEdges));
            self.menu_command(ui, AppCommand::View(ViewFlag::HighContrastEdges));
            self.menu_command(ui, AppCommand::View(ViewFlag::SelectionHalo));
        });
        ui.horizontal(|ui| {
            self.menu_command(ui, AppCommand::View(ViewFlag::Endpoints));
            self.menu_command(ui, AppCommand::View(ViewFlag::Midpoints));
            self.menu_command(ui, AppCommand::View(ViewFlag::Extensions));
            self.menu_command(ui, AppCommand::View(ViewFlag::Jitter));
            self.menu_command(ui, AppCommand::View(ViewFlag::Dashes));
            self.menu_command(ui, AppCommand::View(ViewFlag::ColorByAxis));
            self.menu_command(ui, AppCommand::View(ViewFlag::Halos));
            self.menu_command(ui, AppCommand::PreviousView);
            self.menu_command(ui, AppCommand::HomeView);
        });
        self.menu_command(ui, AppCommand::ZoomFit);
        self.menu_command(ui, AppCommand::ZoomSelection);
        self.menu_command(ui, AppCommand::CenterSelection);
        self.menu_command(ui, AppCommand::ZoomWindow);
        self.menu_command(ui, AppCommand::ZoomIn);
        self.menu_command(ui, AppCommand::ZoomOut);
        self.menu_command(ui, AppCommand::ViewProjection);
    }

    pub(crate) fn pick_result_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        tolerance_px: f64,
    ) -> Option<PickResult> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        self.refresh_interaction_projection_cache(&snapshot);
        let cache = self.hover.projection_cache.borrow();
        let cache = cache.as_ref().expect("interaction cache was built");
        let scale =
            f64::from(self.camera.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
        let mut exact_hits = cache.exact.exact_surface_picks(ray);
        exact_hits.retain(|hit| self.section_keeps(hit.position_mm));
        let exact_hit = exact_hits.first().cloned();
        let mut mesh_hits = cache.mesh.exact_surface_picks(ray);
        mesh_hits.retain(|hit| self.section_keeps(hit.position_mm));
        let mut mesh_hit = mesh_hits.first().cloned().or_else(|| {
            cache
                .mesh
                .surface_pick_with_tolerance(ray, tolerance_px / scale)
                .filter(|hit| self.section_keeps(hit.position_mm))
        });
        let box_pick = cache
            .boxes
            .exact_pick(ray, tolerance_px / scale)
            .and_then(|pick| self.section_kept_pick(pick));
        // A profile drawn onto a solid's face lies in that face's plane, so the ray
        // reaches both at the same distance. The profile is what the user sees on
        // top and wants to push/pull; the face stays reachable where no profile is.
        let nearest_mm = [
            exact_hit.as_ref().map(|hit| hit.ray_distance_mm),
            mesh_hit.as_ref().map(|hit| hit.ray_distance_mm),
            box_pick.as_ref().map(|pick| pick.primary.ray_distance_mm),
        ]
        .into_iter()
        .flatten()
        .fold(f64::INFINITY, f64::min);
        let coplanar_mm = DEFAULT_LINEAR_TOLERANCE_MM * nearest_mm.abs().max(1.0);
        let is_flat_profile = |definition_id: DefinitionId| {
            snapshot
                .definition(definition_id)
                .is_some_and(|definition| {
                    definition.feature_ids().iter().all(|feature_id| {
                        snapshot
                            .feature(*feature_id)
                            .is_some_and(|feature| !feature.kind().produces_body())
                    })
                })
        };
        let flat_mesh = mesh_hits
            .iter()
            .find(|hit| {
                hit.ray_distance_mm <= nearest_mm + coplanar_mm
                    && is_flat_profile(hit.definition_id)
            })
            .cloned();
        let flat_box_hit = box_pick
            .as_ref()
            .filter(|_| flat_mesh.is_none())
            .and_then(|pick| {
                pick.overlapping
                    .iter()
                    .find(|hit| {
                        hit.ray_distance_mm <= nearest_mm + coplanar_mm
                            && is_flat_profile(hit.reference.definition_id)
                    })
                    .cloned()
            });
        let flat_box = flat_box_hit.is_some();
        // The overlap stack is what hover and Tab cycle through; its front entry
        // must be the promoted profile, not whichever reference sorts first.
        let put_in_front = |overlapping: &mut Vec<ExactHit>, primary: &ExactHit| {
            if let Some(index) = overlapping
                .iter()
                .position(|hit| hit.reference == primary.reference)
            {
                let hit = overlapping.remove(index);
                overlapping.insert(0, hit);
            }
        };
        let flat_mesh_wins = flat_mesh.is_some();
        if flat_mesh_wins {
            mesh_hit = flat_mesh;
        }
        let prefer_exact = !flat_box
            && !flat_mesh_wins
            && exact_hit.as_ref().is_some_and(|exact| {
                mesh_hit
                    .as_ref()
                    .is_none_or(|mesh| exact.ray_distance_mm <= mesh.ray_distance_mm)
                    && box_pick
                        .as_ref()
                        .is_none_or(|pick| exact.ray_distance_mm <= pick.primary.ray_distance_mm)
            });
        if prefer_exact && let Some(hit) = exact_hit {
            let element = self.exact_hit_element(&hit)?;
            let reference = SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element,
            };
            let primary = ExactHit {
                reference: reference.clone(),
                position_mm: hit.position_mm,
                ray_distance_mm: hit.ray_distance_mm,
            };
            let scale =
                f64::from(self.camera.zoom) * f64::from(rect.width().min(rect.height())) / 420.0;
            let proxy_pick = cache
                .proxies
                .exact_pick(ray, tolerance_px / scale)
                .and_then(|pick| self.section_kept_pick(pick));
            let snap = self
                .scene_snap_at_screen(pointer, rect, tolerance_px as f32, None)
                .filter(|snap| snap.reference.instance_path == primary.reference.instance_path)
                .unwrap_or(SnapResult {
                    kind: SnapKind::Face,
                    reference,
                    position_mm: hit.position_mm,
                    distance_mm: 0.0,
                });
            let mut overlapping = vec![primary.clone()];
            overlapping.extend(exact_hits.into_iter().skip(1).filter_map(|hit| {
                let element = self.exact_hit_element(&hit)?;
                Some(ExactHit {
                    reference: SelectionId {
                        definition_id: hit.definition_id,
                        instance_path: hit.instance_path,
                        element,
                    },
                    position_mm: hit.position_mm,
                    ray_distance_mm: hit.ray_distance_mm,
                })
            }));
            if let Some(proxy_pick) = proxy_pick {
                overlapping.extend(proxy_pick.overlapping.into_iter().filter(|candidate| {
                    candidate.reference.instance_path != primary.reference.instance_path
                }));
            }
            if let Some(mesh) = mesh_hit.as_ref().filter(|mesh| {
                mesh.instance_path != primary.reference.instance_path
                    && mesh.ray_distance_mm <= primary.ray_distance_mm + APPROXIMATION
            }) {
                overlapping.push(ExactHit {
                    reference: SelectionId {
                        definition_id: mesh.definition_id,
                        instance_path: mesh.instance_path.clone(),
                        element: face_element_from_normal(mesh.outward_normal),
                    },
                    position_mm: mesh.position_mm,
                    ray_distance_mm: mesh.ray_distance_mm,
                });
            }
            overlapping.sort_by(|left, right| {
                left.ray_distance_mm
                    .total_cmp(&right.ray_distance_mm)
                    .then_with(|| left.reference.cmp(&right.reference))
            });
            overlapping.dedup_by(|left, right| left.reference == right.reference);
            return Some(PickResult {
                primary,
                overlapping,
                snap,
            });
        }
        let prefer_mesh = !flat_box
            && (flat_mesh_wins
                || mesh_hit.as_ref().is_some_and(|mesh| {
                    box_pick.as_ref().is_none_or(|pick| {
                        mesh.ray_distance_mm <= pick.primary.ray_distance_mm + APPROXIMATION
                    })
                }));
        if prefer_mesh && let Some(hit) = mesh_hit {
            let reference = SelectionId {
                definition_id: hit.definition_id,
                instance_path: hit.instance_path,
                element: face_element_from_normal(hit.outward_normal),
            };
            let primary = ExactHit {
                reference: reference.clone(),
                position_mm: hit.position_mm,
                ray_distance_mm: hit.ray_distance_mm,
            };
            let mut overlapping = vec![primary.clone()];
            if let Some(pick) = box_pick {
                overlapping.extend(pick.overlapping);
            }
            overlapping.extend(
                mesh_hits
                    .into_iter()
                    .filter(|candidate| candidate.instance_path != primary.reference.instance_path)
                    .map(|candidate| ExactHit {
                        reference: SelectionId {
                            definition_id: candidate.definition_id,
                            instance_path: candidate.instance_path,
                            element: face_element_from_normal(candidate.outward_normal),
                        },
                        position_mm: candidate.position_mm,
                        ray_distance_mm: candidate.ray_distance_mm,
                    }),
            );
            overlapping.extend(exact_hits.into_iter().map(|candidate| {
                let element = self
                    .exact_hit_element(&candidate)
                    .unwrap_or_else(|| face_element_from_normal(candidate.outward_normal));
                ExactHit {
                    reference: SelectionId {
                        definition_id: candidate.definition_id,
                        instance_path: candidate.instance_path,
                        element,
                    },
                    position_mm: candidate.position_mm,
                    ray_distance_mm: candidate.ray_distance_mm,
                }
            }));
            overlapping.sort_by(|a, b| {
                a.ray_distance_mm
                    .total_cmp(&b.ray_distance_mm)
                    .then_with(|| a.reference.cmp(&b.reference))
            });
            overlapping.dedup_by(|a, b| a.reference == b.reference);
            if flat_mesh_wins {
                put_in_front(&mut overlapping, &primary);
            }
            return Some(PickResult {
                primary,
                overlapping,
                snap: SnapResult {
                    kind: SnapKind::Face,
                    reference,
                    position_mm: hit.position_mm,
                    distance_mm: 0.0,
                },
            });
        }

        box_pick.map(|mut pick| {
            if let Some(hit) = flat_box_hit {
                if pick.snap.reference.instance_path != hit.reference.instance_path {
                    pick.snap = SnapResult {
                        kind: SnapKind::Face,
                        reference: hit.reference.clone(),
                        position_mm: hit.position_mm,
                        distance_mm: 0.0,
                    };
                }
                pick.primary = hit;
            }
            pick.overlapping
                .extend(mesh_hits.into_iter().map(|candidate| ExactHit {
                    reference: SelectionId {
                        definition_id: candidate.definition_id,
                        instance_path: candidate.instance_path,
                        element: face_element_from_normal(candidate.outward_normal),
                    },
                    position_mm: candidate.position_mm,
                    ray_distance_mm: candidate.ray_distance_mm,
                }));
            pick.overlapping
                .extend(exact_hits.into_iter().map(|candidate| {
                    ExactHit {
                        reference: SelectionId {
                            definition_id: candidate.definition_id,
                            instance_path: candidate.instance_path,
                            element: candidate
                                .durable_target
                                .as_ref()
                                .and_then(|target| target.body.role())
                                .and_then(exact_face_element)
                                .unwrap_or_else(|| {
                                    face_element_from_normal(candidate.outward_normal)
                                }),
                        },
                        position_mm: candidate.position_mm,
                        ray_distance_mm: candidate.ray_distance_mm,
                    }
                }));
            pick.overlapping.sort_by(|a, b| {
                a.ray_distance_mm
                    .total_cmp(&b.ray_distance_mm)
                    .then_with(|| a.reference.cmp(&b.reference))
            });
            pick.overlapping.dedup_by(|a, b| a.reference == b.reference);
            if flat_box {
                put_in_front(&mut pick.overlapping, &pick.primary);
            }
            pick
        })
    }

    pub(crate) fn topological_selection_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        selection: &SelectionId,
    ) -> Option<SnapshotBoundTopologicalSelection> {
        let ray = self.view_ray(pointer, rect)?;
        let snapshot = self.document.current();
        let topology_results = self.topology_results_for_snapshot(&snapshot)?;
        let projection = ExactInteractionProjection::from_snapshot(&snapshot, topology_results);
        if let ElementId::TopologicalEdge {
            feature_id,
            ordinal,
        } = selection.element
        {
            return projection
                .topological_pick_current(
                    &snapshot,
                    topology_results,
                    &TopologicalPickLocator {
                        instance_path: selection.instance_path.clone(),
                        producer_feature_id: feature_id,
                        kind: TopologicalElementKind::Edge,
                        ordinal,
                    },
                )
                .ok();
        }
        let hit = projection
            .exact_surface_picks(ray)
            .into_iter()
            .find(|hit| {
                let element = self.exact_hit_element(hit);
                hit.definition_id == selection.definition_id
                    && hit.instance_path == selection.instance_path
                    && (!matches!(
                        selection.element,
                        ElementId::Face { .. } | ElementId::TopologicalFace(_)
                    ) || element.as_ref() == Some(&selection.element))
            })?;
        match selection.element {
            ElementId::Face { .. } | ElementId::TopologicalFace(_) => hit.topological_target,
            ElementId::Edge(ordinal) | ElementId::EdgeMidpoint(ordinal) => {
                let producer_feature_id = topology_results
                    .get_render(&snapshot, selection.definition_id)?
                    .producer_feature_id();
                projection
                    .topological_pick_current(
                        &snapshot,
                        topology_results,
                        &TopologicalPickLocator {
                            instance_path: selection.instance_path.clone(),
                            producer_feature_id,
                            kind: TopologicalElementKind::Edge,
                            ordinal: u32::from(ordinal),
                        },
                    )
                    .ok()
            }
            _ => None,
        }
    }

    pub(crate) fn exact_pick_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SelectionId> {
        self.pick_result_at_screen(pointer, rect, 8.0)
            .map(|result| result.primary.reference)
    }

    #[cfg(test)]
    pub(crate) fn interaction_projection_cache_ptrs(
        &self,
    ) -> Option<(*const (), *const (), *const ())> {
        let cache = self.hover.projection_cache.borrow();
        let cache = cache.as_ref()?;
        Some((
            std::ptr::from_ref(&cache.exact).cast(),
            std::ptr::from_ref(&cache.mesh).cast(),
            std::ptr::from_ref(&cache.boxes).cast(),
        ))
    }

    pub(crate) fn update_viewport_inference(&mut self, pointer: Option<Pos2>, rect: Rect) {
        let mut pick = pointer.and_then(|pointer| self.pick_result_at_screen(pointer, rect, 12.0));
        if let Some(pick) = pick.as_mut() {
            self.prioritize_push_pull_profile_pick(pick);
        }
        let previous = self.hover.pick.as_ref().map(overlap_signature);
        let current = pick.as_ref().map(overlap_signature);
        let pointer_moved = self.hover.pointer != pointer;
        self.hover.pointer = pointer;
        if pointer_moved {
            // A deliberate non-front choice stays attached to that face while
            // the moving pointer still intersects it; leaving it resets to front.
            // A choice made with Alt lasts only while Alt is held.
            self.hover.overlap_index = if self.hover.overlap_index == 0
                || (self.hover.alt_choice && !self.face_workflow.alt_pick_through_held())
            {
                0
            } else {
                self.hover
                    .target
                    .as_ref()
                    .and_then(|chosen| {
                        current
                            .as_ref()?
                            .iter()
                            .position(|candidate| candidate == chosen)
                    })
                    .unwrap_or(0)
            };
        } else if previous != current {
            // A background recompute can republish the same stack of bodies
            // under an unmoved pointer. Resetting blindly would throw away the
            // choice the user just cycled to with Tab and select whatever is
            // frontmost instead, so the choice is carried over whenever the
            // chosen body is still under the pointer. Without such a choice
            // the front body stays hovered: a profile just drawn on a face is
            // in front of that face.
            self.hover.overlap_index = if self.hover.overlap_index == 0 {
                0
            } else {
                self.hover
                    .target
                    .as_ref()
                    .and_then(|chosen| {
                        current
                            .as_ref()?
                            .iter()
                            .position(|candidate| candidate == chosen)
                    })
                    .unwrap_or(0)
            };
            if current.is_none() {
                self.face_workflow.set_xray_preview(false);
            }
        }
        if self.hover.overlap_index == 0 {
            self.hover.alt_choice = false;
        }
        self.hover.snap = pointer.and_then(|pointer| {
            if self.uses_drawing_plane() {
                self.drawing_snap_at_screen(pointer, rect)
            } else {
                self.scene_snap_at_screen(pointer, rect, 8.0, None)
                    .or_else(|| {
                        self.face_workflow
                            .snaps_enabled()
                            .then(|| {
                                pick.as_ref().map(|p| SnapResult {
                                    kind: SnapKind::Face,
                                    reference: p.primary.reference.clone(),
                                    position_mm: p.primary.position_mm,
                                    distance_mm: 0.0,
                                })
                            })
                            .flatten()
                    })
            }
        });
        self.hover.pick = pick;
        self.refresh_hover_choice();
    }

    pub(crate) fn refresh_hover_choice(&mut self) {
        self.hover.target = self
            .hover
            .pick
            .as_ref()
            .and_then(|pick| pick.overlap_choice(self.hover.overlap_index))
            .map(|hit| hit.reference.clone());
        if self.active_tool == ActiveTool::Select
            && self.face_workflow.snaps_enabled()
            && let (Some(pointer), Some(rect)) = (self.hover.pointer, self.camera.viewport_rect)
            && let Some(center) = self.select_circle_center_at_screen(pointer, rect)
        {
            self.hover.snap = Some(center);
        }
        if self.active_tool == ActiveTool::PushPull {
            self.hover.snap = self.push_pull_target_snap();
            if let Some(drag) = self.gesture.drag.get::<PushPullDrag>().or(self
                .gesture
                .drag
                .get::<PushPullAnchor>()
                .map(|anchor| &anchor.0))
                && self.push_pull_snap_distance(drag).is_none()
            {
                self.hover.snap = None;
            }
        }
    }

    pub fn cycle_hover_overlap(&mut self) -> bool {
        let Some(count) = self
            .hover
            .pick
            .as_ref()
            .map(|pick| pick.overlapping.len())
            .filter(|count| *count > 1)
        else {
            return false;
        };
        self.hover.overlap_index = (self.hover.overlap_index + 1) % count;
        self.refresh_hover_choice();
        self.digest = self.catalog.format(
            "digest-overlap-choice",
            &BTreeMap::from([
                ("index", (self.hover.overlap_index + 1).to_string()),
                ("count", count.to_string()),
            ]),
        );
        if let Some(drag) = self
            .gesture
            .drag
            .get::<PushPullDrag>()
            .cloned()
            .or_else(|| {
                self.gesture
                    .drag
                    .get::<PushPullAnchor>()
                    .map(|anchor| anchor.0.clone())
            })
            && self.push_pull_snap_distance(&drag).is_some()
        {
            self.update_push_pull_gesture(&drag, drag.pointer_start);
        }
        true
    }

    pub(crate) fn viewport_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        if self.uses_drawing_plane() {
            return self.drawing_input_point(pointer, rect);
        }
        if !self.face_workflow.snaps_enabled()
            || self.origin_snap_at_screen(pointer, rect, plane_z).is_some()
        {
            return self
                .origin_snap_at_screen(pointer, rect, plane_z)
                .or_else(|| self.screen_to_plane(pointer, rect, plane_z));
        }
        self.scene_snap_at_screen(pointer, rect, 8.0, None)
            .map(|snap| snap.position_mm)
            .or_else(|| {
                self.datum_snap_at_screen(
                    pointer,
                    rect,
                    if self.active_tool == ActiveTool::Line {
                        None
                    } else {
                        Some(WorkplaneFrame {
                            origin_mm: [0.0, 0.0, plane_z],
                            x_axis: [1.0, 0.0, 0.0],
                            y_axis: [0.0, 1.0, 0.0],
                            normal: [0.0, 0.0, 1.0],
                        })
                    },
                )
                .map(|(point, _)| point)
            })
            .or_else(|| self.screen_to_plane(pointer, rect, plane_z))
    }

    pub fn measurement_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        self.scene_snap_at_screen(pointer, rect, 8.0, None)
            .map(|snap| snap.position_mm)
            .or_else(|| self.viewport_point_at_screen(pointer, rect, plane_z))
            .or_else(|| self.surface_point_at_screen(pointer, rect))
    }

    pub(crate) fn world_to_clip(&self, rect: Rect) -> [f32; 16] {
        if self.camera.projection_mode == ProjectionMode::Perspective {
            return self.perspective_world_to_clip(rect);
        }
        let yaw_sin = self.camera.yaw.sin();
        let yaw_cos = self.camera.yaw.cos();
        let pitch_sin = self.camera.pitch.sin();
        let pitch_cos = self.camera.pitch.cos();
        let scale = self.camera.zoom * rect.width().min(rect.height()) / 420.0;
        let centre_x = BOX_WIDTH_MM as f32 * 0.5;
        let centre_y = BOX_DEPTH_MM as f32 * 0.5;
        let centre_z = self.camera.target_z as f32;
        let sx = 2.0 / rect.width();
        let sy = 2.0 / rect.height();
        let x_constant = rect.width() * 0.5 + self.camera.pan.x
            - scale * (yaw_cos * centre_x - yaw_sin * centre_y);
        let y_constant = rect.height() * 0.5
            + self.camera.pan.y
            + scale
                * (yaw_sin * pitch_cos * centre_x + yaw_cos * pitch_cos * centre_y
                    - pitch_sin * centre_z);
        [
            sx * scale * yaw_cos,
            sy * scale * yaw_sin * pitch_cos,
            0.0,
            0.0,
            -sx * scale * yaw_sin,
            sy * scale * yaw_cos * pitch_cos,
            0.0,
            0.0,
            0.0,
            -sy * scale * pitch_sin,
            0.0,
            0.0,
            sx * x_constant - 1.0,
            1.0 - sy * y_constant,
            0.0,
            1.0,
        ]
    }

    /// The converging counterpart of [`Self::world_to_clip`].
    ///
    /// The clip `w` carries the eye-space depth, so the fixed-function divide
    /// gives exactly the same picture the CPU painter draws in
    /// [`Self::project`]. There is no depth buffer, so clip `z` stays zero.
    pub(crate) fn perspective_world_to_clip(&self, rect: Rect) -> [f32; 16] {
        let (right, up, forward) = self.camera_basis();
        let eye = self.camera_target() - forward * self.camera_distance();
        let focal = self.camera_focal(rect);
        let sx = f64::from(2.0 / rect.width());
        let sy = f64::from(2.0 / rect.height());
        let a = sx * f64::from(rect.width() * 0.5 + self.camera.pan.x) - 1.0;
        let b = 1.0 - sy * f64::from(rect.height() * 0.5 + self.camera.pan.y);

        let clip_x = forward * a + right * (sx * focal);
        let clip_y = forward * b + up * (sy * focal);
        let clip_w = forward;
        [
            clip_x.x as f32,
            clip_y.x as f32,
            0.0,
            clip_w.x as f32,
            clip_x.y as f32,
            clip_y.y as f32,
            0.0,
            clip_w.y as f32,
            clip_x.z as f32,
            clip_y.z as f32,
            0.0,
            clip_w.z as f32,
            -dot(clip_x, eye) as f32,
            -dot(clip_y, eye) as f32,
            0.0,
            -dot(clip_w, eye) as f32,
        ]
    }

    pub(crate) fn project(&self, point: Vec3, rect: Rect) -> Pos2 {
        let (right, up, forward) = self.camera_basis();
        let centered = point - self.camera_target();
        let view_x = dot(centered, right);
        let view_y = dot(centered, up);
        let scale = match self.camera.projection_mode {
            ProjectionMode::Parallel => self.view_scale(rect),
            ProjectionMode::Perspective => {
                // Depth measured from the eye. Points at or behind the eye have
                // no on-screen position, so they are clamped to a sliver in
                // front of it rather than folded through the origin.
                let depth =
                    (self.camera_distance() + dot(centered, forward)).max(PERSPECTIVE_NEAR_MM);
                self.camera_focal(rect) / depth
            }
        };
        Pos2::new(
            rect.center().x + self.camera.pan.x + (view_x * scale) as f32,
            rect.center().y + self.camera.pan.y - (view_y * scale) as f32,
        )
    }

    /// Clip a long world-space line at the eye plane before projecting it.
    pub(crate) fn project_visible_segment(
        &self,
        mut points: [Vec3; 2],
        rect: Rect,
    ) -> Option<[Pos2; 2]> {
        if self.camera.projection_mode == ProjectionMode::Perspective {
            let (_, _, forward) = self.camera_basis();
            let target = self.camera_target();
            let mut depths =
                points.map(|point| self.camera_distance() + dot(point - target, forward));
            if depths.iter().all(|depth| *depth < PERSPECTIVE_NEAR_MM) {
                return None;
            }
            for index in 0..2 {
                if depths[index] < PERSPECTIVE_NEAR_MM {
                    let other = 1 - index;
                    let amount =
                        (PERSPECTIVE_NEAR_MM - depths[index]) / (depths[other] - depths[index]);
                    points[index] = points[index] + (points[other] - points[index]) * amount;
                    depths[index] = PERSPECTIVE_NEAR_MM;
                }
            }
        }
        Some(points.map(|point| self.project(point, rect)))
    }

    /// Screen bounds that contain the projection of the convex hull of
    /// `points`, or `None` when part of it lies at or behind the eye (where the
    /// projection is clamped and no longer convex).
    pub(crate) fn projected_hull_bounds(&self, points: &[Vec3], rect: Rect) -> Option<Rect> {
        if self.camera.projection_mode == ProjectionMode::Perspective {
            let (_, _, forward) = self.camera_basis();
            let target = self.camera_target();
            let distance = self.camera_distance();
            if points
                .iter()
                .any(|point| distance + dot(*point - target, forward) <= PERSPECTIVE_NEAR_MM)
            {
                return None;
            }
        }
        Some(Rect::from_points(
            &points
                .iter()
                .map(|point| self.project(*point, rect))
                .collect::<Vec<_>>(),
        ))
    }
}

/// Red X, green Y, blue Z — the convention every modeller shares, so it is
/// deliberately independent of the theme palette.
pub(crate) const fn axis_color(axis: Axis) -> Color32 {
    match axis {
        Axis::X => Color32::from_rgb(224, 86, 63),
        Axis::Y => Color32::from_rgb(93, 187, 99),
        Axis::Z => Color32::from_rgb(78, 134, 199),
    }
}

pub(crate) fn dominant_edge_axis(points: [Vec3; 2]) -> Option<Axis> {
    let delta = points[1] - points[0];
    let components = [delta.x.abs(), delta.y.abs(), delta.z.abs()];
    let maximum = components.into_iter().fold(0.0_f64, f64::max);
    if !maximum.is_finite() || maximum <= f64::EPSILON {
        return None;
    }
    if components[0] >= components[1] && components[0] >= components[2] {
        Some(Axis::X)
    } else if components[1] >= components[2] {
        Some(Axis::Y)
    } else {
        Some(Axis::Z)
    }
}

pub(crate) fn box_faces() -> [BoxFace; 6] {
    [
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Minimum,
            },
            corners: [0, 1, 3, 2],
            color: Color32::from_rgb(66, 74, 88),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Z,
                side: Side::Maximum,
            },
            corners: [4, 6, 7, 5],
            color: Color32::from_rgb(126, 145, 166),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Minimum,
            },
            corners: [0, 2, 6, 4],
            color: Color32::from_rgb(82, 96, 113),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::X,
                side: Side::Maximum,
            },
            corners: [1, 5, 7, 3],
            color: Color32::from_rgb(78, 91, 107),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Y,
                side: Side::Minimum,
            },
            corners: [0, 4, 5, 1],
            color: Color32::from_rgb(94, 108, 126),
        },
        BoxFace {
            element: ElementId::Face {
                axis: Axis::Y,
                side: Side::Maximum,
            },
            corners: [2, 3, 7, 6],
            color: Color32::from_rgb(88, 102, 119),
        },
    ]
}

pub(crate) fn exact_face_element(role: ExactFaceRole) -> Option<ElementId> {
    match role {
        ExactFaceRole::Top => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Maximum,
        }),
        ExactFaceRole::Bottom => Some(ElementId::Face {
            axis: Axis::Z,
            side: Side::Minimum,
        }),
        ExactFaceRole::LinearSide | ExactFaceRole::ArcSide | ExactFaceRole::CircleSide => None,
    }
}

fn overlap_signature(pick: &PickResult) -> Vec<SelectionId> {
    pick.overlapping
        .iter()
        .map(|hit| hit.reference.clone())
        .collect()
}

pub(crate) fn point_depth(point: Vec3, forward: Vec3) -> f64 {
    point.x * forward.x + point.y * forward.y + point.z * forward.z
}

fn face_color_from_normal(normal: Vec3) -> Color32 {
    let element = face_element_from_normal(normal);
    box_faces()
        .into_iter()
        .find(|face| face.element == element)
        .map_or(Color32::from_rgb(94, 108, 126), |face| face.color)
}

pub(crate) fn face_is_visible(element: &ElementId, forward: Vec3) -> bool {
    let ElementId::Face { axis, side } = element else {
        return false;
    };
    let direction = match axis {
        Axis::X => forward.x,
        Axis::Y => forward.y,
        Axis::Z => forward.z,
    };
    let outward_dot_forward = match side {
        Side::Minimum => -direction,
        Side::Maximum => direction,
    };
    outward_dot_forward < -ROUNDING
}

pub(crate) fn projected_face_has_area(corners: [usize; 4], projected: &[Pos2; 8]) -> bool {
    let points = corners.map(|index| projected[index]);
    projected_polygon_has_area(&points)
}

pub(crate) fn projected_polygon_has_area(points: &[Pos2]) -> bool {
    let twice_area = (0..points.len())
        .map(|index| {
            let current = points[index];
            let next = points[(index + 1) % points.len()];
            current.x * next.y - next.x * current.y
        })
        .sum::<f32>();
    twice_area.abs() >= 1.0
}

pub(crate) fn adaptive_grid_step(pixels_per_mm: f64) -> f64 {
    const TARGET_SPACING_PX: f64 = 32.0;
    let desired = (TARGET_SPACING_PX / pixels_per_mm.max(ROUNDING)).max(GRID_STEP_MM);
    let magnitude = 10.0_f64.powf(desired.log10().floor());
    let normalized = desired / magnitude;
    let factor = if normalized <= 1.0 {
        1.0
    } else if normalized <= 2.0 {
        2.0
    } else if normalized <= 5.0 {
        5.0
    } else {
        10.0
    };
    factor * magnitude
}

fn projected_bounds(points: &[Vec3], project: impl Fn(Vec3) -> Pos2) -> Rect {
    points
        .iter()
        .map(|point| Rect::from_min_max(project(*point), project(*point)))
        .reduce(|left, right| left.union(right))
        .unwrap_or(Rect::ZERO)
}

fn arc_polyline(arc: ArcGeometry, segments: usize) -> Vec<Vec3> {
    let radius = length(arc.start - arc.center);
    let start_angle = (arc.start.y - arc.center.y).atan2(arc.start.x - arc.center.x);
    let end_angle = (arc.end.y - arc.center.y).atan2(arc.end.x - arc.center.x);
    let mut sweep = end_angle - start_angle;
    if arc.clockwise {
        while sweep >= 0.0 {
            sweep -= std::f64::consts::TAU;
        }
    } else {
        while sweep <= 0.0 {
            sweep += std::f64::consts::TAU;
        }
    }
    (0..=segments)
        .map(|index| {
            let angle = start_angle + sweep * index as f64 / segments as f64;
            Vec3::new(
                arc.center.x + radius * angle.cos(),
                arc.center.y + radius * angle.sin(),
                arc.start.z,
            )
        })
        .collect()
}

pub(crate) fn paint_dashed_segment(painter: &egui::Painter, points: [Pos2; 2], stroke: Stroke) {
    let delta = points[1] - points[0];
    let length = delta.length();
    if length <= f32::EPSILON {
        return;
    }
    let direction = delta / length;
    let mut offset = 0.0_f32;
    while offset < length {
        let end = (offset + 5.0).min(length);
        painter.line_segment(
            [points[0] + direction * offset, points[0] + direction * end],
            stroke,
        );
        offset += 9.0;
    }
}

/// Inputs shared by the box, exact and mesh projection passes of one frame.
#[derive(Clone, Copy)]
struct ViewportProjectionFrame<'a> {
    rect: Rect,
    camera_dragging: bool,
    forward: Vec3,
    snapshot: &'a Snapshot,
    interaction_projection_cache: &'a Option<InteractionProjectionCache>,
    exact_projection: &'a ExactInteractionProjection,
    move_transform_overrides: &'a BTreeMap<InstancePath, Transform>,
    rotate_copies: &'a BTreeMap<InstancePath, Transform>,
    use_wgpu_scene: bool,
    active_context_paths: &'a BTreeSet<InstancePath>,
    occurrence_colors: &'a BTreeMap<InstancePath, Color32>,
}

/// The projected viewport scene of one frame, faces sorted back to front.
struct ProjectedViewportScene {
    scene_plan: Option<Arc<InstancedRenderPlan>>,
    viewport_boxes: Vec<RenderBox>,
    faces: Vec<ProjectedFace>,
    feedback_faces: Vec<ProjectedFace>,
    edges: Vec<ProjectedEdge>,
    hidden_ghost_corners: Vec<[Vec3; 8]>,
}
