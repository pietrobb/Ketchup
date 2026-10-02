//! Painting viewport layers: ground, shadows, faces, edges, selection, guides and feedback.

use crate::*;

impl KetchupApp {
    /// Paint the Rotate protractor: the coloured axis through the body, the
    /// ring it turns in, its snap ticks, and the swept angle once a gesture has
    /// an arm to measure from.
    pub(crate) fn paint_rotation_guide(&self, painter: &egui::Painter, rect: Rect) {
        let Some(guide) = self.rotation_guide() else {
            return;
        };
        let colour = axis_color(guide.axis);
        let faint = Color32::from_rgba_unmultiplied(colour.r(), colour.g(), colour.b(), 130);
        let at = |scale: f64, degrees: f64| {
            self.project(
                rotation_plane_point(
                    guide.centre_mm,
                    guide.axis,
                    guide.radius_mm * scale,
                    degrees,
                ),
                rect,
            )
        };

        let reach = axis_direction(guide.axis) * (guide.radius_mm * 1.8);
        painter.line_segment(
            [
                self.project(guide.centre_mm - reach, rect),
                self.project(guide.centre_mm + reach, rect),
            ],
            Stroke::new(
                if self.gesture.transform.rotate_axis_lock.is_some() {
                    2.4_f32
                } else {
                    1.4_f32
                },
                colour,
            ),
        );

        // The ring is sampled in the rotation plane rather than drawn as a
        // screen circle, so it projects to the ellipse the camera really sees
        // and reads as a plane in space.
        const RING_SAMPLES: u32 = 72;
        painter.add(egui::Shape::line(
            (0..=RING_SAMPLES)
                .map(|step| at(1.0, f64::from(step) * 360.0 / f64::from(RING_SAMPLES)))
                .collect(),
            Stroke::new(1.6_f32, faint),
        ));
        let ticks = (360.0 / ROTATION_SNAP_DEGREES).round() as i32;
        for index in 0..ticks {
            let degrees = f64::from(index) * ROTATION_SNAP_DEGREES;
            let quarter = index % (ticks / 4).max(1) == 0;
            painter.line_segment(
                [
                    at(if quarter { 0.88 } else { 0.95 }, degrees),
                    at(1.0, degrees),
                ],
                Stroke::new(if quarter { 1.6_f32 } else { 1.0_f32 }, faint),
            );
        }
        painter.circle_filled(self.project(guide.centre_mm, rect), 3.5, colour);

        let Some(start) = guide.start_degrees else {
            return;
        };
        let end = start + guide.angle_degrees;
        painter.line_segment(
            [self.project(guide.centre_mm, rect), at(1.0, start)],
            Stroke::new(1.6_f32, Color32::from_gray(190)),
        );
        painter.line_segment(
            [self.project(guide.centre_mm, rect), at(1.06, end)],
            Stroke::new(2.0_f32, colour),
        );
        if guide.angle_degrees.abs() >= 0.5 {
            let steps = (guide.angle_degrees.abs() / 3.0).ceil().max(1.0) as u32;
            painter.add(egui::Shape::line(
                (0..=steps)
                    .map(|step| {
                        at(
                            0.86,
                            start + guide.angle_degrees * f64::from(step) / f64::from(steps),
                        )
                    })
                    .collect(),
                Stroke::new(2.6_f32, colour),
            ));
            let tip = at(0.86, end);
            let tail = at(0.86, end - 6.0 * guide.angle_degrees.signum());
            if (tip - tail).length() > 0.5 {
                let along = (tip - tail).normalized();
                let across = Vec2::new(-along.y, along.x);
                painter.add(egui::Shape::convex_polygon(
                    vec![
                        tip + along * 7.0,
                        tip - along * 4.0 + across * 4.5,
                        tip - along * 4.0 - across * 4.5,
                    ],
                    colour,
                    Stroke::NONE,
                ));
            }
        }
        painter.text(
            at(1.22, end),
            egui::Align2::CENTER_CENTER,
            format!("{}°", format_angle(guide.angle_degrees)),
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
    }

    pub(crate) fn paint_projected_shadows(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        boxes: &[RenderBox],
    ) {
        if !self.view.contains(ViewFlag::Shadows) {
            return;
        }
        let fill = if self.view.contains(ViewFlag::WhiteBackground) {
            Color32::from_rgba_unmultiplied(30, 36, 44, 34)
        } else {
            Color32::from_rgba_unmultiplied(8, 12, 18, 48)
        };
        let snapshot = self.document.current();
        for item in boxes {
            if renderer::canonical_definition_fallback_mesh(&snapshot, item.definition_id).is_some()
            {
                continue;
            }
            let elevation = (item.origin_mm.z + item.size_mm.z).max(0.0);
            let offset_x = elevation * 0.28;
            let offset_y = -elevation * 0.18;
            let corners = [
                Vec3::new(
                    item.origin_mm.x + offset_x,
                    item.origin_mm.y + offset_y,
                    0.0,
                ),
                Vec3::new(
                    item.origin_mm.x + item.size_mm.x + offset_x,
                    item.origin_mm.y + offset_y,
                    0.0,
                ),
                Vec3::new(
                    item.origin_mm.x + item.size_mm.x + offset_x,
                    item.origin_mm.y + item.size_mm.y + offset_y,
                    0.0,
                ),
                Vec3::new(
                    item.origin_mm.x + offset_x,
                    item.origin_mm.y + item.size_mm.y + offset_y,
                    0.0,
                ),
            ]
            .map(|point| self.project(point, rect));
            painter.add(egui::Shape::convex_polygon(
                corners.to_vec(),
                fill,
                Stroke::NONE,
            ));
        }
    }

    pub(crate) fn paint_viewport_fog(&self, painter: &egui::Painter, rect: Rect) {
        if !self.view.contains(ViewFlag::Fog) {
            return;
        }
        let [red, green, blue] = if self.view.contains(ViewFlag::WhiteBackground) {
            [244, 247, 250]
        } else {
            [192, 204, 218]
        };
        let mut haze = egui::Mesh::default();
        haze.colored_vertex(
            rect.left_top(),
            Color32::from_rgba_unmultiplied(red, green, blue, 112),
        );
        haze.colored_vertex(
            rect.right_top(),
            Color32::from_rgba_unmultiplied(red, green, blue, 112),
        );
        haze.colored_vertex(
            rect.right_bottom(),
            Color32::from_rgba_unmultiplied(red, green, blue, 8),
        );
        haze.colored_vertex(
            rect.left_bottom(),
            Color32::from_rgba_unmultiplied(red, green, blue, 8),
        );
        haze.add_triangle(0, 1, 2);
        haze.add_triangle(0, 2, 3);
        painter.add(egui::Shape::mesh(haze));
    }

    pub(crate) fn paint_projected_selection(
        &self,
        painter: &egui::Painter,
        edges: &[ProjectedEdge],
    ) {
        if self.active_tool == ActiveTool::Rotate {
            return;
        }
        let selection_stroke = Stroke::new(1.8_f32, Color32::from_rgb(240, 78, 35));
        let halo_stroke = Stroke::new(
            4.8_f32,
            if self.view.contains(ViewFlag::WhiteBackground) {
                Color32::BLACK
            } else {
                Color32::WHITE
            },
        );
        for edge in edges.iter().filter(|edge| {
            if matches!(
                self.active_tool,
                ActiveTool::PushPull
                    | ActiveTool::SolidSubtract
                    | ActiveTool::SolidTrim
                    | ActiveTool::SolidUnion
                    | ActiveTool::SolidIntersect
                    | ActiveTool::SolidSplit
            ) {
                self.selection.primary.as_ref() == Some(&edge.selection)
            } else {
                self.selection.contains(&edge.selection.instance_path)
            }
        }) {
            if self.view.contains(ViewFlag::SelectionHalo) {
                painter.line_segment(edge.points, halo_stroke);
            }
            painter.line_segment(edge.points, selection_stroke);
        }
    }

    /// Selected topological edges stay visible after the pointer leaves them,
    /// drawn over the bodies because a hole mouth can sit inside another part.
    pub(crate) fn paint_selected_topological_edges(&self, painter: &egui::Painter, rect: Rect) {
        let halo = Stroke::new(
            5.0_f32,
            if self.view.contains(ViewFlag::WhiteBackground) {
                Color32::BLACK
            } else {
                Color32::WHITE
            },
        );
        let stroke = Stroke::new(2.4_f32, Color32::from_rgb(240, 78, 35));
        for path in self.selected_topological_edge_paths() {
            let points: Vec<_> = path
                .into_iter()
                .map(|point| self.project(point, rect))
                .collect();
            painter.add(egui::Shape::line(points.clone(), halo));
            painter.add(egui::Shape::line(points, stroke));
        }
    }

    pub(crate) fn paint_projected_edges(&self, painter: &egui::Painter, edges: &[ProjectedEdge]) {
        if !self.view.contains(ViewFlag::Edges) {
            return;
        }
        let base_width = if self.view.contains(ViewFlag::Profiles) {
            2.75_f32
        } else {
            1.25_f32
        };
        let (minimum_depth, maximum_depth) = edges.iter().fold(
            (f64::INFINITY, f64::NEG_INFINITY),
            |(minimum, maximum), edge| (minimum.min(edge.depth), maximum.max(edge.depth)),
        );
        let depth_span = maximum_depth - minimum_depth;
        let painted_points = |index: usize, points: [Pos2; 2]| {
            if !self.view.contains(ViewFlag::Jitter) {
                return points;
            }
            let offset = match index % 4 {
                0 => Vec2::new(-1.5, -0.75),
                1 => Vec2::new(1.5, -0.75),
                2 => Vec2::new(1.5, 0.75),
                _ => Vec2::new(-1.5, 0.75),
            };
            points.map(|point| point + offset)
        };
        let paint_edge_segment = |points: [Pos2; 2], stroke: Stroke| {
            if self.view.contains(ViewFlag::Dashes) {
                paint_dashed_segment(painter, points, stroke);
            } else {
                painter.line_segment(points, stroke);
            }
        };
        let background_color = if self.view.contains(ViewFlag::WhiteBackground) {
            Color32::from_rgb(244, 247, 250)
        } else {
            Color32::from_rgb(24, 28, 35)
        };
        let edge_color = |edge: &ProjectedEdge| {
            let source = if self.view.contains(ViewFlag::HighContrastEdges) {
                if self.view.contains(ViewFlag::WhiteBackground) {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                }
            } else if self.view.contains(ViewFlag::ColorByAxis) {
                edge.dominant_axis
                    .map_or(Color32::from_rgb(182, 192, 207), axis_color)
            } else {
                Color32::from_rgb(182, 192, 207)
            };
            if !self.view.contains(ViewFlag::FadeDistantEdges) || depth_span <= f64::EPSILON {
                return source;
            }
            let proximity = ((maximum_depth - edge.depth) / depth_span).clamp(0.0, 1.0);
            let source_weight = (35.0 + 65.0 * proximity).round() as u16;
            let blend = |source: u8, background: u8| {
                ((u16::from(source) * source_weight
                    + u16::from(background) * (100 - source_weight))
                    / 100) as u8
            };
            Color32::from_rgb(
                blend(source.r(), background_color.r()),
                blend(source.g(), background_color.g()),
                blend(source.b(), background_color.b()),
            )
        };
        let halo_color = background_color;
        for (index, edge) in edges.iter().enumerate() {
            let width = if self.view.contains(ViewFlag::DepthCue) && depth_span > f64::EPSILON {
                let proximity = ((maximum_depth - edge.depth) / depth_span).clamp(0.0, 1.0);
                base_width * (0.72_f32 + 0.56_f32 * proximity as f32)
            } else {
                base_width
            };
            let points = painted_points(index, edge.points);
            if self.view.contains(ViewFlag::Halos) {
                paint_edge_segment(points, Stroke::new(width + 3.0, halo_color));
            }
            paint_edge_segment(points, Stroke::new(width, edge_color(edge)));
        }
        if self.view.contains(ViewFlag::Extensions) {
            for (index, edge) in edges.iter().enumerate() {
                let points = painted_points(index, edge.points);
                let direction = points[1] - points[0];
                let length = direction.length();
                if length <= f32::EPSILON {
                    continue;
                }
                let extension = direction * (7.0 / length);
                let width = if self.view.contains(ViewFlag::DepthCue) && depth_span > f64::EPSILON {
                    let proximity = ((maximum_depth - edge.depth) / depth_span).clamp(0.0, 1.0);
                    base_width * (0.72_f32 + 0.56_f32 * proximity as f32)
                } else {
                    base_width
                };
                let stroke = Stroke::new(width, edge_color(edge));
                paint_edge_segment([points[0] - extension, points[0]], stroke);
                paint_edge_segment([points[1], points[1] + extension], stroke);
            }
        }
        if self.view.contains(ViewFlag::Midpoints) {
            for (index, edge) in edges.iter().enumerate() {
                let points = painted_points(index, edge.points);
                painter.circle_filled(
                    points[0].lerp(points[1], 0.5),
                    3.25,
                    Color32::from_rgb(96, 201, 138),
                );
            }
        }
        if self.view.contains(ViewFlag::Endpoints) {
            for (index, edge) in edges.iter().enumerate() {
                for point in painted_points(index, edge.points) {
                    painter.circle_filled(point, 3.25, Color32::from_rgb(232, 158, 72));
                }
            }
        }
    }

    pub(crate) fn paint_projected_faces(&self, painter: &egui::Painter, faces: &[ProjectedFace]) {
        if self.view.contains(ViewFlag::Wireframe) {
            return;
        }
        let mut mesh = egui::Mesh::default();
        for face in faces {
            let color = if face.out_of_context {
                Color32::from_rgb(43, 47, 54)
            } else if self.active_tool == ActiveTool::PushPull
                && !face.previewed
                && self.selection.primary.as_ref() == Some(&face.selection)
            {
                Color32::from_rgb(194, 89, 48)
            } else if !face.previewed && self.selection.contains(&face.selection.instance_path) {
                Color32::from_rgb(154, 91, 67)
            } else if !face.previewed && self.hover.target.as_ref() == Some(&face.selection) {
                Color32::from_rgb(76, 111, 158)
            } else if face.previewed {
                Color32::from_rgb(58, 126, 174)
            } else if self.view.contains(ViewFlag::HiddenLine) {
                Color32::from_rgb(214, 218, 224)
            } else if self.view.contains(ViewFlag::Monochrome) {
                let tone = ((u16::from(face.color.r())
                    + u16::from(face.color.g())
                    + u16::from(face.color.b()))
                    / 3) as u8;
                Color32::from_gray(tone)
            } else {
                face.color
            };
            let color = if self.view.contains(ViewFlag::Xray) || self.face_workflow.xray_preview() {
                Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 72)
            } else {
                color
            };
            viewport_feedback::add_filled_polygon(&mut mesh, face.polygon.points(), color);
        }
        // One unfeathered mesh: no seams between faces, each face blended once
        // in Xray, and no anti-aliasing spikes from sliver triangles.
        if !mesh.indices.is_empty() {
            painter.add(egui::Shape::mesh(mesh));
        }
    }

    pub(crate) fn paint_scene_base_layers(
        &self,
        painter: &egui::Painter,
        rect: Rect,
        scene_plan: Option<Arc<InstancedRenderPlan>>,
    ) {
        if self.view.contains(ViewFlag::GridAxes) {
            self.paint_ground_plane(painter, rect);
        }
        if let Some(plan) = scene_plan {
            let (_, _, forward) = self.camera_basis();
            painter.add(eframe::egui_wgpu::Callback::new_paint_callback(
                rect,
                ScenePaintCallback::new(
                    plan,
                    rect,
                    self.world_to_clip(rect),
                    [forward.x as f32, forward.y as f32, forward.z as f32, 0.0],
                ),
            ));
        }
    }

    /// Paint an adaptive construction grid and the three world axes on Z = 0.
    ///
    /// The painted patch follows the visible part of the ground plane. Its
    /// spacing advances through metric 1/2/5 steps so lines stay legible while
    /// the camera moves between millimetre details and kilometre-scale scenes.
    pub(crate) fn paint_ground_plane(&self, painter: &egui::Painter, rect: Rect) {
        let palette = self.palette();
        let scale = self.view_scale(rect).max(ROUNDING);
        let step = adaptive_grid_step(scale);
        let centre = self
            .screen_to_plane(rect.center(), rect, 0.0)
            .unwrap_or_else(|| self.camera_target());
        let half_reach = f64::from(rect.width().max(rect.height())) / scale * 1.25;
        let x_start = ((centre.x - half_reach) / step).floor() as i64;
        let x_end = ((centre.x + half_reach) / step).ceil() as i64;
        let y_start = ((centre.y - half_reach) / step).floor() as i64;
        let y_end = ((centre.y + half_reach) / step).ceil() as i64;
        let x_min = x_start as f64 * step;
        let x_max = x_end as f64 * step;
        let y_min = y_start as f64 * step;
        let y_max = y_end as f64 * step;
        let painter = painter.with_clip_rect(rect);
        let segment = |from: Vec3, to: Vec3, major: bool| {
            let stroke = Stroke::new(
                if major { 1.2_f32 } else { 1.0_f32 },
                if major {
                    palette.grid_major
                } else {
                    palette.grid
                },
            );
            if let Some(points) = self.project_visible_segment([from, to], rect) {
                painter.line_segment(points, stroke);
            }
        };
        for index in x_start..=x_end {
            if index != 0 {
                segment(
                    Vec3::new(index as f64 * step, y_min, 0.0),
                    Vec3::new(index as f64 * step, y_max, 0.0),
                    index % 5 == 0,
                );
            }
        }
        for index in y_start..=y_end {
            if index != 0 {
                segment(
                    Vec3::new(x_min, index as f64 * step, 0.0),
                    Vec3::new(x_max, index as f64 * step, 0.0),
                    index % 5 == 0,
                );
            }
        }
        // The axes are the one place the viewport does not use the palette: red,
        // green and blue for X, Y and Z is a convention the user already knows
        // from every other modeller, and it must not shift with the theme.
        for (from, to, color) in [
            (
                Vec3::new(x_min, 0.0, 0.0),
                Vec3::new(x_max, 0.0, 0.0),
                axis_color(Axis::X),
            ),
            (
                Vec3::new(0.0, y_min, 0.0),
                Vec3::new(0.0, y_max, 0.0),
                axis_color(Axis::Y),
            ),
            (
                Vec3::new(0.0, 0.0, -half_reach),
                Vec3::new(0.0, 0.0, half_reach),
                axis_color(Axis::Z),
            ),
        ] {
            if let Some(points) = self.project_visible_segment([from, to], rect) {
                painter.line_segment(points, Stroke::new(1.6_f32, color));
            }
        }
    }

    /// Paint the plate and glyph of one rail button.
    ///
    /// The active tool is a filled accent tile with a matching glow, hover is
    /// the raised panel tone, and everything else is flat — so which tool is
    /// armed is readable without reading any text.
    pub(crate) fn paint_rail_button(
        &self,
        ui: &egui::Ui,
        response: &egui::Response,
        id: AppCommand,
        enabled: bool,
        active: bool,
    ) {
        let palette = self.palette();
        let painter = ui.painter();
        let corner = egui::CornerRadius::same(8);
        if active {
            painter.rect_filled(response.rect.expand(2.0), corner, palette.accent_wash(56));
            painter.rect_filled(response.rect, corner, palette.accent);
        } else if enabled && response.hovered() {
            painter.rect_filled(response.rect, corner, palette.panel2);
        }
        let ink = if !enabled {
            palette.faint
        } else if active {
            palette.accent_ink
        } else if response.hovered() {
            palette.text
        } else {
            palette.dim
        };
        // On the filled accent tile the accent detail would vanish into the
        // plate, so there the whole glyph is drawn in the tile's ink instead.
        let detail = if enabled && !active {
            palette.accent
        } else {
            ink
        };
        theme::paint_icon(
            painter,
            shrink_to_icon(response.rect, TOOL_ICON_SIZE),
            command_icon(id),
            ink,
            detail,
            1.55,
        );
    }
}

fn rotation_plane_point(centre_mm: Vec3, axis: Axis, radius_mm: f64, degrees: f64) -> Vec3 {
    let (first, second) = axis_plane_frame(axis);
    let (sin, cos) = degrees.to_radians().sin_cos();
    centre_mm + first * (radius_mm * cos) + second * (radius_mm * sin)
}

/// Which drawing represents a command in the rail and the menus.
///
/// One glyph can serve several commands — Zoom Fit and the Zoom tool share the
/// magnifier — so this is a mapping rather than a field on the command spec.
const fn command_icon(id: AppCommand) -> Icon {
    match id {
        AppCommand::Line => Icon::Line,
        AppCommand::Rectangle => Icon::Rectangle,
        AppCommand::Circle => Icon::Circle,
        AppCommand::Arc => Icon::Arc,
        AppCommand::Polygon => Icon::Polygon,
        AppCommand::Ellipse => Icon::Ellipse,
        AppCommand::Spline => Icon::Spline,
        AppCommand::Revolve => Icon::Orbit,
        AppCommand::Shell => Icon::PushPull,
        AppCommand::Fillet | AppCommand::Chamfer => Icon::Tape,
        AppCommand::PushPull => Icon::PushPull,
        AppCommand::Move => Icon::Move,
        AppCommand::Rotate => Icon::Orbit,
        AppCommand::Scale => Icon::Rectangle,
        AppCommand::Measure => Icon::Tape,
        AppCommand::Orbit => Icon::Orbit,
        AppCommand::Pan => Icon::Pan,
        AppCommand::Delete => Icon::Eraser,
        AppCommand::ZoomFit => Icon::Zoom,
        AppCommand::Undo => Icon::Undo,
        AppCommand::Redo => Icon::Redo,
        _ => Icon::Select,
    }
}
