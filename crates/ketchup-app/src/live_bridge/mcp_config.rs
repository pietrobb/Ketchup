//! Reads and writes the `ketchup` server in an AI client's own configuration
//! file, so Connect works without the client's command on PATH: the
//! `mcpServers` JSON of Claude Code, Claude Desktop and Supervisor, and the
//! `[mcp_servers.ketchup]` section of Codex's config.toml. Everything else in
//! the file is kept.

use serde_json::value::RawValue;
use std::io;
use std::path::Path;

/// The server name Kečup registers under.
pub(crate) const SERVER: &str = "ketchup";

/// The keys of a JSON client's network server: its address, headers (a
/// bearer token among them, or Claude Code's command printing them) and
/// sign-in (review 2026-11-06 r23 P3: `headersHelper` stayed).
const JSON_NETWORK_KEYS: [&str; 4] = ["url", "headers", "headersHelper", "oauth"];

/// The keys of a Codex network server: its address, token, headers and
/// sign-in (review 2026-11-06 r23 P3: `scopes` and `oauth_resource` stayed).
const CODEX_NETWORK_KEYS: [&str; 7] = [
    "url",
    "bearer_token",
    "bearer_token_env_var",
    "http_headers",
    "env_http_headers",
    "scopes",
    "oauth_resource",
];

/// A JSON object in the order it was written, its values as written.
type Object = indexmap::IndexMap<String, Box<RawValue>>;

pub(crate) fn invalid_data(
    error: impl Into<Box<dyn std::error::Error + Send + Sync>>,
) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, error)
}

/// A JSON object; serde's error says where the text stops being one.
fn object(text: &str) -> io::Result<Object> {
    serde_json::from_str(text).map_err(invalid_data)
}

/// The object at `path` in the file: an error names the keys, since serde's
/// line and column would count in the value alone (review 2026-11-04 r19 P3:
/// `{"projects":{"/a":null}}` said "line 1 column 4").
fn object_at(raw: &RawValue, path: &[&str]) -> io::Result<Object> {
    serde_json::from_str(raw.get()).map_err(|error: serde_json::Error| {
        let message = error.to_string();
        let position = format!(" at line {} column {}", error.line(), error.column());
        let message = message.strip_suffix(&position).unwrap_or(&message);
        invalid_data(format!("{}: {message}", path.join(" → ")))
    })
}

/// What a client's `ketchup` entry is, when it starts no program though it
/// is there: turned off (`disabled`), or a network server (`url`, a `type`
/// other than stdio). Connect turns it on and makes it a program.
pub(crate) fn json_server_off(server: &serde_json::Value) -> Option<String> {
    if let Some(url) = server["url"].as_str() {
        return Some(url.to_owned());
    }
    if let Some(kind) = server["type"].as_str().filter(|kind| *kind != "stdio") {
        return Some(kind.to_owned());
    }
    (server["disabled"] == true).then(|| server["command"].as_str().unwrap_or_default().to_owned())
}

fn raw(value: &impl serde::Serialize) -> io::Result<Box<RawValue>> {
    let text = serde_json::to_string_pretty(value).map_err(invalid_data)?;
    RawValue::from_string(text).map_err(invalid_data)
}

/// `servers` (an `mcpServers` object at `path`) with its `ketchup` server
/// starting `executable --mcp`: a new one, or the one there with only
/// `command`, `args` and `type` changed, turned on and no network server
/// (review 2026-11-04 r19 P3: Connect said it was set up while the client
/// started nothing).
///
/// What only a network server has goes, its token and headers with it, so no
/// secret stays on the program (review 2026-11-06 SEC-181).
fn with_server(
    servers: Option<&RawValue>,
    path: &[&str],
    executable: &str,
    named_transport: bool,
) -> io::Result<Box<RawValue>> {
    let mut servers = servers.map_or_else(|| Ok(Object::new()), |raw| object_at(raw, path))?;
    let mut server = match servers.get(SERVER) {
        Some(server) => object_at(server, &[path, &[SERVER]].concat())?,
        None => Object::from([("env".to_owned(), raw(&serde_json::json!({}))?)]),
    };
    server.insert("command".to_owned(), raw(&executable)?);
    server.insert("args".to_owned(), raw(&["--mcp"])?);
    for key in JSON_NETWORK_KEYS {
        server.shift_remove(key);
    }
    if named_transport {
        server.insert("type".to_owned(), raw(&"stdio")?);
    } else if server.get("type").is_some_and(|kind| {
        serde_json::from_str::<String>(kind.get()).ok().as_deref() != Some("stdio")
    }) {
        server.shift_remove("type");
    }
    if let Some(disabled) = server.get_mut("disabled") {
        *disabled = raw(&false)?;
    }
    servers.insert(SERVER.to_owned(), raw(&server)?);
    raw(&servers)
}

/// Which `ketchup` entries of a JSON client's file a write changes.
pub(crate) struct Rewrite<'a> {
    /// The user's own, made when there is none.
    pub user: bool,
    /// Whether a project's own entry, as written, changes.
    pub project: &'a dyn Fn(&serde_json::Value) -> bool,
}

impl Rewrite<'static> {
    /// The user's entry and every project's.
    #[cfg(test)]
    pub(crate) fn every() -> Self {
        Self {
            user: true,
            project: &|_| true,
        }
    }
}

/// `text` (empty for a new file) with `mcpServers.ketchup` starting
/// `executable --mcp`. Claude Code also wants the transport named, and a
/// project's own `ketchup` entry (`projects.<folder>.mcpServers`) wins over the
/// user's there, so it changes too. Every other key keeps its place and its
/// value as written (review 2026-11-04 SEC-142: serde_json sorted the whole
/// file and the entry lost its `env`, `timeout` and `disabled`).
///
/// `rewrite` says which entries change: the user's, and of the projects with
/// their own entry those it gives `true` (review 2026-11-06 SEC-180: Connect
/// rewrote a project's entry that started another program that works).
pub(crate) fn with_json_server(
    text: &str,
    executable: &str,
    named_transport: bool,
    rewrite: &Rewrite<'_>,
) -> io::Result<String> {
    let mut config = if text.trim().is_empty() {
        Object::new()
    } else {
        object(text)?
    };
    if rewrite.user {
        let servers = with_server(
            config.get("mcpServers").map(AsRef::as_ref),
            &["mcpServers"],
            executable,
            named_transport,
        )?;
        config.insert("mcpServers".to_owned(), servers);
    }
    if let Some(projects) = config.get("projects") {
        let mut projects = object_at(projects, &["projects"])?;
        for (folder, project) in &mut projects {
            let path = ["projects", folder.as_str(), "mcpServers"];
            let mut settings = object_at(project, &path[..2])?;
            if let Some(servers) = settings.get("mcpServers")
                && let Some(server) = object_at(servers, &path)?.get(SERVER)
                && (rewrite.project)(&serde_json::from_str(server.get()).map_err(invalid_data)?)
            {
                let servers = with_server(Some(servers), &path, executable, named_transport)?;
                settings.insert("mcpServers".to_owned(), servers);
                *project = raw(&settings)?;
            }
        }
        config.insert("projects".to_owned(), raw(&projects)?);
    }
    serde_json::to_string_pretty(&config)
        .map(|text| text + "\n")
        .map_err(invalid_data)
}

/// The `ketchup` server's `command` in a Codex config.toml, in any form TOML
/// gives it: a table, an inline table or dotted keys (review 2026-11-04 SEC-140).
/// A file that is no TOML is its parse error, not "no server".
pub(crate) fn codex_command(toml: &str) -> Result<Option<String>, toml_edit::TomlError> {
    let document: toml_edit::DocumentMut = toml.parse()?;
    Ok(document
        .get("mcp_servers")
        .and_then(|servers| servers.get(SERVER))
        .and_then(|server| server.get("command"))
        .and_then(toml_edit::Item::as_str)
        .map(str::to_owned))
}

/// What Codex's `ketchup` server is when it starts no program: `enabled =
/// false` (its command), or a network server (its `url`).
pub(crate) fn codex_server_off(toml: &str) -> Result<Option<String>, toml_edit::TomlError> {
    let document: toml_edit::DocumentMut = toml.parse()?;
    let Some(server) = document
        .get("mcp_servers")
        .and_then(|servers| servers.get(SERVER))
    else {
        return Ok(None);
    };
    let text = |key| server.get(key).and_then(toml_edit::Item::as_str);
    Ok(text("url")
        .or_else(|| {
            let off = server.get("enabled").and_then(toml_edit::Item::as_bool) == Some(false);
            off.then(|| text("command").unwrap_or_default())
        })
        .map(str::to_owned))
}

/// `toml` with its `ketchup` server starting `executable --mcp`: only its
/// `command` and `args` change, every other line stays as written. A file that
/// is no TOML is refused rather than rewritten (review 2026-11-04 SEC-140: the
/// line editor made Codex's file declare the server twice).
pub(crate) fn with_codex_server(toml: &str, executable: &str) -> io::Result<String> {
    let invalid = |message: String| io::Error::new(io::ErrorKind::InvalidData, message);
    let mut document: toml_edit::DocumentMut = toml
        .parse()
        .map_err(|error: toml_edit::TomlError| invalid(error.to_string()))?;
    let servers = document
        .entry("mcp_servers")
        .or_insert_with(|| {
            let mut servers = toml_edit::Table::new();
            servers.set_implicit(true);
            toml_edit::Item::Table(servers)
        })
        .as_table_like_mut()
        .ok_or_else(|| invalid("mcp_servers is not a table".to_owned()))?;
    let server = servers
        .entry(SERVER)
        .or_insert_with(toml_edit::table)
        .as_table_like_mut()
        .ok_or_else(|| invalid(format!("mcp_servers.{SERVER} is not a table")))?;
    // A literal string keeps a Windows path readable; one with a quote in it
    // is a basic string.
    let command = format!("'{executable}'")
        .parse::<toml_edit::Value>()
        .ok()
        .filter(|value| value.as_str() == Some(executable))
        .unwrap_or_else(|| executable.into());
    server.insert("command", toml_edit::Item::Value(command));
    server.insert(
        "args",
        toml_edit::value(toml_edit::Array::from_iter(["--mcp"])),
    );
    // Turned on and a program, not a network server (review 2026-11-04 r19 P3).
    for key in CODEX_NETWORK_KEYS {
        server.remove(key);
    }
    if server.contains_key("enabled") {
        server.insert("enabled", toml_edit::value(true));
    }
    Ok(document.to_string())
}

/// Replaces `file` with `text`, keeping what the file is (review 2026-11-04
/// SEC-141: `.claude.json` holds the `env` secrets of other servers, and the
/// write left it readable by everyone and replaced a symlink by a plain file).
/// A symlink's target is written; the first replacement keeps the original as
/// `<file>.ketchup.bak`; the text is flushed to disk before it counts. On Unix
/// it goes through a unique sibling with the file's permissions, renamed over
/// it, so a client reading meanwhile sees the old or the new configuration.
/// On Windows `ReplaceFileW` swaps it the same way and gives the new file the
/// old one's ACL, which a renamed file would not keep (review 2026-11-04 r19
/// P3: it was truncated and written in place). A symlink to a file not there
/// yet creates its target (r19 P3: it became a plain file).
pub(crate) fn replace_file(file: &Path, text: &str) -> io::Result<()> {
    write_keeping_file(file, text, true)
}

/// [`replace_file`] without the backup when `backup` is false. A new file is
/// readable by its owner only (review 2026-11-04 r19 P3: the settings file,
/// with the documents opened, was readable by everyone).
pub(crate) fn write_keeping_file(file: &Path, text: &str, backup: bool) -> io::Result<()> {
    let target = match std::fs::canonicalize(file) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let file = link_target(file);
            let directory = file.parent().unwrap_or(Path::new("."));
            let nearest = nearest_folder(directory);
            let written = std::fs::create_dir_all(directory)
                .and_then(|()| write_new(directory, &file, text, nearest.as_ref()));
            // The folders made for a file not written are not left behind (review
            // 2026-11-06 r23 SEC-200), nor those made before a folder that could
            // not be (review 2026-11-07 P3).
            if written.is_err()
                && let Some((folder, _)) = &nearest
            {
                remove_made_folders(folder, directory);
            }
            return written;
        }
        Err(error) => return Err(error),
    };
    let mut copy = target.as_os_str().to_owned();
    copy.push(".ketchup.bak");
    let copy = std::path::PathBuf::from(copy);
    if backup && !copy.exists() {
        std::fs::copy(&target, &copy)?;
    }
    let metadata = std::fs::metadata(&target)?;
    let directory = target.parent().unwrap_or(Path::new("."));
    write_staged(
        directory,
        &target,
        text,
        Some(&metadata),
        Some((&metadata, true)),
    )
}

/// Writes `text` as the new `file` in `directory`, made below `nearest`, the
/// nearest folder that was there, with what it is.
fn write_new(
    directory: &Path,
    file: &Path,
    text: &str,
    nearest: Option<&(std::path::PathBuf, std::fs::Metadata)>,
) -> io::Result<()> {
    #[cfg(unix)]
    if let Some((folder, like)) = nearest {
        give_folders(folder, directory, like)?;
    }
    write_staged(
        directory,
        file,
        text,
        None,
        nearest.map(|(_, like)| (like, false)),
    )
}

/// Removes the folders below `folder` down to `directory` that are empty, the
/// deepest first.
fn remove_made_folders(folder: &Path, directory: &Path) {
    for made in directory.ancestors().take_while(|made| *made != folder) {
        // A folder not made, below one that could not be, is passed over.
        if std::fs::symlink_metadata(made).is_err() {
            continue;
        }
        if let Err(error) = std::fs::remove_dir(made) {
            ketchup_rejection::log!("the folder {} made for it stays: {error}", made.display());
            return;
        }
    }
}

/// The nearest folder at or above `directory` that is there, with what it is:
/// a new file and the folders made for it are its owner's, not root's under
/// `sudo` (review 2026-11-06 P3).
fn nearest_folder(directory: &Path) -> Option<(std::path::PathBuf, std::fs::Metadata)> {
    directory.ancestors().find_map(|folder| {
        std::fs::metadata(folder)
            .ok()
            .map(|like| (folder.to_owned(), like))
    })
}

/// Gives the folders made below `folder` down to `directory` the owner of
/// `folder` (`like`), not following a symlink put in place of one (review
/// 2026-11-06 r23 P3).
#[cfg(unix)]
fn give_folders(folder: &Path, directory: &Path, like: &std::fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    for made in directory.ancestors().take_while(|made| *made != folder) {
        let own = std::fs::symlink_metadata(made)?;
        keep_owner(
            (own.uid(), own.gid()),
            (like.uid(), like.gid()),
            false,
            |uid, gid| std::os::unix::fs::lchown(made, uid, gid),
        )?;
    }
    Ok(())
}

/// The file a chain of symlinks at `file` ends in, though it does not exist.
fn link_target(file: &Path) -> std::path::PathBuf {
    let mut file = file.to_owned();
    // A loop of links stops where the system would.
    let mut hops = 0;
    while hops < 40
        && let Ok(target) = std::fs::read_link(&file)
    {
        file = file.parent().unwrap_or(Path::new(".")).join(target);
        hops += 1;
    }
    file
}

/// Writes `text` to a unique file in `directory`, flushes it and puts it in
/// place of `file`, with the permissions of the file it replaces (`like`)
/// and on Unix the owner of `owner`, that file (`true`: replacing it) or the
/// folder a new one is made in (review 2026-11-04 r19 P3: under `sudo` it
/// became root's).
fn write_staged(
    directory: &Path,
    file: &Path,
    text: &str,
    like: Option<&std::fs::Metadata>,
    #[cfg_attr(not(unix), allow(unused_variables))] owner: Option<(&std::fs::Metadata, bool)>,
) -> io::Result<()> {
    let mut staged = tempfile::Builder::new()
        .prefix(".ketchup-config-")
        .tempfile_in(directory)?;
    #[cfg(unix)]
    if let Some((owner, replacing)) = owner {
        use std::os::unix::fs::MetadataExt;
        let own = staged.as_file().metadata()?;
        keep_owner(
            (own.uid(), own.gid()),
            (owner.uid(), owner.gid()),
            replacing,
            |uid, gid| std::os::unix::fs::fchown(staged.as_file(), uid, gid),
        )?;
    }
    if let Some(like) = like {
        staged.as_file().set_permissions(like.permissions())?;
    }
    io::Write::write_all(&mut staged, text.as_bytes())?;
    staged.as_file().sync_all()?;
    #[cfg(windows)]
    if file.is_file() {
        // Closed for the swap; the staged name is gone after it, and dropping
        // the path then removes nothing.
        return swap_in(file, staged.into_temp_path(), replace_keeping_acl);
    }
    staged.persist(file).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

/// Gives what was just made (owned `own`, user and group, its user the one
/// writing) the owner `like` of the file it replaces (`replacing`) or of the
/// folder it is made in, through `chown`. Only root gives a file to another
/// user, so only root is given one: under `sudo` a new file is the folder
/// owner's, while a user writing into a shared folder or `/tmp` keeps theirs
/// (review 2026-11-06 r23 SEC-200: they were refused with EPERM). A user gives
/// it only to a group they are in: the user's own file in a group they left
/// keeps its user and takes theirs, rather than not being written (review
/// 2026-11-06 P3); another user's file stays theirs or is not written.
#[cfg(any(unix, test))]
fn keep_owner(
    own: (u32, u32),
    like: (u32, u32),
    replacing: bool,
    chown: impl Fn(Option<u32>, Option<u32>) -> io::Result<()>,
) -> io::Result<()> {
    let root = own.0 == 0;
    if own == like || (!root && !replacing) {
        return Ok(());
    }
    match chown(Some(like.0), Some(like.1)) {
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied && own.0 == like.0 => {
            ketchup_rejection::log!("the file keeps its user but not group {}: {error}", like.1);
            Ok(())
        }
        kept => kept,
    }
}

/// Puts `staged` in the place of `file` through `replace`. With no backup
/// name, `ReplaceFileW`'s `ERROR_UNABLE_TO_MOVE_REPLACEMENT` (1176) says the
/// file it replaces is gone and the staged one still under its own name: that
/// one is renamed into place, or where it cannot be, kept and named, never
/// removed (review 2026-11-06 P3).
#[cfg(any(windows, test))]
fn swap_in(
    file: &Path,
    staged: tempfile::TempPath,
    replace: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    const ERROR_UNABLE_TO_MOVE_REPLACEMENT: i32 = 1176;
    match replace(file, &staged) {
        Err(error) if error.raw_os_error() == Some(ERROR_UNABLE_TO_MOVE_REPLACEMENT) => {
            staged.persist(file).map_err(|failed| {
                io::Error::new(
                    failed.error.kind(),
                    NotPutInPlace {
                        file: file.to_owned(),
                        kept: failed.path.keep().map_err(|lost| lost.error),
                        cause: failed.error,
                    },
                )
            })
        }
        // Done, the staged name is gone; dropping it removes nothing.
        done => done,
    }
}

/// The file a swap removed, its new text not renamed into its place: where
/// that text is kept, and why it was not put there.
#[cfg(any(windows, test))]
#[derive(Debug)]
struct NotPutInPlace {
    file: std::path::PathBuf,
    kept: io::Result<std::path::PathBuf>,
    cause: io::Error,
}

#[cfg(any(windows, test))]
impl std::fmt::Display for NotPutInPlace {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} was removed and its new text not put in its place; ",
            self.file.display()
        )?;
        match &self.kept {
            Ok(kept) => write!(formatter, "the new text is kept as {}", kept.display()),
            Err(lost) => write!(formatter, "the new text could not be kept either: {lost}"),
        }
    }
}

#[cfg(any(windows, test))]
impl std::error::Error for NotPutInPlace {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.cause)
    }
}

/// `ReplaceFileW`: `replacement` takes the place of `file` in one step and
/// keeps `file`'s ACL, attributes and creation time.
#[cfg(windows)]
#[allow(unsafe_code)]
fn replace_keeping_acl(file: &Path, replacement: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{REPLACEFILE_WRITE_THROUGH, ReplaceFileW};
    let wide = |path: &Path| -> Vec<u16> { path.as_os_str().encode_wide().chain([0]).collect() };
    let (file, replacement) = (wide(file), wide(replacement));
    // SAFETY: both names are NUL-terminated and outlive the call; no backup
    // name and no reserved pointers are passed.
    let done = unsafe {
        ReplaceFileW(
            file.as_ptr(),
            replacement.as_ptr(),
            std::ptr::null(),
            REPLACEFILE_WRITE_THROUGH,
            std::ptr::null(),
            std::ptr::null(),
        )
    };
    if done == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// The text of `file`, empty when it does not exist yet.
pub(crate) fn read_or_empty(file: &Path) -> io::Result<String> {
    match std::fs::read_to_string(file) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(String::new()),
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review 2026-11-06 P3: a new file, written under `sudo`, was root's. It
    /// and the folders made for it take the owner of the nearest folder there.
    #[test]
    fn a_new_file_takes_the_owner_of_the_nearest_folder() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join(".claude").join("sub").join("x.json");
        let (nearest, like) = nearest_folder(file.parent().unwrap()).unwrap();
        assert_eq!(nearest, directory.path());
        write_keeping_file(&file, "{}", false).unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            for made in [file.as_path(), file.parent().unwrap()] {
                let own = std::fs::metadata(made).unwrap();
                assert_eq!((own.uid(), own.gid()), (like.uid(), like.gid()), "{made:?}");
            }
        }
        #[cfg(not(unix))]
        assert!(like.is_dir());
    }

    /// Review 2026-11-06 P3: `ReplaceFileW` failing with 1176, with no backup
    /// name, has removed the file it replaces and left the new text under its
    /// staged name, which was then removed too: the settings file was lost.
    #[test]
    fn a_swap_that_removed_the_old_file_puts_the_new_one_in_its_place() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory.path().join("settings.json");
        let stage = || {
            let mut staged = tempfile::NamedTempFile::new_in(directory.path()).unwrap();
            io::Write::write_all(&mut staged, b"new").unwrap();
            staged.into_temp_path()
        };
        let removed_the_old = |file: &Path, _: &Path| {
            std::fs::remove_file(file)?;
            Err(io::Error::from_raw_os_error(1176))
        };
        std::fs::write(&file, b"old").unwrap();
        swap_in(&file, stage(), removed_the_old).unwrap();
        assert_eq!(std::fs::read(&file).unwrap(), b"new");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);

        // Where it cannot be renamed either, the new text is kept and named.
        std::fs::write(&file, b"old").unwrap();
        let staged = stage();
        let staged_path = staged.to_path_buf();
        let error = swap_in(
            &directory.path().join("gone").join("x.json"),
            staged,
            |_, _| Err(io::Error::from_raw_os_error(1176)),
        )
        .unwrap_err();
        assert!(staged_path.exists());
        assert!(
            error
                .to_string()
                .contains(&staged_path.display().to_string()),
            "{error}"
        );
    }

    /// Review 2026-11-06 P3: the user's own settings file in a group they are
    /// not in was not saved, `fchown` refusing it with EPERM.
    #[test]
    fn a_file_in_a_group_its_user_left_is_written_keeping_its_user() {
        #[cfg(unix)]
        let refused =
            |_: Option<u32>, _: Option<u32>| Err(io::Error::from_raw_os_error(libc::EPERM));
        #[cfg(not(unix))]
        let refused = |_: Option<u32>, _: Option<u32>| Err(io::ErrorKind::PermissionDenied.into());
        assert!(keep_owner((1000, 1000), (1000, 50), true, refused).is_ok());
        // Another user's file stays theirs or is not written.
        assert!(keep_owner((1000, 1000), (0, 0), true, refused).is_err());
        let given = std::cell::Cell::new(None);
        keep_owner((0, 0), (1000, 50), false, |uid, gid| {
            given.set(Some((uid, gid)));
            Ok(())
        })
        .unwrap();
        assert_eq!(given.get(), Some((Some(1000), Some(50))));
        keep_owner((1000, 50), (1000, 50), true, |_, _| {
            panic!("nothing to give")
        })
        .unwrap();
    }

    /// Review 2026-11-06 r23 SEC-200: a user writing a new file into a folder
    /// of root's with setgid, or into `/tmp`, was refused with EPERM, as the
    /// file was given the folder's owner, which only root can give.
    #[test]
    fn a_user_writing_into_another_users_folder_keeps_the_new_file() {
        let refused = |_: Option<u32>, _: Option<u32>| -> io::Result<()> {
            Err(io::ErrorKind::PermissionDenied.into())
        };
        assert!(keep_owner((1000, 65534), (0, 65534), false, refused).is_ok());
        assert!(keep_owner((1000, 1000), (0, 0), false, refused).is_ok());
        // Under `sudo` it is still the folder owner's.
        let given = std::cell::Cell::new(None);
        keep_owner((0, 0), (1000, 1000), false, |uid, gid| {
            given.set(Some((uid, gid)));
            Ok(())
        })
        .unwrap();
        assert_eq!(given.get(), Some((Some(1000), Some(1000))));
        // `/tmp`, root's with the sticky bit, as the review wrote into.
        #[cfg(unix)]
        {
            let folder =
                std::env::temp_dir().join(format!("ketchup-sec200-{}", std::process::id()));
            let file = folder.join("codex").join("config.toml");
            write_keeping_file(&file, "x = 1\n", false).unwrap();
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "x = 1\n");
            std::fs::remove_dir_all(&folder).unwrap();
        }
    }

    /// Review 2026-11-06 r23 SEC-200: the folders made for a file that was not
    /// written stayed behind, empty.
    #[test]
    fn the_empty_folders_made_for_a_file_not_written_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let made = directory.path().join("shared").join("codex");
        std::fs::create_dir_all(&made).unwrap();
        remove_made_folders(directory.path(), &made);
        assert!(directory.path().exists());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
        // A folder that holds something stays, and the ones above it.
        std::fs::create_dir_all(&made).unwrap();
        std::fs::write(directory.path().join("shared").join("other.json"), "{}").unwrap();
        remove_made_folders(directory.path(), &made);
        assert!(!made.exists());
        assert!(directory.path().join("shared").exists());
    }

    /// Review 2026-11-07 P3: when a folder could not be made, those made above
    /// it stayed behind.
    #[test]
    fn the_folders_made_before_one_that_could_not_be_are_removed() {
        let directory = tempfile::tempdir().unwrap();
        let file = directory
            .path()
            .join("shared")
            .join("x".repeat(300))
            .join("config.toml");
        assert!(write_keeping_file(&file, "x = 1\n", false).is_err());
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_json_server_is_replaced_and_every_other_setting_kept() {
        let text = r#"{"theme": "dark", "mcpServers": {"other": {"command": "x.exe"},
            "ketchup": {"command": "C:\\Supervisor\\Kecup\\ketchup-app.exe", "args": []}}}"#;
        let written = with_json_server(
            text,
            r"C:\Programs\Ketchup\Ketchup.exe",
            false,
            &Rewrite::every(),
        )
        .unwrap();
        let config: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(config["theme"], "dark");
        assert_eq!(config["mcpServers"]["other"]["command"], "x.exe");
        assert_eq!(
            config["mcpServers"]["ketchup"]["command"],
            r"C:\Programs\Ketchup\Ketchup.exe"
        );
        assert_eq!(config["mcpServers"]["ketchup"]["args"][0], "--mcp");
        assert!(config["mcpServers"]["ketchup"].get("type").is_none());
        let fresh = with_json_server("", "k.exe", true, &Rewrite::every()).unwrap();
        let config: serde_json::Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!(config["mcpServers"]["ketchup"]["type"], "stdio");
        assert!(with_json_server("[1, 2]", "k.exe", false, &Rewrite::every()).is_err());
        assert!(with_json_server("{not json", "k.exe", false, &Rewrite::every()).is_err());
    }

    /// Review 2026-11-04 SEC-142: serde_json sorted every key of `.claude.json`
    /// and the whole `ketchup` entry was replaced, so its `env`, `timeout` and
    /// `disabled` were lost. Only `command`, `args` and `type` change, the keys
    /// keep their order, and so does a project's own `ketchup` entry, which wins
    /// over the user's in that project.
    #[test]
    fn the_json_server_keeps_its_order_and_its_own_settings() {
        let text = r#"{
  "theme": "dark",
  "mcpServers": {
    "zeta": {"command": "z.exe"},
    "ketchup": {"command": "C:\\old\\k.exe", "args": [], "env": {"TOKEN": "x"}, "timeout": 30, "disabled": false}
  },
  "projects": {
    "C:/work": {"mcpServers": {"ketchup": {"command": "C:\\old\\k.exe", "env": {"B": "2"}}}},
    "C:/other": {"allowedTools": []}
  },
  "alpha": 1 }"#;
        let written =
            with_json_server(text, r"C:\new\Ketchup.exe", true, &Rewrite::every()).unwrap();
        let at = |key: &str| {
            written
                .find(key)
                .unwrap_or_else(|| panic!("{key}: {written}"))
        };
        assert!(at("\"theme\"") < at("\"mcpServers\""), "{written}");
        assert!(at("\"mcpServers\"") < at("\"projects\""), "{written}");
        assert!(at("\"projects\"") < at("\"alpha\""), "{written}");
        assert!(at("\"zeta\"") < at("\"ketchup\""), "{written}");
        let config: serde_json::Value = serde_json::from_str(&written).unwrap();
        let server = &config["mcpServers"]["ketchup"];
        assert_eq!(server["command"], r"C:\new\Ketchup.exe");
        assert_eq!(server["args"], serde_json::json!(["--mcp"]));
        assert_eq!(server["type"], "stdio");
        assert_eq!(server["env"]["TOKEN"], "x");
        assert_eq!(
            (&server["timeout"], &server["disabled"]),
            (&serde_json::json!(30), &serde_json::json!(false))
        );
        let project = &config["projects"]["C:/work"]["mcpServers"]["ketchup"];
        assert_eq!(project["command"], r"C:\new\Ketchup.exe");
        assert_eq!(project["env"]["B"], "2");
        assert!(config["projects"]["C:/other"].get("mcpServers").is_none());
        assert_eq!(config["mcpServers"]["zeta"]["command"], "z.exe");
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
            codex_command(toml).unwrap().as_deref(),
            Some(r"C:\Program Files\Ketchup\Ketchup.exe")
        );
        assert_eq!(
            codex_command("[mcp_servers.ketchup]\ncommand = 'C:\\k\\Ketchup.exe'\n")
                .unwrap()
                .as_deref(),
            Some(r"C:\k\Ketchup.exe")
        );
        assert_eq!(
            codex_command("[mcp_servers.other]\ncommand = \"x\"\n").unwrap(),
            None
        );
    }

    #[test]
    fn the_codex_server_command_is_replaced_and_the_rest_kept() {
        let toml = "model = \"gpt\"\n\n[mcp_servers.ketchup]\ncommand = 'C:\\old\\ketchup-app.exe'\n\n[mcp_servers.ketchup.env]\nA = \"1\"\n\n[mcp_servers.other]\ncommand = \"other.exe\"\n";
        let written = with_codex_server(toml, r"C:\Programs\Ketchup\Ketchup.exe").unwrap();
        assert_eq!(
            codex_command(&written).unwrap().as_deref(),
            Some(r"C:\Programs\Ketchup\Ketchup.exe")
        );
        assert!(!written.contains("old"), "{written}");
        // Its own settings stay (review 2026-11-04 SEC-142).
        assert!(
            written.contains("[mcp_servers.ketchup.env]\nA = \"1\"\n"),
            "{written}"
        );
        assert!(written.starts_with("model = \"gpt\"\n"), "{written}");
        assert!(written.contains("[mcp_servers.other]\ncommand = \"other.exe\"\n"));
        assert_eq!(written.matches("[mcp_servers.ketchup]").count(), 1);
        let quoted = with_codex_server("", r"C:\Peter's\Ketchup.exe").unwrap();
        assert_eq!(
            codex_command(&quoted).unwrap().as_deref(),
            Some(r"C:\Peter's\Ketchup.exe")
        );
        assert!(quoted.contains("[mcp_servers.ketchup]"), "{quoted}");
    }

    /// Review 2026-11-04 SEC-141: the write made `.claude.json`, which holds
    /// the `env` secrets of other servers, readable by everyone (0600 came out
    /// 0644), replaced a symlink by a plain file, and kept no copy of the file.
    #[test]
    fn replacing_a_configuration_keeps_what_the_file_is_and_a_first_copy() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("config.json");
        replace_file(&file, "new\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        let backup = folder.path().join("config.json.ketchup.bak");
        assert!(!backup.exists(), "nothing to keep of a new file");
        replace_file(&file, "second\n").unwrap();
        replace_file(&file, "third\n").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "third\n");
        assert_eq!(std::fs::read_to_string(&backup).unwrap(), "new\n");
        let names: Vec<_> = std::fs::read_dir(folder.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 2, "no staged file is left: {names:?}");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let secret = folder.path().join(".claude.json");
            std::fs::write(&secret, "{}").unwrap();
            std::fs::set_permissions(&secret, std::fs::Permissions::from_mode(0o600)).unwrap();
            replace_file(&secret, "{\"a\": 1}").unwrap();
            let mode = std::fs::metadata(&secret).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
            let link = folder.path().join("link.json");
            std::os::unix::fs::symlink(&secret, &link).unwrap();
            replace_file(&link, "{\"b\": 2}").unwrap();
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read_to_string(&secret).unwrap(), "{\"b\": 2}");
        }
        #[cfg(windows)]
        {
            let target = folder.path().join("target.json");
            std::fs::write(&target, "{}").unwrap();
            let link = folder.path().join("link.json");
            // Creating a symlink needs developer mode or elevation.
            if std::os::windows::fs::symlink_file(&target, &link).is_ok() {
                replace_file(&link, "{\"b\": 2}").unwrap();
                assert!(
                    std::fs::symlink_metadata(&link)
                        .unwrap()
                        .file_type()
                        .is_symlink()
                );
                assert_eq!(std::fs::read_to_string(&target).unwrap(), "{\"b\": 2}");
            }
        }
    }

    /// Review 2026-11-04 r19 P3: on Windows the file was truncated and written
    /// in place, so a client reading meanwhile saw part of it; a symlink to a
    /// file not there yet became a plain file.
    #[test]
    fn a_configuration_is_swapped_whole_and_a_dangling_link_creates_its_target() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("config.json");
        std::fs::write(&file, "old\n").unwrap();
        let mut reader = std::fs::File::open(&file).unwrap();
        replace_file(&file, "new\n").unwrap();
        let mut seen = String::new();
        io::Read::read_to_string(&mut reader, &mut seen).unwrap();
        assert_eq!(seen, "old\n", "a reader keeps the whole old file");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
        #[cfg(windows)]
        {
            // An ACL of its own (no inherited entries) stays the file's.
            let icacls = |args: &[&str]| {
                let output = std::process::Command::new("icacls")
                    .arg(&file)
                    .args(args)
                    .output()
                    .unwrap();
                String::from_utf8_lossy(&output.stdout).into_owned()
            };
            // Everyone (S-1-1-0), whatever the system's language.
            icacls(&["/inheritance:r", "/grant:r", "*S-1-1-0:F"]);
            assert!(!icacls(&[]).contains("(I)"), "{}", icacls(&[]));
            replace_file(&file, "third\n").unwrap();
            assert!(!icacls(&[]).contains("(I)"), "{}", icacls(&[]));
            assert_eq!(std::fs::read_to_string(&file).unwrap(), "third\n");
        }
        let target = folder.path().join("sub").join("target.json");
        let link = folder.path().join("link.json");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(&target, &link).is_ok();
        // Creating a symlink needs developer mode or elevation.
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_file(&target, &link).is_ok();
        if linked {
            replace_file(&link, "{}\n").unwrap();
            assert!(
                std::fs::symlink_metadata(&link)
                    .unwrap()
                    .file_type()
                    .is_symlink()
            );
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "{}\n");
        }
    }

    /// Review 2026-11-04 r19 P3: Connect said the client was set up while its
    /// entry was turned off (`disabled`, Codex `enabled = false`) or a network
    /// server (`url`, `type` http or sse); an error in a nested value gave a
    /// line and column counted in that value alone.
    #[test]
    fn connect_turns_the_entry_on_and_makes_it_a_program() {
        let json = r#"{"mcpServers":{"ketchup":{"type":"http","url":"http://x/mcp","disabled":true,"env":{"A":"1"}}}}"#;
        let server = |text: &str| {
            serde_json::from_str::<serde_json::Value>(text).unwrap()["mcpServers"][SERVER].clone()
        };
        assert_eq!(
            json_server_off(&server(json)).as_deref(),
            Some("http://x/mcp")
        );
        for named_transport in [false, true] {
            let written = with_json_server(
                json,
                r"C:\k\Ketchup.exe",
                named_transport,
                &Rewrite::every(),
            )
            .unwrap();
            let entry = server(&written);
            assert_eq!(json_server_off(&entry), None, "{written}");
            assert_eq!(entry["disabled"], false);
            assert_eq!(entry["env"]["A"], "1");
            assert!(entry.get("url").is_none(), "{written}");
            assert_eq!(
                entry.get("type").and_then(serde_json::Value::as_str),
                named_transport.then_some("stdio")
            );
        }
        let off = r#"{"mcpServers":{"ketchup":{"command":"C:\\k\\Ketchup.exe","disabled":true}}}"#;
        assert_eq!(
            json_server_off(&server(off)).as_deref(),
            Some(r"C:\k\Ketchup.exe")
        );
        let toml = "[mcp_servers.ketchup]\nurl = \"http://x/mcp\"\nenabled = false\n";
        assert_eq!(
            codex_server_off(toml).unwrap().as_deref(),
            Some("http://x/mcp")
        );
        let written = with_codex_server(toml, r"C:\k\Ketchup.exe").unwrap();
        assert_eq!(codex_server_off(&written).unwrap(), None, "{written}");
        assert!(written.contains("enabled = true"), "{written}");
        let disabled = "[mcp_servers.ketchup]\ncommand = 'C:\\k\\Ketchup.exe'\nenabled = false\n";
        assert_eq!(
            codex_server_off(disabled).unwrap().as_deref(),
            Some(r"C:\k\Ketchup.exe")
        );
        // Review 2026-11-06 SEC-181: the network server's token and headers
        // stayed on the program beside its new command.
        let networked = r#"{"mcpServers":{"ketchup":{"type":"http","url":"http://x/mcp",
            "headers":{"Authorization":"Bearer s3cret"},"oauth":{"clientId":"c"},"env":{"A":"1"},
            "headersHelper":"/bin/print-s3cret"}}}"#;
        let written =
            with_json_server(networked, r"C:\k\Ketchup.exe", true, &Rewrite::every()).unwrap();
        assert!(!written.contains("s3cret"), "{written}");
        let entry = server(&written);
        for key in ["headers", "headersHelper", "oauth", "url"] {
            assert!(entry.get(key).is_none(), "{key}: {written}");
        }
        assert_eq!(entry["env"]["A"], "1");
        let networked = "[mcp_servers.ketchup]\nurl = \"http://x/mcp\"\n\
            bearer_token_env_var = \"KETCHUP_TOKEN\"\nbearer_token = \"s3cret\"\n\
            http_headers = { \"X-Key\" = \"s3cret\" }\nenv_http_headers = { \"X-Env\" = \"KEY\" }\n\
            scopes = [\"read\", \"write\"]\noauth_resource = \"http://x/\"\n\
            startup_timeout_sec = 20\n";
        let written = with_codex_server(networked, r"C:\k\Ketchup.exe").unwrap();
        assert!(!written.contains("s3cret"), "{written}");
        for key in [
            "bearer_token_env_var",
            "env_http_headers",
            "url",
            "scopes",
            "oauth_resource",
        ] {
            assert!(!written.contains(key), "{key}: {written}");
        }
        assert!(written.contains("startup_timeout_sec = 20"), "{written}");
        let error = with_json_server(
            r#"{"projects":{"/a":null}}"#,
            "k.exe",
            true,
            &Rewrite::every(),
        )
        .unwrap_err()
        .to_string();
        assert!(error.starts_with("projects → /a: "), "{error}");
        assert!(!error.contains("column"), "{error}");
    }

    /// Review 2026-11-04 SEC-140: the line editor knew only a bare
    /// `[mcp_servers.ketchup]` header, so a header with a comment, an inline
    /// table, dotted keys or a `"""` string with a line starting `[` came out
    /// declaring the server twice, and Codex no longer started. Each is read and
    /// written as TOML; a file that is no TOML is refused, not rewritten.
    #[test]
    fn every_form_of_the_codex_server_is_read_and_written_as_toml() {
        let forms = [
            "[mcp_servers.ketchup] # Kečup\ncommand = 'C:\\old\\k.exe'\nargs = []\n",
            "[mcp_servers]\nketchup = { command = 'C:\\old\\k.exe', args = [] }\n",
            "mcp_servers.ketchup.command = 'C:\\old\\k.exe'\nmcp_servers.ketchup.args = []\n",
            "[mcp_servers.ketchup]\ncommand = 'C:\\old\\k.exe'\nnote = \"\"\"\n[mcp_servers.ketchup]\n\"\"\"\n",
        ];
        for form in forms {
            assert_eq!(
                codex_command(form).unwrap().as_deref(),
                Some(r"C:\old\k.exe"),
                "{form}"
            );
            let written = with_codex_server(form, r"C:\new\Ketchup.exe").unwrap();
            let parsed: toml_edit::DocumentMut = written.parse().unwrap_or_else(|error| {
                panic!("{form}\n->\n{written}\n{error}");
            });
            let server = &parsed["mcp_servers"][SERVER];
            assert_eq!(
                server["command"].as_str(),
                Some(r"C:\new\Ketchup.exe"),
                "{written}"
            );
            assert_eq!(server["args"][0].as_str(), Some("--mcp"), "{written}");
            assert_eq!(
                codex_command(&written).unwrap().as_deref(),
                Some(r"C:\new\Ketchup.exe")
            );
        }
        assert!(with_codex_server("[mcp_servers.ketchup\ncommand = 1", "k.exe").is_err());
        assert!(with_codex_server("mcp_servers = 5\n", "k.exe").is_err());
    }
}
