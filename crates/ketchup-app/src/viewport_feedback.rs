use super::*;

const HOVER_FILL: Color32 = Color32::from_rgba_premultiplied(24, 94, 120, 120);
const SELECTED_FILL: Color32 = Color32::from_rgba_premultiplied(128, 66, 18, 140);

impl KetchupApp {
    #[must_use]
    pub fn edit_context_readout(&self) -> String {
        let snapshot = self.document.current();
        let mut path = vec![self.catalog.text("viewport-context-root")];
        for context in &self.selection.edit_context {
            let name = match context {
                EditContext::Group(id) => snapshot.group(*id).map(|group| group.name()),
                EditContext::Definition { definition_id, .. } => snapshot
                    .definition(*definition_id)
                    .map(|definition| definition.name()),
            };
            if let Some(name) = name {
                path.push(name.to_owned());
            }
        }
        if self.selection.edit_context.is_empty() {
            self.catalog.text("viewport-context-outside")
        } else {
            self.catalog.format(
                "viewport-context-inside",
                &BTreeMap::from([
                    ("path", path.join(" > ")),
                    ("depth", self.selection.edit_context.len().to_string()),
                ]),
            )
        }
    }

    fn edit_context_sharing_readout(&self) -> Option<String> {
        let EditContext::Definition { definition_id, .. } = self.selection.edit_context.last()?
        else {
            return None;
        };
        let count = self
            .document
            .current()
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.definition_id == *definition_id)
            .count();
        Some(self.catalog.format(
            if count > 1 {
                "viewport-context-shared"
            } else {
                "viewport-context-unique"
            },
            &BTreeMap::from([("count", count.to_string())]),
        ))
    }

    pub(super) fn show_edit_context_bar(&mut self, ui: &mut egui::Ui) -> bool {
        let editing = !self.selection.edit_context.is_empty();
        let accent = self.palette().accent;
        let mut cycled = false;
        let response = egui::Frame::new()
            .fill(self.palette().glass())
            .stroke(Stroke::new(
                if editing { 2.0_f32 } else { 1.0_f32 },
                if editing { accent } else { self.palette().line },
            ))
            .inner_margin(6.0)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.strong(self.edit_context_readout());
                    if let Some(sharing) = self.edit_context_sharing_readout() {
                        ui.label(sharing);
                    }
                    if editing
                        && ui
                            .button(self.catalog.text("viewport-context-up"))
                            .clicked()
                    {
                        self.exit_edit_context();
                    }
                    if let Some((index, count)) = self
                        .hovered_overlap_choice()
                        .filter(|(_, count)| *count > 1)
                    {
                        ui.separator();
                        ui.label(self.catalog.format(
                            "face-workflow-overlap-hint",
                            &BTreeMap::from([
                                ("index", (index + 1).to_string()),
                                ("count", count.to_string()),
                            ]),
                        ));
                        if ui
                            .button(self.catalog.text("viewport-next-target"))
                            .clicked()
                        {
                            cycled = self.cycle_hover_overlap();
                        }
                    }
                });
            })
            .response;
        cycled
            || ui.input(|input| {
                input
                    .pointer
                    .hover_pos()
                    .is_some_and(|point| response.rect.contains(point))
            })
    }

    pub(super) fn origin_snap_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        let origin = Vec3::new(0.0, 0.0, 0.0);
        (self.face_workflow.snaps_enabled()
            && plane_z.abs() < 1.0e-6
            && self.project(origin, rect).distance(pointer) <= 8.0)
            .then_some(origin)
    }

    pub(super) fn paint_origin_snap(
        &self,
        painter: &egui::Painter,
        pointer: Option<Pos2>,
        rect: Rect,
    ) {
        let Some(pointer) = pointer else { return };
        if !self.sketch_mode {
            return;
        }
        let plane_z = self
            .sketch_start
            .map_or_else(|| self.rectangle_plane_z(pointer, rect), |start| start.z);
        let origin = if self.active_tool == ActiveTool::Rectangle {
            self.rectangle_origin_snap(pointer, rect)
        } else {
            self.origin_snap_at_screen(pointer, rect, plane_z)
        };
        let Some(origin) = origin else { return };
        let point = self.project(origin, rect);
        painter.circle_filled(point, 4.0, Color32::from_rgb(80, 230, 190));
        painter.circle_stroke(point, 9.0, Stroke::new(2.0_f32, Color32::WHITE));
        painter.text(
            point + Vec2::new(14.0, -14.0),
            egui::Align2::LEFT_BOTTOM,
            self.catalog.text("viewport-snap-origin"),
            egui::FontId::proportional(13.0),
            Color32::from_rgb(80, 230, 190),
        );
    }

    pub(super) fn paint_face_feedback<'a>(
        &self,
        painter: &egui::Painter,
        faces: impl Iterator<Item = &'a ProjectedFace>,
    ) {
        let faces = faces.collect::<Vec<_>>();
        // Draw the hovered target last, including when it is behind another body.
        for hovered_pass in [false, true] {
            for face in &faces {
                let hovered = self.hovered.as_ref() == Some(&face.selection);
                if hovered != hovered_pass {
                    continue;
                }
                let selected = self.selection.primary.as_ref() == Some(&face.selection);
                let fill = if face.out_of_context && !self.selection.edit_context.is_empty() {
                    Color32::from_rgba_unmultiplied(190, 195, 205, 95)
                } else if hovered {
                    HOVER_FILL
                } else if selected {
                    SELECTED_FILL
                } else {
                    continue;
                };
                painter.add(egui::Shape::convex_polygon(
                    face.polygon.points().to_vec(),
                    fill,
                    Stroke::NONE,
                ));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origin_snap_works_in_empty_document_at_multiple_zooms_and_respects_plane_and_toggle() {
        let mut app = KetchupApp::new();
        app.new_document();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(1000.0, 700.0));
        let origin = Vec3::new(0.0, 0.0, 0.0);
        for zoom in [0.5, 1.0, 4.0] {
            app.zoom = zoom;
            let pointer = app.project(origin, rect) + Vec2::new(5.0, 2.0);
            assert_eq!(
                app.viewport_point_at_screen(pointer, rect, 0.0),
                Some(origin)
            );
            assert_eq!(app.rectangle_point_at_screen(pointer, rect), Some(origin));
            assert_eq!(app.origin_snap_at_screen(pointer, rect, 20.0), None);
            assert_eq!(
                app.origin_snap_at_screen(pointer + Vec2::new(10.0, 0.0), rect, 0.0),
                None
            );
            app.face_workflow.set_snaps_enabled(false);
            assert_ne!(
                app.viewport_point_at_screen(pointer, rect, 0.0),
                Some(origin)
            );
            app.face_workflow.set_snaps_enabled(true);
        }
    }

    #[test]
    fn context_readout_tracks_nested_groups_and_return_to_model() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateGroup {
                    id: GroupId(100),
                    name: "Outer".into(),
                    transform: Transform::identity(),
                    parent: None,
                },
                CanonicalCommand::CreateGroup {
                    id: GroupId(101),
                    name: "Inner".into(),
                    transform: Transform::identity(),
                    parent: Some(GroupId(100)),
                },
            ]))
            .unwrap();
        let outside = app.edit_context_readout();
        assert!(app.enter_group_context(GroupId(100)));
        assert!(app.enter_group_context(GroupId(101)));
        let inside = app.edit_context_readout();
        assert!(inside.contains("Outer > Inner"));
        assert!(inside.contains('2'));
        assert!(app.exit_edit_context());
        assert!(!app.edit_context_readout().contains("Inner"));
        assert!(app.exit_edit_context());
        assert_eq!(app.edit_context_readout(), outside);
        assert!(app.enter_group_context(GroupId(100)));
        let label = app.catalog.text("viewport-context-up");
        let mut harness = egui_kittest::Harness::builder()
            .with_size(Vec2::new(1600.0, 1000.0))
            .build_state(|context, app: &mut KetchupApp| app.ui(context), app);
        use egui_kittest::kittest::Queryable as _;
        harness.run();
        harness.get_by_label(&label).click();
        harness.run();
        assert_eq!(harness.state().edit_context_depth(), 0);
        assert_eq!(harness.state().edit_context_readout(), outside);
    }

    #[test]
    fn active_context_dims_outside_geometry_with_a_visible_overlay() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateGroup {
                id: GroupId(100),
                name: "Editing".into(),
                transform: Transform::identity(),
                parent: None,
            }]))
            .unwrap();
        assert!(app.enter_group_context(GroupId(100)));
        let outside = ProjectedFace {
            selection: SelectionId {
                definition_id: INITIAL_BOX_DEFINITION,
                instance_path: InstancePath::root(OccurrenceId(1)),
                element: ElementId::Face {
                    axis: Axis::Z,
                    side: Side::Maximum,
                },
            },
            polygon: ProjectedPolygon::Triangle([
                Pos2::new(10.0, 10.0),
                Pos2::new(110.0, 10.0),
                Pos2::new(10.0, 110.0),
            ]),
            color: Color32::WHITE,
            depth: 0.0,
            previewed: false,
            out_of_context: true,
        };
        let context = egui::Context::default();
        let output = context.run(egui::RawInput::default(), |context| {
            app.paint_face_feedback(
                &context.layer_painter(egui::LayerId::new(
                    egui::Order::Foreground,
                    egui::Id::new("outside-context-dimming"),
                )),
                std::iter::once(&outside),
            );
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Path(path)
                if path.fill == Color32::from_rgba_unmultiplied(190, 195, 205, 95)
        )));
    }

    #[test]
    fn native_viewport_emits_filled_hover_overlay_after_gpu_callback() {
        let mut app = KetchupApp::new();
        app.wgpu_target_format = Some(eframe::wgpu::TextureFormat::Rgba8Unorm);
        let context = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            ..Default::default()
        };
        let _ = context.run(input(), |context| app.ui(context));
        let pointer = app.viewport_position(Vec3::new(50.0, 30.0, 20.0)).unwrap();
        let mut raw = input();
        raw.events.push(egui::Event::PointerMoved(pointer));
        let output = context.run(raw, |context| app.ui(context));
        assert!(app.hovered.is_some());
        let callback = output
            .shapes
            .iter()
            .position(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
            .expect("native GPU scene");
        let overlay = output
            .shapes
            .iter()
            .position(|shape| {
                matches!(&shape.shape, egui::Shape::Path(path)
                    if path.fill == HOVER_FILL && path.stroke.is_empty())
            })
            .expect("visible face fill, not just internal hover state");
        assert!(overlay > callback);

        app.select_from_viewport(app.hovered.clone(), false);
        let mut raw = input();
        raw.events.push(egui::Event::PointerGone);
        let output = context.run(raw, |context| app.ui(context));
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Path(path)
                if path.fill == SELECTED_FILL && path.stroke.is_empty())));

        assert!(app.copy_selected(Vec3::new(10.0, 0.0, 0.0)));
        app.active_tool = ActiveTool::PushPull;
        let mut raw = input();
        raw.events.push(egui::Event::PointerMoved(pointer));
        let _ = context.run(raw, |context| app.ui(context));
        let before = app.hovered.clone();
        let revision = app.document_revision();
        let mut raw = input();
        raw.modifiers.alt = true;
        let output = context.run(raw, |context| app.ui(context));
        assert_ne!(app.hovered, before);
        assert!(app.face_workflow.xray_preview());
        assert!(
            !app.xray_visible,
            "Alt must not change the persistent view setting"
        );
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Path(path) if path.fill.a() == 72)),
            "Alt must render translucent faces"
        );
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Path(path) if path.fill == HOVER_FILL)),
            "cycled target must have a filled highlight"
        );
        let chosen = app.hovered.clone();
        let output = context.run(input(), |context| app.ui(context));
        assert_eq!(app.hovered, chosen);
        assert!(!app.face_workflow.xray_preview());
        assert!(
            output
                .shapes
                .iter()
                .any(|shape| matches!(shape.shape, egui::Shape::Callback(_)))
        );
        assert_eq!(app.document_revision(), revision);
    }

    #[test]
    fn cycled_back_face_emits_a_filled_hover_overlay() {
        let mut app = KetchupApp::new();
        app.wgpu_target_format = Some(eframe::wgpu::TextureFormat::Rgba8Unorm);
        let context = egui::Context::default();
        let input = || egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
            ..Default::default()
        };
        let _ = context.run(input(), |context| app.ui(context));
        let pointer = app.viewport_position(Vec3::new(50.0, 30.0, 20.0)).unwrap();
        let mut raw = input();
        raw.events.push(egui::Event::PointerMoved(pointer));
        let _ = context.run(raw, |context| app.ui(context));

        let forward = Vec3::new(
            -f64::from(app.yaw.sin() * app.pitch.sin()),
            -f64::from(app.yaw.cos() * app.pitch.sin()),
            -f64::from(app.pitch.cos()),
        );
        let (hidden_index, hidden) = app
            .hover_pick
            .as_ref()
            .expect("the box must be pickable")
            .overlapping
            .iter()
            .enumerate()
            .find(|(_, hit)| !face_is_visible(&hit.reference.element, forward))
            .map(|(index, hit)| (index, hit.reference.clone()))
            .expect("the ray must expose a back face for overlap cycling");
        app.hover_overlap_index = hidden_index;
        app.refresh_hover_choice();
        assert_eq!(app.hovered.as_ref(), Some(&hidden));

        let output = context.run(input(), |context| app.ui(context));
        assert_eq!(app.hovered.as_ref(), Some(&hidden));
        assert!(
            output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Path(path) if path.fill == HOVER_FILL && path.stroke.is_empty())),
            "a cycled back face must remain visibly highlighted"
        );
    }

    #[test]
    fn locked_vertical_line_preview_is_blue_and_keeps_its_spatial_length() {
        let mut app = KetchupApp::new();
        app.active_tool = ActiveTool::Line;
        app.sketch_mode = true;
        app.sketch_start = Some(Vec3::new(0.0, 0.0, 20.0));
        app.sketch_cursor = Some(Vec3::new(0.0, 0.0, 55.0));
        app.line_axis_lock = Some(Axis::Z);
        let context = egui::Context::default();
        let output = context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1600.0, 1000.0))),
                ..Default::default()
            },
            |context| app.ui(context),
        );
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::LineSegment { stroke, .. }
                if stroke.width == 2.5 && stroke.color == Color32::from_rgb(80, 145, 255)
        )));
    }
}
