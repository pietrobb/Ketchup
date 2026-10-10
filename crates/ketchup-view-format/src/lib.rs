//! Version 1: KETCHUPVIEW magic (11 bytes), u16 version, u32 JSON length,
//! u32 GLB length, SHA-256 of both payloads (32 bytes), then JSON and GLB.
//! All integers are little-endian. No archives, compression, executable data,
//! external resources, history, or CAD dependencies. GLB is display geometry.

mod container;
mod glb;
mod schema;
mod validation;

pub use container::{MAX_GLB_BYTES, MAX_MANIFEST_BYTES, Package};
pub use glb::geometry::{Geometry, GeometryNode, GeometryPrimitive, PrimitiveTopology};
use ketchup_rejection::{Rejection, RejectionPhase};
pub use schema::*;

pub(crate) fn reject(target: &str, reason: impl Into<String>) -> Rejection {
    Rejection::new("viewer.package.invalid", RejectionPhase::Request)
        .target(target)
        .reason(reason)
        .fix_hint("Re-export a supported .ketchup-view package from Kečup; do not edit its binary contents.")
}
