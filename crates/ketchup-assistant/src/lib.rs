//! The typed CAD edit program an AI client sends over MCP (`sidecar`), its generated operation catalog (`catalog`) and the plugin gateway that hands plugin programs to the same path (`extension`).
#![forbid(unsafe_code)]

pub mod catalog;
pub mod extension;
pub mod request_invalid;
pub mod sidecar;
