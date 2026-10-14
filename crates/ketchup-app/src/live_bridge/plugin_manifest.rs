//! A plugin folder: its `plugin.json` alone decides the command line, the
//! principal, the permissions and the typed inputs. The caller names the folder
//! and gives input values, which reach the plugin only through the protocol
//! (`QUERY\tINPUTS`), never as process arguments. The consent covers every file
//! under the folder plus any file outside it the command line names.
use super::drawings::local_absolute_path;
use super::plugin_consent::{ConsentFolder, PluginConsent};
use super::*;
use ketchup_model::graph::{sha256_hex, sha256_reader_hex};
use ketchup_scheduler::plugin::PluginCommand;
use std::ffi::OsString;
use std::fs;
use std::path::Component;

pub(super) const MANIFEST: &str = "plugin.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
const MAX_FOLDER_FILES: usize = 1_000;
const MAX_FOLDER_BYTES: u64 = 256 * 1024 * 1024;

/// An absolute file path and its SHA-256.
type HashedFile = (PathBuf, String);

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    name: String,
    /// Absolute path of an interpreter, or of a program relative to the folder.
    executable: String,
    /// Fixed arguments; the process runs in the plugin folder.
    #[serde(default)]
    arguments: Vec<String>,
    principal_id: u64,
    capabilities: Vec<String>,
    #[serde(default)]
    allowed_operations: Vec<String>,
    #[serde(default)]
    inputs: BTreeMap<String, InputDeclaration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InputDeclaration {
    #[serde(rename = "type")]
    kind: InputType,
    #[serde(default)]
    default: Option<Value>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(super) enum InputType {
    Number,
    Integer,
    Boolean,
    String,
}

impl InputType {
    fn accepts(self, value: &Value) -> bool {
        match self {
            Self::Number => value.is_number(),
            Self::Integer => value.is_i64() || value.is_u64(),
            Self::Boolean => value.is_boolean(),
            Self::String => value.is_string(),
        }
    }
}

/// What a plugin folder starts and what the user allows for it.
pub(super) struct FolderLaunch {
    pub command: PluginCommand,
    pub principal_id: u64,
    pub capabilities: Vec<String>,
    pub allowed_operations: Vec<String>,
    pub consent: PluginConsent,
}

fn invalid(reason: impl Into<String>) -> &'static str {
    failure(
        "plugin_manifest_invalid",
        reason,
        json!({"canonical_mutation": false}),
    )
}

fn unreadable(error: std::io::Error) -> &'static str {
    failed_because("plugin_manifest_invalid", error)
}

/// `path` without `.` and `..`, so a prefix test cannot be fooled by them.
fn lexical(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                normal.pop();
            }
            Component::CurDir => {}
            other => normal.push(other),
        }
    }
    normal
}

fn slashed(relative: &Path) -> String {
    relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

/// Every file under `folder` with its SHA-256, by `/`-separated relative path.
fn folder_files(folder: &Path) -> Result<Vec<(String, String)>, &'static str> {
    let mut files = Vec::new();
    let mut bytes = 0u64;
    let mut pending = vec![PathBuf::new()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(folder.join(&directory)).map_err(unreadable)? {
            let entry = entry.map_err(unreadable)?;
            let relative = directory.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path()).map_err(unreadable)?;
            if metadata.file_type().is_symlink() {
                return Err(invalid(format!(
                    "{} is a link; a plugin folder holds its own files only",
                    slashed(&relative)
                )));
            }
            if metadata.is_dir() {
                pending.push(relative);
                continue;
            }
            bytes = bytes.saturating_add(metadata.len());
            if files.len() >= MAX_FOLDER_FILES || bytes > MAX_FOLDER_BYTES {
                return Err(invalid(format!(
                    "the plugin folder holds more than {MAX_FOLDER_FILES} files or {} MiB",
                    MAX_FOLDER_BYTES / (1024 * 1024)
                )));
            }
            let sha256 = fs::File::open(entry.path())
                .and_then(sha256_reader_hex)
                .map_err(unreadable)?;
            files.push((slashed(&relative), sha256));
        }
    }
    files.sort();
    Ok(files)
}

fn read_manifest(folder: &Path) -> Result<(Manifest, String), &'static str> {
    let path = folder.join(MANIFEST);
    let file = fs::File::open(&path).map_err(unreadable)?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(
        &mut std::io::Read::take(file, MAX_MANIFEST_BYTES + 1),
        &mut bytes,
    )
    .map_err(unreadable)?;
    if !u64::try_from(bytes.len()).is_ok_and(|length| length <= MAX_MANIFEST_BYTES) {
        return Err(invalid(format!(
            "{MANIFEST} is larger than {MAX_MANIFEST_BYTES} bytes"
        )));
    }
    let manifest = serde_json::from_slice(&bytes)
        .map_err(|error| failed_because("plugin_manifest_invalid", error))?;
    Ok((manifest, sha256_hex(&bytes)))
}

/// The folder named by `plugin` (the folder or its `plugin.json`), absolute.
fn plugin_folder(plugin: &str) -> Result<PathBuf, &'static str> {
    if !local_absolute_path(plugin) {
        return Err(failure(
            "invalid_path",
            "The plugin is the local absolute path of its folder or of its plugin.json.",
            json!({"canonical_mutation": false}),
        ));
    }
    let path = Path::new(plugin);
    let folder = if path.file_name() == Some(MANIFEST.as_ref()) {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    Ok(lexical(&std::path::absolute(folder).map_err(unreadable)?))
}

/// The executable and the arguments naming a file outside the folder, hashed.
fn outside_files(
    folder: &Path,
    manifest: &Manifest,
    files: &[(String, String)],
) -> Result<(PathBuf, Vec<HashedFile>), &'static str> {
    let executable = Path::new(&manifest.executable);
    let executable = if executable.is_absolute() {
        if !local_absolute_path(&manifest.executable) {
            return Err(failure(
                "invalid_path",
                "The plugin executable is a local absolute path or a file inside the plugin folder.",
                json!({"canonical_mutation": false}),
            ));
        }
        lexical(executable)
    } else if executable
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
        && files
            .iter()
            .any(|(relative, _)| *relative == slashed(executable))
    {
        folder.join(executable)
    } else {
        return Err(invalid(format!(
            "executable {:?} is neither an absolute path nor a file inside the plugin folder",
            manifest.executable
        )));
    };
    let mut outside = Vec::new();
    let candidates = std::iter::once(executable.clone()).chain(
        manifest
            .arguments
            .iter()
            .map(|argument| lexical(&folder.join(argument))),
    );
    for candidate in candidates {
        if candidate.starts_with(folder) {
            continue;
        }
        // Before `is_file`: asking a network share whether a file exists
        // already sends it the user's credentials.
        if !local_absolute_path(&candidate.to_string_lossy()) {
            return Err(failure(
                "invalid_path",
                "No plugin argument names a network share, device or stream.",
                json!({"canonical_mutation": false}),
            ));
        }
        if !candidate.is_file() {
            continue;
        }
        let sha256 = fs::File::open(&candidate)
            .and_then(sha256_reader_hex)
            .map_err(|error| failed_because("plugin_failed", error))?;
        outside.push((candidate, sha256));
    }
    Ok((executable, outside))
}

/// The declared inputs with `given` values or their defaults; an undeclared,
/// missing or mistyped input is refused.
fn inputs(
    manifest: &Manifest,
    given: Option<&serde_json::Map<String, Value>>,
) -> Result<serde_json::Map<String, Value>, &'static str> {
    let empty = serde_json::Map::new();
    let given = given.unwrap_or(&empty);
    if let Some(name) = given
        .keys()
        .find(|name| !manifest.inputs.contains_key(*name))
    {
        return Err(invalid(format!(
            "input {name:?} is not declared in {MANIFEST}"
        )));
    }
    let mut resolved = serde_json::Map::new();
    for (name, declaration) in &manifest.inputs {
        let Some(value) = given.get(name).or(declaration.default.as_ref()) else {
            return Err(invalid(format!(
                "input {name:?} has no value and no default"
            )));
        };
        if !declaration.kind.accepts(value) {
            return Err(invalid(format!(
                "input {name:?} must be a {:?} value",
                declaration.kind
            )));
        }
        resolved.insert(name.clone(), value.clone());
    }
    Ok(resolved)
}

/// A plugin folder read and hashed, nothing started.
struct ReadFolder {
    folder: PathBuf,
    manifest: Manifest,
    files: Vec<(String, String)>,
    executable: PathBuf,
    outside: Vec<HashedFile>,
}

fn read_folder(plugin: &str) -> Result<ReadFolder, &'static str> {
    let folder = plugin_folder(plugin)?;
    if fs::symlink_metadata(&folder)
        .map_err(unreadable)?
        .file_type()
        .is_symlink()
    {
        return Err(invalid(
            "the plugin folder is a link; a plugin folder holds its own files only",
        ));
    }
    let (manifest, manifest_sha256) = read_manifest(&folder)?;
    let files = folder_files(&folder)?;
    // The manifest read is the manifest hashed with the folder.
    if !files.contains(&(MANIFEST.to_owned(), manifest_sha256)) {
        return Err(invalid(format!("{MANIFEST} changed while it was read")));
    }
    let (executable, outside) = outside_files(&folder, &manifest, &files)?;
    Ok(ReadFolder {
        folder,
        manifest,
        files,
        executable,
        outside,
    })
}

/// One input a manifest declares.
pub(super) struct DeclaredInput {
    pub name: String,
    pub kind: InputType,
    pub default: Option<Value>,
}

/// What the window lists for a plugin folder: its name and declared inputs.
pub(super) struct Description {
    pub name: String,
    pub inputs: Vec<DeclaredInput>,
}

/// Reads the plugin folder exactly as a run does, so a folder that could not
/// run is refused here with the same code and reason.
pub(super) fn describe(folder: &Path) -> Result<Description, &'static str> {
    let read = read_folder(&folder.to_string_lossy())?;
    Ok(Description {
        name: read.manifest.name,
        inputs: read
            .manifest
            .inputs
            .into_iter()
            .map(|(name, declaration)| DeclaredInput {
                name,
                kind: declaration.kind,
                default: declaration.default,
            })
            .collect(),
    })
}

/// Reads and hashes the plugin folder; `given` inputs are checked against
/// the declared ones and are not part of the consent.
pub(super) fn load(
    plugin: &str,
    given: Option<&serde_json::Map<String, Value>>,
) -> Result<FolderLaunch, &'static str> {
    let ReadFolder {
        folder,
        manifest,
        files,
        executable,
        outside,
    } = read_folder(plugin)?;
    let inputs = inputs(&manifest, given)?;
    let consent = PluginConsent::of_folder(
        ConsentFolder::new(manifest.name.clone(), folder.clone(), files),
        outside,
        std::iter::once(executable.display().to_string())
            .chain(manifest.arguments.iter().cloned())
            .collect(),
        manifest.principal_id,
        &manifest.capabilities,
        &manifest.allowed_operations,
    );
    Ok(FolderLaunch {
        command: PluginCommand {
            executable,
            arguments: manifest.arguments.iter().map(OsString::from).collect(),
            working_directory: Some(folder),
            inputs,
        },
        principal_id: manifest.principal_id,
        capabilities: manifest.capabilities,
        allowed_operations: manifest.allowed_operations,
        consent,
    })
}
