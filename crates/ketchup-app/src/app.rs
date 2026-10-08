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
#[cfg(test)]
#[path = "app/hidden_selection_tests.rs"]
mod hidden_selection_tests;
#[cfg(test)]
#[path = "app/icon_button_tests.rs"]
mod icon_button_tests;
mod import;
#[cfg(test)]
#[path = "app/interactive_house_tests.rs"]
mod interactive_house_tests;
mod local_outliner;
mod mirror;
mod patterns;
#[cfg(test)]
#[path = "program_outliner_tests.rs"]
mod program_outliner_tests;
pub(crate) mod project_drawings;
#[cfg(test)]
#[path = "app/project_drawings_tests.rs"]
mod project_drawings_tests;
#[cfg(test)]
#[path = "app/push_pull_house_tests.rs"]
mod push_pull_house_tests;
pub(crate) mod saved_views;
#[cfg(test)]
#[path = "app/scene_tabs_tests.rs"]
mod scene_tabs_tests;
mod section;
mod selection;
mod shell;
mod structure;
#[cfg(test)]
#[path = "app/tag_toggle_tests.rs"]
mod tag_toggle_tests;
mod tags;
mod takeoff;
#[cfg(test)]
#[path = "app/takeoff_tests.rs"]
mod takeoff_tests;
mod transform;
mod viewport;
mod viewport_paint;

pub(crate) use assistant::*;
pub(crate) use commands::*;
pub(crate) use drawing::*;
#[cfg(test)]
pub(crate) use section::SECTION_COLOR;
pub(crate) use section::{plane_at, section_json};
pub(crate) use shell::*;
pub(crate) use structure::*;
pub(crate) use transform::*;
pub(crate) use viewport::*;
