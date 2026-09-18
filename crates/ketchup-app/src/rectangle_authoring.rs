use super::*;
use ketchup_core::sketch::{
    SketchConstraint, SketchConstraintId, SketchConstraintKind, SketchEntity, SketchEntityId,
    SketchPointKind, SketchPointRef, SketchSpec, WorkplaneSpec, WorkplaneSupport,
};

impl KetchupApp {
    pub(super) fn datum_rectangle_batch(&self, start: Vec3, end: Vec3) -> Option<CommandBatch> {
        let snapshot = self.document.current();
        let frame = self.rectangle_frame(Some(start));
        let origin = Vec3::new(frame.origin_mm[0], frame.origin_mm[1], frame.origin_mm[2]);
        let x = Vec3::new(frame.x_axis[0], frame.x_axis[1], frame.x_axis[2]);
        let y = Vec3::new(frame.y_axis[0], frame.y_axis[1], frame.y_axis[2]);
        let a = [dot(start - origin, x), dot(start - origin, y)];
        let c = [dot(end - origin, x), dot(end - origin, y)];
        let width = (c[0] - a[0]).abs();
        let depth = (c[1] - a[1]).abs();
        if !width.is_finite() || !depth.is_finite() || width <= 0.01 || depth <= 0.01 {
            return None;
        }
        let workplane = FeatureId(
            snapshot
                .features()
                .map(|f| f.id().0)
                .max()
                .unwrap_or(0)
                .checked_add(1)?,
        );
        let sketch = FeatureId(workplane.0.checked_add(1)?);
        let mut commands = Vec::new();
        // Only entering a definition permits editing shared geometry. A datum sketch
        // outside it is a new part, never a modification of an incidental selection.
        let (definition_id, local_frame) = match self.selection.edit_context.last() {
            Some(EditContext::Definition {
                definition_id,
                instance_path,
            }) => {
                let resolved = snapshot.resolve_instance_path(instance_path).ok()?;
                let inverse = resolved.world_transform.rigid_inverse()?;
                let m = inverse.matrix();
                let vector = |p: [f64; 3]| {
                    [
                        m[0] * p[0] + m[1] * p[1] + m[2] * p[2],
                        m[4] * p[0] + m[5] * p[1] + m[6] * p[2],
                        m[8] * p[0] + m[9] * p[1] + m[10] * p[2],
                    ]
                };
                let mut local_origin = vector(frame.origin_mm);
                for (coordinate, translation) in local_origin.iter_mut().zip([m[3], m[7], m[11]]) {
                    *coordinate += translation;
                }
                (
                    *definition_id,
                    WorkplaneFrame::from_axes(
                        local_origin,
                        vector(frame.x_axis),
                        vector(frame.y_axis),
                    )
                    .ok()?,
                )
            }
            context => {
                let definition_id = DefinitionId(
                    snapshot
                        .definitions()
                        .map(|d| d.id().0)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)?,
                );
                let occurrence_id = OccurrenceId(
                    snapshot
                        .occurrences()
                        .map(|o| o.id().0)
                        .max()
                        .unwrap_or(0)
                        .checked_add(1)?,
                );
                let parent = match context {
                    Some(EditContext::Group(id)) => Some(*id),
                    _ => None,
                };
                let transform = match parent {
                    Some(id) => snapshot.world_transform_for_group(id)?.rigid_inverse()?,
                    None => Transform::identity(),
                };
                let name = self.catalog.format(
                    "model-default-box",
                    &BTreeMap::from([("number", definition_id.0.to_string())]),
                );
                let occurrence_name = self.catalog.format(
                    "model-default-occurrence",
                    &BTreeMap::from([("name", name.clone())]),
                );
                commands.push(CanonicalCommand::CreateDefinition {
                    id: definition_id,
                    name,
                });
                commands.push(CanonicalCommand::CreateOccurrence {
                    id: occurrence_id,
                    definition_id,
                    name: occurrence_name,
                    transform,
                    parent,
                    tag: None,
                    visible: true,
                });
                (definition_id, frame)
            }
        };
        let points = [a, [c[0], a[1]], c, [a[0], c[1]]];
        let point = |entity, point| SketchPointRef {
            entity: SketchEntityId(entity),
            point,
        };
        let mut constraints = Vec::new();
        for entity in 1..=4 {
            constraints.push(SketchConstraint {
                id: SketchConstraintId(entity),
                kind: if entity % 2 == 1 {
                    SketchConstraintKind::Horizontal {
                        entity: SketchEntityId(entity),
                    }
                } else {
                    SketchConstraintKind::Vertical {
                        entity: SketchEntityId(entity),
                    }
                },
            });
            constraints.push(SketchConstraint {
                id: SketchConstraintId(entity + 4),
                kind: SketchConstraintKind::Coincident {
                    a: point(entity, SketchPointKind::End),
                    b: point(entity % 4 + 1, SketchPointKind::Start),
                },
            });
        }
        constraints.push(SketchConstraint {
            id: SketchConstraintId(9),
            kind: SketchConstraintKind::FixedPoint {
                point: point(1, SketchPointKind::Start),
                position_mm: a,
            },
        });
        for (entity, size) in [(1, width), (2, depth)] {
            constraints.push(SketchConstraint {
                id: SketchConstraintId(entity + 9),
                kind: SketchConstraintKind::Distance {
                    a: point(entity, SketchPointKind::Start),
                    b: point(entity, SketchPointKind::End),
                    value: Dimension::new(format_height(size), size).ok()?,
                },
            });
        }
        constraints.sort_by_key(|constraint| constraint.id);
        let plane = self.face_workflow_datum();
        let support = if local_frame == WorkplaneFrame::principal(plane) {
            WorkplaneSupport::Principal(plane)
        } else {
            WorkplaneSupport::Free
        };
        commands.push(CanonicalCommand::CreateFeature {
            id: workplane,
            definition_id,
            name: "Rectangle workplane".into(),
            kind: FeatureKind::Workplane(WorkplaneSpec {
                support,
                frame: local_frame,
            }),
        });
        commands.push(CanonicalCommand::CreateFeature {
            id: sketch,
            definition_id,
            name: self.catalog.text("model-default-profile"),
            kind: FeatureKind::Sketch(SketchSpec {
                workplane,
                entities: (0..4)
                    .map(|i| SketchEntity::Line {
                        id: SketchEntityId(i as u64 + 1),
                        start_mm: points[i],
                        end_mm: points[(i + 1) % 4],
                    })
                    .collect(),
                constraints,
            }),
        });
        Some(CommandBatch::new(commands))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rectangle_batch_is_canonical_and_atomic() {
        let mut app = KetchupApp::new();
        let batch = app
            .datum_rectangle_batch(Vec3::ZERO, Vec3::new(30.0, 15.0, 0.0))
            .unwrap();
        app.document.apply_batch(&batch).unwrap();
    }
}
