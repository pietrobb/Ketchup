use super::*;

pub(super) fn supported_planar_face_frame(
    product: &ProductModel,
    reference: &BodySubshapeRef,
) -> Option<WorkplaneFrame> {
    let snapshot = Snapshot {
        revision_id: 0,
        product: Arc::new(product.clone()),
    };
    let graph = crate::exact_brep_graph::ExactBRepGraph::from_snapshot(
        &snapshot,
        reference.definition_id,
        reference.producer_feature_id,
    )
    .ok()?;
    if !graph.names_durable_reference(reference) {
        return None;
    }
    graph.extrusion_face_frame(reference.profile_feature_id.0, &reference.semantic_role)
}

/// A workplane lies on a face when it has the face's axes and its origin is in
/// the face plane; where in the plane the origin sits is the author's choice.
pub(super) fn lies_on_planar_face(
    frame: WorkplaneFrame,
    face: WorkplaneFrame,
    tolerance_mm: f64,
) -> bool {
    let same = |left: [f64; 3], right: [f64; 3]| {
        (0..3).all(|axis| (left[axis] - right[axis]).abs() <= tolerance_mm)
    };
    let offset = (0..3)
        .map(|axis| (frame.origin_mm[axis] - face.origin_mm[axis]) * face.normal[axis])
        .sum::<f64>();
    same(frame.x_axis, face.x_axis)
        && same(frame.y_axis, face.y_axis)
        && same(frame.normal, face.normal)
        && offset.abs() <= tolerance_mm
}

/// Moves a workplane onto its face after the face moved: the in-plane origin
/// is kept when the face only moved along its normal, otherwise the face frame
/// is adopted.
pub(super) fn frame_on_planar_face(
    stored: WorkplaneFrame,
    face: WorkplaneFrame,
    tolerance_mm: f64,
) -> WorkplaneFrame {
    let offset = (0..3)
        .map(|axis| (face.origin_mm[axis] - stored.origin_mm[axis]) * face.normal[axis])
        .sum::<f64>();
    let moved = WorkplaneFrame {
        origin_mm: [0, 1, 2].map(|axis| stored.origin_mm[axis] + face.normal[axis] * offset),
        ..stored
    };
    if lies_on_planar_face(moved, face, tolerance_mm) {
        moved
    } else {
        face
    }
}

/// `feature` with `kind` in place of its own, everything else kept.
fn with_kind(feature: &Feature, kind: FeatureKind) -> Arc<Feature> {
    Arc::new(Feature {
        kind,
        ..feature.clone()
    })
}

pub(super) fn set_planar_face_reference_health(
    product: &mut ProductModel,
    lineage_digest: &str,
    health: WorkplaneSupportHealth,
) {
    let updated = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::Workplane(
                spec @ WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace { reference, .. },
                    ..
                },
            ) if reference.lineage_digest == lineage_digest => {
                let mut spec = spec.clone();
                spec.support = WorkplaneSupport::PlanarFace {
                    reference: reference.clone(),
                    health,
                };
                Some(with_kind(feature, FeatureKind::Workplane(spec)))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for feature in updated {
        product.features.insert(feature.id, feature);
    }
}

pub(super) fn rebind_planar_face_reference(
    product: &mut ProductModel,
    reference: &BodySubshapeRef,
) -> Result<(), CanonicalError> {
    let updated = product
        .features
        .values()
        .filter_map(|feature| match &feature.kind {
            FeatureKind::Workplane(
                spec @ WorkplaneSpec {
                    support:
                        WorkplaneSupport::PlanarFace {
                            reference: support, ..
                        },
                    ..
                },
            ) if support.lineage_digest == reference.lineage_digest => {
                let mut spec = spec.clone();
                spec.support = WorkplaneSupport::PlanarFace {
                    reference: Box::new(reference.clone()),
                    health: WorkplaneSupportHealth::Resolved,
                };
                Some(with_kind(feature, FeatureKind::Workplane(spec)))
            }
            FeatureKind::Pad(spec)
                if spec
                    .operation
                    .support()
                    .is_some_and(|support| support.lineage_digest == reference.lineage_digest) =>
            {
                let mut spec = spec.clone();
                if let PadOperation::Cut { start, .. } = &mut spec.operation {
                    *start = CutStart::Support(Box::new(reference.clone()));
                }
                Some(with_kind(feature, FeatureKind::Pad(spec)))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    for feature in updated {
        product.features.insert(feature.id, feature);
    }
    refresh_supported_planar_face_frames(product, None)
}

pub(super) fn refresh_supported_planar_face_frames(
    product: &mut ProductModel,
    previous: Option<&Snapshot>,
) -> Result<(), CanonicalError> {
    let tolerance_mm = product.tolerance.linear_mm();
    let updates = product
        .features
        .values()
        .filter_map(|feature| {
            let FeatureKind::Workplane(
                spec @ WorkplaneSpec {
                    support: WorkplaneSupport::PlanarFace { reference, .. },
                    ..
                },
            ) = &feature.kind
            else {
                return None;
            };
            if let Some(previous) = previous {
                let anchor_existed = matches!(
                    previous.feature(feature.id).map(Feature::kind),
                    Some(FeatureKind::Workplane(WorkplaneSpec {
                        support: WorkplaneSupport::PlanarFace { reference: prior, .. },
                        ..
                    })) if prior.lineage_digest == reference.lineage_digest
                );
                let producer_changed = previous
                    .feature(reference.producer_feature_id)
                    .zip(product.features.get(&reference.producer_feature_id))
                    .is_some_and(|(before, after)| before.kind() != &after.kind);
                if !anchor_existed || !producer_changed {
                    return None;
                }
            }
            Some(
                supported_planar_face_frame(product, reference)
                    .ok_or(CanonicalError::Sketch(
                        SketchError::InvalidPlanarFaceSupport,
                    ))
                    .map(|frame| {
                        let mut spec = spec.clone();
                        spec.frame = frame_on_planar_face(spec.frame, frame, tolerance_mm);
                        with_kind(feature, FeatureKind::Workplane(spec))
                    }),
            )
        })
        .collect::<Vec<_>>();
    for feature in updates {
        let feature = feature?;
        product.features.insert(feature.id, feature);
    }
    Ok(())
}
