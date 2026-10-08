//! Model Context Protocol server for Kečup.
//!
//! `ketchup-app --mcp` speaks MCP (JSON-RPC 2.0, one message per line) on
//! stdin/stdout, so any MCP client (Claude Code, Claude Desktop, Codex, Cursor,
//! ...) can drive the Kečup window the user has open. It is a thin client of
//! the window's live bridge: it finds windows in the per-user discovery
//! directory, attaches through the window's local attach endpoint and forwards
//! tool calls as bridge requests. The window validates everything itself.

mod bridge;
mod discovery;
mod docs;
pub mod local_auth;
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
/// Every request the window answers within this, whatever the request.
pub const DEFAULT_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// How long the window waits for a verified edit of `timeout_ms`: the job
/// itself, plus queueing and publishing on the UI thread, never below the
/// default wait. The MCP client waits a delivery margin longer than this.
#[must_use]
pub fn apply_and_verify_response_wait(timeout_ms: u64) -> std::time::Duration {
    (std::time::Duration::from_millis(timeout_ms) + std::time::Duration::from_secs(15))
        .max(DEFAULT_RESPONSE_WAIT)
}
