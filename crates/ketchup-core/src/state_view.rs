use crate::assembly::{AssemblyDofStatus, AssemblyMateKind, AssemblyReferenceHealth};
use crate::assembly_joint::AssemblyJointKind;
use crate::document::{
    EvaluationReport, EvaluationStatus, EvaluatorNodeKind, InstancePathStep, ParameterValueType,
    Snapshot, Transform, UnitSystem,
};
use crate::graph::{OverrideMergePolicy, RuleOutput, SlotResolution, SlotSegment, ValueType};
use crate::mechanical_contract::MechanicalConditionKind;
use crate::mechanical_coupling::AssemblyTransmissionKind;
use crate::space::{ClearanceOwner, ClearanceSeverity};
use crate::validation::ValidationReport;
use std::fmt::Write;

pub const COMPLETE_STATE_VIEW_V1: &str = "ketchup.state-view.complete.v1";
pub const AGENT_STATE_VIEW_V1: &str = "ketchup.state-view.agent.v1";
pub const SEMANTIC_ENCODER_V1: &str = "ketchup.semantic-state.v1";

const fn parameter_value_type_label(value_type: ParameterValueType) -> &'static str {
    match value_type {
        ParameterValueType::Length => "length",
        ParameterValueType::Angle => "angle",
        ParameterValueType::Scalar => "scalar",
    }
}

fn write_feature_extent(
    output: &mut String,
    feature_id: u64,
    extent: &crate::sketch::FeatureExtent,
) {
    use crate::sketch::{FeatureExtent, FeatureExtentEnd};

    fn write_end(output: &mut String, prefix: &str, end: &FeatureExtentEnd) {
        match end {
            FeatureExtentEnd::Blind(distance) => {
                writeln!(output, "{prefix}.mode=blind").unwrap();
                writeln!(output, "{prefix}.source={:?}", distance.source_token()).unwrap();
                writeln!(
                    output,
                    "{prefix}.f64_bits={:016x}",
                    distance.millimetres().to_bits()
                )
                .unwrap();
            }
            FeatureExtentEnd::ThroughAll => writeln!(output, "{prefix}.mode=through_all").unwrap(),
            FeatureExtentEnd::UpToFace(reference) => {
                writeln!(output, "{prefix}.mode=up_to_face").unwrap();
                writeln!(output, "{prefix}.reference={reference:?}").unwrap();
            }
        }
    }

    let prefix = format!("feature.{feature_id}.extent");
    match extent {
        FeatureExtent::Blind(distance) => {
            writeln!(output, "{prefix}.mode=blind").unwrap();
            writeln!(output, "{prefix}.source={:?}", distance.source_token()).unwrap();
            writeln!(
                output,
                "{prefix}.f64_bits={:016x}",
                distance.millimetres().to_bits()
            )
            .unwrap();
        }
        FeatureExtent::ThroughAll => writeln!(output, "{prefix}.mode=through_all").unwrap(),
        FeatureExtent::UpToFace(reference) => {
            writeln!(output, "{prefix}.mode=up_to_face").unwrap();
            writeln!(output, "{prefix}.reference={reference:?}").unwrap();
        }
        FeatureExtent::Symmetric(distance) => {
            writeln!(output, "{prefix}.mode=symmetric").unwrap();
            writeln!(output, "{prefix}.source={:?}", distance.source_token()).unwrap();
            writeln!(
                output,
                "{prefix}.f64_bits={:016x}",
                distance.millimetres().to_bits()
            )
            .unwrap();
        }
        FeatureExtent::Bidirectional { along, opposite } => {
            writeln!(output, "{prefix}.mode=bidirectional").unwrap();
            write_end(output, &format!("{prefix}.along"), along);
            write_end(output, &format!("{prefix}.opposite"), opposite);
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticState {
    complete: String,
    agent: String,
}

#[must_use]
pub fn encode_semantic_state(snapshot: &Snapshot) -> SemanticState {
    encode_semantic_state_with_results(snapshot, None, None)
}

#[must_use]
pub fn encode_semantic_state_with_evaluation(
    snapshot: &Snapshot,
    evaluation: Option<&EvaluationReport>,
) -> SemanticState {
    encode_semantic_state_with_results(snapshot, evaluation, None)
}

#[must_use]
pub fn encode_semantic_state_with_results(
    snapshot: &Snapshot,
    evaluation: Option<&EvaluationReport>,
    validation: Option<&ValidationReport>,
) -> SemanticState {
    let mut complete = String::new();
    let mut agent = String::new();
    write_header(&mut complete, COMPLETE_STATE_VIEW_V1, snapshot);
    write_header(&mut agent, AGENT_STATE_VIEW_V1, snapshot);
    writeln!(
        agent,
        "summary.counts=evaluator_nodes:{},overrides:{},parameter_bindings:{},spaces:{},clearance_volumes:{},persistent_dimensions:{},tags:{},collections:{},definitions:{},features:{},occurrences:{},grounded_occurrences:{},assembly_mates:{},groups:{},local_groups:{},local_occurrences:{}",
        snapshot.evaluator_node_count(), snapshot.overrides().count(),
        snapshot.feature_parameter_bindings().count(), snapshot.spaces().count(), snapshot.clearance_volumes().count(), snapshot.persistent_dimensions().count(), snapshot.tags().count(), snapshot.collections().count(),
        snapshot.definitions().count(), snapshot.features().count(), snapshot.occurrences().count(), snapshot.grounded_occurrences().count(), snapshot.assembly_mates().count(), snapshot.groups().count(),
        snapshot.local_groups().count(), snapshot.local_occurrences().count()
    ).unwrap();

    let evaluation_is_current = evaluation.is_some_and(|report| {
        report.document_id == Some(snapshot.document_id())
            && report.revision_id == Some(snapshot.revision_id())
            && report.canonical_digest.as_deref() == Some(snapshot.canonical_digest().as_str())
    });
    if let Some(report) = evaluation {
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "evaluation.evaluator={:?}",
                report.identity.evaluator
            )
            .unwrap();
            writeln!(output, "evaluation.schema={:?}", report.identity.schema).unwrap();
            writeln!(
                output,
                "evaluation.tolerance={:?}",
                report.identity.tolerance
            )
            .unwrap();
            writeln!(output, "evaluation.backend={:?}", report.identity.backend).unwrap();
            writeln!(
                output,
                "evaluation.document_id={}",
                report
                    .document_id
                    .map_or_else(|| "none".to_owned(), |id| id.0.to_string())
            )
            .unwrap();
            writeln!(
                output,
                "evaluation.revision={}",
                report
                    .revision_id
                    .map_or_else(|| "none".to_owned(), |id| id.to_string())
            )
            .unwrap();
            writeln!(
                output,
                "evaluation.canonical_digest={}",
                report.canonical_digest.as_deref().unwrap_or("none")
            )
            .unwrap();
            writeln!(output, "evaluation.current={evaluation_is_current}").unwrap();
            writeln!(
                output,
                "evaluation.recomputed_nodes={}",
                id_list(report.recomputed_nodes.iter().map(|id| id.0))
            )
            .unwrap();
        }
    } else {
        writeln!(complete, "evaluation=not_supplied").unwrap();
        writeln!(agent, "evaluation=not_supplied").unwrap();
    }

    if let Some(report) = validation {
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "validation.evidence.exact_count={}",
                report.evidence_counts.exact
            )
            .unwrap();
            writeln!(
                output,
                "validation.evidence.tolerant_count={}",
                report.evidence_counts.tolerant
            )
            .unwrap();
        }
    }

    for node in snapshot.evaluator_nodes() {
        let id = node.id().0;
        writeln!(complete, "evaluator_node.{id}.name={:?}", node.name()).unwrap();
        writeln!(complete, "evaluator_node.{id}.kind={}", node.kind().label()).unwrap();
        writeln!(
            complete,
            "evaluator_node.{id}.source={:?}",
            node.kind().source()
        )
        .unwrap();
        writeln!(
            complete,
            "evaluator_node.{id}.dependencies={}",
            id_list(node.dependencies().iter().map(|value| value.0))
        )
        .unwrap();
        write_ports(&mut complete, id, "input", node.input_ports());
        write_ports(&mut complete, id, "output", node.output_ports());
        if let Some(dimension) = node.dimension() {
            writeln!(
                complete,
                "evaluator_node.{id}.dimension.source={:?}",
                dimension.source_token()
            )
            .unwrap();
            writeln!(
                complete,
                "evaluator_node.{id}.dimension.f64_bits={:016x}",
                dimension.millimetres().to_bits()
            )
            .unwrap();
        }
        for parameter in node.allowed_parameters() {
            writeln!(
                complete,
                "evaluator_node.{id}.override_parameter.{:?}.merge_policy={}",
                parameter.name(),
                match parameter.merge_policy() {
                    OverrideMergePolicy::Replace => "replace",
                }
            )
            .unwrap();
        }
        if let EvaluatorNodeKind::Rule { outputs, .. } = node.kind() {
            for (index, path) in output_paths(outputs).into_iter().enumerate() {
                writeln!(
                    complete,
                    "evaluator_node.{id}.rule_output.{index}.slot_path={}",
                    slot_path(&path)
                )
                .unwrap();
            }
        }
        if evaluation_is_current
            && let Some(result) = evaluation.and_then(|report| report.node(node.id()))
        {
            writeln!(
                complete,
                "evaluator_node.{id}.evaluation.input_digest={}",
                result.input_digest
            )
            .unwrap();
            writeln!(
                complete,
                "evaluator_node.{id}.evaluation.result_digest={}",
                result.result_digest
            )
            .unwrap();
            writeln!(
                complete,
                "evaluator_node.{id}.evaluation.status={}",
                status(result.status.clone())
            )
            .unwrap();
        }
        writeln!(
            agent,
            "evaluator_node.{id}=name:{:?},kind:{},source:{:?},depends_on:{}",
            node.name(),
            node.kind().label(),
            node.kind().source(),
            id_list(node.dependencies().iter().map(|value| value.0))
        )
        .unwrap();
    }

    if evaluation_is_current && let Some(report) = evaluation {
        for (index, (identity, result)) in report.outputs.iter().enumerate() {
            writeln!(
                complete,
                "derived_output.{index}.root={}",
                identity.root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                complete,
                "derived_output.{index}.slot_path={}",
                slot_path(identity.slot_path.segments())
            )
            .unwrap();
            writeln!(
                complete,
                "derived_output.{index}.value.f64_bits={:016x}",
                result.value.to_bits()
            )
            .unwrap();
            writeln!(
                complete,
                "derived_output.{index}.input_digest={}",
                result.input_digest
            )
            .unwrap();
            writeln!(
                complete,
                "derived_output.{index}.result_digest={}",
                result.result_digest
            )
            .unwrap();
        }
    }

    for value in snapshot.overrides() {
        let health = match value.health {
            crate::graph::SlotResolution::Resolved => "resolved".to_owned(),
            crate::graph::SlotResolution::Ambiguous { segment_index } => {
                format!("ambiguous:{segment_index}")
            }
            crate::graph::SlotResolution::Lost { segment_index } => format!("lost:{segment_index}"),
        };
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "override.{}.target.root={}",
                value.id, value.target.root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                output,
                "override.{}.target.slot_path={}",
                value.id,
                slot_path(value.target.slot_path.segments())
            )
            .unwrap();
            writeln!(
                output,
                "override.{}.parameter={:?}",
                value.id, value.parameter
            )
            .unwrap();
            writeln!(
                output,
                "override.{}.value.f64_bits={:016x}",
                value.id, value.value_bits
            )
            .unwrap();
            writeln!(output, "override.{}.health={health}", value.id).unwrap();
        }
    }

    for binding in snapshot.feature_parameter_bindings() {
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "parameter_binding.{}.{}.value_type={}",
                binding.target.feature_id.0,
                binding.target.path.as_str(),
                parameter_value_type_label(binding.target.value_type)
            )
            .unwrap();
            writeln!(
                output,
                "parameter_binding.{}.{}.derived_from.root={}",
                binding.target.feature_id.0,
                binding.target.path.as_str(),
                binding.derived_from.root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                output,
                "parameter_binding.{}.{}.derived_from.slot_path={}",
                binding.target.feature_id.0,
                binding.target.path.as_str(),
                slot_path(binding.derived_from.slot_path.segments())
            )
            .unwrap();
        }
    }

    for dimension in snapshot.persistent_dimensions() {
        let target = match &dimension.target {
            crate::document::PersistentDimensionTarget::FeatureParameter(target) => {
                format!("feature:{}:{}", target.feature_id.0, target.path.as_str())
            }
            crate::document::PersistentDimensionTarget::DerivedOutput(target) => format!(
                "slot:{}:{}",
                target.root_rule_node_id.0,
                slot_path(target.slot_path.segments())
            ),
            crate::document::PersistentDimensionTarget::ExactFeatureParameter {
                definition_id,
                producer_feature_id,
                semantic_role,
                source_element_id,
                path,
                value_type,
            } => format!(
                "exact:{}:{}:{semantic_role:?}:{source_element_id:?}:{}:{value_type:?}",
                definition_id.0,
                producer_feature_id.0,
                path.as_str()
            ),
        };
        let value_type = match &dimension.target {
            crate::document::PersistentDimensionTarget::FeatureParameter(target) => {
                Some(target.value_type)
            }
            crate::document::PersistentDimensionTarget::ExactFeatureParameter {
                value_type,
                ..
            } => Some(*value_type),
            crate::document::PersistentDimensionTarget::DerivedOutput(_) => None,
        };
        let projection = snapshot
            .project_persistent_dimension(dimension.id)
            .expect("canonical persistent dimension projects");
        let health = match projection.health {
            crate::document::DimensionReferenceHealth::Resolved => "resolved".to_owned(),
            crate::document::DimensionReferenceHealth::Ambiguous { segment_index } => {
                format!("ambiguous:{segment_index}")
            }
            crate::document::DimensionReferenceHealth::Lost => "lost".to_owned(),
        };
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "persistent_dimension.{}.name={:?}",
                dimension.id.0, dimension.name
            )
            .unwrap();
            writeln!(
                output,
                "persistent_dimension.{}.target={target}",
                dimension.id.0
            )
            .unwrap();
            if let Some(value_type) = value_type {
                writeln!(
                    output,
                    "persistent_dimension.{}.value_type={}",
                    dimension.id.0,
                    parameter_value_type_label(value_type)
                )
                .unwrap();
            }
            writeln!(
                output,
                "persistent_dimension.{}.unit={}",
                dimension.id.0,
                dimension.presentation.unit.label()
            )
            .unwrap();
            writeln!(
                output,
                "persistent_dimension.{}.decimal_places={}",
                dimension.id.0, dimension.presentation.decimal_places
            )
            .unwrap();
            writeln!(
                output,
                "persistent_dimension.{}.health={health}",
                dimension.id.0
            )
            .unwrap();
            writeln!(
                output,
                "persistent_dimension.{}.millimetres.f64_bits={}",
                dimension.id.0,
                projection.millimetres.map_or_else(
                    || "unresolved".to_owned(),
                    |value| format!("{:016x}", value.to_bits())
                )
            )
            .unwrap();
            writeln!(
                output,
                "persistent_dimension.{}.display={:?}",
                dimension.id.0,
                projection.display_text.as_deref().unwrap_or("unresolved")
            )
            .unwrap();
        }
    }

    for joint in snapshot.joints() {
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "joint.{}.participant_a.root={}",
                joint.id().0,
                joint.participant_a().root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                output,
                "joint.{}.participant_a.slot_path={}",
                joint.id().0,
                slot_path(joint.participant_a().slot_path.segments())
            )
            .unwrap();
            writeln!(
                output,
                "joint.{}.participant_b.root={}",
                joint.id().0,
                joint.participant_b().root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                output,
                "joint.{}.participant_b.slot_path={}",
                joint.id().0,
                slot_path(joint.participant_b().slot_path.segments())
            )
            .unwrap();
            let min = joint.volume().min();
            let max = joint.volume().max();
            writeln!(
                output,
                "joint.{}.aabb.f64_bits={:016x},{:016x},{:016x}:{:016x},{:016x},{:016x}",
                joint.id().0,
                min[0].to_bits(),
                min[1].to_bits(),
                min[2].to_bits(),
                max[0].to_bits(),
                max[1].to_bits(),
                max[2].to_bits()
            )
            .unwrap();
        }
    }

    for space in snapshot.spaces() {
        let adjacent = id_list(space.adjacent_to().iter().map(|id| id.0));
        let accessible = id_list(space.accessible_to().iter().map(|id| id.0));
        let min = space.volume().min();
        let max = space.volume().max();
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "space.{}.purpose={:?}",
                space.id().0,
                space.purpose()
            )
            .unwrap();
            writeln!(output, "space.{}.adjacent_to={adjacent}", space.id().0).unwrap();
            writeln!(output, "space.{}.accessible_to={accessible}", space.id().0).unwrap();
            writeln!(
                output,
                "space.{}.aabb.f64_bits={:016x},{:016x},{:016x}:{:016x},{:016x},{:016x}",
                space.id().0,
                min[0].to_bits(),
                min[1].to_bits(),
                min[2].to_bits(),
                max[0].to_bits(),
                max[1].to_bits(),
                max[2].to_bits()
            )
            .unwrap();
        }
    }

    for clearance in snapshot.clearance_volumes() {
        let owner = match clearance.owner() {
            ClearanceOwner::Occurrence(path) => {
                let mut label = format!("occurrence:{}", path.root_occurrence().0);
                for step in path.steps() {
                    match step {
                        InstancePathStep::Group(id) => write!(label, "/group:{}", id.0).unwrap(),
                        InstancePathStep::Occurrence(id) => {
                            write!(label, "/occurrence:{}", id.0).unwrap();
                        }
                    }
                }
                label
            }
            ClearanceOwner::Space(id) => format!("space:{}", id.0),
        };
        let health = match clearance.derived_from() {
            None => "manual".to_owned(),
            Some(identity) => match snapshot.resolve_slot(identity) {
                SlotResolution::Resolved => "resolved".to_owned(),
                SlotResolution::Ambiguous { segment_index } => {
                    format!("ambiguous:{segment_index}")
                }
                SlotResolution::Lost { segment_index } => format!("lost:{segment_index}"),
            },
        };
        let min = clearance.volume().min();
        let max = clearance.volume().max();
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "clearance_volume.{}.owner={owner}",
                clearance.id().0
            )
            .unwrap();
            writeln!(
                output,
                "clearance_volume.{}.reason={:?}",
                clearance.id().0,
                clearance.reason()
            )
            .unwrap();
            writeln!(
                output,
                "clearance_volume.{}.severity={}",
                clearance.id().0,
                match clearance.severity() {
                    ClearanceSeverity::Advisory => "advisory",
                    ClearanceSeverity::Required => "required",
                }
            )
            .unwrap();
            writeln!(
                output,
                "clearance_volume.{}.slot_health={health}",
                clearance.id().0
            )
            .unwrap();
            writeln!(
                output,
                "clearance_volume.{}.aabb.f64_bits={:016x},{:016x},{:016x}:{:016x},{:016x},{:016x}",
                clearance.id().0,
                min[0].to_bits(),
                min[1].to_bits(),
                min[2].to_bits(),
                max[0].to_bits(),
                max[1].to_bits(),
                max[2].to_bits()
            )
            .unwrap();
        }
    }

    for tag in snapshot.tags() {
        writeln!(complete, "tag.{}.name={:?}", tag.id().0, tag.name()).unwrap();
        writeln!(complete, "tag.{}.visible={}", tag.id().0, tag.visible()).unwrap();
        writeln!(
            complete,
            "tag.{}.occurrences={}",
            tag.id().0,
            id_list(
                snapshot
                    .occurrences_with_tag(tag.id())
                    .map(|occurrence| occurrence.id().0)
            )
        )
        .unwrap();
        writeln!(
            agent,
            "tag.{}=name:{:?},visible:{},occurrences:{}",
            tag.id().0,
            tag.name(),
            tag.visible(),
            id_list(
                snapshot
                    .occurrences_with_tag(tag.id())
                    .map(|occurrence| occurrence.id().0)
            )
        )
        .unwrap();
    }

    for collection in snapshot.collections() {
        let occurrence_ids = id_list(collection.occurrence_ids().map(|id| id.0));
        writeln!(
            complete,
            "collection.{}.name={:?}",
            collection.id().0,
            collection.name()
        )
        .unwrap();
        writeln!(
            complete,
            "collection.{}.occurrences={occurrence_ids}",
            collection.id().0
        )
        .unwrap();
        writeln!(
            agent,
            "collection.{}=name:{:?},occurrences:{occurrence_ids}",
            collection.id().0,
            collection.name()
        )
        .unwrap();
    }

    for definition in snapshot.definitions() {
        writeln!(
            complete,
            "definition.{}.name={:?}",
            definition.id().0,
            definition.name()
        )
        .unwrap();
        writeln!(
            complete,
            "definition.{}.features={}",
            definition.id().0,
            id_list(definition.feature_ids().iter().map(|id| id.0))
        )
        .unwrap();
        writeln!(
            complete,
            "definition.{}.active_body={}",
            definition.id().0,
            definition.active_body_id().0
        )
        .unwrap();
        writeln!(
            complete,
            "definition.{}.bodies={}",
            definition.id().0,
            id_list(definition.bodies().map(|body| body.id().0))
        )
        .unwrap();
        for body in definition.bodies() {
            writeln!(
                complete,
                "definition.{}.body.{}=name:{:?},visible:{},consumed_by:{}",
                definition.id().0,
                body.id().0,
                body.name(),
                body.visible(),
                optional_id(body.consumed_by().map(|id| id.0))
            )
            .unwrap();
        }
        writeln!(
            agent,
            "definition.{}=name:{:?},features:{},bodies:{},active_body:{}",
            definition.id().0,
            definition.name(),
            id_list(definition.feature_ids().iter().map(|id| id.0)),
            id_list(definition.bodies().map(|body| body.id().0)),
            definition.active_body_id().0
        )
        .unwrap();
    }
    for feature in snapshot.features() {
        writeln!(
            complete,
            "feature.{}.definition={}",
            feature.id().0,
            feature.definition_id().0
        )
        .unwrap();
        writeln!(
            complete,
            "feature.{}.name={:?}",
            feature.id().0,
            feature.name()
        )
        .unwrap();
        let ownership = snapshot
            .definition(feature.definition_id())
            .and_then(|definition| definition.feature_body_ownership(feature.id()))
            .expect("validated feature body ownership is projected");
        writeln!(
            complete,
            "feature.{}.input_bodies={}",
            feature.id().0,
            id_list(ownership.input_body_ids().iter().map(|id| id.0))
        )
        .unwrap();
        let output_body = ownership
            .output_body_id()
            .map_or_else(|| "none".to_owned(), |id| id.0.to_string());
        writeln!(
            complete,
            "feature.{}.output_body={output_body}",
            feature.id().0
        )
        .unwrap();
        writeln!(
            agent,
            "feature.{}.body_ownership=inputs:{},output:{}",
            feature.id().0,
            id_list(ownership.input_body_ids().iter().map(|id| id.0)),
            output_body
        )
        .unwrap();
        match feature.kind() {
            crate::document::FeatureKind::Workplane(spec) => {
                use crate::sketch::WorkplaneSupport;
                writeln!(complete, "feature.{}.kind=workplane", feature.id().0).unwrap();
                match &spec.support {
                    WorkplaneSupport::Principal(plane) => {
                        writeln!(
                            complete,
                            "feature.{}.support=principal,{plane:?}",
                            feature.id().0
                        )
                        .unwrap();
                    }
                    WorkplaneSupport::Offset { base, distance } => {
                        writeln!(
                            complete,
                            "feature.{}.support=offset,base:{},distance_bits:{:016x},token:{:?}",
                            feature.id().0,
                            base.0,
                            distance.millimetres().to_bits(),
                            distance.source_token()
                        )
                        .unwrap();
                    }
                    WorkplaneSupport::PlanarFace { reference, health } => {
                        let id = feature.id().0;
                        writeln!(complete, "feature.{id}.support=planar_face").unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.document_id={}",
                            reference.document_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.definition_id={}",
                            reference.definition_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.profile_feature_id={}",
                            reference.profile_feature_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.producer_feature_id={}",
                            reference.producer_feature_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.semantic_role={:?}",
                            reference.semantic_role
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.source_element_id={:?}",
                            reference.source_element_id
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.expected_type={:?}",
                            reference.expected_type
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.expected_cardinality={}",
                            reference.expected_cardinality
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.stability={:?}",
                            reference.stability
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.canonical_input_digest={:?}",
                            reference.canonical_input_digest
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.exact_input_digest={:?}",
                            reference.exact_input_digest
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.result_fingerprint={:?}",
                            reference.result_fingerprint
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.evaluator={:?}",
                            reference.evaluator
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.backend={:?}",
                            reference.backend
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.tolerance={:?}",
                            reference.tolerance
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.lineage_digest={:?}",
                            reference.lineage_digest
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{id}.support.geometry_fingerprint={:?}",
                            reference.corroborating_geometry_fingerprint
                        )
                        .unwrap();
                        writeln!(complete, "feature.{id}.support.health={health:?}").unwrap();
                    }
                }
                writeln!(
                    complete,
                    "feature.{}.frame_bits={}",
                    feature.id().0,
                    spec.frame
                        .origin_mm
                        .iter()
                        .chain(spec.frame.x_axis.iter())
                        .chain(spec.frame.y_axis.iter())
                        .chain(spec.frame.normal.iter())
                        .map(|value| format!("{:016x}", value.to_bits()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
                .unwrap();
                let support = match &spec.support {
                    WorkplaneSupport::Principal(plane) => format!("principal:{plane:?}"),
                    WorkplaneSupport::Offset { base, distance } => format!(
                        "offset:base:{},distance_bits:{:016x}",
                        base.0,
                        distance.millimetres().to_bits()
                    ),
                    WorkplaneSupport::PlanarFace { reference, health } => format!(
                        "planar_face:producer:{},role:{:?},health:{health:?}",
                        reference.producer_feature_id.0, reference.semantic_role
                    ),
                };
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:workplane,definition:{},support:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    support
                )
                .unwrap();
            }
            crate::document::FeatureKind::Sketch(spec) => {
                let report = spec.solve().expect("canonical sketch remains solvable");
                writeln!(complete, "feature.{}.kind=sketch", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.workplane={}",
                    feature.id().0,
                    spec.workplane.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.solve_status={:?}",
                    feature.id().0,
                    report.status
                )
                .unwrap();
                for entity in &spec.entities {
                    writeln!(
                        complete,
                        "feature.{}.entity.{}={entity:?}",
                        feature.id().0,
                        entity.id().0
                    )
                    .unwrap();
                }
                for constraint in &spec.constraints {
                    writeln!(
                        complete,
                        "feature.{}.constraint.{}={:?}",
                        feature.id().0,
                        constraint.id.0,
                        constraint.kind
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:sketch,definition:{},workplane:{},entities:{},constraints:{},status:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    spec.workplane.0,
                    report.entity_count,
                    report.constraint_count,
                    report.status
                )
                .unwrap();
            }
            crate::document::FeatureKind::Profile { points_mm } => {
                writeln!(complete, "feature.{}.kind=profile", feature.id().0).unwrap();
                for (index, point) in points_mm.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.point.{index}.f64_bits={:016x},{:016x}",
                        feature.id().0,
                        point[0].to_bits(),
                        point[1].to_bits()
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:profile,definition:{},points:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    points_mm.len()
                )
                .unwrap();
            }
            crate::document::FeatureKind::SegmentProfile { segments, closed } => {
                writeln!(complete, "feature.{}.kind=segment_profile", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.closed={closed}", feature.id().0).unwrap();
                for (index, segment) in segments.iter().enumerate() {
                    match segment {
                        crate::document::ProfileSegment::Line { start_mm, end_mm } => {
                            writeln!(
                                complete,
                                "feature.{}.segment.{index}=line,{:016x},{:016x},{:016x},{:016x}",
                                feature.id().0,
                                start_mm[0].to_bits(),
                                start_mm[1].to_bits(),
                                end_mm[0].to_bits(),
                                end_mm[1].to_bits()
                            )
                            .unwrap();
                        }
                        crate::document::ProfileSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            clockwise,
                        } => {
                            writeln!(
                                complete,
                                "feature.{}.segment.{index}=circular_arc,{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},clockwise:{clockwise}",
                                feature.id().0,
                                start_mm[0].to_bits(),
                                start_mm[1].to_bits(),
                                end_mm[0].to_bits(),
                                end_mm[1].to_bits(),
                                center_mm[0].to_bits(),
                                center_mm[1].to_bits()
                            )
                            .unwrap();
                        }
                        crate::document::ProfileSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => {
                            writeln!(
                                complete,
                                "feature.{}.segment.{index}=cubic_bezier,{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},{:016x},{:016x}",
                                feature.id().0,
                                start_mm[0].to_bits(),
                                start_mm[1].to_bits(),
                                control_1_mm[0].to_bits(),
                                control_1_mm[1].to_bits(),
                                control_2_mm[0].to_bits(),
                                control_2_mm[1].to_bits(),
                                end_mm[0].to_bits(),
                                end_mm[1].to_bits()
                            )
                            .unwrap();
                        }
                    }
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:segment_profile,definition:{},segments:{},closed:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    segments.len(),
                    closed
                )
                .unwrap();
            }
            crate::document::FeatureKind::SpatialPath { segments } => {
                writeln!(complete, "feature.{}.kind=spatial_path", feature.id().0).unwrap();
                let bits = |point: &[f64; 3]| {
                    point
                        .iter()
                        .map(|value| format!("{:016x}", value.to_bits()))
                        .collect::<Vec<_>>()
                        .join(",")
                };
                for (index, segment) in segments.iter().enumerate() {
                    let value = match segment {
                        crate::document::SpatialPathSegment::Line { start_mm, end_mm } => {
                            format!("line,{},{}", bits(start_mm), bits(end_mm))
                        }
                        crate::document::SpatialPathSegment::CircularArc {
                            start_mm,
                            end_mm,
                            center_mm,
                            normal,
                            clockwise,
                        } => format!(
                            "circular_arc,{},{},{},{},clockwise:{}",
                            bits(start_mm),
                            bits(end_mm),
                            bits(center_mm),
                            bits(normal),
                            clockwise
                        ),
                        crate::document::SpatialPathSegment::CubicBezier {
                            start_mm,
                            control_1_mm,
                            control_2_mm,
                            end_mm,
                        } => format!(
                            "cubic_bezier,{},{},{},{}",
                            bits(start_mm),
                            bits(control_1_mm),
                            bits(control_2_mm),
                            bits(end_mm)
                        ),
                    };
                    writeln!(
                        complete,
                        "feature.{}.segment.{index}={value}",
                        feature.id().0
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:spatial_path,definition:{},segments:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    segments.len()
                )
                .unwrap();
            }
            crate::document::FeatureKind::SplineProfile { control_points_mm } => {
                writeln!(complete, "feature.{}.kind=spline_profile", feature.id().0).unwrap();
                for (index, point) in control_points_mm.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.control_point.{index}.f64_bits={:016x},{:016x}",
                        feature.id().0,
                        point[0].to_bits(),
                        point[1].to_bits()
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:spline_profile,definition:{},control_points:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    control_points_mm.len()
                )
                .unwrap();
            }
            crate::document::FeatureKind::Extrusion { profile, height } => {
                writeln!(complete, "feature.{}.kind=extrusion", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.height.source={:?}",
                    feature.id().0,
                    height.source_token()
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.height.f64_bits={:016x}",
                    feature.id().0,
                    height.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:extrusion,definition:{},profile:{},height_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0,
                    height.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::Pad(spec) => {
                writeln!(complete, "feature.{}.kind=pad", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.sketch={}",
                    feature.id().0,
                    spec.sketch.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.region={}",
                    feature.id().0,
                    spec.region.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.direction={:?}",
                    feature.id().0,
                    spec.direction
                )
                .unwrap();
                write_feature_extent(&mut complete, feature.id().0, &spec.extent);
                writeln!(agent, "feature.{}=name:{:?},kind:pad,definition:{},sketch:{},region:{},direction:{:?},extent:{:?}", feature.id().0, feature.name(), feature.definition_id().0, spec.sketch.0, spec.region.0, spec.direction, spec.extent).unwrap();
            }
            crate::document::FeatureKind::SketchPocket(spec) => {
                writeln!(complete, "feature.{}.kind=sketch_pocket", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.target={}",
                    feature.id().0,
                    spec.target.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.sketch={}",
                    feature.id().0,
                    spec.sketch.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.region={}",
                    feature.id().0,
                    spec.region.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.direction={:?}",
                    feature.id().0,
                    spec.direction
                )
                .unwrap();
                write_feature_extent(&mut complete, feature.id().0, &spec.extent);
                writeln!(
                    complete,
                    "feature.{}.support={:?}",
                    feature.id().0,
                    spec.support
                )
                .unwrap();
                writeln!(agent, "feature.{}=name:{:?},kind:sketch_pocket,definition:{},target:{},sketch:{},region:{},direction:{:?},extent:{:?},support_lineage:{:?}", feature.id().0, feature.name(), feature.definition_id().0, spec.target.0, spec.sketch.0, spec.region.0, spec.direction, spec.extent, spec.support.lineage_digest).unwrap();
            }
            crate::document::FeatureKind::BottleProfileControl {
                profile,
                body_radius,
                body_height,
                shoulder_rise,
            } => {
                writeln!(
                    complete,
                    "feature.{}.kind=bottle_profile_control",
                    feature.id().0
                )
                .unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                for (name, dimension) in [
                    ("body_radius", body_radius),
                    ("body_height", body_height),
                    ("shoulder_rise", shoulder_rise),
                ] {
                    writeln!(
                        complete,
                        "feature.{}.{name}.f64_bits={:016x}",
                        feature.id().0,
                        dimension.millimetres().to_bits()
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:bottle_profile_control,definition:{},profile:{},body_radius_mm:{:?},body_height_mm:{:?},shoulder_rise_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0,
                    body_radius.millimetres(),
                    body_height.millimetres(),
                    shoulder_rise.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::Revolve {
                profile,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                writeln!(complete, "feature.{}.kind=revolve", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.axis_start_mm={axis_start_mm:?}",
                    feature.id().0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.axis_end_mm={axis_end_mm:?}",
                    feature.id().0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.angle_degrees={angle_degrees:?}",
                    feature.id().0
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:revolve,definition:{},profile:{},axis_start_mm:{axis_start_mm:?},axis_end_mm:{axis_end_mm:?},angle_degrees:{angle_degrees:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::Shell {
                target,
                removed_faces,
                thickness,
            } => {
                writeln!(complete, "feature.{}.kind=shell", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.removed_faces={:?}",
                    feature.id().0,
                    removed_faces
                        .iter()
                        .map(crate::document::StableFaceRole::as_str)
                        .collect::<Vec<_>>()
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.thickness.source={:?}",
                    feature.id().0,
                    thickness.source_token()
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.thickness.f64_bits={:016x}",
                    feature.id().0,
                    thickness.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:shell,definition:{},target:{},removed_faces:{:?},thickness_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    removed_faces
                        .iter()
                        .map(crate::document::StableFaceRole::as_str)
                        .collect::<Vec<_>>(),
                    thickness.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::BottleEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                let kind = match kind {
                    crate::document::BottleEdgeFinishKind::Fillet => "fillet",
                    crate::document::BottleEdgeFinishKind::Chamfer => "chamfer",
                };
                writeln!(
                    complete,
                    "feature.{}.kind=bottle_edge_finish",
                    feature.id().0
                )
                .unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.edges={:?}",
                    feature.id().0,
                    edges
                        .iter()
                        .map(crate::document::StableEdgeRole::as_str)
                        .collect::<Vec<_>>()
                )
                .unwrap();
                writeln!(complete, "feature.{}.finish_kind={kind}", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.amount.f64_bits={:016x}",
                    feature.id().0,
                    amount.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:bottle_edge_finish,definition:{},target:{},edges:{:?},finish_kind:{kind},amount_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    edges
                        .iter()
                        .map(crate::document::StableEdgeRole::as_str)
                        .collect::<Vec<_>>(),
                    amount.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::TopologyShell {
                target,
                removed_faces,
                thickness,
            } => {
                writeln!(complete, "feature.{}.kind=topology_shell", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                for (index, reference) in removed_faces.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.removed_face.{index}.lineage={:?}",
                        feature.id().0,
                        reference.lineage_digest
                    )
                    .unwrap();
                }
                writeln!(
                    complete,
                    "feature.{}.thickness.f64_bits={:016x}",
                    feature.id().0,
                    thickness.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:topology_shell,definition:{},target:{},removed_faces:{},thickness_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    removed_faces.len(),
                    thickness.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::TopologyEdgeFinish {
                target,
                edges,
                kind,
                amount,
            } => {
                writeln!(
                    complete,
                    "feature.{}.kind=topology_edge_finish",
                    feature.id().0
                )
                .unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                for (index, reference) in edges.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.edge.{index}.lineage={:?}",
                        feature.id().0,
                        reference.lineage_digest
                    )
                    .unwrap();
                }
                writeln!(complete, "feature.{}.finish_kind={kind:?}", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.amount.f64_bits={:016x}",
                    feature.id().0,
                    amount.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:topology_edge_finish,definition:{},target:{},edges:{},finish_kind:{kind:?},amount_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    edges.len(),
                    amount.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::TopologyFaceOffset {
                target,
                face,
                distance,
            } => {
                writeln!(
                    complete,
                    "feature.{}.kind=topology_face_offset",
                    feature.id().0
                )
                .unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.face.lineage={:?}",
                    feature.id().0,
                    face.lineage_digest
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.distance.f64_bits={:016x}",
                    feature.id().0,
                    distance.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:topology_face_offset,definition:{},target:{},distance_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    distance.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::ThroughCut { target, profile } => {
                writeln!(complete, "feature.{}.kind=through_cut", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:through_cut,definition:{},target:{},profile:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    profile.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::Pocket {
                target,
                profile,
                depth,
            } => {
                writeln!(complete, "feature.{}.kind=pocket", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.depth.f64_bits={:016x}",
                    feature.id().0,
                    depth.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:pocket,definition:{},target:{},profile:{},depth_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    profile.0,
                    depth.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::PlanarOffset { profile, distance } => {
                writeln!(complete, "feature.{}.kind=planar_offset", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.distance.source={:?}",
                    feature.id().0,
                    distance.source_token()
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.distance.f64_bits={:016x}",
                    feature.id().0,
                    distance.millimetres().to_bits()
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:planar_offset,definition:{},profile:{},distance_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0,
                    distance.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::Sweep { profile, path } => {
                writeln!(complete, "feature.{}.kind=sweep", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(complete, "feature.{}.path={}", feature.id().0, path.0).unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:sweep,definition:{},profile:{},path:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0,
                    path.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::Loft { sections } => {
                writeln!(complete, "feature.{}.kind=loft", feature.id().0).unwrap();
                for (index, section) in sections.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.section.{index}=profile:{},elevation.f64_bits:{:016x}",
                        feature.id().0,
                        section.profile.0,
                        section.elevation_mm.to_bits()
                    )
                    .unwrap();
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:loft,definition:{},sections:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    sections.len()
                )
                .unwrap();
            }
            crate::document::FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => {
                let operation = match operation {
                    crate::document::BooleanOperation::Cut => "cut",
                    crate::document::BooleanOperation::Union => "union",
                    crate::document::BooleanOperation::Intersect => "intersect",
                    crate::document::BooleanOperation::Split => "split",
                };
                writeln!(complete, "feature.{}.kind=boolean", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.operation={operation}", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(complete, "feature.{}.tool={}", feature.id().0, tool.0).unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:boolean,operation:{operation},definition:{},target:{},tool:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    tool.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::RigidTransform { target, transform } => {
                writeln!(complete, "feature.{}.kind=rigid_transform", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.matrix_bits={}",
                    feature.id().0,
                    transform
                        .matrix()
                        .iter()
                        .map(|value| format!("{:016x}", value.to_bits()))
                        .collect::<Vec<_>>()
                        .join(",")
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:rigid_transform,definition:{},target:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::ImportedExactBody(spec) => {
                let source_sha256 = spec
                    .source_sha256
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect::<String>();
                writeln!(
                    complete,
                    "feature.{}.kind=imported_exact_body",
                    feature.id().0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.import_id={}",
                    feature.id().0,
                    spec.import_id.0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.source_sha256={source_sha256}",
                    feature.id().0
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.source_byte_len={}",
                    feature.id().0,
                    spec.source_byte_len
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.result_fingerprint={:?}",
                    feature.id().0,
                    spec.result_fingerprint
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.solid_count={}",
                    feature.id().0,
                    spec.solid_count
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.volume_mm3={:?}",
                    feature.id().0,
                    spec.volume_mm3
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.bounds_mm={:?}",
                    feature.id().0,
                    spec.bounds_mm
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.backend={:?}",
                    feature.id().0,
                    spec.backend
                )
                .unwrap();
                writeln!(
                    complete,
                    "feature.{}.tolerance={:?}",
                    feature.id().0,
                    spec.tolerance
                )
                .unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:imported_exact_body,definition:{},import:{},solids:{},volume_mm3:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    spec.import_id.0,
                    spec.solid_count,
                    spec.volume_mm3
                )
                .unwrap();
            }
            crate::document::FeatureKind::MeshBody(spec) => {
                writeln!(complete, "feature.{}.kind=mesh_body", feature.id().0).unwrap();
                writeln!(
                    complete,
                    "feature.{}.mesh.schema={:?}",
                    feature.id().0,
                    spec.schema
                )
                .unwrap();
                for (index, vertex) in spec.vertices_mm.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.mesh.vertex.{index}.f64_bits={:016x},{:016x},{:016x}",
                        feature.id().0,
                        vertex[0].to_bits(),
                        vertex[1].to_bits(),
                        vertex[2].to_bits()
                    )
                    .unwrap();
                }
                for (index, triangle) in spec.triangles.iter().enumerate() {
                    writeln!(
                        complete,
                        "feature.{}.mesh.triangle.{index}={},{},{}",
                        feature.id().0,
                        triangle[0],
                        triangle[1],
                        triangle[2]
                    )
                    .unwrap();
                }
                match &spec.authority {
                    crate::document::MeshAuthority::Authored { provenance } => {
                        writeln!(
                            complete,
                            "feature.{}.mesh.authority=authored:{provenance:?}",
                            feature.id().0
                        )
                        .unwrap();
                    }
                    crate::document::MeshAuthority::ImportedStl { import_id } => {
                        writeln!(
                            complete,
                            "feature.{}.mesh.authority=imported_stl:{}",
                            feature.id().0,
                            import_id.0
                        )
                        .unwrap();
                    }
                    crate::document::MeshAuthority::ImportedSketchupScene { import_id } => {
                        writeln!(
                            complete,
                            "feature.{}.mesh.authority=imported_sketchup_scene:{}",
                            feature.id().0,
                            import_id.0
                        )
                        .unwrap();
                    }
                    crate::document::MeshAuthority::ExactConversion(conversion) => {
                        writeln!(
                            complete,
                            "feature.{}.mesh.authority=exact_conversion",
                            feature.id().0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_document={}",
                            feature.id().0,
                            conversion.source_document_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_revision={}",
                            feature.id().0,
                            conversion.source_revision
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_digest={:?}",
                            feature.id().0,
                            conversion.source_digest
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_definition={}",
                            feature.id().0,
                            conversion.source_definition_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_feature={}",
                            feature.id().0,
                            conversion.source_feature_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_result_fingerprint={:?}",
                            feature.id().0,
                            conversion.source_result_fingerprint
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_evaluator={:?}",
                            feature.id().0,
                            conversion.source_evaluator
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_backend={:?}",
                            feature.id().0,
                            conversion.source_backend
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.source_tolerance={:?}",
                            feature.id().0,
                            conversion.source_tolerance
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.tessellation_tolerance={:?}",
                            feature.id().0,
                            conversion.tessellation_tolerance
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.destination_definition={}",
                            feature.id().0,
                            conversion.destination_definition_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.destination_feature={}",
                            feature.id().0,
                            conversion.destination_feature_id.0
                        )
                        .unwrap();
                        writeln!(
                            complete,
                            "feature.{}.mesh.exact_reference_consequence=lost",
                            feature.id().0
                        )
                        .unwrap();
                        for (index, semantic) in conversion.unsupported_semantics.iter().enumerate()
                        {
                            writeln!(
                                complete,
                                "feature.{}.mesh.unsupported.{index}={semantic:?}",
                                feature.id().0
                            )
                            .unwrap();
                        }
                    }
                }
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:mesh_body,definition:{},vertices:{},triangles:{},authority:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    spec.vertices_mm.len(),
                    spec.triangles.len(),
                    match &spec.authority {
                        crate::document::MeshAuthority::Authored { .. } => "authored",
                        crate::document::MeshAuthority::ExactConversion(_) => "exact_conversion",
                        crate::document::MeshAuthority::ImportedStl { .. } => "imported_stl",
                        crate::document::MeshAuthority::ImportedSketchupScene { .. } => {
                            "imported_sketchup_scene"
                        }
                    }
                )
                .unwrap();
            }
        }
    }
    for occurrence in snapshot.occurrences() {
        writeln!(
            complete,
            "occurrence.{}.definition={}",
            occurrence.id().0,
            occurrence.definition_id().0
        )
        .unwrap();
        writeln!(
            complete,
            "occurrence.{}.name={:?}",
            occurrence.id().0,
            occurrence.name()
        )
        .unwrap();
        write_transform(
            &mut complete,
            "occurrence",
            occurrence.id().0,
            occurrence.transform(),
        );
        writeln!(
            complete,
            "occurrence.{}.parent={}",
            occurrence.id().0,
            optional_id(occurrence.parent().map(|id| id.0))
        )
        .unwrap();
        writeln!(
            complete,
            "occurrence.{}.tag={}",
            occurrence.id().0,
            optional_id(occurrence.tag().map(|id| id.0))
        )
        .unwrap();
        writeln!(
            complete,
            "occurrence.{}.visible={}",
            occurrence.id().0,
            occurrence.visible()
        )
        .unwrap();
        writeln!(
            complete,
            "occurrence.{}.grounded={}",
            occurrence.id().0,
            snapshot.occurrence_is_grounded(occurrence.id())
        )
        .unwrap();
        let dof = snapshot
            .assembly_dof_diagnostic(occurrence.id())
            .expect("canonical occurrence has a DOF diagnostic");
        let dof_status = match dof.status() {
            AssemblyDofStatus::Grounded => "grounded",
            AssemblyDofStatus::PendingSolve => "pending_solve",
        };
        let remaining_dof = dof
            .remaining_dof()
            .map_or_else(|| "unknown".to_owned(), |value| value.to_string());
        let incident_mates = id_list(dof.incident_mate_ids().iter().map(|id| id.0));
        writeln!(
            complete,
            "occurrence.{}.dof=status:{dof_status},remaining:{remaining_dof},incident_mates:{incident_mates}",
            occurrence.id().0
        )
        .unwrap();
        writeln!(
            agent,
            "occurrence.{}=name:{:?},definition:{},parent:{},tag:{},visible:{},grounded:{},dof_status:{dof_status},remaining_dof:{remaining_dof},incident_mates:{incident_mates}",
            occurrence.id().0,
            occurrence.name(),
            occurrence.definition_id().0,
            optional_id(occurrence.parent().map(|id| id.0)),
            optional_id(occurrence.tag().map(|id| id.0)),
            occurrence.visible(),
            snapshot.occurrence_is_grounded(occurrence.id())
        )
        .unwrap();
    }
    for mate in snapshot.assembly_mates() {
        let health = |value: AssemblyReferenceHealth| match value {
            AssemblyReferenceHealth::Resolved => "resolved".to_owned(),
            AssemblyReferenceHealth::Broken => "broken".to_owned(),
            AssemblyReferenceHealth::Ambiguous { candidate_count } => {
                format!("ambiguous:{candidate_count}")
            }
            AssemblyReferenceHealth::Lost => "lost".to_owned(),
        };
        writeln!(
            complete,
            "assembly_mate.{}.schema={:?}",
            mate.id().0,
            mate.schema()
        )
        .unwrap();
        for (label, endpoint) in [("a", mate.endpoint_a()), ("b", mate.endpoint_b())] {
            writeln!(
                complete,
                "assembly_mate.{}.endpoint_{label}.occurrence={}",
                mate.id().0,
                endpoint.occurrence_id().0
            )
            .unwrap();
            writeln!(
                complete,
                "assembly_mate.{}.endpoint_{label}.lineage={:?}",
                mate.id().0,
                endpoint.reference().lineage_digest
            )
            .unwrap();
            match endpoint.attachment() {
                crate::assembly::AssemblyMateAttachment::ReferenceOnly(_) => writeln!(
                    complete,
                    "assembly_mate.{}.endpoint_{label}.attachment=reference_only",
                    mate.id().0
                )
                .unwrap(),
                crate::assembly::AssemblyMateAttachment::PlanarFace(attachment) => {
                    writeln!(
                        complete,
                        "assembly_mate.{}.endpoint_{label}.attachment=planar_face",
                        mate.id().0
                    )
                    .unwrap();
                    writeln!(
                        complete,
                        "assembly_mate.{}.endpoint_{label}.origin_f64_bits={:016x},{:016x},{:016x}",
                        mate.id().0,
                        attachment.local_origin_mm()[0].to_bits(),
                        attachment.local_origin_mm()[1].to_bits(),
                        attachment.local_origin_mm()[2].to_bits()
                    )
                    .unwrap();
                    writeln!(
                        complete,
                        "assembly_mate.{}.endpoint_{label}.normal_f64_bits={:016x},{:016x},{:016x}",
                        mate.id().0,
                        attachment.local_unit_normal()[0].to_bits(),
                        attachment.local_unit_normal()[1].to_bits(),
                        attachment.local_unit_normal()[2].to_bits()
                    )
                    .unwrap();
                }
                crate::assembly::AssemblyMateAttachment::Axial(attachment) => {
                    writeln!(
                        complete,
                        "assembly_mate.{}.endpoint_{label}.attachment=axial,kind={}",
                        mate.id().0,
                        match attachment.kind() {
                            crate::assembly::AxialAttachmentKind::Axis => "axis",
                            crate::assembly::AxialAttachmentKind::CylindricalFace =>
                                "cylindrical_face",
                        }
                    )
                    .unwrap();
                    writeln!(
                        complete,
                        "assembly_mate.{}.endpoint_{label}.origin_f64_bits={:016x},{:016x},{:016x}",
                        mate.id().0,
                        attachment.local_origin_mm()[0].to_bits(),
                        attachment.local_origin_mm()[1].to_bits(),
                        attachment.local_origin_mm()[2].to_bits()
                    )
                    .unwrap();
                    writeln!(complete, "assembly_mate.{}.endpoint_{label}.direction_f64_bits={:016x},{:016x},{:016x}", mate.id().0,
                        attachment.local_unit_direction()[0].to_bits(), attachment.local_unit_direction()[1].to_bits(), attachment.local_unit_direction()[2].to_bits()).unwrap();
                }
            }
            writeln!(
                complete,
                "assembly_mate.{}.endpoint_{label}.health={}",
                mate.id().0,
                health(endpoint.health())
            )
            .unwrap();
        }
        let kind = match mate.kind() {
            AssemblyMateKind::CoincidentPlanar {
                offset_mm,
                reversed,
            } => format!(
                "coincident_planar,offset_f64_bits:{:016x},reversed:{reversed}",
                offset_mm.to_bits()
            ),
            AssemblyMateKind::ConcentricAxial { reversed } => {
                format!("concentric_axial,reversed:{reversed}")
            }
            AssemblyMateKind::Distance { distance_mm } => {
                format!("distance,distance_f64_bits:{:016x}", distance_mm.to_bits())
            }
            AssemblyMateKind::Angle { angle_degrees } => {
                format!("angle,angle_f64_bits:{:016x}", angle_degrees.to_bits())
            }
        };
        writeln!(complete, "assembly_mate.{}.kind={kind}", mate.id().0).unwrap();
        writeln!(
            agent,
            "assembly_mate.{}=a:{},b:{},a_health:{},b_health:{},kind:{kind}",
            mate.id().0,
            mate.endpoint_a().occurrence_id().0,
            mate.endpoint_b().occurrence_id().0,
            health(mate.endpoint_a().health()),
            health(mate.endpoint_b().health())
        )
        .unwrap();
    }
    for joint in snapshot.assembly_joints() {
        let kind = match joint.kind() {
            AssemblyJointKind::Fixed => "fixed".to_owned(),
            AssemblyJointKind::Revolute {
                axis,
                limits,
                position_degrees,
            } => format!(
                "revolute,axis:{:?},pivot_mm:{:?},limits:{limits:?},position_degrees_f64_bits:{:016x}",
                axis.direction_in_parent(),
                axis.pivot_in_parent_mm(),
                position_degrees.to_bits()
            ),
            AssemblyJointKind::Prismatic {
                axis,
                limits,
                position_mm,
            } => format!(
                "prismatic,axis:{:?},pivot_mm:{:?},limits:{limits:?},position_mm_f64_bits:{:016x}",
                axis.direction_in_parent(),
                axis.pivot_in_parent_mm(),
                position_mm.to_bits()
            ),
        };
        writeln!(
            complete,
            "assembly_joint.{}=schema:{:?},parent:{},child:{},kind:{kind}",
            joint.id().0,
            joint.schema(),
            joint.parent_occurrence_id().0,
            joint.child_occurrence_id().0
        )
        .unwrap();
        writeln!(
            agent,
            "assembly_joint.{}=parent:{},child:{},kind:{kind}",
            joint.id().0,
            joint.parent_occurrence_id().0,
            joint.child_occurrence_id().0
        )
        .unwrap();
    }
    for coupling in snapshot.assembly_motion_couplings() {
        let detail = match coupling.transmission() {
            AssemblyTransmissionKind::GearPair {
                input_teeth,
                output_teeth,
                mesh,
            } => format!("input_teeth:{input_teeth},output_teeth:{output_teeth},mesh:{mesh:?}"),
            AssemblyTransmissionKind::Belt {
                input_pitch_diameter_mm,
                output_pitch_diameter_mm,
                crossed,
            } => format!(
                "input_pitch_diameter_bits:{:016x},output_pitch_diameter_bits:{:016x},crossed:{crossed}",
                input_pitch_diameter_mm.to_bits(),
                output_pitch_diameter_mm.to_bits()
            ),
            AssemblyTransmissionKind::Chain {
                input_sprocket_teeth,
                output_sprocket_teeth,
            } => format!(
                "input_sprocket_teeth:{input_sprocket_teeth},output_sprocket_teeth:{output_sprocket_teeth}"
            ),
            AssemblyTransmissionKind::RackAndPinion {
                pinion_pitch_diameter_mm,
                direction,
            } => format!(
                "pinion_pitch_diameter_bits:{:016x},direction:{direction:?}",
                pinion_pitch_diameter_mm.to_bits()
            ),
            AssemblyTransmissionKind::LeadScrew {
                lead_mm_per_revolution,
                handedness,
            } => format!(
                "lead_bits:{:016x},handedness:{handedness:?}",
                lead_mm_per_revolution.to_bits()
            ),
        };
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "assembly_motion_coupling.{}=schema:{:?},input:{},output:{},kind:{},input_reference_bits:{:016x},output_reference_bits:{:016x},scale_bits:{:016x},{detail}",
                coupling.id().0,
                coupling.schema(),
                coupling.input_joint_id().0,
                coupling.output_joint_id().0,
                coupling.transmission().label(),
                coupling.input_reference_position().to_bits(),
                coupling.output_reference_position().to_bits(),
                coupling.transmission().scale().to_bits()
            )
            .unwrap();
        }
    }
    for interface in snapshot.mechanical_interfaces() {
        let frame = interface.frame();
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "mechanical_interface.{}=schema:{:?},occurrence:{},role:{},face_ordinal:{},geometry_fingerprint:{:?},origin_bits:{:016x}/{:016x}/{:016x},normal_bits:{:016x}/{:016x}/{:016x},area_bits:{:016x}",
                interface.id().0,
                interface.schema(),
                interface.occurrence_id().0,
                interface.role().label(),
                interface.face_ordinal(),
                interface.geometry_fingerprint(),
                frame.origin_mm()[0].to_bits(),
                frame.origin_mm()[1].to_bits(),
                frame.origin_mm()[2].to_bits(),
                frame.normal()[0].to_bits(),
                frame.normal()[1].to_bits(),
                frame.normal()[2].to_bits(),
                frame.area_mm2().to_bits()
            )
            .unwrap();
        }
    }
    for condition in snapshot.mechanical_conditions() {
        let detail = match condition.kind() {
            MechanicalConditionKind::PlanarContact {
                first,
                second,
                offset_mm,
                tolerance_mm,
            } => format!(
                "first:{},second:{},offset_bits:{:016x},tolerance_bits:{:016x}",
                first.0,
                second.0,
                offset_mm.to_bits(),
                tolerance_mm.to_bits()
            ),
            MechanicalConditionKind::Support {
                supported,
                supporting,
                tolerance_mm,
            } => format!(
                "supported:{},supporting:{},tolerance_bits:{:016x}",
                supported.0,
                supporting.0,
                tolerance_mm.to_bits()
            ),
            MechanicalConditionKind::JointAxisAlignment {
                joint_id,
                interface,
                alignment,
                tolerance_degrees,
            } => format!(
                "joint:{},interface:{},alignment:{},tolerance_bits:{:016x}",
                joint_id.0,
                interface.0,
                alignment.label(),
                tolerance_degrees.to_bits()
            ),
            MechanicalConditionKind::JointTravel {
                joint_id,
                minimum,
                maximum,
            } => format!(
                "joint:{},minimum_bits:{:016x},maximum_bits:{:016x}",
                joint_id.0,
                minimum.to_bits(),
                maximum.to_bits()
            ),
        };
        for output in [&mut complete, &mut agent] {
            writeln!(
                output,
                "mechanical_condition.{}=schema:{:?},kind:{},{detail}",
                condition.id().0,
                condition.schema(),
                condition.kind().label()
            )
            .unwrap();
        }
    }
    for study in snapshot.assembly_motion_studies() {
        let drivers = study
            .drivers()
            .iter()
            .map(|driver| {
                format!(
                    "{}:{:016x}",
                    driver.joint_id().0,
                    driver.position().to_bits()
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            complete,
            "assembly_motion_study.{}=schema:{:?},name:{:?},drivers:{drivers}",
            study.id().0,
            study.schema(),
            study.name()
        )
        .unwrap();
        writeln!(
            agent,
            "assembly_motion_study.{}=name:{:?},drivers:{drivers}",
            study.id().0,
            study.name()
        )
        .unwrap();
    }
    for group in snapshot.groups() {
        writeln!(complete, "group.{}.name={:?}", group.id().0, group.name()).unwrap();
        write_transform(&mut complete, "group", group.id().0, group.transform());
        writeln!(
            complete,
            "group.{}.parent={}",
            group.id().0,
            optional_id(group.parent().map(|id| id.0))
        )
        .unwrap();
        writeln!(
            agent,
            "group.{}=name:{:?},parent:{}",
            group.id().0,
            group.name(),
            optional_id(group.parent().map(|id| id.0))
        )
        .unwrap();
    }
    for group in snapshot.local_groups() {
        writeln!(
            complete,
            "local_group.{}:{}.name={:?}",
            group.key().definition_id.0,
            group.key().local_id.0,
            group.name()
        )
        .unwrap();
        writeln!(
            complete,
            "local_group.{}:{}.transform.f64_bits={}",
            group.key().definition_id.0,
            group.key().local_id.0,
            transform_bits(group.transform())
        )
        .unwrap();
    }
    for occurrence in snapshot.local_occurrences() {
        writeln!(
            complete,
            "local_occurrence.{}:{}.definition={}",
            occurrence.key().definition_id.0,
            occurrence.key().local_id.0,
            occurrence.definition_id().0
        )
        .unwrap();
        writeln!(
            complete,
            "local_occurrence.{}:{}.name={:?}",
            occurrence.key().definition_id.0,
            occurrence.key().local_id.0,
            occurrence.name()
        )
        .unwrap();
        writeln!(
            complete,
            "local_occurrence.{}:{}.transform.f64_bits={}",
            occurrence.key().definition_id.0,
            occurrence.key().local_id.0,
            transform_bits(occurrence.transform())
        )
        .unwrap();
    }
    writeln!(agent, "intended_actions=canonical_command_batch_only").unwrap();
    SemanticState { complete, agent }
}

impl SemanticState {
    #[must_use]
    pub fn complete_v1(&self) -> String {
        self.complete.clone()
    }
    #[must_use]
    pub fn agent_v1(&self) -> String {
        self.agent.clone()
    }
}

fn write_header(output: &mut String, schema: &str, snapshot: &Snapshot) {
    writeln!(output, "schema={schema}").unwrap();
    writeln!(output, "encoder={SEMANTIC_ENCODER_V1}").unwrap();
    writeln!(output, "source.document_id={}", snapshot.document_id().0).unwrap();
    writeln!(output, "source.revision={}", snapshot.revision_id()).unwrap();
    writeln!(
        output,
        "source.canonical_digest={}",
        snapshot.canonical_digest()
    )
    .unwrap();
    writeln!(
        output,
        "source.units={}",
        match snapshot.units() {
            UnitSystem::Millimetres => "millimetres",
        }
    )
    .unwrap();
    writeln!(output, "source.canonical_schema=3").unwrap();
    writeln!(output, "projection.complete=true").unwrap();
    writeln!(output, "projection.stale=false").unwrap();
}
fn write_ports(output: &mut String, id: u64, direction: &str, ports: &[crate::graph::PortSpec]) {
    for (index, port) in ports.iter().enumerate() {
        writeln!(
            output,
            "evaluator_node.{id}.{direction}_port.{index}.name={:?}",
            port.name()
        )
        .unwrap();
        writeln!(
            output,
            "evaluator_node.{id}.{direction}_port.{index}.type={}",
            match port.value_type() {
                ValueType::Number => "number",
            }
        )
        .unwrap();
    }
}
fn output_paths(outputs: &[RuleOutput]) -> Vec<Vec<SlotSegment>> {
    let mut paths = Vec::new();
    let mut stack = outputs
        .iter()
        .rev()
        .map(|output| (output, Vec::new()))
        .collect::<Vec<_>>();
    while let Some((output, mut path)) = stack.pop() {
        path.push(output.segment().clone());
        paths.push(path.clone());
        for child in output.children().iter().rev() {
            stack.push((child, path.clone()));
        }
    }
    paths
}
fn slot_path(path: &[SlotSegment]) -> String {
    path.iter()
        .map(|segment| {
            format!(
                "{}:{:?}:{:?}",
                segment.producer_rule_id.0, segment.output_port, segment.semantic_key
            )
        })
        .collect::<Vec<_>>()
        .join("/")
}
fn status(value: EvaluationStatus) -> String {
    match value {
        EvaluationStatus::Evaluated(number) => format!("evaluated:{:016x}", number.to_bits()),
        EvaluationStatus::Error(items) => format!(
            "error:{}",
            items
                .iter()
                .map(|item| format!("{}:{:?}", item.node_id.0, item.code))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}
fn write_transform(output: &mut String, family: &str, id: u64, transform: Transform) {
    writeln!(
        output,
        "{family}.{id}.transform.f64_bits={}",
        transform_bits(transform)
    )
    .unwrap();
}
fn transform_bits(transform: Transform) -> String {
    transform
        .matrix()
        .iter()
        .map(|value| format!("{:016x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join(",")
}
fn id_list(ids: impl Iterator<Item = u64>) -> String {
    format!(
        "[{}]",
        ids.map(|id| id.to_string()).collect::<Vec<_>>().join(",")
    )
}
fn optional_id(id: Option<u64>) -> String {
    id.map_or_else(|| "none".to_owned(), |id| id.to_string())
}
