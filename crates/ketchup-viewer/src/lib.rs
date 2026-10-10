//! Kečup Viewer: an offline, read-only viewer of `.ketchup-view` packages.
//! The package is never edited; hiding, isolating and the camera are view state.

pub mod app;
pub mod camera;
pub mod gpu;
pub mod model;
pub mod text;

#[cfg(test)]
mod gpu_tests;
