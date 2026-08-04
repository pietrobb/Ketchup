#![forbid(unsafe_code)]

use ketchup_core::document::{DerivedIdentity, NodeId, SlotPath, SlotSegment};
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct AcceptanceIdentity {
    pub document_scope: u64,
    pub derived_identity: DerivedIdentity,
    pub input_digest: String,
    pub evaluator: String,
    pub backend: Option<String>,
    pub schema: String,
    pub tolerance: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct ScheduledIdentity {
    node_id: NodeId,
    acceptance: AcceptanceIdentity,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ScheduledVersion {
    revision_id: u64,
    generation: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobToken {
    pub node_id: NodeId,
    pub revision_id: u64,
    pub generation: u64,
    pub acceptance: AcceptanceIdentity,
}

impl JobToken {
    #[must_use]
    pub fn input_digest(&self) -> &str {
        &self.acceptance.input_digest
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DerivedResult {
    pub token: JobToken,
    pub result_fingerprint: String,
    pub charge_bytes: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InsertOutcome {
    Current,
    Stale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheStats {
    pub entry_count: usize,
    pub used_bytes: usize,
    pub budget_bytes: usize,
    pub evictions: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
struct CacheKey {
    node_id: NodeId,
    revision_id: u64,
    generation: u64,
    acceptance: AcceptanceIdentity,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    result_fingerprint: String,
    charge_bytes: usize,
}

pub struct EvaluationScheduler {
    current_revision: u64,
    generations: BTreeMap<NodeId, u64>,
    scheduled_inputs: BTreeMap<ScheduledIdentity, ScheduledVersion>,
    cache: BTreeMap<CacheKey, CacheEntry>,
    lru: VecDeque<CacheKey>,
    cache_budget_bytes: usize,
    cache_used_bytes: usize,
    evictions: u64,
}

impl EvaluationScheduler {
    #[must_use]
    pub fn new(cache_budget_bytes: usize) -> Self {
        Self {
            current_revision: 0,
            generations: BTreeMap::new(),
            scheduled_inputs: BTreeMap::new(),
            cache: BTreeMap::new(),
            lru: VecDeque::new(),
            cache_budget_bytes,
            cache_used_bytes: 0,
            evictions: 0,
        }
    }

    #[must_use]
    pub const fn current_revision(&self) -> u64 {
        self.current_revision
    }

    pub fn advance_revision(
        &mut self,
        revision_id: u64,
        dirty_nodes: impl IntoIterator<Item = NodeId>,
    ) -> Result<(), SchedulerError> {
        if revision_id <= self.current_revision {
            return Err(SchedulerError::NonMonotonicRevision {
                current: self.current_revision,
                proposed: revision_id,
            });
        }
        self.current_revision = revision_id;
        for node_id in dirty_nodes {
            *self.generations.entry(node_id).or_default() += 1;
            self.scheduled_inputs
                .retain(|identity, _| identity.node_id != node_id);
        }
        Ok(())
    }

    pub fn schedule(
        &mut self,
        node_id: NodeId,
        input_digest: impl Into<String>,
    ) -> Result<JobToken, SchedulerError> {
        let input_digest = input_digest.into();
        if input_digest.is_empty() {
            return Err(SchedulerError::EmptyInputDigest);
        }
        let segment = SlotSegment::new(node_id, "value", "root")
            .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?;
        let identity = AcceptanceIdentity {
            document_scope: 1,
            derived_identity: DerivedIdentity::new(
                node_id,
                SlotPath::new(vec![segment])
                    .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?,
            )
            .map_err(|_| SchedulerError::InvalidAcceptanceIdentity)?,
            input_digest,
            evaluator: ketchup_core::graph::EVALUATOR_ID_V1.to_owned(),
            backend: Some(ketchup_core::graph::DEFAULT_BACKEND_ID.to_owned()),
            schema: ketchup_core::graph::GRAPH_SCHEMA_ID_V1.to_owned(),
            tolerance: ketchup_core::document::TOLERANCE_PROFILE_V1.to_owned(),
        };
        self.schedule_with_identity(node_id, identity)
    }

    pub fn schedule_with_identity(
        &mut self,
        node_id: NodeId,
        acceptance: AcceptanceIdentity,
    ) -> Result<JobToken, SchedulerError> {
        if !is_valid_acceptance_identity(node_id, &acceptance) {
            return Err(SchedulerError::InvalidAcceptanceIdentity);
        }
        let generation = *self.generations.entry(node_id).or_default();
        self.scheduled_inputs.insert(
            ScheduledIdentity {
                node_id,
                acceptance: acceptance.clone(),
            },
            ScheduledVersion {
                revision_id: self.current_revision,
                generation,
            },
        );
        Ok(JobToken {
            node_id,
            revision_id: self.current_revision,
            generation,
            acceptance,
        })
    }

    pub fn accept(&mut self, result: DerivedResult) -> InsertOutcome {
        let expected_generation = self
            .generations
            .get(&result.token.node_id)
            .copied()
            .unwrap_or_default();
        let scheduled_identity = ScheduledIdentity {
            node_id: result.token.node_id,
            acceptance: result.token.acceptance.clone(),
        };
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation: expected_generation,
        };
        if result.token.revision_id != self.current_revision
            || result.token.generation != expected_generation
            || self.scheduled_inputs.get(&scheduled_identity) != Some(&current_version)
        {
            return InsertOutcome::Stale;
        }

        let key = CacheKey {
            node_id: result.token.node_id,
            revision_id: result.token.revision_id,
            generation: result.token.generation,
            acceptance: result.token.acceptance,
        };
        self.insert_cache(
            key,
            CacheEntry {
                result_fingerprint: result.result_fingerprint,
                charge_bytes: result.charge_bytes,
            },
        );
        InsertOutcome::Current
    }

    #[must_use]
    pub fn current_result_fingerprint(&self, node_id: NodeId) -> Option<&str> {
        let generation = self.generations.get(&node_id).copied().unwrap_or_default();
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation,
        };
        let mut current_identities = self
            .scheduled_inputs
            .iter()
            .filter(|(identity, version)| {
                identity.node_id == node_id && **version == current_version
            })
            .map(|(identity, _)| &identity.acceptance);
        let acceptance = current_identities.next()?;
        if current_identities.next().is_some() {
            return None;
        }
        self.current_result_fingerprint_for(node_id, acceptance)
    }

    #[must_use]
    pub fn current_result_fingerprint_for(
        &self,
        node_id: NodeId,
        acceptance: &AcceptanceIdentity,
    ) -> Option<&str> {
        let generation = self.generations.get(&node_id).copied().unwrap_or_default();
        let scheduled_identity = ScheduledIdentity {
            node_id,
            acceptance: acceptance.clone(),
        };
        let current_version = ScheduledVersion {
            revision_id: self.current_revision,
            generation,
        };
        if self.scheduled_inputs.get(&scheduled_identity) != Some(&current_version) {
            return None;
        }
        let key = CacheKey {
            node_id,
            revision_id: self.current_revision,
            generation,
            acceptance: acceptance.clone(),
        };
        self.cache
            .get(&key)
            .map(|entry| entry.result_fingerprint.as_str())
    }

    #[must_use]
    pub fn cache_stats(&self) -> CacheStats {
        CacheStats {
            entry_count: self.cache.len(),
            used_bytes: self.cache_used_bytes,
            budget_bytes: self.cache_budget_bytes,
            evictions: self.evictions,
        }
    }

    fn insert_cache(&mut self, key: CacheKey, entry: CacheEntry) {
        if entry.charge_bytes > self.cache_budget_bytes {
            return;
        }
        if let Some(replaced) = self.cache.remove(&key) {
            self.cache_used_bytes -= replaced.charge_bytes;
            self.lru.retain(|candidate| candidate != &key);
        }
        self.cache_used_bytes += entry.charge_bytes;
        self.lru.push_back(key.clone());
        self.cache.insert(key, entry);

        while self.cache_used_bytes > self.cache_budget_bytes {
            let Some(oldest) = self.lru.pop_front() else {
                break;
            };
            if let Some(evicted) = self.cache.remove(&oldest) {
                self.cache_used_bytes -= evicted.charge_bytes;
                self.evictions += 1;
            }
        }
    }
}

fn is_valid_acceptance_identity(node_id: NodeId, acceptance: &AcceptanceIdentity) -> bool {
    let root_rule_node_id = acceptance.derived_identity.root_rule_node_id;
    let segments = acceptance.derived_identity.slot_path.segments();
    acceptance.document_scope != 0
        && root_rule_node_id == node_id
        && DerivedIdentity::new(
            root_rule_node_id,
            acceptance.derived_identity.slot_path.clone(),
        )
        .is_ok()
        && SlotPath::new(segments.to_vec()).is_ok()
        && segments.iter().all(|segment| {
            segment.producer_rule_id == root_rule_node_id
                && SlotSegment::new(
                    segment.producer_rule_id,
                    &segment.output_port,
                    &segment.semantic_key,
                )
                .is_ok()
        })
        && !acceptance.input_digest.is_empty()
        && !acceptance.evaluator.is_empty()
        && !acceptance.schema.is_empty()
        && !acceptance.tolerance.is_empty()
        && !acceptance.backend.as_ref().is_some_and(String::is_empty)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NonMonotonicRevision { current: u64, proposed: u64 },
    EmptyInputDigest,
    InvalidAcceptanceIdentity,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicRevision { current, proposed } => write!(
                formatter,
                "revision {proposed} does not advance current revision {current}"
            ),
            Self::EmptyInputDigest => formatter.write_str("scheduler input digest is empty"),
            Self::InvalidAcceptanceIdentity => {
                formatter.write_str("scheduler acceptance identity is incomplete")
            }
        }
    }
}

impl std::error::Error for SchedulerError {}

#[derive(Clone, Debug, PartialEq)]
pub struct WorkerExactResult {
    pub backend_duration: Duration,
    pub result_fingerprint: String,
    pub volume_mm3: f64,
    pub bounds_mm: [f64; 6],
    pub topology_counts: [u32; 5],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkerError {
    Spawn(String),
    Transport(String),
    WorkerExited,
    Protocol(String),
    Geometry(String),
}

impl fmt::Display for WorkerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "worker spawn failed: {message}"),
            Self::Transport(message) => write!(formatter, "worker transport failed: {message}"),
            Self::WorkerExited => formatter.write_str("worker exited before replying"),
            Self::Protocol(message) => write!(formatter, "worker protocol error: {message}"),
            Self::Geometry(code) => write!(formatter, "worker geometry error: {code}"),
        }
    }
}

impl std::error::Error for WorkerError {}

pub struct ExactWorkerClient {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl ExactWorkerClient {
    pub fn spawn(executable: impl AsRef<Path>) -> Result<Self, WorkerError> {
        let mut child = Command::new(executable.as_ref())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| WorkerError::Spawn(error.to_string()))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdin was not piped".to_owned()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| WorkerError::Spawn("worker stdout was not piped".to_owned()))?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
        })
    }

    pub fn ping(&mut self) -> Result<(), WorkerError> {
        let response = self.request("PING")?;
        if response == "PONG" {
            Ok(())
        } else {
            Err(WorkerError::Protocol(response))
        }
    }

    pub fn extrude_rectangle(&mut self, height_mm: f64) -> Result<WorkerExactResult, WorkerError> {
        let response = self.request(&format!("EXTRUDE {:016x}", height_mm.to_bits()))?;
        parse_exact_result(&response)
    }

    pub fn exception_probe(&mut self) -> Result<String, WorkerError> {
        let response = self.request("EXCEPTION")?;
        let mut fields = response.split_whitespace();
        match (fields.next(), fields.next(), fields.next()) {
            (Some("ERR"), Some(code), None) => Ok(code.to_owned()),
            _ => Err(WorkerError::Protocol(response)),
        }
    }

    pub fn begin_killable_job(&mut self, duration: Duration) -> Result<(), WorkerError> {
        self.write_request(&format!("SLEEP {}", duration.as_millis()))
    }

    pub fn crash(&mut self) -> Result<(), WorkerError> {
        self.write_request("CRASH")?;
        let mut response = String::new();
        let bytes = self
            .stdout
            .read_line(&mut response)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        if bytes == 0 {
            let _ = self.child.wait();
            Ok(())
        } else {
            Err(WorkerError::Protocol(response))
        }
    }

    pub fn cancel(mut self) -> Result<Duration, WorkerError> {
        let started = Instant::now();
        self.child
            .kill()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        self.child
            .wait()
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        Ok(started.elapsed())
    }

    fn request(&mut self, request: &str) -> Result<String, WorkerError> {
        self.write_request(request)?;
        let mut response = String::new();
        let bytes = self
            .stdout
            .read_line(&mut response)
            .map_err(|error| WorkerError::Transport(error.to_string()))?;
        if bytes == 0 {
            return Err(WorkerError::WorkerExited);
        }
        Ok(response.trim_end().to_owned())
    }

    fn write_request(&mut self, request: &str) -> Result<(), WorkerError> {
        writeln!(self.stdin, "{request}")
            .and_then(|()| self.stdin.flush())
            .map_err(|error| WorkerError::Transport(error.to_string()))
    }
}

impl Drop for ExactWorkerClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn parse_exact_result(response: &str) -> Result<WorkerExactResult, WorkerError> {
    let fields: Vec<_> = response.split_whitespace().collect();
    if fields.first() == Some(&"ERR") {
        return Err(WorkerError::Geometry(
            fields.get(1).copied().unwrap_or("unknown").to_owned(),
        ));
    }
    if fields.len() != 15 || fields[0] != "OK" {
        return Err(WorkerError::Protocol(response.to_owned()));
    }
    let parse_u64 = |index: usize| {
        fields[index]
            .parse::<u64>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_f64 = |index: usize| {
        u64::from_str_radix(fields[index], 16)
            .map(f64::from_bits)
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    let parse_u32 = |index: usize| {
        fields[index]
            .parse::<u32>()
            .map_err(|_| WorkerError::Protocol(response.to_owned()))
    };
    Ok(WorkerExactResult {
        backend_duration: Duration::from_nanos(parse_u64(1)?),
        result_fingerprint: fields[2].to_owned(),
        volume_mm3: parse_f64(3)?,
        bounds_mm: [
            parse_f64(4)?,
            parse_f64(5)?,
            parse_f64(6)?,
            parse_f64(7)?,
            parse_f64(8)?,
            parse_f64(9)?,
        ],
        topology_counts: [
            parse_u32(10)?,
            parse_u32(11)?,
            parse_u32(12)?,
            parse_u32(13)?,
            parse_u32(14)?,
        ],
    })
}
