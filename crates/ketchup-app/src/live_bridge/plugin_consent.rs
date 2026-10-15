//! A plugin runs only after the user allowed it in the window: its files by
//! absolute path and SHA-256, its command line, principal and permissions. A
//! plugin folder is allowed with every file under it; its inputs are not part
//! of the consent. A changed file, another command line or a permission not
//! allowed before ask again. The hashed files are the files that run
//! (`plugin_files`).
use super::drawings::local_absolute_path;
use super::plugin_files::HeldFiles;
use super::*;
use crate::dialogs::HighRiskConfirmationRequest;
use ketchup_model::graph::sha256_hex;
use std::collections::BTreeSet;

/// A plugin folder: every file under it by relative path and SHA-256.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConsentFolder {
    name: String,
    path: PathBuf,
    files: Vec<(String, String)>,
    /// One SHA-256 over the relative paths and hashes of all files.
    sha256: String,
}

impl ConsentFolder {
    pub(super) fn new(name: String, path: PathBuf, files: Vec<(String, String)>) -> Self {
        let listing = files
            .iter()
            .map(|(relative, sha256)| format!("{relative}\t{sha256}\n"))
            .collect::<String>();
        Self {
            name,
            path,
            sha256: sha256_hex(listing.as_bytes()),
            files,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginConsent {
    folder: Option<ConsentFolder>,
    /// Files outside a plugin folder the command line names, with SHA-256.
    files: Vec<(PathBuf, String)>,
    command: Vec<String>,
    principal_id: u64,
    capabilities: BTreeSet<String>,
    operations: BTreeSet<String>,
}

/// The file a plugin process starts from `path`, by `is_file`. On Windows
/// `Command` starts `<path>.exe` for a path without that extension whenever
/// `<path>.exe` exists, and another file does not start as a program; there
/// the executable is the `.exe` file. This is the file hashed and started, so
/// a path naming no such file is refused.
pub(super) fn started_executable(
    path: &Path,
    is_file: impl Fn(&Path) -> bool,
) -> Result<PathBuf, &'static str> {
    let exe = |path: &Path| {
        path.extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
    };
    let started = if cfg!(windows) && !exe(path) {
        let mut appended = path.as_os_str().to_owned();
        appended.push(".exe");
        PathBuf::from(appended)
    } else {
        path.to_owned()
    };
    if is_file(&started) {
        Ok(started)
    } else {
        Err(failure(
            "invalid_path",
            "The plugin executable names an existing file; on Windows an .exe file, which is the file that starts.",
            json!({"canonical_mutation": false}),
        ))
    }
}

impl PluginConsent {
    /// A command line given by the caller: the executable and every argument
    /// naming a file are hashed, the arguments are part of the consent.
    /// Each file is hashed through a handle `held` keeps until the run ends.
    pub(super) fn of_command(
        executable: &str,
        arguments: &[String],
        principal_id: u64,
        capabilities: &[String],
        operations: &[String],
        held: &mut HeldFiles,
    ) -> Result<Self, &'static str> {
        let mut hashed = |path: &Path| {
            let absolute = std::path::absolute(path)?;
            let hash = held.hash(&absolute)?;
            Ok::<_, std::io::Error>((absolute, hash))
        };
        // A network, device or stream path is refused before it is read.
        let remote = |path: &str| Path::new(path).is_absolute() && !local_absolute_path(path);
        if !local_absolute_path(executable) || arguments.iter().any(|a| remote(a)) {
            return Err(failure(
                "invalid_path",
                "The plugin executable is a local absolute path and no argument names a network share, device or stream.",
                json!({"canonical_mutation": false}),
            ));
        }
        let executable = started_executable(Path::new(executable), Path::is_file)?;
        let mut files =
            vec![hashed(&executable).map_err(|error| failed_because("plugin_failed", error))?];
        for argument in arguments {
            let path = Path::new(argument);
            if path.is_file() {
                files.push(hashed(path).map_err(|error| failed_because("plugin_failed", error))?);
            }
        }
        Ok(Self {
            folder: None,
            files,
            command: std::iter::once(executable.display().to_string())
                .chain(arguments.iter().cloned())
                .collect(),
            principal_id,
            capabilities: capabilities.iter().cloned().collect(),
            operations: operations.iter().cloned().collect(),
        })
    }

    pub(super) fn of_folder(
        folder: ConsentFolder,
        outside: Vec<(PathBuf, String)>,
        command: Vec<String>,
        principal_id: u64,
        capabilities: &[String],
        operations: &[String],
    ) -> Self {
        Self {
            folder: Some(folder),
            files: outside,
            command,
            principal_id,
            capabilities: capabilities.iter().cloned().collect(),
            operations: operations.iter().cloned().collect(),
        }
    }

    /// The executable that starts, as the consent names it.
    pub(super) fn executable(&self) -> &str {
        &self.command[0]
    }

    /// The same files, command line and principal, asking for no more than allowed.
    fn covers(&self, wanted: &Self) -> bool {
        self.folder == wanted.folder
            && self.files == wanted.files
            && self.command == wanted.command
            && self.principal_id == wanted.principal_id
            && wanted.capabilities.is_subset(&self.capabilities)
            && wanted.operations.is_subset(&self.operations)
    }

    fn evidence(&self) -> Value {
        let mut evidence = json!({
            "files": self.files.iter().map(|(path, sha256)| {
                json!({"path": path.display().to_string(), "sha256": sha256})
            }).collect::<Vec<_>>(),
            "command": self.command,
            "principal_id": self.principal_id,
            "capabilities": self.capabilities,
            "allowed_operations": self.operations,
        });
        if let Some(folder) = &self.folder {
            evidence["folder"] = json!({
                "name": folder.name,
                "path": folder.path.display().to_string(),
                "sha256": folder.sha256,
                "files": folder.files.iter().map(|(path, sha256)| {
                    json!({"path": path, "sha256": sha256})
                }).collect::<Vec<_>>(),
            });
        }
        evidence
    }
}

fn listed(items: &BTreeSet<String>) -> String {
    if items.is_empty() {
        "—".to_owned()
    } else {
        items
            .iter()
            .map(|item| shown(item))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Text from a plugin as the prompt shows it (SEC-3): a line break, another
/// control character or a bidirectional or invisible format character is shown
/// as `\u{…}`, so a name or path can neither start a forged line nor reorder
/// the text around it.
fn shown(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let format = matches!(
            c,
            '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2069}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
        );
        if c.is_control() || format {
            out.push_str(&format!("\\u{{{:04x}}}", u32::from(c)));
        } else {
            out.push(c);
        }
    }
    out
}

/// A command-line word as shown: quoted when it is empty or holds a space or a
/// quote, so the words of the command cannot be told apart wrongly.
fn shown_word(word: &str) -> String {
    let text = shown(word);
    if word.is_empty() || word.chars().any(|c| c.is_whitespace() || c == '"') {
        format!("\"{}\"", text.replace('"', "\\\""))
    } else {
        text
    }
}

impl KetchupApp {
    /// Asks the user in the window unless an earlier consent covers this run.
    pub(super) fn consent_to_plugin(&mut self, wanted: PluginConsent) -> Result<(), &'static str> {
        if self
            .live
            .plugin_consents
            .iter()
            .any(|allowed| allowed.covers(&wanted))
        {
            return Ok(());
        }
        let text = |key: &str| self.catalog.text(key);
        let mut lines = vec![text("live-plugin-consent-description")];
        if let Some(folder) = &wanted.folder {
            lines.extend([
                format!(
                    "{} {}",
                    text("live-plugin-consent-name"),
                    shown(&folder.name)
                ),
                text("live-plugin-consent-folder"),
                shown(&folder.path.display().to_string()),
                format!("SHA-256 {} ({})", folder.sha256, folder.files.len()),
            ]);
        }
        if !wanted.files.is_empty() {
            lines.push(text("live-plugin-consent-files"));
        }
        for (path, sha256) in &wanted.files {
            lines.push(shown(&path.display().to_string()));
            lines.push(format!("SHA-256 {sha256}"));
        }
        lines.extend([
            format!(
                "{} {}",
                text("live-plugin-consent-command"),
                wanted
                    .command
                    .iter()
                    .map(|word| shown_word(word))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            format!(
                "{} {}",
                text("live-plugin-consent-principal"),
                wanted.principal_id
            ),
            format!(
                "{} {}",
                text("live-plugin-consent-capabilities"),
                listed(&wanted.capabilities)
            ),
            format!(
                "{} {}",
                text("live-plugin-consent-operations"),
                listed(&wanted.operations)
            ),
        ]);
        let description = lines.join("\n");
        let title = text("live-plugin-consent-title");
        if self
            .dialogs
            .confirm_high_risk(HighRiskConfirmationRequest {
                title: &title,
                description: &description,
            })
            .is_none()
        {
            return Err(failure(
                "plugin_consent_required",
                "the user did not allow this plugin with these permissions",
                json!({"canonical_mutation": false, "plugin": wanted.evidence()}),
            ));
        }
        self.live.plugin_consents.push(wanted);
        Ok(())
    }
}
