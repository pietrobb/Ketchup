//! Mirror tool: the selected parts get mirrored copies across the plane of the
//! face the user clicks, the same copies the Assistant's Mirror makes.

use crate::*;

impl KetchupApp {
    /// The root occurrences the Mirror tool copies, or `None` when nothing
    /// mirrorable is selected.
    pub(crate) fn mirror_sources(&self) -> Option<BTreeSet<OccurrenceId>> {
        if self.selection.selected_group.is_some() {
            return None;
        }
        self.selected_root_occurrence_ids()
            .ok()
            .filter(|ids| !ids.is_empty())
    }

    /// The plane of the face under the pointer.
    pub(crate) fn mirror_plane_at_screen(&self, pointer: Pos2, rect: Rect) -> Option<MirrorPlane> {
        self.surface_hit_at_screen(pointer, rect)
            .or_else(|| {
                let pick = self.pick_result_at_screen(pointer, rect, 0.0)?;
                let ElementId::Face { axis, side } = pick.primary.reference.element else {
                    return None;
                };
                let sign = if side == Side::Maximum { 1.0 } else { -1.0 };
                Some((pick.primary.position_mm, axis_vector(axis, sign)))
            })
            .map(|(point_mm, normal)| MirrorPlane { point_mm, normal })
    }

    /// The reflection across the picked face, moved along its outward normal
    /// by the distance in the value box (none typed = the face itself).
    fn mirror_world_transform(&self) -> Option<Transform> {
        let plane = self.gesture.mirror?;
        let input = self.value_box.input.trim();
        let offset_mm = if input.is_empty() {
            0.0
        } else {
            parse_distance_mm(input)?
        };
        let unit = plane.normal * (1.0 / length(plane.normal));
        world_plane_mirror_transform(plane.point_mm + unit * offset_mm, plane.normal).ok()
    }

    /// Corners of the selected parts' boxes as they land after mirroring; empty
    /// while there is no face under the pointer or nothing to mirror.
    #[must_use]
    pub fn mirror_preview_corners(&self) -> Vec<[Vec3; 8]> {
        let Some(mirror) = self
            .mirror_world_transform()
            .filter(|_| self.active_tool == ActiveTool::Mirror && self.mirror_sources().is_some())
        else {
            return Vec::new();
        };
        self.selected_active_boxes()
            .into_iter()
            .map(|item| {
                box_corners(item.size_mm.x, item.size_mm.y, item.size_mm.z).map(|corner| {
                    let [x, y, z] = mirror.transform_point([
                        item.origin_mm.x + corner.x,
                        item.origin_mm.y + corner.y,
                        item.origin_mm.z + corner.z,
                    ]);
                    Vec3::new(x, y, z)
                })
            })
            .collect()
    }

    pub(crate) fn paint_mirror_preview(&self, painter: &egui::Painter, rect: Rect) {
        let stroke = Stroke::new(1.6_f32, Color32::from_rgb(255, 199, 68));
        for corners in self.mirror_preview_corners() {
            // Box corners differ in one coordinate along each of the 12 edges.
            for from in 0..8_usize {
                for bit in [1, 2, 4] {
                    if from & bit == 0 {
                        paint_dashed_segment(
                            painter,
                            [
                                self.project(corners[from], rect),
                                self.project(corners[from | bit], rect),
                            ],
                            stroke,
                        );
                    }
                }
            }
        }
    }

    /// Adds a mirrored copy of every selected part across the picked plane as
    /// one Undo step.
    pub(crate) fn commit_mirror(&mut self) -> bool {
        let Some(sources) = self.mirror_sources() else {
            self.digest = self.catalog.text("digest-mirror-selection-required");
            return false;
        };
        if self.gesture.mirror.is_none() {
            self.digest = self.catalog.text("digest-mirror-face-required");
            return false;
        }
        let Some(mirror) = self.mirror_world_transform() else {
            self.digest = self.catalog.text("digest-mirror-invalid-offset");
            return false;
        };
        let snapshot = self.document.current();
        let mut next_id = snapshot
            .occurrences()
            .map(|occurrence| occurrence.id().0)
            .max()
            .unwrap_or(0);
        let mut commands = Vec::new();
        for source in &sources {
            let copy = next_id.checked_add(1).and_then(|id| {
                next_id = id;
                mirrored_copy_commands(&snapshot, *source, mirror, OccurrenceId(id)).ok()
            });
            let Some(copy) = copy else {
                self.digest = self.catalog.text("digest-mirror-unavailable");
                return false;
            };
            commands.extend(copy);
        }
        if self
            .apply_batch_with_work_recovery(&CommandBatch::new(commands))
            .is_err()
        {
            return false;
        }
        self.value_box.input.clear();
        self.status_key = "status-mirror-created";
        self.digest = self.catalog.format(
            "digest-mirror-created",
            &BTreeMap::from([("count", sources.len().to_string())]),
        );
        true
    }
}
