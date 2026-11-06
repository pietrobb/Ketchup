//! Connects this Kečup as the `ketchup` MCP server of the AI clients the user
//! has (Claude Code, Codex, Claude Desktop, Supervisor), and tells what each one
//! would start now. Connect writes the client's own configuration file, so it
//! works when the client's command is not on the window's PATH. A client whose
//! entry names a program that is gone only reports "connection closed", so the
//! window and `--mcp-setup` name that program and replace it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

use super::mcp_config::{
    Rewrite, SERVER, codex_command, codex_server_off, invalid_data, json_server_off, read_or_empty,
    replace_file, with_codex_server, with_json_server,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Harness {
    ClaudeCode,
    Codex,
    ClaudeDesktop,
    Supervisor,
}

impl Harness {
    pub const ALL: [Harness; 4] = [
        Harness::ClaudeCode,
        Harness::Codex,
        Harness::ClaudeDesktop,
        Harness::Supervisor,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
            Harness::ClaudeDesktop => "Claude Desktop",
            Harness::Supervisor => "Supervisor",
        }
    }

    /// The client's command line program, for the clients that have one.
    fn cli(self) -> Option<&'static str> {
        match self {
            Harness::ClaudeCode => Some("claude"),
            Harness::Codex => Some("codex"),
            Harness::ClaudeDesktop | Harness::Supervisor => None,
        }
    }

    /// The command line to paste into a terminal: `program mcp add`, for every
    /// project of the user.
    pub fn add_command_line(self, program: &str, executable: &str) -> String {
        // Without a scope Claude Code keeps the server for the current folder only.
        let scope = if self == Harness::ClaudeCode {
            " --scope user"
        } else {
            ""
        };
        format!("{program} mcp add{scope} {SERVER} -- \"{executable}\" --mcp")
    }
}

/// The development checkout's server script, relative to the checkout.
const CHECKOUT_SERVER: [&str; 2] = ["scripts", "ketchup-mcp-server.cmd"];

/// The program a client should start for this Kečup. A build inside a
/// checkout's `target` (a cargo build, or a window's copy under
/// `target\mcp-windows`) is replaced or deleted by the next build, so there it
/// is the checkout's server script, which always starts the newest build.
pub fn entry_point(executable: &Path) -> PathBuf {
    executable
        .ancestors()
        .filter(|directory| directory.file_name() == Some(OsStr::new("target")))
        .filter_map(Path::parent)
        .map(|checkout| checkout.join(CHECKOUT_SERVER.iter().collect::<PathBuf>()))
        .find(|script| script.is_file())
        .unwrap_or_else(|| executable.to_owned())
}

/// What a client starts for `ketchup`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Registration {
    /// The client is not on this computer.
    NotInstalled,
    /// Installed, without a `ketchup` server for every project.
    NotSetUp,
    /// It starts this Kečup.
    ThisKetchup,
    /// It starts another program: another Kečup, a development build, or one
    /// that no longer exists.
    Elsewhere { command: String, exists: bool },
    /// Its `ketchup` entry starts no program: it is turned off or a network
    /// server (`entry` is its command or URL). Connect turns it on (review
    /// 2026-11-04 r19 P3: it showed as set up).
    Off { entry: String },
    /// Its file could not be read or is not JSON or TOML. Connect would refuse
    /// to write it, so the row offers no Connect.
    Unreadable { reason: RowError },
    /// Supervisor only: the running instances could not be listed.
    InstancesUnlisted { reason: RowError },
}

/// Why a row cannot tell what a client starts: the error itself, shared by the
/// copies of the row. Two are equal when they say the same.
#[derive(Clone, Debug)]
pub struct RowError(std::sync::Arc<std::io::Error>);

impl From<std::io::Error> for RowError {
    fn from(error: std::io::Error) -> Self {
        Self(std::sync::Arc::new(error))
    }
}

impl PartialEq for RowError {
    fn eq(&self, other: &Self) -> bool {
        self.0.kind() == other.0.kind() && self.0.to_string() == other.0.to_string()
    }
}

impl Eq for RowError {}

impl std::fmt::Display for RowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for RowError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

/// One client, what it starts and, after a Connect, how that went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessRow {
    pub harness: Harness,
    /// The configuration file the row reads and Connect writes, if known.
    pub file: Option<PathBuf>,
    pub registration: Registration,
    pub setup_error: Option<String>,
}

/// Where the clients keep their configuration and how they are found.
pub struct Environment {
    pub home: PathBuf,
    pub path: OsString,
    /// `%APPDATA%`: Claude Desktop's folder and npm's commands on Windows.
    pub app_data: Option<PathBuf>,
    /// `%LOCALAPPDATA%`: the installed Supervisor on Windows.
    pub local_app_data: Option<PathBuf>,
    /// `CLAUDE_CONFIG_DIR`, where Claude Code keeps `.claude.json` instead of home.
    pub claude_config_dir: Option<PathBuf>,
    /// `CODEX_HOME`, Codex's folder instead of `~/.codex`.
    pub codex_home: Option<PathBuf>,
    /// Folders of Supervisor instances. Each reads the MCP servers from its
    /// own `skills\mcp_servers.json`, next to its `skills\mcp_bridge.py`. An
    /// error when the system did not list the running programs.
    pub supervisor_folders: Result<Vec<PathBuf>, RowError>,
}

impl Environment {
    /// Only `home`: no PATH and no other folders.
    pub fn in_home(home: PathBuf) -> Self {
        Self {
            home,
            path: OsString::new(),
            app_data: None,
            local_app_data: None,
            claude_config_dir: None,
            codex_home: None,
            supervisor_folders: Ok(Vec::new()),
        }
    }

    pub fn current() -> Option<Self> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
        let folder = |name| std::env::var_os(name).map(PathBuf::from);
        Some(Self {
            path: std::env::var_os("PATH").unwrap_or_default(),
            app_data: folder("APPDATA"),
            local_app_data: folder("LOCALAPPDATA"),
            claude_config_dir: folder("CLAUDE_CONFIG_DIR"),
            codex_home: folder("CODEX_HOME"),
            supervisor_folders: super::running_programs::folders_of_running("supervisor")
                .map_err(RowError::from),
            ..Self::in_home(PathBuf::from(home))
        })
    }

    /// The file where `harness` keeps its MCP servers, if it has one here.
    pub fn config_file(&self, harness: Harness) -> Option<PathBuf> {
        Some(match harness {
            Harness::ClaudeCode => self
                .claude_config_dir
                .clone()
                .unwrap_or_else(|| self.home.clone())
                .join(".claude.json"),
            Harness::Codex => self
                .codex_home
                .clone()
                .unwrap_or_else(|| self.home.join(".codex"))
                .join("config.toml"),
            Harness::ClaudeDesktop => {
                let folder = if cfg!(windows) {
                    self.app_data.as_ref()?.join("Claude")
                } else if cfg!(target_os = "macos") {
                    self.home
                        .join("Library")
                        .join("Application Support")
                        .join("Claude")
                } else {
                    self.home.join(".config").join("Claude")
                };
                folder.join("claude_desktop_config.json")
            }
            Harness::Supervisor => self
                .local_app_data
                .as_ref()?
                .join("Programs")
                .join("Supervisor")
                .join("skills")
                .join("mcp_servers.json"),
        })
    }

    /// The client's command: on PATH, or where its installer puts it, since a
    /// window keeps the PATH of the time the user signed in.
    fn cli(&self, harness: Harness) -> Option<PathBuf> {
        let name = harness.cli()?;
        let mut folders: Vec<PathBuf> = std::env::split_paths(&self.path).collect();
        folders.push(self.home.join(".local").join("bin"));
        folders.extend(self.app_data.iter().map(|folder| folder.join("npm")));
        find_program(&folders, name)
    }

    /// Whether the client is on this computer: its command, or the folder it
    /// keeps its configuration in (for Claude Code, the file itself, since that
    /// is in the home folder).
    fn installed(&self, harness: Harness, file: Option<&Path>) -> bool {
        if self.cli(harness).is_some() {
            return true;
        }
        match file {
            Some(file) if harness == Harness::ClaudeCode => file.is_file(),
            Some(file) => file.parent().is_some_and(Path::is_dir),
            None => false,
        }
    }

    /// Every file `harness` reads its MCP servers from: one per Supervisor
    /// instance (the installed one first), one for each other client.
    pub fn config_files(&self, harness: Harness) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = self.config_file(harness).into_iter().collect();
        if harness == Harness::Supervisor {
            for folder in self.supervisor_folders.iter().flatten() {
                let skills = folder.join("skills");
                let file = skills.join("mcp_servers.json");
                // Without the bridge the instance reads no MCP servers.
                if skills.join("mcp_bridge.py").is_file()
                    && !files.iter().any(|known| same_program(known, &file))
                {
                    files.push(file);
                }
            }
        }
        files
    }

    /// The program the client starts as `ketchup` for every project, if any,
    /// and those Claude Code starts in a project of its own `ketchup` entry,
    /// which wins over the user's there (review 2026-11-04 SEC-142). A missing
    /// or empty file has none; one that cannot be read or is not its format is
    /// the error that says why (review 2026-11-04 P3: it showed as not set up).
    fn registered_commands(
        harness: Harness,
        file: &Path,
    ) -> std::io::Result<(Option<Started>, Vec<Started>)> {
        let text = match std::fs::read_to_string(file) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            read => read?,
        };
        if text.trim().is_empty() {
            return Ok((None, Vec::new()));
        }
        match harness {
            Harness::Codex => {
                let off = codex_server_off(&text).map_err(invalid_data)?;
                let command = codex_command(&text).map_err(invalid_data)?;
                Ok((
                    off.map(Started::Off).or(command.map(Started::Program)),
                    Vec::new(),
                ))
            }
            Harness::ClaudeCode | Harness::ClaudeDesktop | Harness::Supervisor => {
                let config: serde_json::Value =
                    serde_json::from_str(&text).map_err(invalid_data)?;
                let command = |servers: &serde_json::Value| started(harness, &servers[SERVER]);
                let projects = config["projects"]
                    .as_object()
                    .filter(|_| harness == Harness::ClaudeCode)
                    .into_iter()
                    .flat_map(|projects| projects.values())
                    .filter_map(|project| command(&project["mcpServers"]))
                    .collect();
                Ok((command(&config["mcpServers"]), projects))
            }
        }
    }

    /// What the client starts, as its default file says.
    pub fn registration(&self, harness: Harness, executable: &Path) -> Registration {
        self.registration_in(harness, self.config_file(harness).as_deref(), executable)
    }

    fn registration_in(
        &self,
        harness: Harness,
        file: Option<&Path>,
        executable: &Path,
    ) -> Registration {
        let (user, projects) = match file.map(|file| Self::registered_commands(harness, file)) {
            None => (None, Vec::new()),
            Some(Ok(found)) => found,
            Some(Err(error)) => {
                return Registration::Unreadable {
                    reason: error.into(),
                };
            }
        };
        // Any entry starting another program, or none, is what some project
        // starts. One the client may still start wins (r19 SEC-160); one
        // turned off starts nothing either, so it ranks with a gone one
        // (review 2026-11-06 SEC-180).
        let elsewhere = user
            .iter()
            .chain(&projects)
            .filter_map(|started| match started {
                Started::Off(entry) => Some(Registration::Off {
                    entry: entry.clone(),
                }),
                Started::Program(command) => match self.program_of(command) {
                    Some(Some(program)) if same_program(&program, executable) => None,
                    program => Some(Registration::Elsewhere {
                        // One Kečup cannot find as the client does counts as there.
                        exists: program.is_none_or(|program| program.is_some()),
                        command: command.clone(),
                    }),
                },
            })
            .min_by_key(|found| !matches!(found, Registration::Elsewhere { exists: true, .. }));
        match (elsewhere, user.is_some()) {
            (Some(registration), _) => registration,
            (None, true) => Registration::ThisKetchup,
            (None, false) if self.installed(harness, file) => Registration::NotSetUp,
            (None, false) => Registration::NotInstalled,
        }
    }

    /// The program a client starts for `command`, found as the client finds
    /// it (review 2026-11-04 r19 SEC-160): a name without a folder on PATH, a
    /// path from `~` in the home folder, on Windows a path without its
    /// extension by PATHEXT. `Some(None)` when it is certainly not there;
    /// `None` when Kečup cannot tell: a name not on its own PATH (the client
    /// may have another), a path relative to the client's folder, or a path
    /// the system cannot answer for.
    fn program_of(&self, command: &str) -> Option<Option<PathBuf>> {
        let path = match command.strip_prefix('~') {
            Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
                self.home.join(rest.trim_start_matches(['/', '\\']))
            }
            _ => PathBuf::from(command),
        };
        if path.is_absolute() {
            let mut candidates = vec![path.clone()];
            if cfg!(windows)
                && path.extension().is_none()
                && let (Some(folder), Some(name)) = (
                    path.parent(),
                    path.file_name().and_then(|name| name.to_str()),
                )
            {
                candidates.extend(
                    extensions(name)
                        .iter()
                        .map(|extension| folder.join(format!("{name}{extension}"))),
                );
            }
            let mut unknown = false;
            for candidate in candidates {
                match presence(&candidate) {
                    Some(true) => return Some(Some(candidate)),
                    Some(false) => {}
                    None => unknown = true,
                }
            }
            return (!unknown).then_some(None);
        }
        if path.components().count() > 1 {
            return None;
        }
        let folders: Vec<PathBuf> = std::env::split_paths(&self.path).collect();
        find_program(&folders, command).map(Some)
    }

    fn row(&self, harness: Harness, file: Option<PathBuf>, executable: &Path) -> HarnessRow {
        HarnessRow {
            harness,
            registration: self.registration_in(harness, file.as_deref(), executable),
            file,
            setup_error: None,
        }
    }

    /// A row per configuration file: a client whose folder is unknown here
    /// still gets one, as not installed. When the running Supervisors could
    /// not be listed, a row says why.
    pub fn rows(&self, executable: &Path) -> Vec<HarnessRow> {
        Harness::ALL
            .into_iter()
            .flat_map(|harness| {
                let files = self.config_files(harness);
                let mut rows: Vec<HarnessRow> = if files.is_empty() {
                    vec![self.row(harness, None, executable)]
                } else {
                    files
                        .into_iter()
                        .map(|file| self.row(harness, Some(file), executable))
                        .collect()
                };
                if harness == Harness::Supervisor
                    && let Err(reason) = &self.supervisor_folders
                {
                    rows.push(HarnessRow {
                        harness,
                        file: None,
                        registration: Registration::InstancesUnlisted {
                            reason: reason.clone(),
                        },
                        setup_error: None,
                    });
                }
                rows
            })
            .collect()
    }

    /// Replaces the client's `ketchup` server with this executable, in its
    /// default configuration file.
    pub fn set_up(&self, harness: Harness, executable: &Path) -> HarnessRow {
        self.set_up_in(harness, self.config_file(harness), executable)
    }

    /// `--mcp-setup`: connects every installed client in its default file, so
    /// of Supervisor only the installed one; another running instance changes
    /// only when the user clicks Connect on its row. A client already starting
    /// another program that exists keeps it (review 2026-11-04 SEC-142: the
    /// installer runs this hidden); only a missing or gone one is replaced,
    /// each gone entry on its own (review 2026-11-06 SEC-180: a gone project
    /// entry stayed broken while another project's worked).
    pub fn set_up_installed(&self, executable: &Path) -> Vec<HarnessRow> {
        Harness::ALL
            .into_iter()
            .map(|harness| {
                let found = self.row(harness, self.config_file(harness), executable);
                let set_up = match found.registration {
                    Registration::NotSetUp => true,
                    Registration::ThisKetchup
                    | Registration::Elsewhere { .. }
                    | Registration::Off { .. } => false,
                    _ => return found,
                };
                self.written(harness, found.file, executable, Write::Installer { set_up })
            })
            .collect()
    }

    /// Replaces the `ketchup` server in `file`, the one a row describes, and
    /// of the projects' own entries those that are gone.
    pub fn set_up_in(
        &self,
        harness: Harness,
        file: Option<PathBuf>,
        executable: &Path,
    ) -> HarnessRow {
        self.written(harness, file, executable, Write::Connect)
    }

    fn written(
        &self,
        harness: Harness,
        file: Option<PathBuf>,
        executable: &Path,
        how: Write,
    ) -> HarnessRow {
        let setup_error = self
            .write(harness, file.as_deref(), executable, how)
            .err()
            .map(|error| format!("{error}"));
        HarnessRow {
            setup_error,
            ..self.row(harness, file, executable)
        }
    }

    /// Whether the JSON entry `server` starts a program that is certainly gone.
    fn gone(&self, harness: Harness, server: &serde_json::Value) -> bool {
        matches!(started(harness, server),
            Some(Started::Program(command)) if matches!(self.program_of(&command), Some(None)))
    }

    fn write(
        &self,
        harness: Harness,
        file: Option<&Path>,
        executable: &Path,
        how: Write,
    ) -> Result<(), WriteError> {
        let file = file.ok_or(WriteError::NoConfigurationFolder)?.to_owned();
        let failed = |error| WriteError::File {
            file: file.clone(),
            error,
        };
        let text = read_or_empty(&file).map_err(failed)?;
        let executable = executable.to_string_lossy();
        let installer_set_up = match how {
            Write::Connect => None,
            Write::Installer { set_up } => Some(set_up),
        };
        let text = match harness {
            Harness::Codex => {
                if installer_set_up == Some(false) {
                    let off =
                        codex_server_off(&text).map_err(|error| failed(invalid_data(error)))?;
                    let command =
                        codex_command(&text).map_err(|error| failed(invalid_data(error)))?;
                    let gone = off.is_none()
                        && command
                            .is_some_and(|command| matches!(self.program_of(&command), Some(None)));
                    if !gone {
                        return Ok(());
                    }
                }
                with_codex_server(&text, &executable).map_err(failed)?
            }
            Harness::ClaudeCode | Harness::ClaudeDesktop | Harness::Supervisor => {
                let config: serde_json::Value = if text.trim().is_empty() {
                    serde_json::Value::Null
                } else {
                    serde_json::from_str(&text).map_err(|error| failed(invalid_data(error)))?
                };
                let gone = |server: &serde_json::Value| self.gone(harness, server);
                let user = installer_set_up.is_none_or(|set_up| {
                    set_up || config["mcpServers"].get(SERVER).is_some_and(gone)
                });
                let a_project_gone = config["projects"].as_object().is_some_and(|projects| {
                    projects
                        .values()
                        .any(|project| project["mcpServers"].get(SERVER).is_some_and(gone))
                });
                if !user && !a_project_gone {
                    return Ok(());
                }
                let rewrite = Rewrite {
                    user,
                    project: &gone,
                };
                with_json_server(&text, &executable, harness == Harness::ClaudeCode, &rewrite)
                    .map_err(failed)?
            }
        };
        replace_file(&file, &text).map_err(failed)
    }
}

/// Who writes a client's file: Connect, which the user asked for, or the
/// installer, which only sets up a client without one (`set_up`) and
/// replaces entries that are gone.
#[derive(Clone, Copy)]
enum Write {
    Connect,
    Installer { set_up: bool },
}

/// Why Connect did not write a client's configuration.
#[derive(Debug)]
pub enum WriteError {
    /// The folder the client keeps it in is unknown (no `%APPDATA%`).
    NoConfigurationFolder,
    /// The file could not be read, is not a configuration, or could not be written.
    File {
        file: PathBuf,
        error: std::io::Error,
    },
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoConfigurationFolder => {
                f.write_str("the client's configuration folder is unknown")
            }
            Self::File { file, error } => write!(f, "{}: {error}", file.display()),
        }
    }
}

impl std::error::Error for WriteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoConfigurationFolder => None,
            Self::File { error, .. } => Some(error),
        }
    }
}

/// Why `--mcp-setup` left no client starting this Kečup.
#[derive(Debug)]
pub enum SetupFailure {
    /// This executable's own path is unknown.
    Executable(std::io::Error),
    /// Neither USERPROFILE nor HOME is set.
    UnknownHome,
    /// No client starts this Kečup; the report says what each one starts.
    NoHarness { report: String },
}

impl std::fmt::Display for SetupFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Executable(error) => writeln!(f, "this program's path is unknown: {error}"),
            Self::UnknownHome => writeln!(f, "the home folder is unknown"),
            Self::NoHarness { report } => f.write_str(report),
        }
    }
}

impl std::error::Error for SetupFailure {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Executable(error) => Some(error),
            Self::UnknownHome | Self::NoHarness { .. } => None,
        }
    }
}

/// What the JSON `ketchup` entry `server` of `harness` starts, if it is one.
fn started(harness: Harness, server: &serde_json::Value) -> Option<Started> {
    if let Some(off) = json_server_off(server) {
        return Some(Started::Off(off));
    }
    let command = server["command"].as_str()?;
    Some(Started::Program(if harness == Harness::Supervisor {
        expand_variables(command)
    } else {
        command.to_owned()
    }))
}

/// Whether the file at `path` is there: `None` when the system cannot say,
/// as for a folder it may not read, a volume not mounted or a disk gone
/// (review 2026-11-06 SEC-180: each such error counted as no file, and the
/// installer replaced the entry).
fn presence(path: &Path) -> Option<bool> {
    match std::fs::metadata(path) {
        Ok(metadata) => Some(metadata.is_file()),
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ) =>
        {
            Some(false)
        }
        Err(error) => {
            ketchup_rejection::log!("cannot tell whether {} is there: {error}", path.display());
            None
        }
    }
}

/// What a client's `ketchup` entry starts.
enum Started {
    Program(String),
    /// Nothing: turned off or a network server; its command or URL.
    Off(String),
}

/// The endings a shell tries after `name`: on Windows those of PATHEXT when
/// it has none of its own, else only `name` as written.
fn extensions(name: &str) -> Vec<String> {
    if !cfg!(windows) || Path::new(name).extension().is_some() {
        vec![String::new()]
    } else {
        std::env::var_os("PATHEXT")
            .map_or_else(
                || ".COM;.EXE;.BAT;.CMD".to_owned(),
                |extensions| extensions.to_string_lossy().into_owned(),
            )
            .split(';')
            .filter(|extension| extension.starts_with('.'))
            .map(str::to_lowercase)
            .collect()
    }
}

/// The program `name` in one of `folders`, as a shell would find it: on
/// Windows a name with its extension as written, one without it by PATHEXT.
fn find_program(folders: &[PathBuf], name: &str) -> Option<PathBuf> {
    let extensions = extensions(name);
    folders.iter().find_map(|folder| {
        extensions
            .iter()
            .map(|extension| folder.join(format!("{name}{extension}")))
            .find(|candidate| candidate.is_file())
    })
}

/// `text` with its variables replaced by their values, as Supervisor's
/// `mcp_bridge.py` reads its commands with `os.path.expandvars` on Windows
/// (Python's `ntpath`, review 2026-11-04 P3): `%NAME%`, `$NAME` (letters,
/// digits, `_` and `-`) and `${NAME}`; `%%` and `$$` are one `%` or `$`; text
/// in single quotes stays as written, and so does an unknown or unclosed name.
fn expand_variables(text: &str) -> String {
    let value = |name: &str| {
        // The system has no variable named so; asking for one may panic.
        (!name.is_empty() && !name.contains(['=', '\0']))
            .then(|| std::env::var_os(name))
            .flatten()
            .map(|value| value.to_string_lossy().into_owned())
    };
    let mut expanded = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(['\'', '%', '$']) {
        expanded.push_str(&rest[..start]);
        let marker = &rest[start..=start];
        let after = &rest[start + 1..];
        // What the marker and the start of `after` stand for, and how much of
        // `after` that takes.
        let (replacement, taken) = if marker == "'" {
            let taken = after.find('\'').map_or(after.len(), |end| end + 1);
            (format!("'{}", &after[..taken]), taken)
        } else if after.starts_with(marker) {
            (marker.to_owned(), 1)
        } else if marker == "%" {
            match after.find('%') {
                Some(end) => {
                    let name = &after[..end];
                    (value(name).unwrap_or_else(|| format!("%{name}%")), end + 1)
                }
                None => (format!("%{after}"), after.len()),
            }
        } else if let Some(braced) = after.strip_prefix('{') {
            match braced.find('}') {
                Some(end) => {
                    let name = &braced[..end];
                    (
                        value(name).unwrap_or_else(|| format!("${{{name}}}")),
                        end + 2,
                    )
                }
                None => (format!("${after}"), after.len()),
            }
        } else {
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(after.len());
            let name = &after[..end];
            (value(name).unwrap_or_else(|| format!("${name}")), end)
        };
        expanded.push_str(&replacement);
        rest = &after[taken..];
    }
    expanded.push_str(rest);
    expanded
}

fn same_program(command: &Path, executable: &Path) -> bool {
    match (command.canonicalize(), executable.canonicalize()) {
        (Ok(command), Ok(executable)) => command == executable,
        _ if cfg!(windows) => {
            command.to_string_lossy().to_lowercase() == executable.to_string_lossy().to_lowercase()
        }
        _ => command == executable,
    }
}

/// `Ketchup --mcp-setup`: connects every installed client and says what each
/// starts now. Ok when at least one client starts this Kečup.
pub fn set_up_installed_harnesses() -> Result<String, SetupFailure> {
    let executable = entry_point(&std::env::current_exe().map_err(SetupFailure::Executable)?);
    let environment = Environment::current().ok_or(SetupFailure::UnknownHome)?;
    let mut report = String::new();
    let mut any = false;
    for row in environment.set_up_installed(&executable) {
        any |= row.registration == Registration::ThisKetchup;
        report.push_str(&describe(&row, &executable));
        report.push('\n');
    }
    if any {
        report.push_str(
            "Restart the AI client; its ketchup tools then work in the open Kečup window.\n",
        );
        Ok(report)
    } else {
        report.push_str(&format!(
            "No client was set up. Add this command as an MCP server: \"{}\" --mcp\n",
            executable.display()
        ));
        Err(SetupFailure::NoHarness { report })
    }
}

fn describe(row: &HarnessRow, executable: &Path) -> String {
    let name = row.harness.name();
    let state = match &row.registration {
        Registration::NotInstalled => "not installed".to_owned(),
        Registration::NotSetUp => "installed, Kečup not set up".to_owned(),
        Registration::ThisKetchup => format!("starts {} --mcp", executable.display()),
        Registration::Elsewhere {
            command,
            exists: true,
        } => format!("starts another program: {command}"),
        Registration::Elsewhere {
            command,
            exists: false,
        } => format!("starts a program that does not exist: {command}"),
        Registration::Off { entry } => {
            format!("its ketchup entry is turned off or a network server: {entry}")
        }
        Registration::Unreadable { reason } => format!("could not be read: {reason}"),
        Registration::InstancesUnlisted { reason } => {
            format!("the running instances could not be listed: {reason}")
        }
    };
    let name = match &row.file {
        Some(file) if row.registration != Registration::NotInstalled => {
            format!("{name} ({})", file.display())
        }
        _ => name.to_owned(),
    };
    match &row.setup_error {
        Some(error) => format!("{name}: {state} (set-up failed: {error})"),
        None => format!("{name}: {state}"),
    }
}

#[cfg(test)]
#[path = "mcp_setup_tests.rs"]
mod tests;
