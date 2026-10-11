//! The typed CAD edit program an AI client sends over MCP (`sidecar`), its generated operation catalog (`catalog`), workflow intents and the plugin gateway.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod extension;
pub mod intent;
pub mod request_invalid;
pub mod sidecar;
