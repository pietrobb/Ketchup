use super::*;

pub(super) fn point_in_frame(point: Vec3, frame: WorkplaneFrame) -> bool {
    let origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
    let normal = Vec3::new(frame.normal[0], frame.normal[1], frame.normal[2]);
    dot(point - origin, normal).abs() <= 1.0e-7
}

pub(super) fn local_point(frame: WorkplaneFrame, point: Vec3) -> Vec3 {
    let delta = point - Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
    let component = |axis: [f64; 3]| dot(delta, Vec3::new(axis[0], axis[1], axis[2]));
    Vec3::new(
        component(frame.x_axis),
        component(frame.y_axis),
        component(frame.normal),
    )
}

pub(super) fn world_point(frame: WorkplaneFrame, point: Vec3) -> Vec3 {
    let vector = |v: [f64; 3]| Vec3::new(v[0], v[1], v[2]);
    vector(frame.origin_mm)
        + vector(frame.x_axis) * point.x
        + vector(frame.y_axis) * point.y
        + vector(frame.normal) * point.z
}

impl KetchupApp {
    pub(super) fn uses_drawing_plane(&self) -> bool {
        matches!(
            self.active_tool,
            ActiveTool::Rectangle | ActiveTool::Circle | ActiveTool::Arc
        )
    }

    pub(super) fn drawing_input_frame(&self, pointer: Pos2, rect: Rect) -> WorkplaneFrame {
        // The first point locates the plane; only later points are constrained by it.
        let point = self
            .sketch_start
            .or_else(|| {
                self.scene_snap_at_screen(pointer, rect, 8.0, None)
                    .map(|snap| snap.position_mm)
                    .or_else(|| {
                        self.datum_snap_at_screen(pointer, rect, None)
                            .map(|(p, _)| p)
                    })
            })
            .unwrap_or_else(|| {
                self.surface_point_at_screen(pointer, rect)
                    .unwrap_or(Vec3::ZERO)
            });
        self.drawing_frame(Some(point))
    }

    pub(super) fn drawing_origin_snap(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        self.datum_snap_at_screen(
            pointer,
            rect,
            self.sketch_start.map(|p| self.drawing_frame(Some(p))),
        )
        .filter(|(_, axis)| axis.is_none())
        .map(|(point, _)| point)
    }

    pub(super) fn drawing_snap_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SnapResult> {
        if self.drawing_origin_snap(pointer, rect).is_some() {
            return None;
        }
        self.scene_snap_at_screen(
            pointer,
            rect,
            8.0,
            self.sketch_start.map(|p| self.drawing_frame(Some(p))),
        )
    }

    pub(super) fn drawing_input_point(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        let frame = self.drawing_input_frame(pointer, rect);
        self.drawing_origin_snap(pointer, rect)
            .or_else(|| {
                self.drawing_snap_at_screen(pointer, rect)
                    .map(|snap| snap.position_mm)
            })
            .or_else(|| {
                self.datum_snap_at_screen(pointer, rect, self.sketch_start.map(|_| frame))
                    .map(|(point, _)| point)
            })
            .or_else(|| self.screen_to_workplane(pointer, rect, frame))
    }

    pub(super) fn drawing_transform(&self, origin: Vec3) -> Transform {
        let f = self.drawing_frame(Some(origin));
        let (x, y, n) = (f.x_axis, f.y_axis, f.normal);
        Transform::from_matrix([
            x[0], y[0], n[0], origin.x, x[1], y[1], n[1], origin.y, x[2], y[2], n[2], origin.z,
            0.0, 0.0, 0.0, 1.0,
        ])
        .expect("principal drawing frame is rigid")
    }

    pub(super) fn drawing_local_delta(&self, start: Vec3, point: Vec3) -> Vec3 {
        let frame = self.drawing_frame(Some(start));
        local_point(frame, point) - local_point(frame, start)
    }

    pub(super) fn drawing_world_delta(&self, start: Vec3, local: Vec3) -> Vec3 {
        let frame = self.drawing_frame(Some(start));
        start + world_point(frame, local) - world_point(frame, Vec3::ZERO)
    }

    pub(super) fn drawing_bulge(&self, start: Vec3, end: Vec3, point: Vec3) -> f64 {
        point_line_signed_distance(
            self.drawing_local_delta(start, point),
            Vec3::ZERO,
            self.drawing_local_delta(start, end),
        )
    }

    pub(super) fn drawing_arc(&self, start: Vec3, end: Vec3, point: Vec3) -> Option<ArcGeometry> {
        arc_geometry(
            Vec3::ZERO,
            self.drawing_local_delta(start, end),
            self.drawing_local_delta(start, point),
        )
    }

    pub(super) fn drawing_rectangle_corners(&self, start: Vec3, end: Vec3) -> [Vec3; 4] {
        let local = self.drawing_local_delta(start, end);
        [
            start,
            self.drawing_world_delta(start, Vec3::new(local.x, 0.0, 0.0)),
            end,
            self.drawing_world_delta(start, Vec3::new(0.0, local.y, 0.0)),
        ]
    }

    pub(super) fn set_drawing_plane(&mut self, plane: PrincipalPlane) {
        let start = self.sketch_start.unwrap_or(Vec3::ZERO);
        let end = self.sketch_end.map(|p| self.drawing_local_delta(start, p));
        let cursor = self
            .sketch_cursor
            .map(|p| self.drawing_local_delta(start, p));
        self.face_workflow.set_datum(plane);
        self.sketch_end = end.map(|p| self.drawing_world_delta(start, p));
        self.sketch_cursor = cursor.map(|p| self.drawing_world_delta(start, p));
        self.hover_snap = None;
    }
}
