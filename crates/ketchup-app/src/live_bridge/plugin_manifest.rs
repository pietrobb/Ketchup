//! A plugin folder: its `plugin.json` alone decides the command line, the
//! principal, the permissions and the typed inputs. The caller names the folder
//! and gives input values, which reach the plugin only through the protocol
//! (`QUERY\tINPUTS`), never as process arguments. The consent covers every file
//! under the folder plus any file outside it the command line names. A run
//! hashes and starts a staged copy of the folder (`plugin_files`), so what runs
//! is what the user allowed.
use super::drawings::local_absolute_path;
use super::plugin_consent::{ConsentFolder, PluginConsent, started_executable};
use super::plugin_files::{
    HeldFiles, MAX_PLUGIN_BYTES, MAX_PLUGIN_FILES, open_regular, past_limits,
};
use super::*;
use ketchup_model::graph::sha256_hex;
use ketchup_scheduler::plugin::PluginCommand;
use std::ffi::OsString;
use std::fs;
use std::path::Component;

pub(super) const MANIFEST: &str = "plugin.json";
const MAX_MANIFEST_BYTES: u64 = 64 * 1024;
/// The most arguments a plugin's command line has (review 2026-10-18 MCP-5).
pub(super) const MAX_ARGUMENTS: usize = 1_000;

/// A file a run could not hash: past the limits of `plugin_files` it is
/// refused with `limit`, a cancelled request stops, anything else fails the
/// run; each keeps the error as its cause.
pub(super) fn unhashed(error: std::io::Error, limit: &'static str) -> &'static str {
    let code = match error.kind() {
        std::io::ErrorKind::FileTooLarge => limit,
        std::io::ErrorKind::Interrupted => "request_cancelled",
        _ => "plugin_failed",
    };
    failed_because(code, error)
}

/// The refusal of a command line with more than `MAX_ARGUMENTS` arguments.
pub(super) fn too_many_arguments(arguments: &[String]) -> Option<String> {
    (arguments.len() > MAX_ARGUMENTS).then(|| {
        format!(
            "the command line has {} arguments; a plugin's has at most {MAX_ARGUMENTS}",
            arguments.len()
        )
    })
}

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
            if files.len() >= MAX_PLUGIN_FILES || bytes > MAX_PLUGIN_BYTES {
                return Err(invalid(past_limits("the plugin folder holds")));
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
    let mut remaining = MAX_PLUGIN_BYTES;
    for relative in walk(folder)? {
        held.check_cancelled().map_err(unreadable)?;
        let target = stage.join(&relative);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(unreadable)?;
        }
        let source = open_regular(&folder.join(&relative)).map_err(unreadable)?;
        let mut copy = fs::File::create_new(target).map_err(unreadable)?;
        let copied = std::io::copy(&mut std::io::Read::take(source, remaining + 1), &mut copy)
            .map_err(unreadable)?;
        remaining = remaining
            .checked_sub(copied)
            .ok_or_else(|| invalid(past_limits("the plugin folder holds")))?;
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

/// Why the shape of an argument is refused, if it is. A plugin splits an
/// option off its value itself and opens what the value names, which is no
/// path of the argument, so nothing it names is staged or hashed; listing
/// refused shapes (`..`, an absolute path, a URL, `@file`) never ended
/// (review 2026-10-16 R-2, 2026-10-17 D-6). Only these shapes pass:
/// `-x`, `-xVALUE`, `--name`, `--name=VALUE` and `name=VALUE`, with a name
/// of letters, digits, `-` and `_` and a value of plain words and relative
/// paths ([`plain_value`]); or a word with no `=` and no leading `-` or `@`,
/// a path given as its own argument or plain text, never a URL or a stream.
pub(super) fn refused_shape(argument: &str) -> Option<&'static str> {
    const JOINED: &str = "joins a path that is not relative, a URL or other text than plain words \
                          to an option; give a path as its own argument";
    if argument.starts_with('@') {
        return Some("reads further arguments from a file; give the arguments themselves");
    }
    let (name, value) = if let Some(long) = argument.strip_prefix("--") {
        long.split_once('=')
            .map_or((long, None), |(name, value)| (name, Some(value)))
    } else if let Some(short) = argument.strip_prefix('-') {
        let letter = short.chars().next().map_or(0, char::len_utf8);
        (
            &short[..letter],
            Some(&short[letter..]).filter(|value| !value.is_empty()),
        )
    } else if let Some((name, value)) = argument.split_once('=') {
        (name, Some(value))
    } else if starts_with_scheme(argument) {
        return Some("is a URL or a stream, not a local path");
    } else {
        return None;
    };
    let name_ok = name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    (!name_ok || value.is_some_and(|value| !plain_value(value))).then_some(JOINED)
}

/// The options an interpreter may get before its script, by the name of the
/// file that starts (`python3.12.exe` is `python`), and whether their letter
/// case matters; any other executable gets none. None of them runs code nobody
/// hashed; docs/plugins.md lists them all (review 2026-10-18 SEC-8).
pub(super) const SCRIPT_OPTIONS: [(&[&str], &[&str], bool); 4] = [
    (
        &["python", "pythonw", "py"],
        &["-B", "-E", "-I", "-O", "-OO", "-q", "-s", "-S", "-u"],
        true,
    ),
    (&["node"], &["--no-deprecation", "--no-warnings"], true),
    (
        &["pwsh", "powershell"],
        &["-File", "-NoLogo", "-NonInteractive", "-NoProfile"],
        false,
    ),
    (&["ruby", "perl"], &["-w"], true),
];

/// The option that must stand right before the script, by interpreter name.
/// Windows PowerShell reads a bare first word as `-Command` and joins every
/// word after it into that command, so `powershell hashed.ps1 ; Remove-Item …`
/// ran the words after the hashed file (review 2026-10-19 SEC-9).
const SCRIPT_FLAGS: [(&[&str], &str); 1] = [(&["pwsh", "powershell"], "-File")];

/// The interpreter name of `executable`: `python3.12.exe` is `python`.
fn interpreter(executable: &Path) -> String {
    let stem = executable
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    stem.trim_end_matches(|c: char| c.is_ascii_digit() || c == '.')
        .to_owned()
}

/// Whether `option` may stand before the script of `executable`.
fn script_option(executable: &Path, option: &str) -> bool {
    let family = interpreter(executable);
    SCRIPT_OPTIONS
        .iter()
        .find(|(names, _, _)| names.contains(&family.as_str()))
        .is_some_and(|(_, options, case)| {
            options.iter().any(|allowed| {
                if *case {
                    *allowed == option
                } else {
                    allowed.eq_ignore_ascii_case(option)
                }
            })
        })
}

/// An interpreter's command line is `[option]* script [word]*`: the first
/// argument that is no option of [`SCRIPT_OPTIONS`] must name a hashed file
/// (`names_file`), the script the interpreter runs. A word after it reaches
/// the script, not the interpreter. A list of refused options was never
/// complete: `pwsh -C cmdlet hashed.txt` ran a cmdlet, and any hashed file
/// anywhere on the line counted as the script (review 2026-10-18 SEC-8). An
/// interpreter of [`SCRIPT_FLAGS`] gets its script right after that option.
/// `refuse` gives the caller's refusal for the reason.
pub(super) fn script_first(
    executable: &Path,
    arguments: &[String],
    names_file: &[bool],
    refuse: impl FnOnce(String) -> &'static str,
) -> Result<(), &'static str> {
    let family = interpreter(executable);
    let flag = SCRIPT_FLAGS
        .iter()
        .find(|(names, _)| names.contains(&family.as_str()))
        .map(|(_, flag)| *flag);
    for (index, (argument, file)) in arguments.iter().zip(names_file).enumerate() {
        if *file {
            if let Some(flag) = flag {
                let flagged = index
                    .checked_sub(1)
                    .is_some_and(|before| arguments[before].eq_ignore_ascii_case(flag));
                if !flagged {
                    return Err(refuse(format!(
                        "{} runs a script only after {flag}; give {flag} right before {argument:?}",
                        executable.display()
                    )));
                }
            }
            return Ok(());
        }
        if !script_option(executable, argument) {
            return Err(refuse(format!(
                "argument {argument:?} stands before the script and is no option {} may \
                 take there; name the script first (docs/plugins.md lists the options)",
                executable.display()
            )));
        }
    }
    Err(refuse(NO_SCRIPT.to_owned()))
}

/// Refuses an executable that starts with `#!`: it starts the interpreter its
/// first line names, which nobody hashed (review 2026-10-18 SEC-8). `refuse`
/// gives the caller's refusal; a file that cannot be read is `plugin_failed`
/// with the read error as its cause.
pub(super) fn not_a_shebang(
    executable: &Path,
    refuse: impl FnOnce(String) -> &'static str,
) -> Result<(), &'static str> {
    use std::io::Read;
    let mut start = Vec::with_capacity(2);
    open_regular(executable)
        .and_then(|file| file.take(2).read_to_end(&mut start))
        .map_err(|error| failed_because("plugin_failed", error))?;
    if start == b"#!" {
        return Err(refuse(format!(
            "{} starts with #! and so runs the interpreter its first line names, which is \
             not hashed; give that interpreter as the executable and this file as its script",
            executable.display()
        )));
    }
    Ok(())
}

/// Whether the text starts with a URL scheme (`file:`, `data:`, `https:`): two
/// or more letters, digits or `+.-` from a letter up to a colon. A drive
/// letter (`C:`) is one letter, so a Windows path is no URL.
fn starts_with_scheme(text: &str) -> bool {
    text.split_once(':').is_some_and(|(scheme, _)| {
        scheme.len() >= 2
            && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    })
}

/// Whether an option's value is plain words and relative paths: letters,
/// digits and `-_.,+/\` only, no part that starts at a root or leads up a
/// folder (`..`, `x..`), and so again after every `=` (`-Dkey=value`).
pub(super) fn plain_value(value: &str) -> bool {
    value.split('=').all(|piece| {
        piece
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.,+/\\".contains(c))
            && !piece.starts_with(['/', '\\'])
            && piece.split(['/', '\\']).all(|part| !part.ends_with(".."))
    })
}

/// One argument of a plugin's command line, resolved once into the text the
/// process gets and the consent shows, and the file outside `folder` it
/// names, hashed with `held` (left unhashed without it: a listing does not
/// read files). Both a plugin folder and a command line resolve their words
/// here (review 2026-10-15 structural point 7). A relative argument is
/// relative to `base`. Inside `folder`, whose files are staged, the argument
/// stays as given, with `..` resolved so it cannot lead out of the stage; a
/// file outside is given by its absolute path; an argument naming no file
/// reaches the process as given only when it cannot leave the run's private
/// directory. `refuse` gives the caller's refusal for an argument that joins
/// `..` or an absolute path to an option, `limit` its code for files past the
/// limits of `plugin_files`.
pub(super) fn resolve_argument(
    argument: &str,
    base: &Path,
    folder: Option<&Path>,
    held: Option<&mut HeldFiles>,
    refuse: impl FnOnce(String) -> &'static str,
    limit: &'static str,
) -> Result<(String, Option<HashedFile>), &'static str> {
    if let Some(reason) = refused_shape(argument) {
        return Err(refuse(format!("argument {argument:?} {reason}")));
    }
    let candidate = lexical(&base.join(argument));
    let parent_dir = Path::new(argument)
        .components()
        .any(|component| component == Component::ParentDir);
    if folder.is_some_and(|folder| candidate.starts_with(folder)) {
        if !plain_value(argument) && !candidate.is_file() {
            return Err(refuse(format!("argument {argument:?} {NOT_PLAIN}")));
        }
        let given = if parent_dir {
            candidate.display().to_string()
        } else {
            argument.to_owned()
        };
        return Ok((given, None));
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
        let sha256 = match held {
            Some(held) => held
                .hash(&candidate)
                .map_err(|error| unhashed(error, limit))?,
            None => String::new(),
        };
        return Ok((candidate.display().to_string(), Some((candidate, sha256))));
    }
    if folder.is_none()
        && Path::new(argument)
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
    {
        if !plain_value(argument) {
            return Err(refuse(format!("argument {argument:?} {NOT_PLAIN}")));
        }
        // Resolves in the run's empty private directory, if at all.
        return Ok((argument.to_owned(), None));
    }
    Err(not_a_file(&candidate))
}

/// Why a word that names no file is refused when it is not plain words: an
/// interpreter could take it as code (`exec(open(…).read())`, review
/// 2026-10-17 SEC-7).
const NOT_PLAIN: &str = "names no file and is not plain words (letters, digits and -_.,+/\\); \
                         text an interpreter could run is no plugin argument";

/// The refusal of a command line that names no hashed script: an interpreter
/// started without one reads its code from standard input or the environment.
pub(super) const NO_SCRIPT: &str = "The plugin's command line names no script file: an \
                                    interpreter is given the hashed script it runs.";

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
    if !executable.starts_with(folder) {
        // A listing does not read the file; a run hashes it.
        let sha256 = match held.as_deref_mut() {
            Some(held) => held
                .hash(&executable)
                .map_err(|error| unhashed(error, "plugin_manifest_invalid"))?,
            None => String::new(),
        };
        outside.push((executable.clone(), sha256));
    }
    let in_folder = |path: &Path| {
        path.strip_prefix(folder)
            .is_ok_and(|relative| files.iter().any(|(file, _)| *file == slashed(relative)))
    };
    not_a_shebang(&executable, invalid)?;
    if let Some(reason) = too_many_arguments(&manifest.arguments) {
        return Err(invalid(reason));
    }
    let mut arguments = Vec::new();
    let mut names_file = Vec::new();
    for argument in &manifest.arguments {
        let (given, file) = resolve_argument(
            argument,
            folder,
            Some(folder),
            held.as_deref_mut(),
            invalid,
            "plugin_manifest_invalid",
        )?;
        names_file.push(file.is_some() || in_folder(&lexical(&folder.join(&given))));
        arguments.push(given);
        outside.extend(file);
    }
    // An executable in the folder is the plugin's hashed code itself.
    if !in_folder(&executable) {
        script_first(&executable, &arguments, &names_file, invalid)?;
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
    // Review 2026-10-17 P3: inputs too large for the protocol line failed only
    // after the user had allowed the run and the process had started. They go
    // out as one compact JSON line, encoded here exactly as the host sends it.
    let bytes = Value::Object(resolved.clone()).to_string().len();
    let limit = ketchup_scheduler::plugin::MAX_PLUGIN_INPUTS_BYTES;
    let largest = resolved
        .iter()
        .max_by_key(|(name, value)| name.len() + value.to_string().len());
    if let Some((name, _)) = largest.filter(|_| bytes > limit) {
        return Err(invalid(format!(
            "input {name:?} makes the inputs {bytes} bytes as JSON; a plugin receives at most {limit}"
        )));
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

/// Reads `plugin`; a run (`staged`, with its request's cancellation) copies it
/// into a stage first and hashes the copy, held until the run ends.
fn read_folder(plugin: &str, staged: Option<&Arc<AtomicBool>>) -> Result<ReadFolder, &'static str> {
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
    let mut held = match staged {
        Some(cancelled) => {
            let held = HeldFiles::staged(Arc::clone(cancelled)).map_err(unreadable)?;
            stage_folder(&folder, &held)?;
            Some(held)
        }
        None => None,
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
    let read = read_folder(&folder.to_string_lossy(), None)?;
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
/// part of the consent. Reading stops at the next file once `cancelled` is set.
pub(super) fn load(
    plugin: &str,
    given: Option<&serde_json::Map<String, Value>>,
    cancelled: &Arc<AtomicBool>,
) -> Result<FolderLaunch, &'static str> {
    let ReadFolder {
        folder,
        manifest,
        files,
        executable,
        arguments,
        outside,
        held,
    } = read_folder(plugin, Some(cancelled))?;
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
