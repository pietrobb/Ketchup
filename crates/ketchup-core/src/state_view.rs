use crate::document::{
    DefinitionId, DocumentId, FeatureId, FeatureKind, GroupId, NodeId, OccurrenceId, Snapshot,
    TagId, Transform, UnitSystem,
};
use std::fmt::Write;

pub const COMPLETE_STATE_VIEW_V1: &str = "ketchup.state-view.complete.v1";
pub const AGENT_STATE_VIEW_V1: &str = "ketchup.state-view.agent.v1";
pub const SEMANTIC_ENCODER_V1: &str = "ketchup.semantic-state.v1";

#[derive(Clone, Debug, PartialEq)]
pub struct SemanticState {
    document_id: DocumentId,
    source_revision: u64,
    canonical_digest: String,
    units: UnitSystem,
    nodes: Vec<SemanticNode>,
    definitions: Vec<SemanticDefinition>,
    features: Vec<SemanticFeature>,
    occurrences: Vec<SemanticOccurrence>,
    groups: Vec<SemanticGroup>,
}

#[derive(Clone, Debug, PartialEq)]
struct SemanticNode {
    id: NodeId,
    name: String,
    source_token: String,
    millimetres: f64,
    dependencies: Vec<NodeId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SemanticDefinition {
    id: DefinitionId,
    name: String,
    feature_ids: Vec<FeatureId>,
}

#[derive(Clone, Debug, PartialEq)]
struct SemanticFeature {
    id: FeatureId,
    definition_id: DefinitionId,
    name: String,
    kind: SemanticFeatureKind,
}

#[derive(Clone, Debug, PartialEq)]
enum SemanticFeatureKind {
    Profile {
        points_mm: Vec<[f64; 2]>,
    },
    Extrusion {
        profile: FeatureId,
        source_token: String,
        millimetres: f64,
    },
}

#[derive(Clone, Debug, PartialEq)]
struct SemanticOccurrence {
    id: OccurrenceId,
    definition_id: DefinitionId,
    name: String,
    transform: Transform,
    parent: Option<GroupId>,
    tag: Option<TagId>,
    visible: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct SemanticGroup {
    id: GroupId,
    name: String,
    transform: Transform,
    parent: Option<GroupId>,
}

#[must_use]
pub fn encode_semantic_state(snapshot: &Snapshot) -> SemanticState {
    let nodes = snapshot
        .node_ids()
        .map(|id| {
            let node = snapshot.node(id).expect("node ID came from this snapshot");
            SemanticNode {
                id,
                name: node.name().to_owned(),
                source_token: node.dimension().source_token().to_owned(),
                millimetres: node.dimension().millimetres(),
                dependencies: node.dependencies().to_vec(),
            }
        })
        .collect();
    let definitions = snapshot
        .definitions()
        .map(|definition| SemanticDefinition {
            id: definition.id(),
            name: definition.name().to_owned(),
            feature_ids: definition.feature_ids().to_vec(),
        })
        .collect();
    let features = snapshot
        .features()
        .map(|feature| SemanticFeature {
            id: feature.id(),
            definition_id: feature.definition_id(),
            name: feature.name().to_owned(),
            kind: match feature.kind() {
                FeatureKind::Profile { points_mm } => SemanticFeatureKind::Profile {
                    points_mm: points_mm.clone(),
                },
                FeatureKind::Extrusion { profile, height } => SemanticFeatureKind::Extrusion {
                    profile: *profile,
                    source_token: height.source_token().to_owned(),
                    millimetres: height.millimetres(),
                },
            },
        })
        .collect();
    let occurrences = snapshot
        .occurrences()
        .map(|occurrence| SemanticOccurrence {
            id: occurrence.id(),
            definition_id: occurrence.definition_id(),
            name: occurrence.name().to_owned(),
            transform: occurrence.transform(),
            parent: occurrence.parent(),
            tag: occurrence.tag(),
            visible: occurrence.visible(),
        })
        .collect();
    let groups = snapshot
        .groups()
        .map(|group| SemanticGroup {
            id: group.id(),
            name: group.name().to_owned(),
            transform: group.transform(),
            parent: group.parent(),
        })
        .collect();

    SemanticState {
        document_id: snapshot.document_id(),
        source_revision: snapshot.revision_id(),
        canonical_digest: snapshot.canonical_digest(),
        units: snapshot.units(),
        nodes,
        definitions,
        features,
        occurrences,
        groups,
    }
}

impl SemanticState {
    #[must_use]
    pub fn complete_v1(&self) -> String {
        let mut output = String::new();
        writeln!(output, "schema={COMPLETE_STATE_VIEW_V1}").unwrap();
        writeln!(output, "encoder={SEMANTIC_ENCODER_V1}").unwrap();
        self.write_projection_header(&mut output);

        for node in &self.nodes {
            writeln!(output, "node.{}.name={:?}", node.id.0, node.name).unwrap();
            writeln!(
                output,
                "node.{}.dimension.source={:?}",
                node.id.0, node.source_token
            )
            .unwrap();
            writeln!(
                output,
                "node.{}.dimension.f64_bits={:016x}",
                node.id.0,
                node.millimetres.to_bits()
            )
            .unwrap();
            writeln!(
                output,
                "node.{}.dependencies={}",
                node.id.0,
                id_list(node.dependencies.iter().map(|id| id.0))
            )
            .unwrap();
        }
        for definition in &self.definitions {
            writeln!(
                output,
                "definition.{}.name={:?}",
                definition.id.0, definition.name
            )
            .unwrap();
            writeln!(
                output,
                "definition.{}.features={}",
                definition.id.0,
                id_list(definition.feature_ids.iter().map(|id| id.0))
            )
            .unwrap();
        }
        for feature in &self.features {
            writeln!(
                output,
                "feature.{}.definition={}",
                feature.id.0, feature.definition_id.0
            )
            .unwrap();
            writeln!(output, "feature.{}.name={:?}", feature.id.0, feature.name).unwrap();
            match &feature.kind {
                SemanticFeatureKind::Profile { points_mm } => {
                    writeln!(output, "feature.{}.kind=profile", feature.id.0).unwrap();
                    writeln!(
                        output,
                        "feature.{}.point_count={}",
                        feature.id.0,
                        points_mm.len()
                    )
                    .unwrap();
                    for (index, point) in points_mm.iter().enumerate() {
                        writeln!(
                            output,
                            "feature.{}.point.{index}.f64_bits={:016x},{:016x}",
                            feature.id.0,
                            point[0].to_bits(),
                            point[1].to_bits()
                        )
                        .unwrap();
                    }
                }
                SemanticFeatureKind::Extrusion {
                    profile,
                    source_token,
                    millimetres,
                } => {
                    writeln!(output, "feature.{}.kind=extrusion", feature.id.0).unwrap();
                    writeln!(output, "feature.{}.profile={}", feature.id.0, profile.0).unwrap();
                    writeln!(
                        output,
                        "feature.{}.height.source={source_token:?}",
                        feature.id.0
                    )
                    .unwrap();
                    writeln!(
                        output,
                        "feature.{}.height.f64_bits={:016x}",
                        feature.id.0,
                        millimetres.to_bits()
                    )
                    .unwrap();
                }
            }
        }
        for occurrence in &self.occurrences {
            writeln!(
                output,
                "occurrence.{}.definition={}",
                occurrence.id.0, occurrence.definition_id.0
            )
            .unwrap();
            writeln!(
                output,
                "occurrence.{}.name={:?}",
                occurrence.id.0, occurrence.name
            )
            .unwrap();
            write_transform(
                &mut output,
                "occurrence",
                occurrence.id.0,
                occurrence.transform,
            );
            writeln!(
                output,
                "occurrence.{}.parent={}",
                occurrence.id.0,
                optional_id(occurrence.parent.map(|id| id.0))
            )
            .unwrap();
            writeln!(
                output,
                "occurrence.{}.tag={}",
                occurrence.id.0,
                optional_id(occurrence.tag.map(|id| id.0))
            )
            .unwrap();
            writeln!(
                output,
                "occurrence.{}.visible={}",
                occurrence.id.0, occurrence.visible
            )
            .unwrap();
        }
        for group in &self.groups {
            writeln!(output, "group.{}.name={:?}", group.id.0, group.name).unwrap();
            write_transform(&mut output, "group", group.id.0, group.transform);
            writeln!(
                output,
                "group.{}.parent={}",
                group.id.0,
                optional_id(group.parent.map(|id| id.0))
            )
            .unwrap();
        }
        output
    }

    #[must_use]
    pub fn agent_v1(&self) -> String {
        let mut output = String::new();
        writeln!(output, "schema={AGENT_STATE_VIEW_V1}").unwrap();
        writeln!(output, "encoder={SEMANTIC_ENCODER_V1}").unwrap();
        self.write_projection_header(&mut output);
        writeln!(
            output,
            "summary.counts=nodes:{},definitions:{},features:{},occurrences:{},groups:{}",
            self.nodes.len(),
            self.definitions.len(),
            self.features.len(),
            self.occurrences.len(),
            self.groups.len()
        )
        .unwrap();
        for node in &self.nodes {
            writeln!(
                output,
                "node.{}=name:{:?},dimension_mm:{:?},source_token:{:?},depends_on:{}",
                node.id.0,
                node.name,
                node.millimetres,
                node.source_token,
                id_list(node.dependencies.iter().map(|id| id.0))
            )
            .unwrap();
        }
        for definition in &self.definitions {
            let sharing = self
                .occurrences
                .iter()
                .filter(|occurrence| occurrence.definition_id == definition.id)
                .count();
            writeln!(
                output,
                "definition.{}=name:{:?},features:{},instances:{sharing}",
                definition.id.0,
                definition.name,
                id_list(definition.feature_ids.iter().map(|id| id.0))
            )
            .unwrap();
        }
        for feature in &self.features {
            match &feature.kind {
                SemanticFeatureKind::Profile { points_mm } => {
                    let bounds = profile_bounds(points_mm);
                    writeln!(
                        output,
                        "feature.{}=name:{:?},kind:profile,definition:{},points:{},bounds_mm:{}",
                        feature.id.0,
                        feature.name,
                        feature.definition_id.0,
                        points_mm.len(),
                        bounds
                    )
                    .unwrap();
                }
                SemanticFeatureKind::Extrusion {
                    profile,
                    source_token,
                    millimetres,
                } => {
                    writeln!(
                        output,
                        "feature.{}=name:{:?},kind:extrusion,definition:{},profile:{},height_mm:{millimetres:?},source_token:{source_token:?}",
                        feature.id.0, feature.name, feature.definition_id.0, profile.0
                    )
                    .unwrap();
                }
            }
        }
        for occurrence in &self.occurrences {
            writeln!(
                output,
                "occurrence.{}=name:{:?},definition:{},parent:{},tag:{},visible:{}",
                occurrence.id.0,
                occurrence.name,
                occurrence.definition_id.0,
                optional_id(occurrence.parent.map(|id| id.0)),
                optional_id(occurrence.tag.map(|id| id.0)),
                occurrence.visible
            )
            .unwrap();
        }
        for group in &self.groups {
            writeln!(
                output,
                "group.{}=name:{:?},parent:{}",
                group.id.0,
                group.name,
                optional_id(group.parent.map(|id| id.0))
            )
            .unwrap();
        }
        writeln!(output, "rules=unavailable_in_schema_2").unwrap();
        writeln!(output, "references=tag_ids_only").unwrap();
        writeln!(output, "validation_health=not_evaluated").unwrap();
        writeln!(output, "intended_actions=canonical_command_batch_only").unwrap();
        output
    }

    fn write_projection_header(&self, output: &mut String) {
        writeln!(output, "source.document_id={}", self.document_id.0).unwrap();
        writeln!(output, "source.revision={}", self.source_revision).unwrap();
        writeln!(output, "source.canonical_digest={}", self.canonical_digest).unwrap();
        writeln!(output, "source.units={}", unit_name(self.units)).unwrap();
        writeln!(output, "projection.complete=true").unwrap();
        writeln!(output, "projection.stale=false").unwrap();
    }
}

fn write_transform(output: &mut String, family: &str, id: u64, transform: Transform) {
    let bits = transform
        .matrix()
        .iter()
        .map(|value| format!("{:016x}", value.to_bits()))
        .collect::<Vec<_>>()
        .join(",");
    writeln!(output, "{family}.{id}.transform.f64_bits={bits}").unwrap();
}

fn profile_bounds(points: &[[f64; 2]]) -> String {
    let Some(first) = points.first() else {
        return "empty".to_owned();
    };
    let (mut min_x, mut max_x, mut min_y, mut max_y) = (first[0], first[0], first[1], first[1]);
    for point in &points[1..] {
        min_x = min_x.min(point[0]);
        max_x = max_x.max(point[0]);
        min_y = min_y.min(point[1]);
        max_y = max_y.max(point[1]);
    }
    format!("{:?}x{:?}", max_x - min_x, max_y - min_y)
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

const fn unit_name(units: UnitSystem) -> &'static str {
    match units {
        UnitSystem::Millimetres => "millimetres",
    }
}
