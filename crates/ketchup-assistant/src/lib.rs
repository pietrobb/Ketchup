//! AI assistant surface of Ketchup: the typed CAD edit program an agent sends (`sidecar`), its generated operation catalog (`catalog`), workflow intents and the plugin gateway.
#![forbid(unsafe_code)]

pub mod catalog;
pub mod extension;
pub mod intent;
pub mod sidecar;
