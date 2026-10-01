//! Geometry vocabulary of a Ketchup model: typed IDs, dimensions, face references, derived
//! rule-output identities, sketches and prismatic checks. It depends on nothing but the
//! tolerance crate, so a change here never waits for the document model to build.
#![forbid(unsafe_code)]

pub mod derived;
pub mod dimension;
pub mod helix;
pub mod id;
pub mod linalg;
pub mod prismatic;
pub mod reference;
pub mod sketch;
pub mod slot;
pub use ketchup_tolerance as tolerance;
