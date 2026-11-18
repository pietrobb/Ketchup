//! Arrow keys pin a direction to a coloured axis: → red X, ← green Y, ↑ blue Z,
//! ↓ or the held arrow again frees it. For the Line tool the line follows the
//! axis; for the Measure tool the second point does, through the first, so the
//! reading is the distance along that axis alone, to a snapped point too. The
//! measured line takes the axis colour and names the axis.

use crate::*;

/// The colour of a locked axis, as on the axis lines of the view.
pub(crate) const fn axis_color(axis: Option<Axis>) -> Option<Color32> {
    match axis {
        Some(Axis::X) => Some(Color32::from_rgb(230, 80, 80)),
        Some(Axis::Y) => Some(Color32::from_rgb(80, 205, 120)),
        Some(Axis::Z) => Some(Color32::from_rgb(80, 145, 255)),
        None => None,
    }
}

const MEASURE_COLOR: Color32 = Color32::from_rgb(120, 205, 255);

impl KetchupApp {
    pub(crate) fn set_line_axis_lock(&mut self, axis: Option<Axis>) {
        self.gesture.sketch.axis_lock = axis;
        self.digest = self.catalog.format(
            "digest-line-axis-locked",
            &BTreeMap::from([(
                "axis",
                self.catalog
                    .text(axis.map_or("axis-name-plane", axis_name_key)),
            )]),
        );
    }

    pub(crate) fn set_measure_axis_lock(&mut self, axis: Option<Axis>) {
        self.gesture.measure.axis_lock = axis;
        self.digest = match axis {
            Some(axis) => self.catalog.format(
                "digest-measure-axis-locked",
                &BTreeMap::from([("axis", self.catalog.text(axis_name_key(axis)))]),
            ),
            None => self.catalog.text("digest-measure-axis-free"),
        };
    }

    /// The axis the Measure tool pins its second point to, if any.
    #[must_use]
    pub const fn measure_axis_lock(&self) -> Option<Axis> {
        self.gesture.measure.axis_lock
    }

    /// The point a click at `pointer` measures to: a snap, else the point on
    /// the plane at `plane_z` or a surface. With an axis locked while the
    /// Measure tool waits for its second point, that point is moved onto the
    /// axis through the first one; without a snap the pointer slides along it.
    pub fn measurement_point_at_screen(
        &self,
        pointer: Pos2,
        rect: Rect,
        plane_z: f64,
    ) -> Option<Vec3> {
        let snap = self
            .scene_snap_at_screen(pointer, rect, 8.0, None)
            .map(|snap| snap.position_mm);
        if self.active_tool == ActiveTool::Measure
            && let (Some(start), Some(axis)) = (self.measure_anchor(), self.measure_axis_lock())
        {
            let direction = axis_direction(axis);
            let travel = match snap {
                Some(point) => dot(point - start, direction),
                None => axis_travel_along(&self.view_ray(pointer, rect)?, start, axis)?,
            };
            return Some(start + direction * travel);
        }
        snap.or_else(|| self.viewport_point_at_screen(pointer, rect, plane_z))
            .or_else(|| self.surface_point_at_screen(pointer, rect))
    }

    /// The measured segment with its length; in the axis colour, the axis
    /// named before the length, when it was measured along the locked axis.
    pub(crate) fn paint_measure_span(&self, painter: &egui::Painter, rect: Rect) {
        let Some((start, end)) = self.measure_span() else {
            return;
        };
        let delta = end - start;
        let along = self.measure_axis_lock().filter(|axis| {
            let direction = axis_direction(*axis);
            length(delta - direction * dot(delta, direction)) <= ROUNDING
        });
        let color = axis_color(along).unwrap_or(MEASURE_COLOR);
        let from = self.project(start, rect);
        let to = self.project(end, rect);
        painter.line_segment([from, to], Stroke::new(2.0_f32, color));
        painter.circle_filled(from, 3.5, color);
        painter.circle_filled(to, 3.5, color);
        let distance = format_height(length(delta));
        painter.text(
            from.lerp(to, 0.5) - Vec2::new(0.0, 14.0),
            egui::Align2::CENTER_CENTER,
            match along {
                Some(axis) => format!("{} · {distance} mm", self.catalog.text(axis_name_key(axis))),
                None => format!("{distance} mm"),
            },
            egui::FontId::proportional(14.0),
            Color32::WHITE,
        );
    }
}
