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
        let mut files = vec![
            hashed(Path::new(executable))
                .map_err(|error| failed_because("plugin_failed", error))?,
        ];
        for argument in arguments {
            let path = Path::new(argument);
            if path.is_file() {
                files.push(hashed(path).map_err(|error| failed_because("plugin_failed", error))?);
            }
        }
        Ok(Self {
            folder: None,
            files,
            command: std::iter::once(executable.to_owned())
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
        items.iter().cloned().collect::<Vec<_>>().join(", ")
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
                format!("{} {}", text("live-plugin-consent-name"), folder.name),
                text("live-plugin-consent-folder"),
                folder.path.display().to_string(),
                format!("SHA-256 {} ({})", folder.sha256, folder.files.len()),
            ]);
        }
        if !wanted.files.is_empty() {
            lines.push(text("live-plugin-consent-files"));
        }
        for (path, sha256) in &wanted.files {
            lines.push(path.display().to_string());
            lines.push(format!("SHA-256 {sha256}"));
        }
        lines.extend([
            format!(
                "{} {}",
                text("live-plugin-consent-command"),
                wanted.command.join(" ")
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
