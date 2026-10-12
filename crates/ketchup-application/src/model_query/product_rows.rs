//! Rows of the product entities that are not geometry: spaces, clearance
//! volumes, joints between derived values or placed parts and persistent
//! dimensions.

use super::{bounded_text, qualified_path};
use ketchup_geometry::prismatic::{Aabb, JointId};
use ketchup_model::document::{
    DerivedIdentity, DimensionReferenceHealth, InstancePath, JointParticipant, ParameterValueType,
    PersistentDimensionId, PersistentDimensionTarget, SlotResolution, Snapshot,
};
use ketchup_model::space::{ClearanceOwner, ClearanceSeverity, ClearanceVolumeId, SpaceId};
use serde_json::{Value, json};

pub(super) fn space_row(snapshot: &Snapshot, id: u64) -> Option<Value> {
    let space = snapshot.space(SpaceId(id))?;
    let ids = |ids: &[SpaceId]| ids.iter().map(|id| id.0).collect::<Vec<_>>();
    Some(json!({"id":id,"purpose":bounded_text(space.purpose()),
        "volume":volume_value(space.volume()),
        "adjacent_to":ids(space.adjacent_to()),"accessible_to":ids(space.accessible_to())}))
}

pub(super) fn clearance_volume_row(snapshot: &Snapshot, id: u64) -> Option<Value> {
    let clearance = snapshot.clearance_volume(ClearanceVolumeId(id))?;
    let owner = match clearance.owner() {
        ClearanceOwner::Occurrence(path) => instance_value(snapshot, path),
        ClearanceOwner::Space(space) => json!({"kind":"space","space_id":space.0,
            "resolved":snapshot.space(*space).is_some()}),
    };
    Some(
        json!({"id":id,"reason":bounded_text(clearance.reason()),"owner":owner,
        "volume":volume_value(clearance.volume()),
        "tolerance_mm":clearance.tolerance().linear_mm(),
        "severity":match clearance.severity() {
            ClearanceSeverity::Advisory => "advisory",
            ClearanceSeverity::Required => "required",
        },
        "derived_from":clearance.derived_from().map(|identity| derived_value(snapshot, identity))}),
    )
}

pub(super) fn joint_row(snapshot: &Snapshot, id: u64) -> Option<Value> {
    let joint = snapshot.joint(JointId(id))?;
    let participant = |participant: &JointParticipant| match participant {
        JointParticipant::Derived(identity) => {
            let mut value = derived_value(snapshot, identity);
            value["kind"] = json!("derived");
            value
        }
        JointParticipant::Part(path) => instance_value(snapshot, path),
    };
    Some(json!({"id":id,
        "participant_a":participant(joint.participant_a()),
        "participant_b":participant(joint.participant_b()),
        "volume":volume_value(joint.volume())}))
}

fn instance_value(snapshot: &Snapshot, path: &InstancePath) -> Value {
    let resolved = qualified_path(snapshot, path, path.steps().len());
    json!({"kind":"instance","root_occurrence_id":path.root_occurrence().0,
        "resolved":resolved.is_some(),
        "instance_path":resolved.map(|(path, _, _)| path)})
}

pub(super) fn dimension_row(snapshot: &Snapshot, id: u64) -> Option<Value> {
    let dimension = snapshot.persistent_dimension(PersistentDimensionId(id))?;
    let projection = snapshot.project_persistent_dimension(dimension.id)?;
    let (status, ambiguous_segment) = match projection.health {
        DimensionReferenceHealth::Resolved => ("resolved", None),
        DimensionReferenceHealth::Ambiguous { segment_index } => ("ambiguous", Some(segment_index)),
        DimensionReferenceHealth::Lost => ("lost", None),
    };
    let target = match &dimension.target {
        PersistentDimensionTarget::FeatureParameter(target) => json!({"kind":"feature_parameter",
            "feature_id":target.feature_id.0,"path":target.path.as_str(),
            "value_type":value_type_name(target.value_type)}),
        PersistentDimensionTarget::DerivedOutput(identity) => json!({"kind":"derived_output",
            "output":derived_value(snapshot, identity)}),
        PersistentDimensionTarget::ExactFeatureParameter {
            definition_id,
            producer_feature_id,
            semantic_role,
            source_element_id,
            path,
            value_type,
        } => json!({"kind":"exact_feature_parameter","definition_id":definition_id.0,
            "producer_feature_id":producer_feature_id.0,
            "semantic_role":bounded_text(semantic_role),
            "source_element_id":bounded_text(source_element_id),
            "path":path.as_str(),"value_type":value_type_name(*value_type)}),
    };
    Some(
        json!({"id":id,"name":bounded_text(&dimension.name),"target":target,
        "presentation":{"unit":dimension.presentation.unit.label(),
            "decimal_places":dimension.presentation.decimal_places},
        "status":status,"ambiguous_segment_index":ambiguous_segment,
        "value_mm":projection.millimetres,"display_value":projection.display_value,
        "display_text":projection.display_text}),
    )
}

fn volume_value(volume: Aabb) -> Value {
    json!({"min_mm":volume.min(),"max_mm":volume.max(),"coordinate_space":"world_mm"})
}

fn derived_value(snapshot: &Snapshot, identity: &DerivedIdentity) -> Value {
    let slot_path = identity
        .slot_path
        .segments()
        .iter()
        .map(|segment| {
            json!({"producer_rule_id":segment.producer_rule_id.0,
                "output_port":segment.output_port,"semantic_key":segment.semantic_key})
        })
        .collect::<Vec<_>>();
    let resolution = match snapshot.resolve_slot(identity) {
        SlotResolution::Resolved => json!({"status":"resolved"}),
        SlotResolution::Ambiguous { segment_index } => {
            json!({"status":"ambiguous","segment_index":segment_index})
        }
        SlotResolution::Lost { .. } => json!({"status":"lost"}),
    };
    json!({"root_rule_node_id":identity.root_rule_node_id.0,"slot_path":slot_path,
        "resolution":resolution})
}

fn value_type_name(value_type: ParameterValueType) -> &'static str {
    match value_type {
        ParameterValueType::Length => "length",
        ParameterValueType::Angle => "angle",
        ParameterValueType::Scalar => "scalar",
    }
}
