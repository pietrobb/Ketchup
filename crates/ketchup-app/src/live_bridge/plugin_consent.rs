//! A plugin runs only after the user allowed it in the window: its files by
//! absolute path and SHA-256, its arguments, principal and permissions. A
//! changed file, other arguments or a permission not allowed before ask again.
use super::drawings::local_absolute_path;
use super::*;
use crate::dialogs::HighRiskConfirmationRequest;
use ketchup_model::graph::sha256_reader_hex;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PluginConsent {
    /// The executable and every argument naming a file, with their SHA-256.
    files: Vec<(PathBuf, String)>,
    arguments: Vec<String>,
    principal_id: u64,
    capabilities: BTreeSet<String>,
    operations: BTreeSet<String>,
}

impl PluginConsent {
    fn of(request: &RunPluginRequest) -> Result<Self, &'static str> {
        let hashed = |path: &Path| {
            let absolute = std::path::absolute(path)?;
            let hash = std::fs::File::open(&absolute).and_then(sha256_reader_hex)?;
            Ok::<_, std::io::Error>((absolute, hash))
        };
        // A network, device or stream path is refused before it is read.
        let remote = |path: &str| Path::new(path).is_absolute() && !local_absolute_path(path);
        if !local_absolute_path(&request.executable) || request.arguments.iter().any(|a| remote(a))
        {
            return Err(failure(
                "invalid_path",
                "The plugin executable is a local absolute path and no argument names a network share, device or stream.",
                json!({"canonical_mutation": false}),
            ));
        }
        let mut files = vec![
            hashed(Path::new(&request.executable))
                .map_err(|error| failed_because("plugin_failed", error))?,
        ];
        for argument in &request.arguments {
            let path = Path::new(argument);
            if path.is_file() {
                files.push(hashed(path).map_err(|error| failed_because("plugin_failed", error))?);
            }
        }
        Ok(Self {
            files,
            arguments: request.arguments.clone(),
            principal_id: request.principal_id,
            capabilities: request.capabilities.iter().cloned().collect(),
            operations: request.allowed_operations.iter().cloned().collect(),
        })
    }

    /// The same files, arguments and principal, asking for no more than allowed.
    fn covers(&self, wanted: &Self) -> bool {
        self.files == wanted.files
            && self.arguments == wanted.arguments
            && self.principal_id == wanted.principal_id
            && wanted.capabilities.is_subset(&self.capabilities)
            && wanted.operations.is_subset(&self.operations)
    }

    fn evidence(&self) -> Value {
        json!({
            "files": self.files.iter().map(|(path, sha256)| {
                json!({"path": path.display().to_string(), "sha256": sha256})
            }).collect::<Vec<_>>(),
            "arguments": self.arguments,
            "principal_id": self.principal_id,
            "capabilities": self.capabilities,
            "allowed_operations": self.operations,
        })
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
    pub(super) fn consent_to_plugin(
        &mut self,
        request: &RunPluginRequest,
    ) -> Result<(), &'static str> {
        let wanted = PluginConsent::of(request)?;
        if self
            .live
            .plugin_consents
            .iter()
            .any(|allowed| allowed.covers(&wanted))
        {
            return Ok(());
        }
        let text = |key: &str| self.catalog.text(key);
        let mut lines = vec![
            text("live-plugin-consent-description"),
            text("live-plugin-consent-files"),
        ];
        for (path, sha256) in &wanted.files {
            lines.push(path.display().to_string());
            lines.push(format!("SHA-256 {sha256}"));
        }
        lines.extend([
            format!(
                "{} {}",
                text("live-plugin-consent-arguments"),
                wanted.arguments.join(" ")
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
