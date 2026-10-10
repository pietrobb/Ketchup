//! Conversion of desktop saved views to Viewer scenes: camera, hidden
//! components and display style.

use super::{invalid, view};
use ketchup_geometry::linalg::Vec3;
use ketchup_model::document::{
    DefinitionId, LocalOccurrenceId, LocalOccurrenceKey, OccurrenceId, SavedCamera, SavedView,
    Snapshot,
};
use ketchup_rejection::Rejection;

/// The UI evaluates each saved camera in the export viewport and supplies its
/// actual scale/distance, including scene clearance. Old saved views contain
/// pixel pan but no capture viewport; this cannot recover that missing history.
#[derive(Clone, Copy, Debug)]
pub struct DesktopCameraContext {
    pub viewport_points: [f64; 2],
    pub orbit_target_xy_mm: [f64; 2],
    pub points_per_mm: f64,
    pub eye_distance_mm: f64,
}

impl DesktopCameraContext {
    /// The scene camera of `saved` as evaluated in this export viewport.
    pub fn camera(&self, saved: &SavedCamera) -> Result<view::Camera, Rejection> {
        if self
            .viewport_points
            .iter()
            .any(|v| !v.is_finite() || *v <= 0.0)
            || self.orbit_target_xy_mm.iter().any(|v| !v.is_finite())
            || !self.points_per_mm.is_finite()
            || self.points_per_mm <= 0.0
            || !self.eye_distance_mm.is_finite()
            || self.eye_distance_mm <= 0.0
            || !saved.zoom.is_finite()
            || saved.zoom <= 0.0
            || saved.pan.iter().any(|v| !v.is_finite())
        {
            return Err(invalid(
                "scenes.camera.context",
                "A saved scene needs a finite positive viewport, evaluated scale and eye distance.",
            ));
        }
        let (sy, cy) = saved.yaw_rad.sin_cos();
        let (sp, cp) = saved.pitch_rad.sin_cos();
        let right = Vec3::new(cy, -sy, 0.0);
        let up = Vec3::new(sy * cp, cy * cp, -sp);
        let forward = Vec3::new(-sy * sp, -cy * sp, -cp);
        let mut target = Vec3::new(
            self.orbit_target_xy_mm[0],
            self.orbit_target_xy_mm[1],
            saved.target_z_mm,
        );
        let short = self.viewport_points[0].min(self.viewport_points[1]);
        let short_span_mm = short / self.points_per_mm;
        let projection = if saved.parallel {
            // A parallel screen translation is exactly a target/eye translation.
            target = target - right * (saved.pan[0] / self.points_per_mm)
                + up * (saved.pan[1] / self.points_per_mm);
            view::Projection::Orthographic { short_span_mm }
        } else {
            // Translating the eye instead would change parallax at other depths.
            view::Projection::Perspective {
                short_fov_radians: 2.0 * (short_span_mm / (2.0 * self.eye_distance_mm)).atan(),
                lens_shift_short: [saved.pan[0] / short, -saved.pan[1] / short],
            }
        };
        let camera = view::Camera {
            eye_mm: (target - forward * self.eye_distance_mm).into(),
            target_mm: target.into(),
            up: up.into(),
            projection,
        };
        camera.validate()?;
        Ok(camera)
    }
}

/// A Viewer scene and the notes of anything it could not carry.
pub(super) fn saved_scene(
    snapshot: &Snapshot,
    saved: &SavedView,
    context: &DesktopCameraContext,
    occurrences: &[view::Occurrence],
) -> Result<(view::Scene, Vec<String>), Rejection> {
    if saved
        .hidden_tags
        .iter()
        .any(|id| snapshot.tag(*id).is_none())
    {
        return Err(invalid(
            "scenes.hidden_tags",
            "A saved scene refers to a missing layer.",
        ));
    }
    let mut hidden = Vec::new();
    for occurrence in occurrences {
        if hidden_in_scene(snapshot, saved, &occurrence.path)? {
            hidden.push(occurrence.path.clone());
        }
    }
    let style = if saved.style.contains("wireframe") {
        view::DisplayStyle::Wireframe
    } else if saved.style.contains("edges") {
        view::DisplayStyle::ShadedEdges
    } else {
        view::DisplayStyle::Shaded
    };
    let omitted = saved
        .style
        .iter()
        .filter(|flag| !matches!(flag.as_str(), "wireframe" | "edges"))
        .cloned()
        .collect();
    let scene = view::Scene {
        id: saved.id.0,
        name: saved.name.clone(),
        camera: context.camera(&saved.camera)?,
        hidden,
        style,
        section: saved.section.map(|plane| view::Section {
            normal: plane.normal,
            offset_mm: plane
                .normal
                .iter()
                .zip(plane.point_mm)
                .map(|(n, p)| n * p)
                .sum(),
        }),
        // Desktop saved views do not yet carry a dimension visibility list.
        visible_dimensions: Vec::new(),
    };
    Ok((scene, omitted))
}

/// Whether the saved view hides this occurrence or one of its layers.
fn hidden_in_scene(
    snapshot: &Snapshot,
    saved: &SavedView,
    path: &view::InstancePath,
) -> Result<bool, Rejection> {
    let root = snapshot
        .occurrence(OccurrenceId(path.root_occurrence_id))
        .ok_or_else(|| invalid("scenes.hidden", "An exported root occurrence is missing."))?;
    let mut hidden = !root.visible() || !root.tags().is_disjoint(&saved.hidden_tags);
    let mut definition = root.definition_id();
    for step in &path.steps {
        match step {
            view::PathStep::Group { .. } => {}
            view::PathStep::Occurrence {
                owner_definition_id,
                local_id,
            } => {
                if DefinitionId(*owner_definition_id) != definition {
                    return Err(invalid(
                        "scenes.hidden",
                        "An exported path has the wrong owner definition.",
                    ));
                }
                let local = snapshot
                    .local_occurrence(LocalOccurrenceKey {
                        definition_id: definition,
                        local_id: LocalOccurrenceId(*local_id),
                    })
                    .ok_or_else(|| {
                        invalid("scenes.hidden", "An exported nested occurrence is missing.")
                    })?;
                hidden |= !local.visible() || !local.tags().is_disjoint(&saved.hidden_tags);
                definition = local.definition_id();
            }
        }
    }
    Ok(hidden)
}
