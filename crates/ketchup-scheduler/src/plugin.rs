use ketchup_assistant::extension::{
    PLUGIN_PROTOCOL_V1, PluginCapability, PluginGateway, PluginGatewayError, PluginGrant,
    PluginLimits, PluginManifest, PluginRequest, PluginResponse,
};
use ketchup_model::document::{DocumentStore, FeatureId, NodeId, Proposal, ProposalBudget};
#[cfg(windows)]
use process_wrap::std::JobObject;
use process_wrap::std::{ChildWrapper, CommandWrap};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const MAX_PLUGIN_REQUEST_LINE_BYTES: usize = 4 * 1024;
// STATE hex-encodes every gateway-authorized query byte plus a bounded header.
pub const MAX_PLUGIN_RESPONSE_LINE_BYTES: usize = 2 * PluginLimits::HOST_MAX.max_query_bytes + 32;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

fn spawn_plugin_process(
    executable: &Path,
    arguments: &[OsString],
) -> Result<Box<dyn ChildWrapper>, PluginHostError> {
    let mut command = Command::new(executable);
    command
        .args(arguments)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut command = CommandWrap::from(command);
    #[cfg(windows)]
    command.wrap(JobObject);
    command
        .spawn()
        .map_err(|error| PluginHostError::Spawn(error.to_string()))
}

#[derive(Debug)]
pub struct PluginRunResult {
    pub manifest: PluginManifest,
    pub query_count: usize,
    pub proposal: Option<Proposal>,
}

pub fn run_plugin_process(
    executable: impl AsRef<Path>,
    arguments: &[OsString],
    store: &DocumentStore,
    grant: PluginGrant,
    timeout: Duration,
    cancelled: &AtomicBool,
) -> Result<PluginRunResult, PluginHostError> {
    if cancelled.load(Ordering::Acquire) {
        return Err(PluginHostError::Cancelled);
    }
    Instant::now()
        .checked_add(timeout)
        .ok_or(PluginHostError::InvalidTimeout)?;
    let mut child = spawn_plugin_process(executable.as_ref(), arguments)?;
    let stdin = child
        .stdin()
        .take()
        .ok_or_else(|| PluginHostError::Spawn("plugin stdin was not piped".to_owned()))?;
    let writer = spawn_bounded_writer(stdin);
    let stdout = child
        .stdout()
        .take()
        .ok_or_else(|| PluginHostError::Spawn("plugin stdout was not piped".to_owned()))?;
    let receiver = spawn_bounded_reader(stdout);
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or(PluginHostError::InvalidTimeout)?;

    let outcome = (|| {
        let hello = receive_line(&receiver, deadline, cancelled)?
            .ok_or(PluginHostError::ExitedBeforeDone)?;
        let manifest = parse_manifest(&hello)?;
        let mut gateway = PluginGateway::new(manifest.clone(), grant)?;
        send_line(
            &writer,
            "READY\tketchup.plugin.v1".to_owned(),
            deadline,
            cancelled,
        )?;

        let mut query_count = 0usize;
        let mut proposal = None;
        loop {
            let line = receive_line(&receiver, deadline, cancelled)?
                .ok_or(PluginHostError::ExitedBeforeDone)?;
            if line == "DONE" {
                send_line(&writer, "BYE".to_owned(), deadline, cancelled)?;
                break;
            }
            let request = parse_request(&line)?;
            match gateway.handle(store, request)? {
                PluginResponse::AgentState(state) => {
                    query_count = query_count.saturating_add(1);
                    send_line(
                        &writer,
                        format!("STATE\t{}\t{}", state.len(), hex_encode(state.as_bytes())),
                        deadline,
                        cancelled,
                    )?;
                }
                PluginResponse::Proposal(candidate) => {
                    if proposal.is_some() {
                        return Err(PluginHostError::MultipleProposals);
                    }
                    send_line(
                        &writer,
                        format!(
                            "PROPOSAL\t{}\t{}\t{}\t{}\t{}",
                            candidate.command_digest(),
                            candidate.intended_result_digest(),
                            candidate.cost().commands,
                            candidate.cost().read_dependencies,
                            candidate.cost().write_targets
                        ),
                        deadline,
                        cancelled,
                    )?;
                    proposal = Some(*candidate);
                }
            }
        }
        Ok(PluginRunResult {
            manifest,
            query_count,
            proposal,
        })
    })();

    match outcome {
        Ok(result) => {
            // Descendants must be terminated even when the plugin exits unsuccessfully.
            let exit = wait_for_exit(child.as_mut(), deadline, cancelled);
            terminate(child.as_mut())?;
            exit?;
            Ok(result)
        }
        Err(error) => match terminate(child.as_mut()) {
            Ok(()) => Err(error),
            Err(cleanup_error) => Err(cleanup_error),
        },
    }
}

fn parse_manifest(line: &str) -> Result<PluginManifest, PluginHostError> {
    let fields = line.split('\t').collect::<Vec<_>>();
    let [
        "HELLO",
        protocol,
        plugin_id,
        name,
        principal_id,
        capabilities,
        max_requests,
        max_query_bytes,
        max_commands,
        max_read_dependencies,
        max_write_targets,
    ] = fields.as_slice()
    else {
        return Err(PluginHostError::MalformedProtocol(
            "expected 11-field HELLO".to_owned(),
        ));
    };
    if *protocol != PLUGIN_PROTOCOL_V1 {
        return Err(PluginHostError::ProtocolMismatch((*protocol).to_owned()));
    }
    let principal_id = parse_u64(principal_id, "principal id")?;
    let capabilities = if capabilities.is_empty() {
        Vec::new()
    } else {
        capabilities
            .split(',')
            .map(parse_capability)
            .collect::<Result<Vec<_>, _>>()?
    };
    let limits = PluginLimits {
        max_requests: parse_usize(max_requests, "max requests")?,
        max_query_bytes: parse_usize(max_query_bytes, "max query bytes")?,
        proposal_budget: ProposalBudget {
            max_commands: parse_usize(max_commands, "max commands")?,
            max_read_dependencies: parse_usize(max_read_dependencies, "max reads")?,
            max_write_targets: parse_usize(max_write_targets, "max writes")?,
        },
    };
    PluginManifest::new(*plugin_id, *name, principal_id, capabilities, limits)
        .map_err(PluginHostError::Gateway)
}

fn parse_capability(value: &str) -> Result<PluginCapability, PluginHostError> {
    match value {
        "query.agent-state.v1" => Ok(PluginCapability::QueryAgentState),
        "intent.set-rule-dimension.v1" => Ok(PluginCapability::SetRuleDimension),
        "intent.set-feature-dimension.v1" => Ok(PluginCapability::SetFeatureDimension),
        _ => Err(PluginHostError::MalformedProtocol(format!(
            "unknown capability {value:?}"
        ))),
    }
}

fn parse_request(line: &str) -> Result<PluginRequest, PluginHostError> {
    let fields = line.split('\t').collect::<Vec<_>>();
    match fields.as_slice() {
        ["QUERY", "AGENT_STATE"] => Ok(PluginRequest::QueryAgentState),
        ["INTENT", "SET_RULE_DIMENSION", target, value] => Ok(PluginRequest::SetRuleDimension {
            target: NodeId(parse_u64(target, "rule target")?),
            value_text: (*value).to_owned(),
        }),
        ["INTENT", "SET_FEATURE_DIMENSION", target, value] => {
            Ok(PluginRequest::SetFeatureDimension {
                target: FeatureId(parse_u64(target, "feature target")?),
                value_text: (*value).to_owned(),
            })
        }
        _ => Err(PluginHostError::MalformedProtocol(
            "request is outside the bounded plugin vocabulary".to_owned(),
        )),
    }
}

fn parse_u64(value: &str, field: &str) -> Result<u64, PluginHostError> {
    value.parse().map_err(|_: std::num::ParseIntError| {
        PluginHostError::MalformedProtocol(format!("{field} must be an unsigned integer"))
    })
}

fn parse_usize(value: &str, field: &str) -> Result<usize, PluginHostError> {
    value.parse().map_err(|_: std::num::ParseIntError| {
        PluginHostError::MalformedProtocol(format!("{field} must be an unsigned integer"))
    })
}

fn write_line(writer: &mut impl Write, line: &str) -> Result<(), PluginHostError> {
    if line.len() > MAX_PLUGIN_RESPONSE_LINE_BYTES {
        return Err(PluginHostError::ResponseLineTooLarge);
    }
    writeln!(writer, "{line}")
        .and_then(|()| writer.flush())
        .map_err(|error| PluginHostError::Transport(error.to_string()))
}

struct PluginWriteRequest {
    line: String,
    acknowledgment: mpsc::Sender<Result<(), PluginHostError>>,
}

fn spawn_bounded_writer(
    mut writer: impl Write + Send + 'static,
) -> mpsc::Sender<PluginWriteRequest> {
    let (sender, receiver) = mpsc::channel::<PluginWriteRequest>();
    let _ = std::thread::spawn(move || {
        for request in receiver {
            let result = write_line(&mut writer, &request.line);
            let terminal = result.is_err();
            if request.acknowledgment.send(result).is_err() || terminal {
                break;
            }
        }
    });
    sender
}

fn send_line(
    writer: &mpsc::Sender<PluginWriteRequest>,
    line: String,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<(), PluginHostError> {
    if line.len() > MAX_PLUGIN_RESPONSE_LINE_BYTES {
        return Err(PluginHostError::ResponseLineTooLarge);
    }
    let (acknowledgment, receiver) = mpsc::channel();
    writer
        .send(PluginWriteRequest {
            line,
            acknowledgment,
        })
        .map_err(|_: mpsc::SendError<PluginWriteRequest>| {
            PluginHostError::Transport("plugin writer disconnected".to_owned())
        })?;
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(PluginHostError::Cancelled);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(PluginHostError::TimedOut);
        };
        match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => return Err(error),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(PluginHostError::Transport(
                    "plugin writer disconnected".to_owned(),
                ));
            }
        }
    }
}

fn spawn_bounded_reader(
    stdout: impl io::Read + Send + 'static,
) -> Receiver<io::Result<Option<String>>> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let _ = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let line = read_bounded_line(&mut reader, MAX_PLUGIN_REQUEST_LINE_BYTES);
            let terminal = !matches!(line, Ok(Some(_)));
            if sender.send(line).is_err() || terminal {
                break;
            }
        }
    });
    receiver
}

fn read_bounded_line(reader: &mut impl BufRead, max_bytes: usize) -> io::Result<Option<String>> {
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if bytes.is_empty() {
                return Ok(None);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let take = newline.map_or(available.len(), |index| index + 1);
        if bytes.len().saturating_add(take) > max_bytes {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "plugin request line exceeded byte limit",
            ));
        }
        bytes.extend_from_slice(&available[..take]);
        reader.consume(take);
        if newline.is_some() {
            break;
        }
    }
    while matches!(bytes.last(), Some(b'\n' | b'\r')) {
        bytes.pop();
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.utf8_error()))
}

fn receive_line(
    receiver: &Receiver<io::Result<Option<String>>>,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<Option<String>, PluginHostError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(PluginHostError::Cancelled);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(PluginHostError::TimedOut);
        };
        match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
            Ok(Ok(line)) => return Ok(line),
            Ok(Err(error)) => return Err(PluginHostError::Transport(error.to_string())),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(PluginHostError::Transport(
                    "plugin reader disconnected".to_owned(),
                ));
            }
        }
    }
}

fn wait_for_exit(
    child: &mut dyn ChildWrapper,
    deadline: Instant,
    cancelled: &AtomicBool,
) -> Result<(), PluginHostError> {
    loop {
        if cancelled.load(Ordering::Acquire) {
            terminate(child)?;
            return Err(PluginHostError::Cancelled);
        }
        if let Some(status) = child
            .try_wait()
            .map_err(|error| PluginHostError::Transport(error.to_string()))?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(PluginHostError::ExitedUnsuccessfully)
            };
        }
        if Instant::now() >= deadline {
            terminate(child)?;
            return Err(PluginHostError::TimedOut);
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn terminate(child: &mut dyn ChildWrapper) -> Result<(), PluginHostError> {
    // Terminate the entire job without waiting for Windows kernel teardown.
    crate::child_process::terminate(child)
        .map_err(|error| PluginHostError::Transport(error.to_string()))
}

fn hex_encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

#[derive(Debug)]
pub enum PluginHostError {
    Spawn(String),
    Transport(String),
    ProtocolMismatch(String),
    MalformedProtocol(String),
    Gateway(PluginGatewayError),
    MultipleProposals,
    ResponseLineTooLarge,
    ExitedBeforeDone,
    ExitedUnsuccessfully,
    InvalidTimeout,
    TimedOut,
    Cancelled,
}

impl fmt::Display for PluginHostError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(message) => write!(formatter, "plugin spawn failed: {message}"),
            Self::Transport(message) => write!(formatter, "plugin transport failed: {message}"),
            Self::ProtocolMismatch(protocol) => {
                write!(formatter, "unsupported plugin protocol {protocol:?}")
            }
            Self::MalformedProtocol(message) => {
                write!(formatter, "malformed plugin protocol: {message}")
            }
            Self::Gateway(error) => error.fmt(formatter),
            Self::MultipleProposals => formatter.write_str("plugin emitted more than one proposal"),
            Self::ResponseLineTooLarge => {
                formatter.write_str("plugin response line exceeded byte limit")
            }
            Self::ExitedBeforeDone => formatter.write_str("plugin exited before DONE"),
            Self::ExitedUnsuccessfully => {
                formatter.write_str("plugin process exited unsuccessfully")
            }
            Self::InvalidTimeout => formatter.write_str("plugin timeout is not representable"),
            Self::TimedOut => formatter.write_str("plugin process timed out"),
            Self::Cancelled => formatter.write_str("plugin process was cancelled"),
        }
    }
}

impl std::error::Error for PluginHostError {}

impl From<PluginGatewayError> for PluginHostError {
    fn from(error: PluginGatewayError) -> Self {
        Self::Gateway(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_utf8_request_line_keeps_the_decoding_position_as_its_cause() {
        let mut reader = io::Cursor::new(b"{\"a\":\xfe}\n".to_vec());
        let error = read_bounded_line(&mut reader, 64).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let cause = error
            .get_ref()
            .and_then(|inner| inner.downcast_ref::<std::str::Utf8Error>())
            .expect("the UTF-8 decoding error is the cause");
        assert_eq!(cause.valid_up_to(), 5);
    }
}
