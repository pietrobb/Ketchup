//! Saved views evaluated for the Viewer export without activating them.

use super::*;
use ketchup_manufacturing::viewer_export::DesktopCameraContext;
use ketchup_model::document::{InstancePathStep, LocalOccurrenceKey};
use ketchup_rejection::RejectionPhase;

impl KetchupApp {
    /// Evaluate selected scenes without activating them or changing document layers.
    /// Legacy scene pan is interpreted in this export viewport, not an unknown capture viewport.
    pub fn viewer_saved_view_contexts(
        &self,
        ids: &[SavedViewId],
        viewport: Rect,
    ) -> Result<BTreeMap<SavedViewId, DesktopCameraContext>, Rejection> {
        let viewport_points = [f64::from(viewport.width()), f64::from(viewport.height())];
        if viewport_points
            .iter()
            .any(|size| !size.is_finite() || *size <= 0.0)
        {
            return Err(scene_context_error(
                "viewport",
                "The export viewport must have a finite positive size.",
            ));
        }
        let snapshot = self.document.current();
        let selected = ids
            .iter()
            .map(|id| {
                snapshot.saved_view(*id).ok_or_else(|| {
                    scene_context_error(
                        "scenes",
                        "A selected saved scene no longer exists in this document.",
                    )
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        let projection = CanonicalInteractionProjection::from_snapshot(&snapshot);
        let mut local_bounds = BTreeMap::new();
        let mut world_bounds = Vec::new();
        for occurrence in projection.occurrences() {
            let bounds = *local_bounds
                .entry(occurrence.body.definition_id)
                .or_insert_with(|| {
                    self.definition_local_bounds(
                        &snapshot,
                        occurrence.body.definition_id,
                        occurrence.local_box,
                        true,
                    )
                });
            let Some([minimum, maximum]) = bounds else {
                continue;
            };
            // Match the viewport's mesh bounds: transformed vertices, not a rotated local AABB.
            let mesh = definition_mesh_body(&snapshot, occurrence.body.definition_id);
            let bounds = if let Some(mesh) = mesh {
                bounds_of(mesh.vertices_mm.iter().map(|point| {
                    transform_model_point(occurrence.canonical_world_transform, Vec3::from(*point))
                }))
            } else {
                let size = maximum - minimum;
                bounds_of(
                    box_corners(size.x, size.y, size.z)
                        .into_iter()
                        .map(|point| {
                            transform_model_point(
                                occurrence.canonical_world_transform,
                                point + minimum,
                            )
                        }),
                )
            };
            if let Some(bounds) = bounds {
                world_bounds.push((&occurrence.instance_path, bounds));
            }
        }
        let mut contexts = BTreeMap::new();
        for saved in selected {
            let camera = &saved.camera;
            let target = Vec3::new(BOX_WIDTH_MM * 0.5, BOX_DEPTH_MM * 0.5, camera.target_z_mm);
            let radius = world_bounds
                .iter()
                .filter(|(path, _)| scene_path_visible(&snapshot, saved, path))
                .flat_map(|(_, [minimum, maximum])| {
                    let size = *maximum - *minimum;
                    box_corners(size.x, size.y, size.z)
                        .map(|point| length(point + *minimum - target))
                })
                .fold(0.0_f64, f64::max);
            let context = DesktopCameraContext {
                viewport_points,
                orbit_target_xy_mm: [target.x, target.y],
                points_per_mm: camera.zoom * viewport_points[0].min(viewport_points[1]) / 420.0,
                eye_distance_mm: (420.0 / camera.zoom)
                    .max(radius * CAMERA_CLEARANCE)
                    .max(PERSPECTIVE_NEAR_MM),
            };
            context.camera(camera)?;
            contexts.insert(saved.id, context);
        }
        Ok(contexts)
    }
}

/// Whether `path` is shown in `saved`, its layers and hidden components included.
fn scene_path_visible(snapshot: &Snapshot, saved: &SavedView, path: &InstancePath) -> bool {
    let root = snapshot
        .occurrence(path.root_occurrence())
        .expect("projected root exists");
    if !root.visible() || !root.tags().is_disjoint(&saved.hidden_tags) {
        return false;
    }
    let mut definition_id = root.definition_id();
    for step in path.steps() {
        if let InstancePathStep::Occurrence(local_id) = step {
            let local = snapshot
                .local_occurrence(LocalOccurrenceKey {
                    definition_id,
                    local_id: *local_id,
                })
                .expect("projected nested occurrence exists");
            if !local.visible() || !local.tags().is_disjoint(&saved.hidden_tags) {
                return false;
            }
            definition_id = local.definition_id();
        }
    }
    true
}

fn scene_context_error(target: &str, reason: &str) -> Rejection {
    Rejection::new("viewer.scene_context", RejectionPhase::Validation)
        .target(target)
        .reason(reason)
        .fix_hint("Choose existing saved scenes and a nonempty export viewport, then export again.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_model::document::TagId;

    #[test]
    fn viewer_contexts_match_each_activated_scene_without_mutation() {
        let mut app = KetchupApp::new();
        app.camera.projection_mode = ProjectionMode::Perspective;
        app.camera.pan = Vec2::new(47.0, -23.0);
        app.camera.zoom = 1.7;
        let first = app.save_view("Perspective").unwrap();
        app.camera.projection_mode = ProjectionMode::Parallel;
        app.camera.zoom = 0.8;
        app.camera.target_z = 300.0;
        let second = app.save_view("Parallel").unwrap();
        let before = app.camera_view_state();
        let revision = app.document_revision();
        let undo = app.undo_step_count();
        let rect = Rect::from_min_size(Pos2::new(80.0, 50.0), Vec2::new(900.0, 600.0));
        let contexts = app
            .viewer_saved_view_contexts(&[first, second], rect)
            .unwrap();
        assert_eq!(app.camera_view_state(), before);
        assert_eq!(app.document_revision(), revision);
        assert_eq!(app.undo_step_count(), undo);
        assert_eq!(contexts.len(), 2);
        for id in [first, second] {
            assert!(app.activate_saved_view(id));
            let context = &contexts[&id];
            assert!((context.eye_distance_mm - app.camera_distance()).abs() < 1.0e-6);
            assert!((context.points_per_mm - app.view_scale(rect)).abs() < 1.0e-6);
            assert_eq!(
                context.orbit_target_xy_mm,
                [app.camera_target().x, app.camera_target().y]
            );
        }
        assert!(
            app.viewer_saved_view_contexts(&[SavedViewId(u64::MAX)], rect)
                .is_err()
        );
        assert!(
            app.viewer_saved_view_contexts(&[first], Rect::NOTHING)
                .is_err()
        );
    }

    #[test]
    fn viewer_context_uses_saved_layers_even_when_current_layers_hide_distant_geometry() {
        let mut app = KetchupApp::new();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::CreateTag {
                    id: TagId(1),
                    name: "Distant".into(),
                    visible: true,
                },
                CanonicalCommand::CreateDefinition {
                    id: DefinitionId(9),
                    name: "Assembly".into(),
                },
                CanonicalCommand::CreateLocalOccurrence {
                    key: LocalOccurrenceKey {
                        definition_id: DefinitionId(9),
                        local_id: ketchup_model::document::LocalOccurrenceId(1),
                    },
                    definition_id: INITIAL_BOX_DEFINITION,
                    name: "Nested distant shared part".into(),
                    transform: Transform::from_translation(10_000.0, 0.0, 0.0).unwrap(),
                    parent: None,
                    tags: BTreeSet::from([TagId(1)]),
                    visible: true,
                },
                CanonicalCommand::CreateOccurrence {
                    id: OccurrenceId(9),
                    definition_id: DefinitionId(9),
                    name: "Assembly".into(),
                    transform: Transform::identity(),
                    parent: None,
                    tags: BTreeSet::new(),
                    visible: true,
                },
            ]))
            .unwrap();
        let shown = app.save_view("Shown").unwrap();
        app.document
            .apply_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetTagVisibility {
                    id: TagId(1),
                    visible: false,
                },
            ]))
            .unwrap();
        let hidden = app.save_view("Hidden").unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0));
        let revision = app.document_revision();
        let contexts = app
            .viewer_saved_view_contexts(&[shown, hidden], rect)
            .unwrap();
        assert!(contexts[&shown].eye_distance_mm > 20_000.0);
        assert!(contexts[&hidden].eye_distance_mm < 1_000.0);
        assert!(!app.document.current().tag(TagId(1)).unwrap().visible());
        assert_eq!(app.document_revision(), revision);
        for id in [shown, hidden] {
            assert!(app.activate_saved_view(id));
            assert!((contexts[&id].eye_distance_mm - app.camera_distance()).abs() < 1.0e-6);
        }
    }
}
