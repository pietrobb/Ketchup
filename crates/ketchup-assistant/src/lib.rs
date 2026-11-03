//! The typed CAD edit program an AI client sends over MCP (`sidecar`), its generated operation catalog (`catalog`) and the plugin gateway that hands plugin programs to the same path (`extension`).
#![forbid(unsafe_code)]
// An error is carried, never dropped (review 2026-11-02 TST-121).
#![deny(clippy::let_underscore_must_use, clippy::map_err_ignore)]

pub mod catalog;
pub mod extension;
pub mod request_invalid;
pub mod sidecar;
