//! Sets this Kečup up as the `ketchup` MCP server of the AI harnesses the user
//! has (Claude Code, Codex), and tells what each one would start now. A harness
//! whose entry names a program that is gone only reports "connection closed",
//! so the window and `--mcp-setup` name that program and replace it.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The server name Kečup registers under.
const SERVER: &str = "ketchup";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Harness {
    ClaudeCode,
    Codex,
}

impl Harness {
    pub const ALL: [Harness; 2] = [Harness::ClaudeCode, Harness::Codex];

    pub fn name(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "Claude Code",
            Harness::Codex => "Codex",
        }
    }

    fn cli(self) -> &'static str {
        match self {
            Harness::ClaudeCode => "claude",
            Harness::Codex => "codex",
        }
    }

    /// The command that sets this executable up, for every project of the user.
    pub fn add_arguments(self, executable: &str) -> Vec<String> {
        let mut arguments = vec!["mcp".to_owned(), "add".to_owned()];
        if self == Harness::ClaudeCode {
            // Without a scope Claude Code keeps the server for the current folder only.
            arguments.extend(["--scope".to_owned(), "user".to_owned()]);
        }
        arguments.extend([
            SERVER.to_owned(),
            "--".to_owned(),
            executable.to_owned(),
            "--mcp".to_owned(),
        ]);
        arguments
    }

    fn remove_arguments(self) -> Vec<&'static str> {
        match self {
            Harness::ClaudeCode => vec!["mcp", "remove", SERVER, "--scope", "user"],
            Harness::Codex => vec!["mcp", "remove", SERVER],
        }
    }

    /// The command line to paste into a terminal.
    pub fn add_command_line(self, executable: &str) -> String {
        let mut line = self.cli().to_owned();
        for argument in self.add_arguments(executable) {
            line.push(' ');
            if argument == executable {
                line.push_str(&format!("\"{argument}\""));
            } else {
                line.push_str(&argument);
            }
        }
        line
    }

    /// The program the harness starts as `ketchup` for every project, if any.
    fn registered_command(self, home: &Path) -> Option<String> {
        match self {
            Harness::ClaudeCode => {
                let directory = std::env::var_os("CLAUDE_CONFIG_DIR")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.to_owned());
                let text = std::fs::read_to_string(directory.join(".claude.json")).ok()?;
                let config: serde_json::Value = serde_json::from_str(&text).ok()?;
                config["mcpServers"][SERVER]["command"]
                    .as_str()
                    .map(str::to_owned)
            }
            Harness::Codex => {
                let directory = std::env::var_os("CODEX_HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| home.join(".codex"));
                let text = std::fs::read_to_string(directory.join("config.toml")).ok()?;
                codex_command(&text)
            }
        }
    }
}

/// The development checkout's server script, relative to the checkout.
const CHECKOUT_SERVER: [&str; 2] = ["scripts", "ketchup-mcp-server.cmd"];

/// The program a harness should start for this Kečup. A build inside a
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

/// What a harness starts for `ketchup`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Registration {
    /// The harness's command is not installed.
    NotInstalled,
    /// Installed, without a `ketchup` server for every project.
    NotSetUp,
    /// It starts this Kečup.
    ThisKetchup,
    /// It starts another program: another Kečup, a development build, or one
    /// that no longer exists.
    Elsewhere { command: String, exists: bool },
}

/// One harness, what it starts and, after a set-up, how that went.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessRow {
    pub harness: Harness,
    pub registration: Registration,
    pub setup_error: Option<String>,
}

/// Where the harnesses keep their configuration and how they are found.
pub struct Environment {
    pub home: PathBuf,
    pub path: OsString,
}

impl Environment {
    pub fn current() -> Option<Self> {
        let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })?;
        Some(Self {
            home: PathBuf::from(home),
            path: std::env::var_os("PATH").unwrap_or_default(),
        })
    }

    fn cli(&self, harness: Harness) -> Option<PathBuf> {
        find_program(&self.path, harness.cli())
    }

    pub fn registration(&self, harness: Harness, executable: &Path) -> Registration {
        match harness.registered_command(&self.home) {
            Some(command) if same_program(Path::new(&command), executable) => {
                Registration::ThisKetchup
            }
            Some(command) => Registration::Elsewhere {
                exists: Path::new(&command).is_file(),
                command,
            },
            None if self.cli(harness).is_some() => Registration::NotSetUp,
            None => Registration::NotInstalled,
        }
    }

    pub fn rows(&self, executable: &Path) -> Vec<HarnessRow> {
        Harness::ALL
            .into_iter()
            .map(|harness| HarnessRow {
                harness,
                registration: self.registration(harness, executable),
                setup_error: None,
            })
            .collect()
    }

    /// Replaces the harness's `ketchup` server with this executable.
    pub fn set_up(&self, harness: Harness, executable: &Path) -> HarnessRow {
        let setup_error = match self.cli(harness) {
            None => Some(format!("the {} command was not found", harness.cli())),
            Some(cli) => {
                // A missing entry makes the removal fail; only the addition counts.
                let _ = run(&cli, harness.remove_arguments().into_iter().map(OsStr::new));
                let executable = executable.to_string_lossy().into_owned();
                run(
                    &cli,
                    harness
                        .add_arguments(&executable)
                        .iter()
                        .map(|argument| OsStr::new(argument.as_str())),
                )
                .err()
                .map(|error| format!("{error}"))
            }
        };
        HarnessRow {
            harness,
            registration: self.registration(harness, executable),
            setup_error,
        }
    }
}

/// Why a harness command did not register Kečup.
#[derive(Debug)]
pub enum RunError {
    /// The command could not start.
    Start(std::io::Error),
    /// It ran and refused, with what it printed.
    Refused { output: String },
}

impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Start(error) => write!(f, "{error}"),
            Self::Refused { output } => f.write_str(output),
        }
    }
}

impl std::error::Error for RunError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start(error) => Some(error),
            Self::Refused { .. } => None,
        }
    }
}

/// Why `--mcp-setup` left no harness starting this Kečup.
#[derive(Debug)]
pub enum SetupFailure {
    /// This executable's own path is unknown.
    Executable(std::io::Error),
    /// Neither USERPROFILE nor HOME is set.
    UnknownHome,
    /// No harness starts this Kečup; the report says what each one starts.
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

fn run<'a>(program: &Path, arguments: impl Iterator<Item = &'a OsStr>) -> Result<(), RunError> {
    let mut command = Command::new(program);
    command.args(arguments);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(RunError::Start)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let message = if stderr.trim().is_empty() {
        stdout
    } else {
        stderr
    };
    Err(RunError::Refused {
        output: message.trim().to_owned(),
    })
}

/// The program `name` on `path`, as a shell would find it.
fn find_program(path: &OsStr, name: &str) -> Option<PathBuf> {
    let extensions: &[&str] = if cfg!(windows) {
        &[".exe", ".cmd", ".bat"]
    } else {
        &[""]
    };
    std::env::split_paths(path).find_map(|directory| {
        extensions
            .iter()
            .map(|extension| directory.join(format!("{name}{extension}")))
            .find(|candidate| candidate.is_file())
    })
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

/// The `command` of `[mcp_servers.ketchup]` in a Codex config.toml.
fn codex_command(toml: &str) -> Option<String> {
    let mut in_section = false;
    for line in toml.lines().map(str::trim) {
        if line.starts_with('[') {
            let header = line.trim_matches(|c| c == '[' || c == ']').trim();
            in_section = header == format!("mcp_servers.{SERVER}")
                || header == format!("mcp_servers.\"{SERVER}\"");
            continue;
        }
        if !in_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "command" {
            continue;
        }
        let value = value.trim();
        if let Some(literal) = value.strip_prefix('\'') {
            return literal.split('\'').next().map(str::to_owned);
        }
        let basic = value.strip_prefix('"')?;
        let mut command = String::new();
        let mut characters = basic.chars();
        while let Some(character) = characters.next() {
            match character {
                '"' => return Some(command),
                '\\' => command.push(characters.next()?),
                other => command.push(other),
            }
        }
        return None;
    }
    None
}

/// `Ketchup --mcp-setup`: sets up every installed harness and says what each
/// starts now. Ok when at least one harness starts this Kečup.
pub fn set_up_installed_harnesses() -> Result<String, SetupFailure> {
    let executable = entry_point(&std::env::current_exe().map_err(SetupFailure::Executable)?);
    let environment = Environment::current().ok_or(SetupFailure::UnknownHome)?;
    let mut report = String::new();
    let mut any = false;
    for harness in Harness::ALL {
        let row = if environment.cli(harness).is_some() {
            environment.set_up(harness, &executable)
        } else {
            HarnessRow {
                harness,
                registration: Registration::NotInstalled,
                setup_error: None,
            }
        };
        any |= row.registration == Registration::ThisKetchup;
        report.push_str(&describe(&row, &executable));
        report.push('\n');
    }
    if any {
        report.push_str(
            "Start a new session of the harness; its ketchup tools then work in the open Kečup window.\n",
        );
        Ok(report)
    } else {
        report.push_str(&format!(
            "No harness was set up. Add this command as an MCP server: \"{}\" --mcp\n",
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
    };
    match &row.setup_error {
        Some(error) => format!("{name}: {state} (set-up failed: {error})"),
        None => format!("{name}: {state}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temporary_home(name: &str) -> PathBuf {
        let home =
            std::env::temp_dir().join(format!("ketchup-mcp-setup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        home
    }

    #[test]
    fn claude_code_is_set_up_for_every_project_and_codex_with_the_same_server() {
        let executable = r"C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe";
        assert_eq!(
            Harness::ClaudeCode.add_command_line(executable),
            r#"claude mcp add --scope user ketchup -- "C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe" --mcp"#
        );
        assert_eq!(
            Harness::Codex.add_command_line(executable),
            r#"codex mcp add ketchup -- "C:\Users\me\AppData\Local\Programs\Ketchup\Ketchup.exe" --mcp"#
        );
    }

    #[test]
    fn a_claude_code_entry_naming_a_deleted_build_is_reported_with_its_path() {
        let home = temporary_home("stale");
        let gone = home.join("target").join("release").join("ketchup-app.exe");
        let config = serde_json::json!({
            "mcpServers": {"ketchup": {"type": "stdio", "command": gone, "args": ["--mcp"]}}
        });
        std::fs::write(home.join(".claude.json"), config.to_string()).unwrap();
        let environment = Environment {
            home: home.clone(),
            path: OsString::new(),
        };
        let this = std::env::current_exe().unwrap();
        assert_eq!(
            environment.registration(Harness::ClaudeCode, &this),
            Registration::Elsewhere {
                command: gone.to_string_lossy().into_owned(),
                exists: false
            }
        );
        let config = serde_json::json!({
            "mcpServers": {"ketchup": {"command": this, "args": ["--mcp"]}}
        });
        std::fs::write(home.join(".claude.json"), config.to_string()).unwrap();
        assert_eq!(
            environment.registration(Harness::ClaudeCode, &this),
            Registration::ThisKetchup
        );
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_build_inside_a_checkout_is_started_through_its_server_script() {
        let checkout = temporary_home("checkout");
        let script = checkout.join("scripts").join("ketchup-mcp-server.cmd");
        std::fs::create_dir_all(script.parent().unwrap()).unwrap();
        std::fs::write(&script, "").unwrap();
        let target = checkout.join("target");
        for build in [
            target.join("release").join("ketchup-app.exe"),
            target
                .join("mcp-windows")
                .join("0123abcd")
                .join("ketchup-app.exe"),
            target
                .join("worktree")
                .join("release")
                .join("ketchup-app.exe"),
        ] {
            assert_eq!(entry_point(&build), script, "{}", build.display());
        }
        let installed = checkout
            .join("Programs")
            .join("Ketchup")
            .join("Ketchup.exe");
        assert_eq!(entry_point(&installed), installed);
        std::fs::remove_file(&script).unwrap();
        let build = target.join("release").join("ketchup-app.exe");
        assert_eq!(entry_point(&build), build);
        let _ = std::fs::remove_dir_all(checkout);
    }

    #[test]
    fn a_harness_without_its_command_or_entry_is_told_apart() {
        let home = temporary_home("absent");
        let bin = home.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let program = if cfg!(windows) {
            "claude.cmd"
        } else {
            "claude"
        };
        std::fs::write(bin.join(program), "").unwrap();
        let environment = Environment {
            home: home.clone(),
            path: std::env::join_paths([&bin]).unwrap(),
        };
        let this = std::env::current_exe().unwrap();
        assert_eq!(
            environment.registration(Harness::ClaudeCode, &this),
            Registration::NotSetUp
        );
        let rows = environment.rows(&this);
        assert_eq!(rows[1].registration, Registration::NotInstalled);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn the_codex_command_is_read_from_its_ketchup_section_only() {
        let toml = r#"
model = "gpt"
[mcp_servers.other]
command = "other.exe"

[mcp_servers.ketchup]
args = ["--mcp"]
command = "C:\\Program Files\\Ketchup\\Ketchup.exe"
"#;
        assert_eq!(
            codex_command(toml).as_deref(),
            Some(r"C:\Program Files\Ketchup\Ketchup.exe")
        );
        assert_eq!(
            codex_command("[mcp_servers.ketchup]\ncommand = 'C:\\k\\Ketchup.exe'\n").as_deref(),
            Some(r"C:\k\Ketchup.exe")
        );
        assert_eq!(
            codex_command("[mcp_servers.other]\ncommand = \"x\"\n"),
            None
        );
    }
}
