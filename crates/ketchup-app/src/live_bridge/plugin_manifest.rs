//! A plugin folder: its `plugin.json` alone decides the command line, the
//! principal, the permissions and the typed inputs. The caller names the folder
//! and gives input values, which reach the plugin only through the protocol
//! (`QUERY\tINPUTS`), never as process arguments. The consent covers every file
//! under the folder plus any file outside it the command line names. A run
//! hashes and starts a staged copy of the folder (`plugin_files`), so what runs
//! is what the user allowed.
use super::drawings::local_absolute_path;
use super::plugin_consent::{ConsentFolder, PluginConsent, started_executable};
use super::plugin_files::{HeldFiles, open_regular};
use super::*;
use ketchup_model::graph::sha256_hex;
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
    /// Held until the plugin process has ended.
    pub held: HeldFiles,
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

/// Every file under `folder` by relative path; a link or too much is refused.
fn walk(folder: &Path) -> Result<Vec<PathBuf>, &'static str> {
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
            if !metadata.is_file() {
                return Err(invalid(format!(
                    "{} is not a regular file; a plugin folder holds files and folders only",
                    slashed(&relative)
                )));
            }
            bytes = bytes.saturating_add(metadata.len());
            if files.len() >= MAX_FOLDER_FILES || bytes > MAX_FOLDER_BYTES {
                return Err(invalid(format!(
                    "the plugin folder holds more than {MAX_FOLDER_FILES} files or {} MiB",
                    MAX_FOLDER_BYTES / (1024 * 1024)
                )));
            }
            files.push(relative);
        }
    }
    Ok(files)
}

/// Every file under `folder` by `/`-separated relative path; for a run (with
/// `held`) with its SHA-256, read through a handle held until the run ends. A
/// listing names the files without reading them.
fn folder_files(
    folder: &Path,
    mut held: Option<&mut HeldFiles>,
) -> Result<Vec<(String, String)>, &'static str> {
    let mut files = Vec::new();
    for relative in walk(folder)? {
        let sha256 = match held.as_deref_mut() {
            Some(held) => held.hash(&folder.join(&relative)).map_err(unreadable)?,
            None => String::new(),
        };
        files.push((slashed(&relative), sha256));
    }
    files.sort();
    Ok(files)
}

/// Copies every file under `folder` into the stage of `held`, each read
/// through a handle checked to be a regular file, and no more bytes in all
/// than a plugin folder may hold, however the files grew after the walk.
fn stage_folder(folder: &Path, held: &HeldFiles) -> Result<(), &'static str> {
    let stage = held.stage().expect("a staged run has a stage");
    let mut remaining = MAX_FOLDER_BYTES;
    for relative in walk(folder)? {
        let target = stage.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(unreadable)?;
        }
        let source = open_regular(&folder.join(&relative)).map_err(unreadable)?;
        let mut copy = fs::File::create_new(target).map_err(unreadable)?;
        let copied = std::io::copy(&mut std::io::Read::take(source, remaining + 1), &mut copy)
            .map_err(unreadable)?;
        remaining = remaining.checked_sub(copied).ok_or_else(|| {
            invalid(format!(
                "the plugin folder holds more than {} MiB",
                MAX_FOLDER_BYTES / (1024 * 1024)
            ))
        })?;
    }
    Ok(())
}

fn read_manifest(
    folder: &Path,
    held: Option<&mut HeldFiles>,
) -> Result<(Manifest, String), &'static str> {
    let path = folder.join(MANIFEST);
    let bytes = match held {
        Some(held) => held.read(&path, MAX_MANIFEST_BYTES),
        None => open_regular(&path).and_then(|file| {
            let mut bytes = Vec::new();
            std::io::Read::read_to_end(
                &mut std::io::Read::take(file, MAX_MANIFEST_BYTES + 1),
                &mut bytes,
            )
            .map(|_| bytes)
        }),
    }
    .map_err(|error| match error.kind() {
        std::io::ErrorKind::NotFound => invalid(format!(
            "the folder has no {MANIFEST}; it is no plugin folder"
        )),
        _ => unreadable(error),
    })?;
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

/// Whether `..` ends a part of the argument without being that whole part
/// (`--script=../x`, `-s..`): the plugin splits the option off itself and
/// resolves the rest against its stage, outside the files that were hashed.
fn embeds_parent_dir(argument: &str) -> bool {
    argument
        .split(['/', '\\'])
        .any(|part| part != ".." && part.ends_with(".."))
}

/// The executable, the arguments and every file outside the folder they name,
/// hashed (with `held`, through handles held until the run ends). A relative
/// argument naming a file outside the folder is given as its absolute path,
/// so it names the same file wherever the process runs.
fn outside_files(
    folder: &Path,
    manifest: &Manifest,
    files: &[(String, String)],
    mut held: Option<&mut HeldFiles>,
) -> Result<(PathBuf, Vec<String>, Vec<HashedFile>), &'static str> {
    let executable = Path::new(&manifest.executable);
    let executable = if executable.is_absolute() {
        if !local_absolute_path(&manifest.executable) {
            return Err(failure(
                "invalid_path",
                "The plugin executable is a local absolute path or a file inside the plugin folder.",
                json!({"canonical_mutation": false}),
            ));
        }
        started_executable(&lexical(executable), Path::is_file)?
    } else if executable
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        let in_folder = |path: &Path| files.iter().any(|(relative, _)| *relative == slashed(path));
        folder.join(started_executable(executable, in_folder)?)
    } else {
        return Err(invalid(format!(
            "executable {:?} is neither an absolute path nor a file inside the plugin folder",
            manifest.executable
        )));
    };
    let mut outside = Vec::new();
    let mut arguments = Vec::new();
    let candidates = std::iter::once((None, executable.clone())).chain(
        manifest
            .arguments
            .iter()
            .map(|argument| (Some(argument), lexical(&folder.join(argument)))),
    );
    for (argument, candidate) in candidates {
        if let Some(argument) = argument
            && embeds_parent_dir(argument)
        {
            return Err(invalid(format!(
                "argument {argument:?} joins `..` to an option; give the path as its own argument"
            )));
        }
        let mut given = argument.cloned();
        if candidate.starts_with(folder) {
            // Through `..` the text would lead out of the stage the process
            // runs in; it gets the folder's file it names, which is staged.
            if let Some(given) = given.as_mut()
                && Path::new(given.as_str())
                    .components()
                    .any(|component| component == Component::ParentDir)
            {
                *given = candidate.display().to_string();
            }
            arguments.extend(given);
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
        if candidate.is_file() {
            // A listing does not read the file; a run hashes it.
            let sha256 = match held.as_deref_mut() {
                Some(held) => held
                    .hash(&candidate)
                    .map_err(|error| failed_because("plugin_failed", error))?,
                None => String::new(),
            };
            if let Some(given) = given.as_mut() {
                *given = candidate.display().to_string();
            }
            outside.push((candidate, sha256));
        } else {
            return Err(not_a_file(&candidate));
        }
        arguments.extend(given);
    }
    Ok((executable, arguments, outside))
}

/// The refusal of a path outside the plugin folder that the command line
/// names but that is no file: a folder could gain files nobody hashed, and a
/// file created where nothing is yet, even while the plugin runs, would be
/// opened unhashed (review 2026-10-15 N-9).
pub(super) fn not_a_file(path: &Path) -> &'static str {
    let why = match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            "names no file; a file created there later would run unhashed"
        }
        Err(error) => return unreadable(error),
        Ok(_) => {
            "is no file; a plugin's command line names files, and the files of a folder outside the plugin folder would run unhashed"
        }
    };
    failure(
        "invalid_path",
        format!("{} {why}", path.display()),
        json!({"canonical_mutation": false}),
    )
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
    arguments: Vec<String>,
    outside: Vec<HashedFile>,
    /// For a run: the stage the folder was copied into and every hashed file.
    held: Option<HeldFiles>,
}

/// Reads `plugin`; `staged` (a run) copies it into a stage first and hashes
/// the copy, held until the run ends.
fn read_folder(plugin: &str, staged: bool) -> Result<ReadFolder, &'static str> {
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
    // A folder without a valid manifest is no plugin: refused before any of
    // its files is walked, copied or hashed.
    let (manifest, _) = read_manifest(&folder, None)?;
    let mut held = if staged {
        let held = HeldFiles::staged().map_err(unreadable)?;
        stage_folder(&folder, &held)?;
        Some(held)
    } else {
        None
    };
    let root = held
        .as_ref()
        .and_then(HeldFiles::stage)
        .unwrap_or(&folder)
        .to_owned();
    let files = folder_files(&root, held.as_mut())?;
    let manifest = match held.as_mut() {
        // A run uses the staged manifest, which is the one hashed with the folder.
        Some(held) => {
            let (manifest, manifest_sha256) = read_manifest(&root, Some(held))?;
            if !files.contains(&(MANIFEST.to_owned(), manifest_sha256)) {
                return Err(invalid(format!("{MANIFEST} changed while it was read")));
            }
            manifest
        }
        None => manifest,
    };
    let (executable, arguments, outside) =
        outside_files(&folder, &manifest, &files, held.as_mut())?;
    Ok(ReadFolder {
        folder,
        manifest,
        files,
        executable,
        arguments,
        outside,
        held,
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

/// Reads the plugin folder as a run does, so a folder that could not run is
/// refused here with the same code and reason; on the UI thread, so only its
/// manifest is read and its other files are named, not read or hashed.
pub(super) fn describe(folder: &Path) -> Result<Description, &'static str> {
    let read = read_folder(&folder.to_string_lossy(), false)?;
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

/// Copies the plugin folder into a stage and hashes the copy, which is what
/// runs; `given` inputs are checked against the declared ones and are not
/// part of the consent.
pub(super) fn load(
    plugin: &str,
    given: Option<&serde_json::Map<String, Value>>,
) -> Result<FolderLaunch, &'static str> {
    let ReadFolder {
        folder,
        manifest,
        files,
        executable,
        arguments,
        outside,
        held,
    } = read_folder(plugin, true)?;
    let held = held.expect("a run reads a staged folder");
    let stage = held.stage().expect("a staged run has a stage").to_owned();
    let inputs = inputs(&manifest, given)?;
    // The consent names the folder; the process runs its staged copy.
    let in_stage = |path: &str| match Path::new(path).strip_prefix(&folder) {
        Ok(relative) => OsString::from(stage.join(relative)),
        Err(_) => OsString::from(path),
    };
    let command = PluginCommand {
        executable: PathBuf::from(in_stage(&executable.display().to_string())),
        arguments: arguments
            .iter()
            .map(|argument| in_stage(argument))
            .collect(),
        working_directory: Some(stage.clone()),
        inputs,
    };
    let consent = PluginConsent::of_folder(
        ConsentFolder::new(manifest.name.clone(), folder, files),
        outside,
        std::iter::once(executable.display().to_string())
            .chain(arguments)
            .collect(),
        manifest.principal_id,
        &manifest.capabilities,
        &manifest.allowed_operations,
    );
    Ok(FolderLaunch {
        command,
        principal_id: manifest.principal_id,
        capabilities: manifest.capabilities,
        allowed_operations: manifest.allowed_operations,
        consent,
        held,
    })
}
