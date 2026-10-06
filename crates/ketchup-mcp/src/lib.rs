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

pub use server::serve_stdio;

/// Host response budget includes program planning, publication and the bounded exact check.
/// The MCP client adds a delivery margin rather than timing out before the host.
pub const PROGRAM_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(45);
/// Opening asks the user in the window first, then loads a possibly large document.
pub const OPEN_RESPONSE_WAIT: std::time::Duration = std::time::Duration::from_secs(120);
