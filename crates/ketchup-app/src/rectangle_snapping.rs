use super::*;

pub(super) fn point_in_frame(point: Vec3, frame: WorkplaneFrame) -> bool {
    let origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
    let normal = Vec3::new(frame.normal[0], frame.normal[1], frame.normal[2]);
    dot(point - origin, normal).abs() <= 1.0e-7
}

impl KetchupApp {
    pub(super) fn rectangle_input_frame(&self, pointer: Pos2, rect: Rect) -> WorkplaneFrame {
        if self.face_workflow_datum() != PrincipalPlane::Xy {
            return self.rectangle_frame(None);
        }
        let point = self.sketch_start.unwrap_or_else(|| {
            self.scene_snap_at_screen(pointer, rect, 8.0, None)
                .map_or_else(
                    || Vec3::new(0.0, 0.0, self.rectangle_plane_z(pointer, rect)),
                    |snap| snap.position_mm,
                )
        });
        self.rectangle_frame(Some(point))
    }

    pub(super) fn rectangle_origin_snap(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        self.datum_snap_at_screen(
            pointer,
            rect,
            Some(self.rectangle_input_frame(pointer, rect)),
        )
        .filter(|(_, axis)| axis.is_none())
        .map(|(point, _)| point)
    }

    pub(super) fn rectangle_snap_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SnapResult> {
        if self.rectangle_origin_snap(pointer, rect).is_some() {
            return None;
        }
        self.scene_snap_at_screen(
            pointer,
            rect,
            8.0,
            Some(self.rectangle_input_frame(pointer, rect)),
        )
    }

    pub(super) fn rectangle_input_point(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        let frame = self.rectangle_input_frame(pointer, rect);
        self.rectangle_origin_snap(pointer, rect)
            .or_else(|| {
                self.rectangle_snap_at_screen(pointer, rect)
                    .map(|snap| snap.position_mm)
            })
            .or_else(|| {
                self.datum_snap_at_screen(pointer, rect, Some(frame))
                    .map(|(point, _)| point)
            })
            .or_else(|| self.screen_to_workplane(pointer, rect, frame))
    }
}
