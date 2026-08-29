use ketchup_core::assistant_sidecar::{
    AssistantApiDiagnostics, AssistantCadEditProgram, AssistantCapability, AssistantChatResult,
    AssistantDistribution, AssistantHandshake, AssistantModelIntent,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const MAX_ASSISTANT_REQUEST_LINE_BYTES: usize = 128 * 1024;
pub const MAX_ASSISTANT_RESPONSE_LINE_BYTES: usize = 256 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);

#[derive(Clone, Debug, Default)]
pub struct AssistantCancellation(Arc<AtomicBool>);

impl AssistantCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantProcessChatResult {
    pub result: AssistantChatResult,
    pub cad_edit_program: Option<AssistantCadEditProgram>,
    pub diagnostics: Option<AssistantApiDiagnostics>,
}

#[derive(Debug)]
pub struct AssistantProcessClient {
    child: Child,
    stdin: Option<ChildStdin>,
    receiver: Receiver<Result<Option<String>, String>>,
    timeout: Duration,
    cancelled: AssistantCancellation,
    closed: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum SidecarResponse {
    Ready {
        protocol_version: u16,
        distribution: AssistantDistribution,
        provider: String,
        model: String,
        capabilities: BTreeSet<AssistantCapability>,
    },
    ChatResult {
        request_id: String,
        message: String,
        model_intent: Box<Option<AssistantModelIntent>>,
        #[serde(default)]
        cad_edit_program: Box<Option<AssistantCadEditProgram>>,
        diagnostics: Option<Box<AssistantApiDiagnostics>>,
    },
    Error {
        error: String,
    },
    Bye,
}

#[derive(Serialize)]
struct HelloRequest<'a> {
    #[serde(rename = "type")]
    request_type: &'static str,
    protocol_version: u16,
    distribution: AssistantDistribution,
    provider: &'a str,
    model: &'a str,
    capabilities: &'a BTreeSet<AssistantCapability>,
}

#[derive(Serialize)]
struct ChatRequest<'a> {
    #[serde(rename = "type")]
    request_type: &'static str,
    request_id: &'a str,
    message: &'a str,
    context: &'a Value,
}

impl AssistantProcessClient {
    pub fn spawn(
        executable: impl AsRef<Path>,
        arguments: &[OsString],
        handshake: AssistantHandshake,
        timeout: Duration,
    ) -> Result<Self, AssistantProcessError> {
        Self::spawn_with_cancellation(
            executable,
            arguments,
            handshake,
            timeout,
            AssistantCancellation::default(),
        )
    }

    pub fn spawn_with_cancellation(
        executable: impl AsRef<Path>,
        arguments: &[OsString],
        handshake: AssistantHandshake,
        timeout: Duration,
        cancelled: AssistantCancellation,
    ) -> Result<Self, AssistantProcessError> {
        handshake
            .validate()
            .map_err(|error| AssistantProcessError::Protocol(error.to_string()))?;
        let mut child = Command::new(executable.as_ref())
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| AssistantProcessError::Spawn(error.to_string()))?;
        let stdin = child.stdin.take().ok_or_else(|| {
            AssistantProcessError::Spawn("assistant stdin was not piped".to_owned())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            AssistantProcessError::Spawn("assistant stdout was not piped".to_owned())
        })?;
        let receiver = spawn_bounded_reader(stdout);
        let mut client = Self {
            child,
            stdin: Some(stdin),
            receiver,
            timeout,
            cancelled,
            closed: false,
        };
        let hello = HelloRequest {
            request_type: "hello",
            protocol_version: handshake.protocol_version,
            distribution: handshake.distribution,
            provider: &handshake.provider,
            model: &handshake.model,
            capabilities: &handshake.capabilities,
        };
        if let Err(error) =
            client
                .write_json(&hello)
                .and_then(|()| match client.receive_response()? {
                    SidecarResponse::Ready {
                        protocol_version,
                        distribution,
                        provider,
                        model,
                        capabilities,
                    } if protocol_version == handshake.protocol_version
                        && distribution == handshake.distribution
                        && provider == handshake.provider
                        && model == handshake.model
                        && capabilities == handshake.capabilities =>
                    {
                        Ok(())
                    }
                    SidecarResponse::Error { error } => Err(AssistantProcessError::Remote(error)),
                    _ => Err(AssistantProcessError::Protocol(
                        "assistant returned a mismatched handshake".to_owned(),
                    )),
                })
        {
            client.terminate();
            return Err(error);
        }
        Ok(client)
    }

    pub fn chat(
        &mut self,
        request_id: &str,
        message: &str,
        context: &Value,
    ) -> Result<AssistantChatResult, AssistantProcessError> {
        self.chat_exchange(request_id, message, context)
            .map(|exchange| exchange.result)
    }

    pub fn chat_exchange(
        &mut self,
        request_id: &str,
        message: &str,
        context: &Value,
    ) -> Result<AssistantProcessChatResult, AssistantProcessError> {
        if self.closed {
            return Err(AssistantProcessError::Closed);
        }
        if request_id.is_empty() || message.is_empty() {
            return Err(AssistantProcessError::Protocol(
                "request id and message must be non-empty".to_owned(),
            ));
        }
        self.write_json(&ChatRequest {
            request_type: "chat",
            request_id,
            message,
            context,
        })?;
        match self.receive_response()? {
            SidecarResponse::ChatResult {
                request_id: returned_id,
                message,
                model_intent,
                cad_edit_program,
                diagnostics,
            } if returned_id == request_id => {
                let result = AssistantChatResult {
                    message,
                    model_intent: *model_intent,
                };
                result.validate().map_err(AssistantProcessError::Protocol)?;
                if result.model_intent.is_some() && cad_edit_program.is_some() {
                    return Err(AssistantProcessError::Protocol(
                        "assistant returned multiple mutation programs".to_owned(),
                    ));
                }
                if let Some(program) = cad_edit_program.as_ref() {
                    program
                        .validate()
                        .map_err(AssistantProcessError::Protocol)?;
                }
                if let Some(diagnostics) = diagnostics.as_ref() {
                    diagnostics
                        .validate()
                        .map_err(AssistantProcessError::Protocol)?;
                }
                Ok(AssistantProcessChatResult {
                    result,
                    cad_edit_program: *cad_edit_program,
                    diagnostics: diagnostics.map(|diagnostics| *diagnostics),
                })
            }
            SidecarResponse::Error { error } => Err(AssistantProcessError::Remote(error)),
            _ => self.fail_protocol("assistant returned a mismatched chat response"),
        }
    }

    pub fn cancellation(&self) -> AssistantCancellation {
        self.cancelled.clone()
    }

    pub fn shutdown(&mut self) -> Result<(), AssistantProcessError> {
        if self.closed {
            return Ok(());
        }
        self.write_json(&serde_json::json!({"type": "shutdown"}))?;
        match self.receive_response()? {
            SidecarResponse::Bye => {
                self.stdin.take();
                self.wait_for_exit()?;
                self.closed = true;
                Ok(())
            }
            SidecarResponse::Error { error } => Err(AssistantProcessError::Remote(error)),
            _ => self.fail_protocol("assistant did not acknowledge shutdown"),
        }
    }

    fn write_json(&mut self, value: &impl Serialize) -> Result<(), AssistantProcessError> {
        let line = serde_json::to_string(value)
            .map_err(|error| AssistantProcessError::Protocol(error.to_string()))?;
        if line.len() > MAX_ASSISTANT_REQUEST_LINE_BYTES {
            return Err(AssistantProcessError::RequestLineTooLarge);
        }
        let stdin = self.stdin.as_mut().ok_or(AssistantProcessError::Closed)?;
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|error| AssistantProcessError::Transport(error.to_string()))
    }

    fn receive_response(&mut self) -> Result<SidecarResponse, AssistantProcessError> {
        let deadline = Instant::now() + self.timeout;
        let line = match receive_line(&self.receiver, deadline, &self.cancelled) {
            Ok(Some(line)) => line,
            Ok(None) => {
                self.terminate();
                return Err(AssistantProcessError::Exited);
            }
            Err(error) => {
                self.terminate();
                return Err(error);
            }
        };
        serde_json::from_str(&line).map_err(|error| {
            self.terminate();
            AssistantProcessError::Protocol(error.to_string())
        })
    }

    fn fail_protocol<T>(&mut self, message: &str) -> Result<T, AssistantProcessError> {
        self.terminate();
        Err(AssistantProcessError::Protocol(message.to_owned()))
    }

    fn wait_for_exit(&mut self) -> Result<(), AssistantProcessError> {
        let deadline = Instant::now() + self.timeout;
        loop {
            if self.cancelled.is_cancelled() {
                self.terminate();
                return Err(AssistantProcessError::Cancelled);
            }
            if let Some(status) = self
                .child
                .try_wait()
                .map_err(|error| AssistantProcessError::Transport(error.to_string()))?
            {
                return if status.success() {
                    Ok(())
                } else {
                    Err(AssistantProcessError::Exited)
                };
            }
            if Instant::now() >= deadline {
                self.terminate();
                return Err(AssistantProcessError::TimedOut);
            }
            std::thread::sleep(POLL_INTERVAL);
        }
    }

    fn terminate(&mut self) {
        self.stdin.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.closed = true;
    }
}

impl Drop for AssistantProcessClient {
    fn drop(&mut self) {
        if !self.closed {
            self.terminate();
        }
    }
}

fn spawn_bounded_reader(
    stdout: impl io::Read + Send + 'static,
) -> Receiver<Result<Option<String>, String>> {
    let (sender, receiver) = mpsc::channel();
    let _ = std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        loop {
            let line = read_bounded_line(&mut reader, MAX_ASSISTANT_RESPONSE_LINE_BYTES)
                .map_err(|error| error.to_string());
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
                "assistant response line exceeded byte limit",
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
    String::from_utf8(bytes).map(Some).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "assistant response was not UTF-8",
        )
    })
}

fn receive_line(
    receiver: &Receiver<Result<Option<String>, String>>,
    deadline: Instant,
    cancelled: &AssistantCancellation,
) -> Result<Option<String>, AssistantProcessError> {
    loop {
        if cancelled.is_cancelled() {
            return Err(AssistantProcessError::Cancelled);
        }
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(AssistantProcessError::TimedOut);
        };
        match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
            Ok(Ok(line)) => return Ok(line),
            Ok(Err(error)) => return Err(AssistantProcessError::Transport(error)),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                return Err(AssistantProcessError::Transport(
                    "assistant reader disconnected".to_owned(),
                ));
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AssistantProcessError {
    Spawn(String),
    Transport(String),
    Protocol(String),
    Remote(String),
    RequestLineTooLarge,
    Exited,
    TimedOut,
    Cancelled,
    Closed,
}

impl fmt::Display for AssistantProcessError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spawn(error) => write!(formatter, "assistant spawn failed: {error}"),
            Self::Transport(error) => write!(formatter, "assistant transport failed: {error}"),
            Self::Protocol(error) => write!(formatter, "assistant protocol failed: {error}"),
            Self::Remote(error) => write!(formatter, "assistant failed: {error}"),
            Self::RequestLineTooLarge => {
                formatter.write_str("assistant request exceeded byte limit")
            }
            Self::Exited => formatter.write_str("assistant process exited unexpectedly"),
            Self::TimedOut => formatter.write_str("assistant process timed out"),
            Self::Cancelled => formatter.write_str("assistant process was cancelled"),
            Self::Closed => formatter.write_str("assistant process is closed"),
        }
    }
}

impl std::error::Error for AssistantProcessError {}
