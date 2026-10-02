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
#[cfg(test)]
mod tests;
mod tools;

pub use server::serve_stdio;
