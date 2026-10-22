//! `KetchupApp` methods grouped by responsibility. Each child module adds one
//! `impl KetchupApp` block next to the free helpers it needs; the state stays
//! in the crate root.

mod cad_program;
mod commands;
mod connect_ai;
pub(crate) mod dimensions_ui;
#[cfg(test)]
#[path = "app/dimensions_ui_tests.rs"]
mod dimensions_ui_tests;
mod document;
pub(crate) mod draft;
mod drawing;
pub(crate) mod drawing_export_activity;
pub(crate) mod drawing_sheet_export;
#[cfg(test)]
#[path = "app/drawing_sheet_export_tests.rs"]
mod drawing_sheet_export_tests;
pub(crate) mod dwg_import;
#[cfg(test)]
#[path = "app/dwg_import_tests.rs"]
mod dwg_import_tests;
mod exact;
pub(crate) mod exact_import_activity;
#[cfg(test)]
#[path = "app/exact_import_activity_tests.rs"]
mod exact_import_activity_tests;
pub(crate) mod exchange_export;
mod export;
mod fea_review;
pub(crate) mod file_activity;
#[cfg(test)]
#[path = "app/hidden_selection_tests.rs"]
mod hidden_selection_tests;
pub(crate) mod hole;
#[cfg(test)]
#[path = "app/icon_button_tests.rs"]
mod icon_button_tests;
mod import;
pub(crate) mod import_file;
#[cfg(test)]
#[path = "app/interactive_house_tests.rs"]
mod interactive_house_tests;
pub(crate) mod joints_ui;
#[cfg(test)]
#[path = "app/joints_ui_tests.rs"]
mod joints_ui_tests;
mod local_outliner;
mod mirror;
mod outliner;
mod patterns;
#[cfg(test)]
#[path = "app/production_export_tests.rs"]
mod production_export_tests;
#[cfg(test)]
#[path = "app/program_isolation_tests.rs"]
mod program_isolation_tests;
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
pub(crate) mod received_comments;
mod save_activity;
pub(crate) mod saved_views;
mod scale_grips;
#[cfg(test)]
#[path = "app/scene_tabs_tests.rs"]
mod scene_tabs_tests;
mod section;
mod selection;
mod shell;
pub(crate) mod spaces_ui;
#[cfg(test)]
#[path = "app/spaces_ui_tests.rs"]
mod spaces_ui_tests;
mod startup_arguments;
mod structure;
#[cfg(test)]
#[path = "app/tag_toggle_tests.rs"]
mod tag_toggle_tests;
mod tags;
mod takeoff;
#[cfg(test)]
#[path = "app/takeoff_tests.rs"]
mod takeoff_tests;
mod thread_display;
#[cfg(test)]
#[path = "app/thread_display_tests.rs"]
mod thread_display_tests;
pub(crate) mod tool_gesture;
mod transform;
pub(crate) mod underlay;
pub(crate) mod underlay_fit_tool;
pub(crate) mod underlay_snapping;
#[cfg(test)]
#[path = "app/underlay_snapping_tests.rs"]
mod underlay_snapping_tests;
#[cfg(test)]
#[path = "app/underlay_tests.rs"]
mod underlay_tests;
pub(crate) mod underlay_transform;
#[cfg(test)]
#[path = "app/underlay_transform_tests.rs"]
mod underlay_transform_tests;
mod viewer_export;
mod viewer_export_activity;
pub(crate) mod viewer_settings;
#[cfg(test)]
#[path = "app/viewer_settings_tests.rs"]
mod viewer_settings_tests;
mod viewport;
mod viewport_paint;

pub(crate) use commands::*;
pub(crate) use document::SaveFailure;
pub(crate) use drawing::*;
pub(crate) use scale_grips::{ScaleInput, axis_drives};
#[cfg(test)]
pub(crate) use section::SECTION_COLOR;
pub(crate) use section::{plane_at, section_json};
pub(crate) use shell::*;
pub(crate) use structure::*;
pub(crate) use transform::*;
pub(crate) use viewport::*;
