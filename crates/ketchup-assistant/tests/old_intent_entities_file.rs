//! `fixtures/intent_entities.ketchup` was saved through the intents that created
//! expressions, rules, overrides, bindings, persistent dimensions, spaces,
//! clearance volumes, joints and collections, before those intents were deleted.
//! The file must still open with every one of those entities intact.

use ketchup_geometry::prismatic::{Aabb, CanonicalJoint, JointId};
use ketchup_model::document::{
    CanonicalOverride, CollectionId, DerivedIdentity, DimensionDisplayUnit, DimensionPresentation,
    EvaluatorNodeKind, FeatureId, FeatureParameterBinding, FeatureParameterTarget, NodeId,
    ParameterValueType, PersistentDimension, PersistentDimensionId, PersistentDimensionTarget,
    SlotPath, SlotResolution, SlotSegment,
};
use ketchup_model::persistence;
use ketchup_model::space::{
    CanonicalClearanceVolume, CanonicalSpace, ClearanceOwner, ClearanceSeverity, ClearanceVolumeId,
    SpaceId,
};
use ketchup_model::tolerance::TolerancePolicy;

const RULE_OUTPUTS: NodeId = NodeId(4);
const EXTRUSION: FeatureId = FeatureId(12);

fn output(rule: NodeId, key: &str) -> DerivedIdentity {
    DerivedIdentity::new(
        rule,
        SlotPath::new(vec![SlotSegment::new(rule, "result", key).unwrap()]).unwrap(),
    )
    .unwrap()
}

#[test]
fn a_file_saved_with_the_deleted_intents_still_opens_with_its_entities() {
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/intent_entities.ketchup"
    );
    let loaded = persistence::load_file(path).unwrap();
    let snapshot = loaded.snapshot();

    let expression = snapshot.evaluator_node(NodeId(40)).unwrap();
    assert_eq!(expression.name(), "fixture expression");
    assert!(matches!(
        expression.kind(),
        EvaluatorNodeKind::Expression { source, .. } if source == "$1 + 5"
    ));
    let rule = snapshot.evaluator_node(NodeId(41)).unwrap();
    assert_eq!(rule.name(), "fixture rule");
    assert!(matches!(
        rule.kind(),
        EvaluatorNodeKind::Rule { source, .. } if source == "$2 * 3"
    ));

    assert_eq!(
        snapshot.override_by_id(31),
        Some(
            &CanonicalOverride::new(
                31,
                output(NodeId(30), "left"),
                "offset",
                2.5,
                SlotResolution::Resolved,
            )
            .unwrap()
        )
    );

    let extent =
        FeatureParameterTarget::new(EXTRUSION, "extent.distance", ParameterValueType::Length)
            .unwrap();
    assert_eq!(
        snapshot.feature_parameter_binding(&extent),
        Some(&FeatureParameterBinding {
            target: extent.clone(),
            derived_from: output(RULE_OUTPUTS, "left"),
        })
    );
    assert_eq!(
        snapshot.persistent_dimension(PersistentDimensionId(90)),
        Some(
            &PersistentDimension::new(
                PersistentDimensionId(90),
                "Reviewed height",
                PersistentDimensionTarget::FeatureParameter(extent),
                DimensionPresentation::new(DimensionDisplayUnit::Centimetres, 2).unwrap(),
            )
            .unwrap()
        )
    );

    assert_eq!(
        snapshot.space(SpaceId(70)),
        Some(
            &CanonicalSpace::new(
                SpaceId(70),
                "Service zone",
                Aabb::new([0.0, 0.0, 0.0], [100.0, 200.0, 300.0]).unwrap(),
                Vec::new(),
                Vec::new(),
            )
            .unwrap()
        )
    );
    assert_eq!(
        snapshot.clearance_volume(ClearanceVolumeId(71)),
        Some(
            &CanonicalClearanceVolume::new(
                ClearanceVolumeId(71),
                ClearanceOwner::Space(SpaceId(70)),
                "Door swing",
                Aabb::new([0.0, 0.0, 0.0], [50.0, 60.0, 70.0]).unwrap(),
                TolerancePolicy::new(0.5).unwrap(),
                ClearanceSeverity::Required,
                None,
            )
            .unwrap()
        )
    );
    assert_eq!(
        snapshot.joint(JointId(98)),
        Some(
            &CanonicalJoint::new(
                JointId(98),
                output(RULE_OUTPUTS, "left"),
                output(RULE_OUTPUTS, "right"),
                Aabb::new([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).unwrap(),
            )
            .unwrap()
        )
    );
    assert_eq!(
        snapshot.collection(CollectionId(80)).unwrap().name(),
        "Fixture set"
    );
}
