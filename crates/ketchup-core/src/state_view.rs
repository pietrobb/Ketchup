use crate::document::{
    EvaluationReport, EvaluationStatus, EvaluatorNodeKind, Snapshot, Transform, UnitSystem,
};
use crate::graph::{OverrideMergePolicy, RuleOutput, SlotSegment, ValueType};
use crate::validation::ValidationReport;
use std::fmt::Write;

pub const COMPLETE_STATE_VIEW_V1: &str = "ketchup.state-view.complete.v1";
pub const AGENT_STATE_VIEW_V1: &str = "ketchup.state-view.agent.v1";
pub const SEMANTIC_ENCODER_V1: &str = "ketchup.semantic-state.v1";

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
        "summary.counts=evaluator_nodes:{},overrides:{},parameter_bindings:{},definitions:{},features:{},occurrences:{},groups:{},local_groups:{},local_occurrences:{}",
        snapshot.evaluator_node_count(), snapshot.overrides().count(),
        snapshot.feature_parameter_bindings().count(), snapshot.definitions().count(),
        snapshot.features().count(), snapshot.occurrences().count(), snapshot.groups().count(),
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
                "parameter_binding.{}.{}.derived_from.root={}",
                binding.target.feature_id.0,
                binding.target.slot.label(),
                binding.derived_from.root_rule_node_id.0
            )
            .unwrap();
            writeln!(
                output,
                "parameter_binding.{}.{}.derived_from.slot_path={}",
                binding.target.feature_id.0,
                binding.target.slot.label(),
                slot_path(binding.derived_from.slot_path.segments())
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
            agent,
            "definition.{}=name:{:?},features:{}",
            definition.id().0,
            definition.name(),
            id_list(definition.feature_ids().iter().map(|id| id.0))
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
        match feature.kind() {
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
            crate::document::FeatureKind::Revolve { profile } => {
                writeln!(complete, "feature.{}.kind=revolve", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.profile={}", feature.id().0, profile.0).unwrap();
                writeln!(
                    agent,
                    "feature.{}=name:{:?},kind:revolve,definition:{},profile:{}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    profile.0
                )
                .unwrap();
            }
            crate::document::FeatureKind::Shell { target, thickness } => {
                writeln!(complete, "feature.{}.kind=shell", feature.id().0).unwrap();
                writeln!(complete, "feature.{}.target={}", feature.id().0, target.0).unwrap();
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
                    "feature.{}=name:{:?},kind:shell,definition:{},target:{},thickness_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    thickness.millimetres()
                )
                .unwrap();
            }
            crate::document::FeatureKind::BottleEdgeFinish {
                target,
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
                    "feature.{}=name:{:?},kind:bottle_edge_finish,definition:{},target:{},finish_kind:{kind},amount_mm:{:?}",
                    feature.id().0,
                    feature.name(),
                    feature.definition_id().0,
                    target.0,
                    amount.millimetres()
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
            crate::document::FeatureKind::Boolean {
                operation,
                target,
                tool,
            } => {
                let operation = match operation {
                    crate::document::BooleanOperation::Cut => "cut",
                    crate::document::BooleanOperation::Union => "union",
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
            agent,
            "occurrence.{}=name:{:?},definition:{},parent:{},tag:{},visible:{}",
            occurrence.id().0,
            occurrence.name(),
            occurrence.definition_id().0,
            optional_id(occurrence.parent().map(|id| id.0)),
            optional_id(occurrence.tag().map(|id| id.0)),
            occurrence.visible()
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
