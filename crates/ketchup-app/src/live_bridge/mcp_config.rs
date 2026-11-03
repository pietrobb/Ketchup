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

/// Whether a TOML table header names the `ketchup` server or one of its
/// subtables, such as `[mcp_servers.ketchup.env]`.
fn is_codex_server_header(line: &str) -> bool {
    let header = line.trim_matches(|c| c == '[' || c == ']').trim();
    [
        format!("mcp_servers.{SERVER}"),
        format!("mcp_servers.\"{SERVER}\""),
    ]
    .iter()
    .any(|name| header == name || header.starts_with(&format!("{name}.")))
}

/// The `command` of `[mcp_servers.ketchup]` in a Codex config.toml.
pub(crate) fn codex_command(toml: &str) -> Option<String> {
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

/// `toml` with its `ketchup` server, subtables included, replaced by one
/// starting `executable --mcp`; the other lines stay as they were.
pub(crate) fn with_codex_server(toml: &str, executable: &str) -> String {
    let mut kept = String::new();
    let mut in_server = false;
    for line in toml.lines() {
        if line.trim_start().starts_with('[') {
            in_server = is_codex_server_header(line.trim());
        }
        if !in_server {
            kept.push_str(line);
            kept.push('\n');
        }
    }
    let mut text = kept.trim_end().to_owned();
    if !text.is_empty() {
        text.push_str("\n\n");
    }
    // A literal string keeps a Windows path readable; one with a quote in it
    // is escaped instead.
    let command = if executable.contains('\'') {
        format!(
            "\"{}\"",
            executable.replace('\\', "\\\\").replace('"', "\\\"")
        )
    } else {
        format!("'{executable}'")
    };
    text.push_str(&format!(
        "[mcp_servers.{SERVER}]\ncommand = {command}\nargs = [\"--mcp\"]\n"
    ));
    text
}

/// Replaces `file` with `text` through a sibling file, so a client reading it
/// meanwhile sees the old or the new configuration, never half of one.
pub(crate) fn replace_file(file: &Path, text: &str) -> io::Result<()> {
    if let Some(directory) = file.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let mut staged = file.as_os_str().to_owned();
    staged.push(".ketchup-new");
    std::fs::write(&staged, text)?;
    std::fs::rename(&staged, file)
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
    fn the_codex_server_and_its_subtables_are_replaced_and_the_rest_kept() {
        let toml = "model = \"gpt\"\n\n[mcp_servers.ketchup]\ncommand = 'C:\\old\\ketchup-app.exe'\n\n[mcp_servers.ketchup.env]\nA = \"1\"\n\n[mcp_servers.other]\ncommand = \"other.exe\"\n";
        let written = with_codex_server(toml, r"C:\Programs\Ketchup\Ketchup.exe");
        assert_eq!(
            codex_command(&written).as_deref(),
            Some(r"C:\Programs\Ketchup\Ketchup.exe")
        );
        assert!(!written.contains("old"), "{written}");
        assert!(!written.contains("A = "), "{written}");
        assert!(written.starts_with("model = \"gpt\"\n"), "{written}");
        assert!(written.contains("[mcp_servers.other]\ncommand = \"other.exe\"\n"));
        assert_eq!(written.matches("[mcp_servers.ketchup]").count(), 1);
        let quoted = with_codex_server("", r"C:\Peter's\Ketchup.exe");
        assert_eq!(
            codex_command(&quoted).as_deref(),
            Some(r"C:\Peter's\Ketchup.exe")
        );
    }
}
