#![forbid(unsafe_code)]

use ketchup_core::document::NodeId;
use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JobToken {
    pub node_id: NodeId,
    pub revision_id: u64,
    pub generation: u64,
    pub input_digest: String,
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
    input_digest: String,
}

#[derive(Clone, Debug)]
struct CacheEntry {
    result_fingerprint: String,
    charge_bytes: usize,
}

pub struct EvaluationScheduler {
    current_revision: u64,
    generations: BTreeMap<NodeId, u64>,
    scheduled_inputs: BTreeMap<NodeId, String>,
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
            self.scheduled_inputs.remove(&node_id);
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
        self.scheduled_inputs.insert(node_id, input_digest.clone());
        Ok(JobToken {
            node_id,
            revision_id: self.current_revision,
            generation: *self.generations.entry(node_id).or_default(),
            input_digest,
        })
    }

    pub fn accept(&mut self, result: DerivedResult) -> InsertOutcome {
        let expected_generation = self
            .generations
            .get(&result.token.node_id)
            .copied()
            .unwrap_or_default();
        let expected_digest = self.scheduled_inputs.get(&result.token.node_id);
        if result.token.revision_id != self.current_revision
            || result.token.generation != expected_generation
            || expected_digest != Some(&result.token.input_digest)
        {
            return InsertOutcome::Stale;
        }

        let key = CacheKey {
            node_id: result.token.node_id,
            revision_id: result.token.revision_id,
            generation: result.token.generation,
            input_digest: result.token.input_digest,
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
        let input_digest = self.scheduled_inputs.get(&node_id)?;
        let key = CacheKey {
            node_id,
            revision_id: self.current_revision,
            generation,
            input_digest: input_digest.clone(),
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SchedulerError {
    NonMonotonicRevision { current: u64, proposed: u64 },
    EmptyInputDigest,
}

impl fmt::Display for SchedulerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonMonotonicRevision { current, proposed } => write!(
                formatter,
                "revision {proposed} does not advance current revision {current}"
            ),
            Self::EmptyInputDigest => formatter.write_str("scheduler input digest is empty"),
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
