use super::*;

pub(super) fn point_in_frame(point: Vec3, frame: WorkplaneFrame) -> bool {
    let origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
    let normal = Vec3::new(frame.normal[0], frame.normal[1], frame.normal[2]);
    dot(point - origin, normal).abs() <= 1.0e-7
}

impl KetchupApp {
    fn rectangle_input_frame(&self, pointer: Pos2, rect: Rect) -> WorkplaneFrame {
        if self.face_workflow_datum() != PrincipalPlane::Xy {
            return self.rectangle_frame(None);
        }
        let point = self.sketch_start.unwrap_or_else(|| {
            // The first XY point selects elevation; later points stay on that plane.
            let snap = self
                .face_workflow
                .snaps_enabled()
                .then(|| {
                    self.box_snap_at_screen(pointer, rect, 8.0).or_else(|| {
                        self.pick_result_at_screen(pointer, rect, 8.0)
                            .map(|pick| pick.snap)
                            .filter(|snap| snap.kind != SnapKind::Face)
                    })
                })
                .flatten();
            snap.map_or_else(
                || Vec3::new(0.0, 0.0, self.rectangle_plane_z(pointer, rect)),
                |snap| snap.position_mm,
            )
        });
        self.rectangle_frame(Some(point))
    }

    pub(super) fn rectangle_origin_snap(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        self.origin_snap_at_screen(pointer, rect, 0.0)
            .filter(|point| point_in_frame(*point, self.rectangle_input_frame(pointer, rect)))
    }

    pub(super) fn rectangle_snap_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<SnapResult> {
        if !self.face_workflow.snaps_enabled()
            || self.rectangle_origin_snap(pointer, rect).is_some()
        {
            return None;
        }
        let frame = self.rectangle_input_frame(pointer, rect);
        let in_plane = |snap: &SnapResult| {
            snap.kind != SnapKind::Face && point_in_frame(snap.position_mm, frame)
        };
        self.box_snap_on_plane(pointer, rect, 8.0, Some(frame))
            .or_else(|| {
                self.pick_result_at_screen(pointer, rect, 8.0)
                    .map(|pick| pick.snap)
                    .filter(in_plane)
            })
            .or_else(|| {
                self.profile_special_snap_at_screen(pointer, rect, frame.origin_mm[2])
                    .filter(in_plane)
            })
    }

    pub(super) fn rectangle_input_point(&self, pointer: Pos2, rect: Rect) -> Option<Vec3> {
        self.rectangle_origin_snap(pointer, rect)
            .or_else(|| {
                self.rectangle_snap_at_screen(pointer, rect)
                    .map(|snap| snap.position_mm)
            })
            .or_else(|| {
                self.screen_to_workplane(pointer, rect, self.rectangle_input_frame(pointer, rect))
            })
    }
}
