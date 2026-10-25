//! Model Context Protocol server for Kečup.
//!
//! `ketchup-app --mcp` speaks MCP (JSON-RPC 2.0, one message per line) on
//! stdin/stdout, so any MCP client (Claude Code, Claude Desktop, Codex, Cursor,
//! ...) can drive the Kečup window the user has open. It is a thin client of
//! the window's live bridge: it finds windows in the per-user discovery
//! directory, attaches through the window's local attach endpoint and forwards
//! tool calls as bridge requests. The window validates everything itself.
// The tool schemas are single `json!` literals deeper than the default limit of 128.
#![recursion_limit = "256"]

mod bridge;
mod discovery;
mod docs;
pub mod local_auth;
pub mod path_policy;
mod schema;
mod server;
pub mod stage;
#[cfg(test)]
mod tests;
mod tools;

pub use bridge::MAX_UNAUTHENTICATED_REQUEST_BYTES;
pub use server::serve_stdio;

/// Host response budget includes program planning, publication and the bounded exact check.
/// The MCP client adds a delivery margin rather than timing out before the host.
pub const PROGRAM_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(45);
/// Opening asks the user in the window first, then loads a possibly large document.
pub const OPEN_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
/// An import is given [`OPEN_RESPONSE_WAIT`] plus this per started MiB of its file,
/// as reading it (a DWG converted or read as an underlay, a STEP) grows with its bytes.
const IMPORT_WAIT_PER_MIB: std::time::Duration = std::time::Duration::from_secs(4);
/// No import is given longer.
const IMPORT_WAIT_MOST: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// How long the window waits for importing the file at `path`; a file it cannot
/// read is refused at once and adds nothing. A path the window refuses
/// ([`path_policy::local_absolute_path`]) is not looked up at all: asking for
/// the size of `\\server\share\x.dwg` would already sign in to that server.
#[must_use]
pub fn import_response_wait(path: &str) -> std::time::Duration {
    if !path_policy::local_absolute_path(path) {
        return OPEN_RESPONSE_WAIT;
    }
    let bytes = std::fs::metadata(path).map_or(0, |metadata| metadata.len());
    let mib = u32::try_from(bytes.div_ceil(1 << 20)).unwrap_or(u32::MAX);
    OPEN_RESPONSE_WAIT
        .saturating_add(IMPORT_WAIT_PER_MIB.saturating_mul(mib))
        .min(IMPORT_WAIT_MOST)
}
/// Every request the window answers within this, whatever the request.
pub const DEFAULT_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);
/// The window's last frame on a connection it ends because another client
/// attached: id of the request in flight, or 0 when the client was idle.
pub const SUPERSEDED_BY_ANOTHER_CLIENT: &str = "superseded_by_another_client";

/// How long the window waits for a verified edit of `timeout_ms`: the job
/// itself, plus queueing and publishing on the UI thread, never below the
/// default wait. The MCP client waits a delivery margin longer than this.
#[must_use]
pub fn apply_and_verify_response_wait(timeout_ms: u64) -> std::time::Duration {
    (std::time::Duration::from_millis(timeout_ms) + std::time::Duration::from_secs(15))
        .max(DEFAULT_RESPONSE_WAIT)
}

/// How long the user may take to allow a plugin in the window.
pub const PLUGIN_CONSENT_WAIT: std::time::Duration = std::time::Duration::from_secs(60);

/// How long the window waits for a plugin run: the user's consent, the plugin
/// process of `timeout_ms`, then publishing its program like a verified edit
/// of `publish_ms`.
#[must_use]
pub fn run_plugin_response_wait(timeout_ms: u64, publish_ms: u64) -> std::time::Duration {
    apply_and_verify_response_wait(timeout_ms.saturating_add(publish_ms)) + PLUGIN_CONSENT_WAIT
}
