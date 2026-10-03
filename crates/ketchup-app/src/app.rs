//! `KetchupApp` methods grouped by responsibility. Each child module adds one
//! `impl KetchupApp` block next to the free helpers it needs; the state stays
//! in the crate root.

mod assistant;
mod assistant_panel;
mod commands;
mod document;
mod drawing;
mod exact;
mod export;
mod import;
mod local_outliner;
mod mirror;
mod patterns;
#[cfg(test)]
#[path = "program_outliner_tests.rs"]
mod program_outliner_tests;
mod selection;
mod shell;
mod structure;
mod tags;
mod transform;
mod viewport;
mod viewport_paint;

pub(crate) use assistant::*;
pub(crate) use commands::*;
pub(crate) use drawing::*;
pub(crate) use shell::*;
pub(crate) use structure::*;
pub(crate) use transform::*;
pub(crate) use viewport::*;
