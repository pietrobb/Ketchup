//! Kečup Viewer: an offline, read-only viewer of `.ketchup-view` packages.
//! The package is never edited; hiding, isolating and the camera are view state.
// An error is carried, never dropped (review 2026-11-02 TST-121).
#![deny(clippy::map_err_ignore)]

pub mod app;
pub mod camera;
#[cfg(not(target_arch = "wasm32"))]
pub mod desktop;
pub mod dimensions;
pub mod gpu;
pub mod guides;
pub mod measure;
pub mod model;
pub mod preferences;
pub mod review;
pub mod share;
pub mod text;

#[cfg(test)]
mod gpu_tests;
#[cfg(test)]
mod measure_tests;
#[cfg(all(test, windows, target_feature = "crt-static"))]
mod windows_runtime_tests;
