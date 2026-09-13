use ketchup_core::assistant_sidecar::{
    AssistantApiDiagnostics, AssistantCadEditProgram, AssistantCapability, AssistantChatResult,
    AssistantDistribution, AssistantFeaReviewRequest, AssistantHandshake, AssistantModelIntent,
};
use ketchup_core::graph::sha256_hex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

pub const MAX_ASSISTANT_REQUEST_LINE_BYTES: usize = 128 * 1024;
pub const MAX_ASSISTANT_RESPONSE_LINE_BYTES: usize = 256 * 1024;
const POLL_INTERVAL: Duration = Duration::from_millis(10);
pub const MAX_ASSISTANT_EXECUTABLE_BYTES: u64 = 96 * 1024 * 1024;

fn open_guarded_executable(path: &Path) -> io::Result<fs::File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        const FILE_SHARE_READ: u32 = 0x0000_0001;
        fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        fs::File::open(path)
    }
}

#[derive(Clone, Debug, Default)]
pub struct AssistantCancellation(Arc<AtomicBool>);

impl AssistantCancellation {
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }

    pub fn shared_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AssistantProcessChatResult {
    pub result: AssistantChatResult,
    pub cad_edit_program: Option<AssistantCadEditProgram>,
    pub fea_review: Option<AssistantFeaReviewRequest>,
    pub diagnostics: Option<AssistantApiDiagnostics>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssistantProcessLaunch {
    pub executable: PathBuf,
    pub executable_sha256: String,
    pub arguments: Vec<OsString>,
    pub working_directory: PathBuf,
    pub environment: Vec<(OsString, OsString)>,
}

struct AssistantWriteRequest {
    line: String,
    acknowledgment: mpsc::Sender<Result<(), String>>,
}

#[derive(Debug)]
pub struct AssistantProcessClient {
    child: Child,
    write_sender: Option<mpsc::Sender<AssistantWriteRequest>>,
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
        #[serde(default)]
        fea_review: Box<Option<AssistantFeaReviewRequest>>,
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
        let mut command = Command::new(executable.as_ref());
        command.args(arguments);
        Self::spawn_command(command, handshake, timeout, cancelled)
    }

    pub fn spawn_isolated_with_cancellation(
        launch: &AssistantProcessLaunch,
        handshake: AssistantHandshake,
        timeout: Duration,
        cancelled: AssistantCancellation,
    ) -> Result<Self, AssistantProcessError> {
        if !launch.executable.is_absolute() || !launch.working_directory.is_absolute() {
            return Err(AssistantProcessError::Spawn(
                "isolated assistant paths must be absolute".to_owned(),
            ));
        }
        if launch.executable_sha256.len() != 64
            || !launch
                .executable_sha256
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        {
            return Err(AssistantProcessError::Spawn(
                "isolated assistant executable identity is invalid".to_owned(),
            ));
        }
        if cancelled.is_cancelled() {
            return Err(AssistantProcessError::Cancelled);
        }
        let executable_file = open_guarded_executable(&launch.executable)
            .map_err(|error| AssistantProcessError::Spawn(error.to_string()))?;
        let metadata = executable_file
            .metadata()
            .map_err(|error| AssistantProcessError::Spawn(error.to_string()))?;
        if !metadata.is_file() || metadata.len() > MAX_ASSISTANT_EXECUTABLE_BYTES {
            return Err(AssistantProcessError::Spawn(
                "isolated assistant executable is not a bounded file".to_owned(),
            ));
        }
        let mut executable = Vec::new();
        (&executable_file)
            .take(MAX_ASSISTANT_EXECUTABLE_BYTES + 1)
            .read_to_end(&mut executable)
            .map_err(|error| AssistantProcessError::Spawn(error.to_string()))?;
        if executable.len() as u64 > MAX_ASSISTANT_EXECUTABLE_BYTES {
            return Err(AssistantProcessError::Spawn(
                "isolated assistant executable is not a bounded file".to_owned(),
            ));
        }
        if sha256_hex(&executable) != launch.executable_sha256 {
            return Err(AssistantProcessError::Spawn(
                "isolated assistant executable identity mismatch".to_owned(),
            ));
        }
        let mut command = Command::new(&launch.executable);
        command
            .args(&launch.arguments)
            .current_dir(&launch.working_directory)
            .env_clear()
            .envs(launch.environment.iter().cloned());
        let result = Self::spawn_command(command, handshake, timeout, cancelled);
        drop(executable_file);
        result
    }

    fn spawn_command(
        mut command: Command,
        handshake: AssistantHandshake,
        timeout: Duration,
        cancelled: AssistantCancellation,
    ) -> Result<Self, AssistantProcessError> {
        handshake
            .validate()
            .map_err(|error| AssistantProcessError::Protocol(error.to_string()))?;
        let mut child = command
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
        let write_sender = spawn_bounded_writer(stdin);
        let receiver = spawn_bounded_reader(stdout);
        let mut client = Self {
            child,
            write_sender: Some(write_sender),
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
                fea_review,
                diagnostics,
            } if returned_id == request_id => {
                let result = AssistantChatResult {
                    message,
                    model_intent: *model_intent,
                };
                if let Err(error) = result.validate() {
                    return self.fail(AssistantProcessError::Protocol(error));
                }
                let response_action_count = usize::from(result.model_intent.is_some())
                    + usize::from(cad_edit_program.is_some())
                    + usize::from(fea_review.is_some());
                if response_action_count > 1 {
                    return self.fail(AssistantProcessError::Protocol(
                        "assistant returned multiple action programs".to_owned(),
                    ));
                }
                if let Some(program) = cad_edit_program.as_ref()
                    && let Err(error) = program.validate()
                {
                    return self.fail(AssistantProcessError::Protocol(error));
                }
                if let Some(request) = fea_review.as_ref()
                    && let Err(error) = request.validate()
                {
                    return self.fail(AssistantProcessError::Protocol(error));
                }
                if let Some(diagnostics) = diagnostics.as_ref()
                    && let Err(error) = diagnostics.validate()
                {
                    return self.fail(AssistantProcessError::Protocol(error));
                }
                Ok(AssistantProcessChatResult {
                    result,
                    cad_edit_program: *cad_edit_program,
                    fea_review: *fea_review,
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
                self.write_sender.take();
                self.wait_for_exit()?;
                self.closed = true;
                Ok(())
            }
            SidecarResponse::Error { error } => self.fail(AssistantProcessError::Remote(error)),
            _ => self.fail_protocol("assistant did not acknowledge shutdown"),
        }
    }

    fn write_json(&mut self, value: &impl Serialize) -> Result<(), AssistantProcessError> {
        let line = serde_json::to_string(value)
            .map_err(|error| AssistantProcessError::Protocol(error.to_string()))?;
        if line.len() > MAX_ASSISTANT_REQUEST_LINE_BYTES {
            return Err(AssistantProcessError::RequestLineTooLarge);
        }
        let sender = self
            .write_sender
            .as_ref()
            .ok_or(AssistantProcessError::Closed)?
            .clone();
        let (acknowledgment, receiver) = mpsc::channel();
        sender
            .send(AssistantWriteRequest {
                line,
                acknowledgment,
            })
            .map_err(|_| AssistantProcessError::Closed)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if self.cancelled.is_cancelled() {
                self.terminate();
                return Err(AssistantProcessError::Cancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                self.terminate();
                return Err(AssistantProcessError::TimedOut);
            };
            match receiver.recv_timeout(remaining.min(POLL_INTERVAL)) {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => {
                    self.terminate();
                    return Err(AssistantProcessError::Transport(error));
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.terminate();
                    return Err(AssistantProcessError::Transport(
                        "assistant request writer disconnected".to_owned(),
                    ));
                }
            }
        }
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

    fn fail<T>(&mut self, error: AssistantProcessError) -> Result<T, AssistantProcessError> {
        self.terminate();
        Err(error)
    }

    fn fail_protocol<T>(&mut self, message: &str) -> Result<T, AssistantProcessError> {
        self.fail(AssistantProcessError::Protocol(message.to_owned()))
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
        self.write_sender.take();
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

fn spawn_bounded_writer(stdin: ChildStdin) -> mpsc::Sender<AssistantWriteRequest> {
    let (sender, receiver) = mpsc::channel::<AssistantWriteRequest>();
    let _ = std::thread::spawn(move || {
        let mut stdin = stdin;
        while let Ok(request) = receiver.recv() {
            let result = writeln!(stdin, "{}", request.line)
                .and_then(|()| stdin.flush())
                .map_err(|error| error.to_string());
            let failed = result.is_err();
            let _ = request.acknowledgment.send(result);
            if failed {
                break;
            }
        }
    });
    sender
}

fn spawn_bounded_reader(
    stdout: impl io::Read + Send + 'static,
) -> Receiver<Result<Option<String>, String>> {
    let (sender, receiver) = mpsc::sync_channel(1);
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

#[cfg(all(test, windows))]
mod tests {
    use super::open_guarded_executable;
    use std::fs;

    #[test]
    fn executable_guard_denies_replacement_until_released() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("assistant.exe");
        fs::write(&path, b"trusted executable").unwrap();

        let guard = open_guarded_executable(&path).unwrap();
        assert!(
            fs::OpenOptions::new()
                .write(true)
                .truncate(true)
                .open(&path)
                .is_err()
        );
        assert!(fs::remove_file(&path).is_err());

        drop(guard);
        fs::write(&path, b"replacement").unwrap();
        fs::remove_file(path).unwrap();
    }
}
