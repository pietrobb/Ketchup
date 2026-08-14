use crate::document::{Dimension, NodeId};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::Arc;

pub const EVALUATOR_ID_V1: &str = "ketchup.evaluator.numeric.v1";
pub const GRAPH_SCHEMA_ID_V1: &str = "ketchup.graph.schema.v1";
pub const DEFAULT_BACKEND_ID: &str = "ketchup.backend.in-process.numeric.v1";
pub const MAX_EXPRESSION_BYTES: usize = 4096;
pub const MAX_EXPRESSION_TOKENS: usize = 1024;
pub const MAX_EXPRESSION_DEPTH: usize = 64;
pub const MAX_SLOT_PATH_SEGMENTS: usize = 64;
pub const MAX_RULE_OUTPUT_DEPTH: usize = 64;
pub const MAX_RULE_OUTPUTS: usize = 16_384;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValueType {
    Number,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PortSpec {
    name: String,
    value_type: ValueType,
}

impl PortSpec {
    pub fn number(name: impl Into<String>) -> Result<Self, GraphError> {
        let name = name.into();
        ensure_semantic_key(&name)?;
        Ok(Self {
            name,
            value_type: ValueType::Number,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn value_type(&self) -> ValueType {
        self.value_type
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ExpressionAst {
    Number(f64),
    Node(NodeId),
    Add(Box<Self>, Box<Self>),
    Subtract(Box<Self>, Box<Self>),
    Multiply(Box<Self>, Box<Self>),
    Divide(Box<Self>, Box<Self>),
    Negate(Box<Self>),
}

impl ExpressionAst {
    pub fn parse(source: &str) -> Result<Self, GraphError> {
        if source.is_empty() || source.len() > MAX_EXPRESSION_BYTES {
            return Err(GraphError::ExpressionLimit);
        }
        let tokens = tokenize(source)?;
        if tokens.is_empty() || tokens.len() > MAX_EXPRESSION_TOKENS {
            return Err(GraphError::ExpressionLimit);
        }
        let mut parser = Parser { tokens, cursor: 0 };
        let expression = parser.parse_sum(0)?;
        if parser.cursor != parser.tokens.len() {
            return Err(GraphError::UnexpectedToken);
        }
        Ok(expression)
    }

    pub fn dependencies(&self) -> Vec<NodeId> {
        let mut dependencies = BTreeSet::new();
        self.collect_dependencies(&mut dependencies);
        dependencies.into_iter().collect()
    }

    fn collect_dependencies(&self, dependencies: &mut BTreeSet<NodeId>) {
        match self {
            Self::Number(_) => {}
            Self::Node(id) => {
                dependencies.insert(*id);
            }
            Self::Add(left, right)
            | Self::Subtract(left, right)
            | Self::Multiply(left, right)
            | Self::Divide(left, right) => {
                left.collect_dependencies(dependencies);
                right.collect_dependencies(dependencies);
            }
            Self::Negate(value) => value.collect_dependencies(dependencies),
        }
    }

    fn evaluate(&self, values: &BTreeMap<NodeId, f64>) -> Result<f64, DiagnosticCode> {
        let value = match self {
            Self::Number(value) => *value,
            Self::Node(id) => *values.get(id).ok_or(DiagnosticCode::DependencyFailed)?,
            Self::Add(left, right) => left.evaluate(values)? + right.evaluate(values)?,
            Self::Subtract(left, right) => left.evaluate(values)? - right.evaluate(values)?,
            Self::Multiply(left, right) => left.evaluate(values)? * right.evaluate(values)?,
            Self::Divide(left, right) => {
                let divisor = right.evaluate(values)?;
                if divisor == 0.0 {
                    return Err(DiagnosticCode::DivisionByZero);
                }
                left.evaluate(values)? / divisor
            }
            Self::Negate(value) => -value.evaluate(values)?,
        };
        if value.is_finite() {
            Ok(value)
        } else {
            Err(DiagnosticCode::NonFiniteResult)
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct SlotSegment {
    pub producer_rule_id: NodeId,
    pub output_port: String,
    pub semantic_key: String,
}

impl SlotSegment {
    pub fn new(
        producer_rule_id: NodeId,
        output_port: impl Into<String>,
        semantic_key: impl Into<String>,
    ) -> Result<Self, GraphError> {
        if producer_rule_id.0 == 0 {
            return Err(GraphError::ReservedNodeId);
        }
        let output_port = output_port.into();
        let semantic_key = semantic_key.into();
        ensure_semantic_key(&output_port)?;
        ensure_semantic_key(&semantic_key)?;
        Ok(Self {
            producer_rule_id,
            output_port,
            semantic_key,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct SlotPath(Vec<SlotSegment>);

impl SlotPath {
    pub fn new(segments: Vec<SlotSegment>) -> Result<Self, GraphError> {
        if segments.is_empty() {
            return Err(GraphError::EmptySlotPath);
        }
        if segments.len() > MAX_SLOT_PATH_SEGMENTS {
            return Err(GraphError::SlotPathLimit);
        }
        Ok(Self(segments))
    }

    #[must_use]
    pub fn segments(&self) -> &[SlotSegment] {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct DerivedIdentity {
    pub root_rule_node_id: NodeId,
    pub slot_path: SlotPath,
}

impl DerivedIdentity {
    pub fn new(root_rule_node_id: NodeId, slot_path: SlotPath) -> Result<Self, GraphError> {
        if root_rule_node_id.0 == 0 {
            return Err(GraphError::ReservedNodeId);
        }
        Ok(Self {
            root_rule_node_id,
            slot_path,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum OverrideMergePolicy {
    Replace,
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct OverrideParameterSpec {
    name: String,
    merge_policy: OverrideMergePolicy,
}

impl OverrideParameterSpec {
    pub fn replace(name: impl Into<String>) -> Result<Self, GraphError> {
        let name = name.into();
        ensure_semantic_key(&name)?;
        Ok(Self {
            name,
            merge_policy: OverrideMergePolicy::Replace,
        })
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn merge_policy(&self) -> OverrideMergePolicy {
        self.merge_policy
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuleOutput {
    segment: SlotSegment,
    children: Vec<RuleOutput>,
}

impl RuleOutput {
    pub fn new(segment: SlotSegment, children: Vec<Self>) -> Result<Self, GraphError> {
        let mut count = 1_usize;
        let mut stack = children
            .iter()
            .map(|child| (child, 2_usize))
            .collect::<Vec<_>>();
        while let Some((output, depth)) = stack.pop() {
            if depth > MAX_RULE_OUTPUT_DEPTH {
                return Err(GraphError::RuleOutputDepthLimit);
            }
            count = count.checked_add(1).ok_or(GraphError::RuleOutputLimit)?;
            if count > MAX_RULE_OUTPUTS {
                return Err(GraphError::RuleOutputLimit);
            }
            stack.extend(output.children.iter().map(|child| (child, depth + 1)));
        }
        Ok(Self { segment, children })
    }

    #[must_use]
    pub const fn segment(&self) -> &SlotSegment {
        &self.segment
    }

    #[must_use]
    pub fn children(&self) -> &[Self] {
        &self.children
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SlotResolution {
    Resolved,
    Ambiguous { segment_index: usize },
    Lost { segment_index: usize },
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvaluatorNodeKind {
    Parameter {
        value: Dimension,
    },
    Expression {
        source: String,
        ast: ExpressionAst,
    },
    Rule {
        source: String,
        ast: ExpressionAst,
        outputs: Vec<RuleOutput>,
        allowed_parameters: Vec<OverrideParameterSpec>,
    },
}

impl EvaluatorNodeKind {
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Parameter { .. } => "parameter",
            Self::Expression { .. } => "expression",
            Self::Rule { .. } => "rule",
        }
    }

    #[must_use]
    pub fn source(&self) -> &str {
        match self {
            Self::Parameter { value } => value.source_token(),
            Self::Expression { source, .. } | Self::Rule { source, .. } => source,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvaluatorNode {
    pub(crate) id: NodeId,
    pub(crate) name: String,
    pub(crate) kind: EvaluatorNodeKind,
    pub(crate) input_ports: Vec<PortSpec>,
    pub(crate) output_ports: Vec<PortSpec>,
    pub(crate) dependencies: Vec<NodeId>,
}

impl EvaluatorNode {
    pub(crate) fn parameter(
        id: NodeId,
        name: String,
        value: Dimension,
        dependencies: Vec<NodeId>,
    ) -> Result<Self, GraphError> {
        validate_node_header(id, &name, &dependencies)?;
        Ok(Self {
            id,
            name,
            kind: EvaluatorNodeKind::Parameter { value },
            input_ports: dependencies
                .iter()
                .map(|dependency| PortSpec::number(format!("node_{}", dependency.0)))
                .collect::<Result<_, _>>()?,
            output_ports: vec![PortSpec::number("value")?],
            dependencies,
        })
    }

    pub(crate) fn expression(id: NodeId, name: String, source: String) -> Result<Self, GraphError> {
        let ast = ExpressionAst::parse(&source)?;
        let dependencies = ast.dependencies();
        validate_node_header(id, &name, &dependencies)?;
        Ok(Self {
            id,
            name,
            kind: EvaluatorNodeKind::Expression { source, ast },
            input_ports: dependencies
                .iter()
                .map(|dependency| PortSpec::number(format!("node_{}", dependency.0)))
                .collect::<Result<_, _>>()?,
            output_ports: vec![PortSpec::number("value")?],
            dependencies,
        })
    }

    pub(crate) fn rule(
        id: NodeId,
        name: String,
        source: String,
        input_ports: Vec<PortSpec>,
        output_ports: Vec<PortSpec>,
        outputs: Vec<RuleOutput>,
        allowed_parameters: Vec<OverrideParameterSpec>,
    ) -> Result<Self, GraphError> {
        let ast = ExpressionAst::parse(&source)?;
        let dependencies = ast.dependencies();
        validate_node_header(id, &name, &dependencies)?;
        validate_ports(&input_ports)?;
        validate_ports(&output_ports)?;
        validate_override_parameters(&allowed_parameters)?;
        if output_ports.is_empty() {
            return Err(GraphError::EmptyOutputPorts);
        }
        validate_rule_outputs(id, &output_ports, &outputs)?;
        Ok(Self {
            id,
            name,
            kind: EvaluatorNodeKind::Rule {
                source,
                ast,
                outputs,
                allowed_parameters,
            },
            input_ports,
            output_ports,
            dependencies,
        })
    }

    #[must_use]
    pub const fn id(&self) -> NodeId {
        self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub const fn kind(&self) -> &EvaluatorNodeKind {
        &self.kind
    }

    #[must_use]
    pub fn input_ports(&self) -> &[PortSpec] {
        &self.input_ports
    }

    #[must_use]
    pub fn output_ports(&self) -> &[PortSpec] {
        &self.output_ports
    }

    #[must_use]
    pub fn dependencies(&self) -> &[NodeId] {
        &self.dependencies
    }

    #[must_use]
    pub fn dimension(&self) -> Option<&Dimension> {
        match &self.kind {
            EvaluatorNodeKind::Parameter { value } => Some(value),
            EvaluatorNodeKind::Expression { .. } | EvaluatorNodeKind::Rule { .. } => None,
        }
    }

    #[must_use]
    pub fn allowed_parameters(&self) -> &[OverrideParameterSpec] {
        match &self.kind {
            EvaluatorNodeKind::Rule {
                allowed_parameters, ..
            } => allowed_parameters,
            EvaluatorNodeKind::Parameter { .. } | EvaluatorNodeKind::Expression { .. } => &[],
        }
    }

    #[must_use]
    pub fn resolve_slot_path(&self, path: &SlotPath) -> SlotResolution {
        let EvaluatorNodeKind::Rule { outputs, .. } = &self.kind else {
            return SlotResolution::Lost { segment_index: 0 };
        };
        resolve_outputs(outputs, path)
    }

    pub(crate) fn canonical_spec_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        push_u64(&mut bytes, self.id.0);
        push_string(&mut bytes, &self.name);
        push_ports(&mut bytes, &self.input_ports);
        push_ports(&mut bytes, &self.output_ports);
        push_ids(&mut bytes, &self.dependencies);
        match &self.kind {
            EvaluatorNodeKind::Parameter { value } => {
                bytes.push(1);
                push_string(&mut bytes, value.source_token());
                push_u64(&mut bytes, value.millimetres().to_bits());
            }
            EvaluatorNodeKind::Expression { source, .. } => {
                bytes.push(2);
                push_string(&mut bytes, source);
            }
            EvaluatorNodeKind::Rule {
                source,
                outputs,
                allowed_parameters,
                ..
            } => {
                bytes.push(3);
                push_string(&mut bytes, source);
                push_outputs(&mut bytes, outputs);
                push_u64(&mut bytes, allowed_parameters.len() as u64);
                for parameter in allowed_parameters {
                    push_string(&mut bytes, parameter.name());
                    bytes.push(match parameter.merge_policy() {
                        OverrideMergePolicy::Replace => 1,
                    });
                }
            }
        }
        bytes
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CanonicalOverride {
    pub id: u64,
    pub target: DerivedIdentity,
    pub parameter: String,
    pub value_bits: u64,
    pub health: SlotResolution,
}

impl CanonicalOverride {
    pub fn new(
        id: u64,
        target: DerivedIdentity,
        parameter: impl Into<String>,
        value: f64,
        health: SlotResolution,
    ) -> Result<Self, GraphError> {
        if id == 0 {
            return Err(GraphError::ReservedOverrideId);
        }
        let parameter = parameter.into();
        ensure_semantic_key(&parameter)?;
        if !value.is_finite() {
            return Err(GraphError::NonFiniteOverride);
        }
        Ok(Self {
            id,
            target,
            parameter,
            value_bits: value.to_bits(),
            health,
        })
    }

    #[must_use]
    pub fn value(&self) -> f64 {
        f64::from_bits(self.value_bits)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub struct EvaluationIdentity {
    pub evaluator: String,
    pub schema: String,
    pub tolerance: String,
    pub backend: Option<String>,
}

impl Default for EvaluationIdentity {
    fn default() -> Self {
        Self {
            evaluator: EVALUATOR_ID_V1.to_owned(),
            schema: GRAPH_SCHEMA_ID_V1.to_owned(),
            tolerance: crate::document::TOLERANCE_PROFILE_V1.to_owned(),
            backend: Some(DEFAULT_BACKEND_ID.to_owned()),
        }
    }
}

impl EvaluationIdentity {
    pub fn validate(&self) -> Result<(), GraphError> {
        if self.evaluator.is_empty()
            || self.schema.is_empty()
            || self.tolerance.is_empty()
            || self.backend.as_ref().is_some_and(String::is_empty)
        {
            Err(GraphError::EmptyEvaluationIdentity)
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiagnosticCode {
    DependencyFailed,
    DivisionByZero,
    NonFiniteResult,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationDiagnostic {
    pub node_id: NodeId,
    pub code: DiagnosticCode,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum EvaluationStatus {
    Evaluated(f64),
    Error(Vec<EvaluationDiagnostic>),
}

#[derive(Clone, Debug, PartialEq)]
pub struct NodeEvaluation {
    pub status: EvaluationStatus,
    pub input_digest: String,
    pub result_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DerivedOutput {
    pub value: f64,
    pub input_digest: String,
    pub result_digest: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvaluationReport {
    pub identity: EvaluationIdentity,
    pub document_id: Option<crate::document::DocumentId>,
    pub revision_id: Option<u64>,
    pub canonical_digest: Option<String>,
    pub nodes: BTreeMap<NodeId, NodeEvaluation>,
    pub outputs: BTreeMap<DerivedIdentity, DerivedOutput>,
    pub recomputed_nodes: BTreeSet<NodeId>,
}

impl EvaluationReport {
    #[must_use]
    pub fn node(&self, id: NodeId) -> Option<&NodeEvaluation> {
        self.nodes.get(&id)
    }
}

pub fn evaluate_graph(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    identity: &EvaluationIdentity,
) -> Result<EvaluationReport, GraphError> {
    let all = nodes.keys().copied().collect();
    evaluate_affected(nodes, identity, None, &all)
}

pub fn evaluate_affected(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    identity: &EvaluationIdentity,
    previous: Option<&EvaluationReport>,
    affected: &BTreeSet<NodeId>,
) -> Result<EvaluationReport, GraphError> {
    identity.validate()?;
    validate_graph(nodes)?;
    let order = topological_order(nodes)?;
    let mut values = BTreeMap::new();
    let mut evaluations = BTreeMap::new();
    let mut recomputed_nodes = BTreeSet::new();
    let can_reuse = previous.is_some_and(|report| report.identity == *identity);
    let mut required = affected.clone();
    let mut pending = affected.iter().copied().collect::<Vec<_>>();
    while let Some(id) = pending.pop() {
        let node = nodes.get(&id).ok_or(GraphError::MissingDependency(id))?;
        for dependency in &node.dependencies {
            let reusable = can_reuse
                && previous
                    .and_then(|report| report.nodes.get(dependency))
                    .is_some();
            if !reusable && required.insert(*dependency) {
                pending.push(*dependency);
            }
        }
    }
    for id in order {
        if !required.contains(&id) {
            if can_reuse && let Some(prior) = previous.and_then(|report| report.nodes.get(&id)) {
                if let EvaluationStatus::Evaluated(value) = prior.status {
                    values.insert(id, value);
                }
                evaluations.insert(id, prior.clone());
            }
            continue;
        }
        recomputed_nodes.insert(id);
        let node = &nodes[&id];
        let dependency_results = node
            .dependencies
            .iter()
            .map(|dependency| &evaluations[dependency].result_digest)
            .collect::<Vec<_>>();
        let input_digest = node_input_digest(node, &dependency_results, identity);
        let dependency_failed = node
            .dependencies
            .iter()
            .any(|dependency| matches!(evaluations[dependency].status, EvaluationStatus::Error(_)));
        let value = if dependency_failed {
            Err(DiagnosticCode::DependencyFailed)
        } else {
            match &node.kind {
                EvaluatorNodeKind::Parameter { value } => Ok(value.millimetres()),
                EvaluatorNodeKind::Expression { ast, .. } | EvaluatorNodeKind::Rule { ast, .. } => {
                    ast.evaluate(&values)
                }
            }
        };
        let status = match value {
            Ok(value) => {
                values.insert(id, value);
                EvaluationStatus::Evaluated(value)
            }
            Err(code) => EvaluationStatus::Error(vec![EvaluationDiagnostic {
                node_id: id,
                code,
                message: diagnostic_message(code).to_owned(),
            }]),
        };
        let result_digest = node_result_digest(&input_digest, &status);
        evaluations.insert(
            id,
            NodeEvaluation {
                status,
                input_digest,
                result_digest,
            },
        );
    }
    let mut outputs = BTreeMap::new();
    for (id, node) in nodes {
        let EvaluatorNodeKind::Rule {
            outputs: rule_outputs,
            ..
        } = &node.kind
        else {
            continue;
        };
        let Some(NodeEvaluation {
            status: EvaluationStatus::Evaluated(value),
            input_digest,
            result_digest,
        }) = evaluations.get(id)
        else {
            continue;
        };
        let mut stack = rule_outputs
            .iter()
            .rev()
            .map(|output| (output, Vec::<SlotSegment>::new()))
            .collect::<Vec<_>>();
        while let Some((output, mut path)) = stack.pop() {
            path.push(output.segment.clone());
            let slot_path = SlotPath::new(path.clone())?;
            let derived = DerivedIdentity::new(*id, slot_path)?;
            outputs.insert(
                derived,
                DerivedOutput {
                    value: *value,
                    input_digest: input_digest.clone(),
                    result_digest: result_digest.clone(),
                },
            );
            for child in output.children.iter().rev() {
                stack.push((child, path.clone()));
            }
        }
    }
    Ok(EvaluationReport {
        identity: identity.clone(),
        document_id: None,
        revision_id: None,
        canonical_digest: None,
        nodes: evaluations,
        outputs,
        recomputed_nodes,
    })
}

#[must_use]
pub fn resolve_derived_identity(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
    identity: &DerivedIdentity,
) -> SlotResolution {
    let Some(root) = nodes.get(&identity.root_rule_node_id) else {
        return SlotResolution::Lost { segment_index: 0 };
    };
    if !matches!(root.kind, EvaluatorNodeKind::Rule { .. }) {
        return SlotResolution::Lost { segment_index: 0 };
    }
    for (index, segment) in identity.slot_path.segments().iter().enumerate() {
        if segment.producer_rule_id != identity.root_rule_node_id {
            return SlotResolution::Lost {
                segment_index: index,
            };
        }
        let Some(producer) = nodes.get(&segment.producer_rule_id) else {
            return SlotResolution::Lost {
                segment_index: index,
            };
        };
        if !matches!(producer.kind, EvaluatorNodeKind::Rule { .. })
            || !producer
                .output_ports
                .iter()
                .any(|port| port.name == segment.output_port)
        {
            return SlotResolution::Lost {
                segment_index: index,
            };
        }
    }
    root.resolve_slot_path(&identity.slot_path)
}

pub fn validate_graph(nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>) -> Result<(), GraphError> {
    for (id, node) in nodes {
        if *id != node.id {
            return Err(GraphError::NodeKeyMismatch);
        }
        validate_node_header(node.id, &node.name, &node.dependencies)?;
        for dependency in &node.dependencies {
            if !nodes.contains_key(dependency) {
                return Err(GraphError::MissingDependency(*dependency));
            }
        }
        if let EvaluatorNodeKind::Rule { outputs, .. } = &node.kind {
            validate_rule_outputs(node.id, &node.output_ports, outputs)?;
        }
    }
    topological_order(nodes).map(|_| ())
}

#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[must_use]
pub fn sha256_bytes(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn node_input_digest(
    node: &EvaluatorNode,
    dependency_results: &[&String],
    identity: &EvaluationIdentity,
) -> String {
    let mut bytes = b"ketchup.node.input.v1".to_vec();
    push_bytes(&mut bytes, &node.canonical_spec_bytes());
    push_u64(&mut bytes, dependency_results.len() as u64);
    for digest in dependency_results {
        push_string(&mut bytes, digest);
    }
    push_string(&mut bytes, &identity.evaluator);
    push_string(&mut bytes, &identity.schema);
    push_string(&mut bytes, &identity.tolerance);
    match &identity.backend {
        Some(backend) => {
            bytes.push(1);
            push_string(&mut bytes, backend);
        }
        None => bytes.push(0),
    }
    sha256_hex(&bytes)
}

fn node_result_digest(input_digest: &str, status: &EvaluationStatus) -> String {
    let mut bytes = b"ketchup.node.result.v1".to_vec();
    push_string(&mut bytes, input_digest);
    match status {
        EvaluationStatus::Evaluated(value) => {
            bytes.push(1);
            push_u64(&mut bytes, value.to_bits());
        }
        EvaluationStatus::Error(diagnostics) => {
            bytes.push(2);
            push_u64(&mut bytes, diagnostics.len() as u64);
            for diagnostic in diagnostics {
                push_u64(&mut bytes, diagnostic.node_id.0);
                bytes.push(match diagnostic.code {
                    DiagnosticCode::DependencyFailed => 1,
                    DiagnosticCode::DivisionByZero => 2,
                    DiagnosticCode::NonFiniteResult => 3,
                });
                push_string(&mut bytes, &diagnostic.message);
            }
        }
    }
    sha256_hex(&bytes)
}

fn resolve_outputs(outputs: &[RuleOutput], path: &SlotPath) -> SlotResolution {
    let mut candidates = outputs;
    for (index, segment) in path.segments().iter().enumerate() {
        let matches = candidates
            .iter()
            .filter(|output| output.segment == *segment)
            .collect::<Vec<_>>();
        match matches.as_slice() {
            [] => {
                return SlotResolution::Lost {
                    segment_index: index,
                };
            }
            [resolved] => candidates = &resolved.children,
            _ => {
                return SlotResolution::Ambiguous {
                    segment_index: index,
                };
            }
        }
    }
    SlotResolution::Resolved
}

fn validate_node_header(id: NodeId, name: &str, dependencies: &[NodeId]) -> Result<(), GraphError> {
    if id.0 == 0 {
        return Err(GraphError::ReservedNodeId);
    }
    if name.trim().is_empty() {
        return Err(GraphError::EmptyNodeName);
    }
    if !dependencies.windows(2).all(|pair| pair[0] < pair[1]) {
        return Err(GraphError::DependenciesNotCanonical);
    }
    if dependencies.contains(&id) {
        return Err(GraphError::DependencyCycle(id));
    }
    Ok(())
}

fn validate_ports(ports: &[PortSpec]) -> Result<(), GraphError> {
    let mut names = BTreeSet::new();
    for port in ports {
        ensure_semantic_key(&port.name)?;
        if !names.insert(&port.name) {
            return Err(GraphError::DuplicatePort);
        }
    }
    Ok(())
}

fn validate_override_parameters(parameters: &[OverrideParameterSpec]) -> Result<(), GraphError> {
    let mut names = BTreeSet::new();
    for parameter in parameters {
        ensure_semantic_key(&parameter.name)?;
        if !names.insert(&parameter.name) {
            return Err(GraphError::DuplicateOverrideParameter);
        }
    }
    Ok(())
}

fn validate_rule_outputs(
    rule_id: NodeId,
    ports: &[PortSpec],
    outputs: &[RuleOutput],
) -> Result<(), GraphError> {
    let port_names = ports
        .iter()
        .map(|port| port.name.as_str())
        .collect::<BTreeSet<_>>();
    let mut count = 0_usize;
    let mut stack = outputs
        .iter()
        .map(|output| (output, 1_usize))
        .collect::<Vec<_>>();
    while let Some((output, depth)) = stack.pop() {
        count = count.checked_add(1).ok_or(GraphError::RuleOutputLimit)?;
        if count > MAX_RULE_OUTPUTS {
            return Err(GraphError::RuleOutputLimit);
        }
        if depth > MAX_RULE_OUTPUT_DEPTH {
            return Err(GraphError::RuleOutputDepthLimit);
        }
        if output.segment.producer_rule_id != rule_id {
            return Err(GraphError::InvalidOutputProducer);
        }
        if !port_names.contains(output.segment.output_port.as_str()) {
            return Err(GraphError::UnknownOutputPort);
        }
        stack.extend(output.children.iter().map(|child| (child, depth + 1)));
    }
    Ok(())
}

fn topological_order(
    nodes: &BTreeMap<NodeId, Arc<EvaluatorNode>>,
) -> Result<Vec<NodeId>, GraphError> {
    let mut indegree = BTreeMap::new();
    let mut dependents = BTreeMap::<NodeId, Vec<NodeId>>::new();
    for (id, node) in nodes {
        indegree.insert(*id, node.dependencies.len());
        for dependency in &node.dependencies {
            if !nodes.contains_key(dependency) {
                return Err(GraphError::MissingDependency(*dependency));
            }
            dependents.entry(*dependency).or_default().push(*id);
        }
    }
    let mut ready = indegree
        .iter()
        .filter_map(|(id, degree)| (*degree == 0).then_some(*id))
        .collect::<BTreeSet<_>>();
    let mut order = Vec::new();
    order
        .try_reserve_exact(nodes.len())
        .map_err(|_| GraphError::GraphLimit)?;
    while let Some(id) = ready.pop_first() {
        order.push(id);
        if let Some(children) = dependents.get(&id) {
            for child in children {
                let degree = indegree
                    .get_mut(child)
                    .ok_or(GraphError::MissingDependency(*child))?;
                *degree -= 1;
                if *degree == 0 {
                    ready.insert(*child);
                }
            }
        }
    }
    if order.len() != nodes.len() {
        let id = indegree
            .into_iter()
            .find_map(|(id, degree)| (degree != 0).then_some(id))
            .unwrap_or(NodeId(0));
        return Err(GraphError::DependencyCycle(id));
    }
    Ok(order)
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Token {
    Number(u64),
    Node(NodeId),
    Plus,
    Minus,
    Star,
    Slash,
    LeftParen,
    RightParen,
}

fn tokenize(source: &str) -> Result<Vec<Token>, GraphError> {
    let bytes = source.as_bytes();
    let mut cursor = 0;
    let mut tokens = Vec::new();
    while cursor < bytes.len() {
        match bytes[cursor] {
            b' ' | b'\t' | b'\r' | b'\n' => cursor += 1,
            b'+' => {
                tokens.push(Token::Plus);
                cursor += 1;
            }
            b'-' => {
                tokens.push(Token::Minus);
                cursor += 1;
            }
            b'*' => {
                tokens.push(Token::Star);
                cursor += 1;
            }
            b'/' => {
                tokens.push(Token::Slash);
                cursor += 1;
            }
            b'(' => {
                tokens.push(Token::LeftParen);
                cursor += 1;
            }
            b')' => {
                tokens.push(Token::RightParen);
                cursor += 1;
            }
            b'$' => {
                cursor += 1;
                let start = cursor;
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                if start == cursor {
                    return Err(GraphError::InvalidNodeReference);
                }
                let id = source[start..cursor]
                    .parse::<u64>()
                    .map_err(|_| GraphError::InvalidNodeReference)?;
                if id == 0 {
                    return Err(GraphError::ReservedNodeId);
                }
                tokens.push(Token::Node(NodeId(id)));
            }
            b'0'..=b'9' | b'.' => {
                let start = cursor;
                let mut exponent = false;
                cursor += 1;
                while cursor < bytes.len() {
                    let byte = bytes[cursor];
                    if byte.is_ascii_digit() || byte == b'.' {
                        cursor += 1;
                    } else if (byte == b'e' || byte == b'E') && !exponent {
                        exponent = true;
                        cursor += 1;
                        if cursor < bytes.len() && (bytes[cursor] == b'+' || bytes[cursor] == b'-')
                        {
                            cursor += 1;
                        }
                    } else {
                        break;
                    }
                }
                let value = source[start..cursor]
                    .parse::<f64>()
                    .map_err(|_| GraphError::InvalidNumber)?;
                if !value.is_finite() {
                    return Err(GraphError::InvalidNumber);
                }
                tokens.push(Token::Number(value.to_bits()));
            }
            _ => return Err(GraphError::UnexpectedToken),
        }
        if tokens.len() > MAX_EXPRESSION_TOKENS {
            return Err(GraphError::ExpressionLimit);
        }
    }
    Ok(tokens)
}

struct Parser {
    tokens: Vec<Token>,
    cursor: usize,
}

impl Parser {
    fn parse_sum(&mut self, depth: usize) -> Result<ExpressionAst, GraphError> {
        check_depth(depth)?;
        let mut left = self.parse_product(depth + 1)?;
        loop {
            match self.tokens.get(self.cursor) {
                Some(Token::Plus) => {
                    self.cursor += 1;
                    left = ExpressionAst::Add(
                        Box::new(left),
                        Box::new(self.parse_product(depth + 1)?),
                    );
                }
                Some(Token::Minus) => {
                    self.cursor += 1;
                    left = ExpressionAst::Subtract(
                        Box::new(left),
                        Box::new(self.parse_product(depth + 1)?),
                    );
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_product(&mut self, depth: usize) -> Result<ExpressionAst, GraphError> {
        check_depth(depth)?;
        let mut left = self.parse_unary(depth + 1)?;
        loop {
            match self.tokens.get(self.cursor) {
                Some(Token::Star) => {
                    self.cursor += 1;
                    left = ExpressionAst::Multiply(
                        Box::new(left),
                        Box::new(self.parse_unary(depth + 1)?),
                    );
                }
                Some(Token::Slash) => {
                    self.cursor += 1;
                    left = ExpressionAst::Divide(
                        Box::new(left),
                        Box::new(self.parse_unary(depth + 1)?),
                    );
                }
                _ => return Ok(left),
            }
        }
    }

    fn parse_unary(&mut self, depth: usize) -> Result<ExpressionAst, GraphError> {
        check_depth(depth)?;
        if self.tokens.get(self.cursor) == Some(&Token::Minus) {
            self.cursor += 1;
            return Ok(ExpressionAst::Negate(Box::new(
                self.parse_unary(depth + 1)?,
            )));
        }
        self.parse_primary(depth + 1)
    }

    fn parse_primary(&mut self, depth: usize) -> Result<ExpressionAst, GraphError> {
        check_depth(depth)?;
        let token = self
            .tokens
            .get(self.cursor)
            .cloned()
            .ok_or(GraphError::UnexpectedEnd)?;
        self.cursor += 1;
        match token {
            Token::Number(bits) => Ok(ExpressionAst::Number(f64::from_bits(bits))),
            Token::Node(id) => Ok(ExpressionAst::Node(id)),
            Token::LeftParen => {
                let value = self.parse_sum(depth + 1)?;
                if self.tokens.get(self.cursor) != Some(&Token::RightParen) {
                    return Err(GraphError::UnclosedParenthesis);
                }
                self.cursor += 1;
                Ok(value)
            }
            _ => Err(GraphError::UnexpectedToken),
        }
    }
}

fn check_depth(depth: usize) -> Result<(), GraphError> {
    if depth > MAX_EXPRESSION_DEPTH {
        Err(GraphError::ExpressionLimit)
    } else {
        Ok(())
    }
}

fn ensure_semantic_key(value: &str) -> Result<(), GraphError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(GraphError::InvalidSemanticKey)
    } else {
        Ok(())
    }
}

fn diagnostic_message(code: DiagnosticCode) -> &'static str {
    match code {
        DiagnosticCode::DependencyFailed => "a dependency did not evaluate",
        DiagnosticCode::DivisionByZero => "division by zero",
        DiagnosticCode::NonFiniteResult => "expression produced a non-finite result",
    }
}

fn push_u64(bytes: &mut Vec<u8>, value: u64) {
    bytes.extend_from_slice(&value.to_le_bytes());
}

fn push_bytes(bytes: &mut Vec<u8>, value: &[u8]) {
    push_u64(bytes, value.len() as u64);
    bytes.extend_from_slice(value);
}

fn push_string(bytes: &mut Vec<u8>, value: &str) {
    push_bytes(bytes, value.as_bytes());
}

fn push_ids(bytes: &mut Vec<u8>, values: &[NodeId]) {
    push_u64(bytes, values.len() as u64);
    for value in values {
        push_u64(bytes, value.0);
    }
}

fn push_ports(bytes: &mut Vec<u8>, ports: &[PortSpec]) {
    push_u64(bytes, ports.len() as u64);
    for port in ports {
        push_string(bytes, &port.name);
        bytes.push(match port.value_type {
            ValueType::Number => 1,
        });
    }
}

fn push_outputs(bytes: &mut Vec<u8>, outputs: &[RuleOutput]) {
    push_u64(bytes, outputs.len() as u64);
    let mut stack = outputs.iter().rev().collect::<Vec<_>>();
    while let Some(output) = stack.pop() {
        push_u64(bytes, output.segment.producer_rule_id.0);
        push_string(bytes, &output.segment.output_port);
        push_string(bytes, &output.segment.semantic_key);
        push_u64(bytes, output.children.len() as u64);
        stack.extend(output.children.iter().rev());
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GraphError {
    ReservedNodeId,
    ReservedOverrideId,
    EmptyNodeName,
    DependenciesNotCanonical,
    DependencyCycle(NodeId),
    MissingDependency(NodeId),
    NodeKeyMismatch,
    ExpressionLimit,
    InvalidNumber,
    InvalidNodeReference,
    UnexpectedToken,
    UnexpectedEnd,
    UnclosedParenthesis,
    InvalidSemanticKey,
    EmptySlotPath,
    SlotPathLimit,
    RuleOutputLimit,
    RuleOutputDepthLimit,
    GraphLimit,
    DuplicatePort,
    DuplicateOverrideParameter,
    InvalidOutputProducer,
    EmptyOutputPorts,
    UnknownOutputPort,
    NonFiniteOverride,
    EmptyEvaluationIdentity,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ReservedNodeId => formatter.write_str("node ID zero is reserved"),
            Self::ReservedOverrideId => formatter.write_str("override ID zero is reserved"),
            Self::EmptyNodeName => formatter.write_str("node name is empty"),
            Self::DependenciesNotCanonical => {
                formatter.write_str("dependencies must be unique and strictly sorted")
            }
            Self::DependencyCycle(id) => write!(formatter, "dependency cycle at node {}", id.0),
            Self::MissingDependency(id) => write!(formatter, "dependency {} does not exist", id.0),
            Self::NodeKeyMismatch => formatter.write_str("node map key does not match node ID"),
            Self::ExpressionLimit => formatter.write_str("expression exceeds parser limits"),
            Self::InvalidNumber => formatter.write_str("expression number is invalid"),
            Self::InvalidNodeReference => {
                formatter.write_str("expression node reference is invalid")
            }
            Self::UnexpectedToken => formatter.write_str("expression contains an unexpected token"),
            Self::UnexpectedEnd => formatter.write_str("expression ends unexpectedly"),
            Self::UnclosedParenthesis => formatter.write_str("expression parenthesis is unclosed"),
            Self::InvalidSemanticKey => formatter.write_str("semantic key is invalid"),
            Self::EmptySlotPath => formatter.write_str("slot path must not be empty"),
            Self::SlotPathLimit => formatter.write_str("slot path exceeds its segment limit"),
            Self::RuleOutputLimit => formatter.write_str("rule outputs exceed their limit"),
            Self::RuleOutputDepthLimit => {
                formatter.write_str("rule outputs exceed their depth limit")
            }
            Self::GraphLimit => formatter.write_str("graph exceeds an allocation limit"),
            Self::DuplicatePort => formatter.write_str("typed port names must be unique"),
            Self::DuplicateOverrideParameter => {
                formatter.write_str("override parameter declarations must be unique")
            }
            Self::InvalidOutputProducer => {
                formatter.write_str("rule output has an invalid producer")
            }
            Self::EmptyOutputPorts => formatter.write_str("rule requires an output port"),
            Self::UnknownOutputPort => formatter.write_str("rule output names an undeclared port"),
            Self::NonFiniteOverride => formatter.write_str("override value is non-finite"),
            Self::EmptyEvaluationIdentity => {
                formatter.write_str("evaluation identity is incomplete")
            }
        }
    }
}

impl std::error::Error for GraphError {}
