//! The section plane: cuts the viewport open so the inside of a model can be seen.
//! It changes only what is painted, never the document or its program. Faces seen
//! from behind through the cut are painted in the section colour, which marks the cut.

use crate::*;

/// The colour of the inside of a cut solid.
pub(crate) const SECTION_COLOR: Color32 = Color32::from_rgb(214, 74, 64);

fn distance(section: &SectionPlane, point: Vec3) -> f64 {
    section.signed_distance([point.x, point.y, point.z])
}

/// The part of a planar polygon on the kept side of `section`; `None` when nothing is left.
pub(crate) fn clip_polygon(section: &SectionPlane, points: &[Vec3]) -> Option<Vec<Vec3>> {
    let distances = points
        .iter()
        .map(|point| distance(section, *point))
        .collect::<Vec<_>>();
    let mut kept = Vec::with_capacity(points.len() + 1);
    for index in 0..points.len() {
        let next = (index + 1) % points.len();
        let (from, to) = (distances[index], distances[next]);
        if from <= 0.0 {
            kept.push(points[index]);
        }
        if (from <= 0.0) != (to <= 0.0) {
            kept.push(points[index].lerp(points[next], from / (from - to)));
        }
    }
    (kept.len() >= 3).then_some(kept)
}

/// The part of a segment on the kept side of `section`.
pub(crate) fn clip_segment(section: &SectionPlane, [from, to]: [Vec3; 2]) -> Option<[Vec3; 2]> {
    let (from_distance, to_distance) = (distance(section, from), distance(section, to));
    match (from_distance <= 0.0, to_distance <= 0.0) {
        (true, true) => Some([from, to]),
        (false, false) => None,
        (true, false) => Some([
            from,
            from.lerp(to, from_distance / (from_distance - to_distance)),
        ]),
        (false, true) => Some([
            from.lerp(to, from_distance / (from_distance - to_distance)),
            to,
        ]),
    }
}

/// Azimuth and elevation of a unit normal, in degrees.
fn normal_angles(normal: [f64; 3]) -> (f64, f64) {
    (
        normal[1].atan2(normal[0]).to_degrees(),
        normal[2].clamp(-1.0, 1.0).asin().to_degrees(),
    )
}

fn normal_from_angles(azimuth_deg: f64, elevation_deg: f64) -> [f64; 3] {
    let (azimuth, elevation) = (azimuth_deg.to_radians(), elevation_deg.to_radians());
    [
        elevation.cos() * azimuth.cos(),
        elevation.cos() * azimuth.sin(),
        elevation.sin(),
    ]
}

/// Distance of the plane from the origin along its normal.
fn offset_mm(section: &SectionPlane) -> f64 {
    (0..3)
        .map(|axis| section.point_mm[axis] * section.normal[axis])
        .sum()
}

/// The plane with this normal (any length) at `offset_mm` from the origin along it.
pub(crate) fn plane_at(normal: [f64; 3], offset_mm: f64) -> Option<SectionPlane> {
    let unit = SectionPlane::new([0.0; 3], normal)?.normal;
    SectionPlane::new(unit.map(|value| value * offset_mm), unit)
}

/// The section as agents read it: unit normal and offset, or null when closed.
pub(crate) fn section_json(section: Option<SectionPlane>) -> serde_json::Value {
    section.map_or(
        serde_json::Value::Null,
        |section| serde_json::json!({"normal":section.normal,"offset_mm":offset_mm(&section)}),
    )
}

impl KetchupApp {
    #[must_use]
    pub fn section(&self) -> Option<SectionPlane> {
        self.section
    }

    /// Open, move or close (`None`) the section plane. Only the viewport changes.
    pub fn set_section(&mut self, section: Option<SectionPlane>) {
        self.section = section;
        self.digest = match section {
            Some(section) => self.catalog.format(
                "digest-section-shown",
                &BTreeMap::from([("offset", format!("{:.0}", offset_mm(&section)))]),
            ),
            None => self.catalog.text("digest-section-hidden"),
        };
    }

    /// A horizontal cut through the middle of what is shown, looking down into the model.
    pub fn open_section(&mut self) {
        let bounds = self.active_frame_bounds();
        let middle_z = if bounds.is_empty() {
            0.0
        } else {
            let low = bounds.iter().map(|(low, _)| low.z).fold(f64::MAX, f64::min);
            let high = bounds
                .iter()
                .map(|(_, high)| high.z)
                .fold(f64::MIN, f64::max);
            (low + high) / 2.0
        };
        self.set_section(plane_at([0.0, 0.0, 1.0], middle_z));
    }

    /// Whether faces turned away from the camera are painted: only through an open cut.
    pub(crate) fn paints_back_faces(&self) -> bool {
        self.section.is_some()
    }

    /// The colour of a face, or the section colour when it is seen from behind through the cut.
    pub(crate) fn section_face_color(&self, front: bool, color: Color32) -> Color32 {
        if front || self.section.is_none() {
            color
        } else {
            SECTION_COLOR
        }
    }

    /// The projected part of a world-space face the section keeps, if it has area.
    pub(crate) fn section_polygon(
        &self,
        points_mm: &[Vec3],
        rect: Rect,
    ) -> Option<ProjectedPolygon> {
        let clipped;
        let points_mm = match &self.section {
            Some(section) => {
                clipped = clip_polygon(section, points_mm)?;
                clipped.as_slice()
            }
            None => points_mm,
        };
        let projected = points_mm
            .iter()
            .map(|point| self.project(*point, rect))
            .collect::<Vec<_>>();
        if !projected_polygon_has_area(&projected) {
            return None;
        }
        Some(match projected.as_slice() {
            [a, b, c] => ProjectedPolygon::Triangle([*a, *b, *c]),
            [a, b, c, d] => ProjectedPolygon::Quad([*a, *b, *c, *d]),
            _ => ProjectedPolygon::Polygon(projected),
        })
    }

    /// The part of a world-space edge the section keeps.
    pub(crate) fn section_edge(&self, points_mm: [Vec3; 2]) -> Option<[Vec3; 2]> {
        match &self.section {
            Some(section) => clip_segment(section, points_mm),
            None => Some(points_mm),
        }
    }

    /// The plane as the GPU scene clips it: unit normal and offset, all zero when closed.
    pub(crate) fn section_clip_plane(&self) -> [f32; 4] {
        self.section.map_or([0.0; 4], |section| {
            let [x, y, z] = section.normal.map(|value| value as f32);
            [x, y, z, offset_mm(&section) as f32]
        })
    }

    /// Section controls under the saved views: open/close, direction, position and turn.
    pub(crate) fn section_ui(&mut self, ui: &mut egui::Ui) {
        ui.separator();
        section_header(ui, self.palette(), &self.catalog.text("section"));
        let mut open = self.section.is_some();
        let toggle = ui.checkbox(&mut open, self.catalog.text("section-open"));
        if toggle.changed() {
            if open {
                self.open_section();
            } else {
                self.set_section(None);
            }
        }
        let Some(section) = self.section else {
            return;
        };
        let (mut azimuth, mut elevation) = normal_angles(section.normal);
        let mut offset = offset_mm(&section);
        let mut normal = section.normal;
        let mut changed = false;
        let mut turned = false;
        ui.horizontal(|ui| {
            for (label, axis) in [
                ("X", [1.0, 0.0, 0.0]),
                ("Y", [0.0, 1.0, 0.0]),
                ("Z", [0.0, 0.0, 1.0]),
            ] {
                let button = ui.selectable_label(normal == axis, label);
                name_widget(
                    &button,
                    true,
                    &self.catalog.format(
                        "section-axis",
                        &BTreeMap::from([("axis", label.to_owned())]),
                    ),
                );
                if button.clicked() {
                    normal = axis;
                    changed = true;
                }
            }
            let flip = icon_button(ui, true, "↕", &self.catalog.text("section-flip"));
            if flip.clicked() {
                normal = normal.map(|value| -value);
                offset = -offset;
                changed = true;
            }
        });
        for (key, value, speed, suffix) in [
            ("section-offset", &mut offset, 10.0, " mm"),
            ("section-azimuth", &mut azimuth, 1.0, "°"),
            ("section-elevation", &mut elevation, 1.0, "°"),
        ] {
            let label = self.catalog.text(key);
            ui.horizontal(|ui| {
                ui.label(label.as_str());
                let drag = ui.add(egui::DragValue::new(value).speed(speed).suffix(suffix));
                drag.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::DragValue, true, &label)
                });
                if drag.changed() {
                    changed = true;
                    turned |= key != "section-offset";
                }
            });
        }
        if changed {
            if turned {
                normal = normal_from_angles(azimuth, elevation.clamp(-90.0, 90.0));
            }
            if let Some(section) = plane_at(normal, offset) {
                self.set_section(Some(section));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cut_at(z: f64) -> SectionPlane {
        SectionPlane::new([0.0, 0.0, z], [0.0, 0.0, 1.0]).unwrap()
    }

    #[test]
    fn a_polygon_keeps_only_the_side_behind_the_normal() {
        let square = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 0.0),
            Vec3::new(10.0, 0.0, 10.0),
            Vec3::new(0.0, 0.0, 10.0),
        ];
        assert_eq!(clip_polygon(&cut_at(20.0), &square).unwrap(), square);
        assert_eq!(clip_polygon(&cut_at(-1.0), &square), None);
        let lower = clip_polygon(&cut_at(4.0), &square).unwrap();
        assert_eq!(
            lower,
            [
                Vec3::new(0.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 0.0),
                Vec3::new(10.0, 0.0, 4.0),
                Vec3::new(0.0, 0.0, 4.0),
            ]
        );
    }

    #[test]
    fn a_segment_is_cut_where_it_crosses_the_plane() {
        let edge = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 10.0)];
        assert_eq!(
            clip_segment(&cut_at(2.5), edge),
            Some([edge[0], Vec3::new(0.0, 0.0, 2.5)])
        );
        assert_eq!(
            clip_segment(&cut_at(2.5), [edge[1], edge[0]]),
            Some([Vec3::new(0.0, 0.0, 2.5), edge[0]])
        );
        assert_eq!(clip_segment(&cut_at(-1.0), edge), None);
        assert_eq!(clip_segment(&cut_at(10.0), edge), Some(edge));
    }

    #[test]
    fn angles_and_offset_round_trip_through_the_plane() {
        let normal = normal_from_angles(30.0, 20.0);
        let (azimuth, elevation) = normal_angles(normal);
        assert!((azimuth - 30.0).abs() < 1e-9 && (elevation - 20.0).abs() < 1e-9);
        let plane = plane_at(normal, 750.0).unwrap();
        assert!((offset_mm(&plane) - 750.0).abs() < 1e-9);
    }
}
