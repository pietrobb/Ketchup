//! Reads and writes the `ketchup` server in an AI client's own configuration
//! file, so Connect works without the client's command on PATH: the
//! `mcpServers` JSON of Claude Code, Claude Desktop and Supervisor, and the
//! `[mcp_servers.ketchup]` section of Codex's config.toml. Everything else in
//! the file is kept.

use std::io;
use std::path::Path;

/// The server name Kečup registers under.
pub(crate) const SERVER: &str = "ketchup";

/// `text` (empty for a new file) with `mcpServers.ketchup` starting
/// `executable --mcp`. Claude Code also wants the transport named.
pub(crate) fn with_json_server(
    text: &str,
    executable: &str,
    named_transport: bool,
) -> io::Result<String> {
    let mut config: serde_json::Value = if text.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(text)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
    };
    let not_an_object = || io::Error::new(io::ErrorKind::InvalidData, "not a JSON object");
    let servers = config
        .as_object_mut()
        .ok_or_else(not_an_object)?
        .entry("mcpServers")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(not_an_object)?;
    let mut server = serde_json::json!({"command": executable, "args": ["--mcp"], "env": {}});
    if named_transport {
        server["type"] = serde_json::json!("stdio");
    }
    servers.insert(SERVER.to_owned(), server);
    serde_json::to_string_pretty(&config)
        .map(|text| text + "\n")
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// The `ketchup` server's `command` in a Codex config.toml, in any form TOML
/// gives it: a table, an inline table or dotted keys (review 2026-11-04 SEC-140).
pub(crate) fn codex_command(toml: &str) -> Option<String> {
    let document: toml_edit::DocumentMut = toml.parse().ok()?;
    document
        .get("mcp_servers")?
        .get(SERVER)?
        .get("command")?
        .as_str()
        .map(str::to_owned)
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
    Ok(document.to_string())
}

/// Replaces `file` with `text`, keeping what the file is (review 2026-11-04
/// SEC-141: `.claude.json` holds the `env` secrets of other servers, and the
/// write left it readable by everyone and replaced a symlink by a plain file).
/// A symlink's target is written; the first replacement keeps the original as
/// `<file>.ketchup.bak`; the text is flushed to disk before it counts. On Unix
/// it goes through a unique sibling with the file's permissions, renamed over
/// it, so a client reading meanwhile sees the old or the new configuration. On
/// Windows the file is written in place, which keeps its ACL: a new file would
/// take the folder's.
pub(crate) fn replace_file(file: &Path, text: &str) -> io::Result<()> {
    let target = match std::fs::canonicalize(file) {
        Ok(target) => target,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let directory = file.parent().unwrap_or(Path::new("."));
            std::fs::create_dir_all(directory)?;
            return write_staged(directory, file, text, None);
        }
        Err(error) => return Err(error),
    };
    let mut backup = target.as_os_str().to_owned();
    backup.push(".ketchup.bak");
    let backup = std::path::PathBuf::from(backup);
    if !backup.exists() {
        std::fs::copy(&target, &backup)?;
    }
    if cfg!(windows) {
        let mut written = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&target)?;
        io::Write::write_all(&mut written, text.as_bytes())?;
        written.sync_all()
    } else {
        let permissions = std::fs::metadata(&target)?.permissions();
        let directory = target.parent().unwrap_or(Path::new("."));
        write_staged(directory, &target, text, Some(permissions))
    }
}

/// Writes `text` to a unique file in `directory`, flushes it and renames it to `file`.
fn write_staged(
    directory: &Path,
    file: &Path,
    text: &str,
    permissions: Option<std::fs::Permissions>,
) -> io::Result<()> {
    let mut staged = tempfile::Builder::new()
        .prefix(".ketchup-config-")
        .tempfile_in(directory)?;
    if let Some(permissions) = permissions {
        staged.as_file().set_permissions(permissions)?;
    }
    io::Write::write_all(&mut staged, text.as_bytes())?;
    staged.as_file().sync_all()?;
    staged.persist(file).map_err(|error| error.error)?;
    #[cfg(unix)]
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
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

    #[test]
    fn the_json_server_is_replaced_and_every_other_setting_kept() {
        let text = r#"{"theme": "dark", "mcpServers": {"other": {"command": "x.exe"},
            "ketchup": {"command": "C:\\Supervisor\\Kecup\\ketchup-app.exe", "args": []}}}"#;
        let written = with_json_server(text, r"C:\Programs\Ketchup\Ketchup.exe", false).unwrap();
        let config: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(config["theme"], "dark");
        assert_eq!(config["mcpServers"]["other"]["command"], "x.exe");
        assert_eq!(
            config["mcpServers"]["ketchup"]["command"],
            r"C:\Programs\Ketchup\Ketchup.exe"
        );
        assert_eq!(config["mcpServers"]["ketchup"]["args"][0], "--mcp");
        assert!(config["mcpServers"]["ketchup"].get("type").is_none());
        let fresh = with_json_server("", "k.exe", true).unwrap();
        let config: serde_json::Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!(config["mcpServers"]["ketchup"]["type"], "stdio");
        assert!(with_json_server("[1, 2]", "k.exe", false).is_err());
        assert!(with_json_server("{not json", "k.exe", false).is_err());
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

    #[test]
    fn the_codex_server_command_is_replaced_and_the_rest_kept() {
        let toml = "model = \"gpt\"\n\n[mcp_servers.ketchup]\ncommand = 'C:\\old\\ketchup-app.exe'\n\n[mcp_servers.ketchup.env]\nA = \"1\"\n\n[mcp_servers.other]\ncommand = \"other.exe\"\n";
        let written = with_codex_server(toml, r"C:\Programs\Ketchup\Ketchup.exe").unwrap();
        assert_eq!(
            codex_command(&written).as_deref(),
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
            codex_command(&quoted).as_deref(),
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
                codex_command(form).as_deref(),
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
                codex_command(&written).as_deref(),
                Some(r"C:\new\Ketchup.exe")
            );
        }
        assert!(with_codex_server("[mcp_servers.ketchup\ncommand = 1", "k.exe").is_err());
        assert!(with_codex_server("mcp_servers = 5\n", "k.exe").is_err());
    }
}
