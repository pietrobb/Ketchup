use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::time::{Duration, Instant};

pub const ASSEMBLY_WORKFLOW_TRACE_SCHEMA_V1: &str = "ketchup.assembly-workflow-trace.v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowEvidenceKind {
    DiagnosticIntrospection,
    SyntheticHost,
    LlmEndToEnd,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowPhase {
    ModelContext,
    LlmWait,
    ToolWait,
    Plan,
    Commit,
    Exact,
    Validators,
    Save,
    Viewport,
}

impl WorkflowEvidenceKind {
    fn required_phases(self) -> &'static [WorkflowPhase] {
        const HOST: &[WorkflowPhase] = &[
            WorkflowPhase::ModelContext,
            WorkflowPhase::Plan,
            WorkflowPhase::Commit,
            WorkflowPhase::Exact,
            WorkflowPhase::Validators,
            WorkflowPhase::Save,
            WorkflowPhase::Viewport,
        ];
        const END_TO_END: &[WorkflowPhase] = &[
            WorkflowPhase::ModelContext,
            WorkflowPhase::LlmWait,
            WorkflowPhase::ToolWait,
            WorkflowPhase::Plan,
            WorkflowPhase::Commit,
            WorkflowPhase::Exact,
            WorkflowPhase::Validators,
            WorkflowPhase::Save,
            WorkflowPhase::Viewport,
        ];
        match self {
            Self::DiagnosticIntrospection => &[],
            Self::SyntheticHost => HOST,
            Self::LlmEndToEnd => END_TO_END,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowRunProfile {
    pub build_version: String,
    pub build_profile: String,
    pub operating_system: String,
    pub architecture: String,
    pub logical_cpu_count: usize,
    pub provider: Option<String>,
    pub model: Option<String>,
}

impl WorkflowRunProfile {
    pub fn current(provider: Option<String>, model: Option<String>) -> Self {
        Self {
            build_version: env!("CARGO_PKG_VERSION").to_owned(),
            build_profile: if cfg!(debug_assertions) {
                "debug".to_owned()
            } else {
                "release".to_owned()
            },
            operating_system: std::env::consts::OS.to_owned(),
            architecture: std::env::consts::ARCH.to_owned(),
            logical_cpu_count: std::thread::available_parallelism().map_or(1, usize::from),
            provider,
            model,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowPhaseTiming {
    pub phase: WorkflowPhase,
    pub elapsed_ms: f64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssemblyWorkflowTrace {
    pub schema: String,
    pub request_id: String,
    pub evidence_kind: WorkflowEvidenceKind,
    pub fixture_sha256: String,
    pub profile: WorkflowRunProfile,
    pub phases: Vec<WorkflowPhaseTiming>,
    pub total_elapsed_ms: f64,
    pub sequential_tool_round_trips: u32,
    pub host_process_launches: u32,
    pub complete: bool,
    pub missing_phases: Vec<WorkflowPhase>,
}

impl AssemblyWorkflowTrace {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.schema != ASSEMBLY_WORKFLOW_TRACE_SCHEMA_V1
            || self.request_id.trim().is_empty()
            || self.fixture_sha256.len() != 64
            || !self
                .fixture_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !self.total_elapsed_ms.is_finite()
            || self.total_elapsed_ms < 0.0
            || self
                .phases
                .iter()
                .any(|timing| !timing.elapsed_ms.is_finite() || timing.elapsed_ms < 0.0)
        {
            return Err("assembly workflow trace is invalid");
        }
        let present = self
            .phases
            .iter()
            .map(|timing| timing.phase)
            .collect::<BTreeSet<_>>();
        let missing = self
            .evidence_kind
            .required_phases()
            .iter()
            .copied()
            .filter(|phase| !present.contains(phase))
            .collect::<Vec<_>>();
        if self.missing_phases != missing || self.complete != missing.is_empty() {
            return Err("assembly workflow trace coverage is inconsistent");
        }
        if self.evidence_kind == WorkflowEvidenceKind::LlmEndToEnd
            && (self.profile.provider.as_deref().is_none_or(str::is_empty)
                || self.profile.model.as_deref().is_none_or(str::is_empty))
        {
            return Err("end-to-end workflow trace requires provider and model");
        }
        Ok(())
    }
}

pub struct AssemblyWorkflowTraceRecorder {
    started: Instant,
    request_id: String,
    evidence_kind: WorkflowEvidenceKind,
    fixture_sha256: String,
    profile: WorkflowRunProfile,
    phases: Vec<WorkflowPhaseTiming>,
    sequential_tool_round_trips: u32,
    host_process_launches: u32,
}

impl AssemblyWorkflowTraceRecorder {
    pub fn new(
        request_id: impl Into<String>,
        evidence_kind: WorkflowEvidenceKind,
        fixture_sha256: impl Into<String>,
        profile: WorkflowRunProfile,
    ) -> Self {
        Self {
            started: Instant::now(),
            request_id: request_id.into(),
            evidence_kind,
            fixture_sha256: fixture_sha256.into(),
            profile,
            phases: Vec::new(),
            sequential_tool_round_trips: 0,
            host_process_launches: 0,
        }
    }

    pub fn record_phase(&mut self, phase: WorkflowPhase, elapsed: Duration) {
        self.phases.push(WorkflowPhaseTiming {
            phase,
            elapsed_ms: elapsed.as_secs_f64() * 1_000.0,
        });
    }

    pub fn measure<T>(&mut self, phase: WorkflowPhase, operation: impl FnOnce() -> T) -> T {
        let started = Instant::now();
        let result = operation();
        self.record_phase(phase, started.elapsed());
        result
    }

    pub fn record_tool_round_trip(&mut self) {
        self.sequential_tool_round_trips = self.sequential_tool_round_trips.saturating_add(1);
    }

    pub fn record_host_process_launch(&mut self) {
        self.host_process_launches = self.host_process_launches.saturating_add(1);
    }

    pub fn finish(self) -> AssemblyWorkflowTrace {
        let present = self
            .phases
            .iter()
            .map(|timing| timing.phase)
            .collect::<BTreeSet<_>>();
        let missing_phases = self
            .evidence_kind
            .required_phases()
            .iter()
            .copied()
            .filter(|phase| !present.contains(phase))
            .collect::<Vec<_>>();
        AssemblyWorkflowTrace {
            schema: ASSEMBLY_WORKFLOW_TRACE_SCHEMA_V1.to_owned(),
            request_id: self.request_id,
            evidence_kind: self.evidence_kind,
            fixture_sha256: self.fixture_sha256,
            profile: self.profile,
            phases: self.phases,
            total_elapsed_ms: self.started.elapsed().as_secs_f64() * 1_000.0,
            sequential_tool_round_trips: self.sequential_tool_round_trips,
            host_process_launches: self.host_process_launches,
            complete: missing_phases.is_empty(),
            missing_phases,
        }
    }
}
