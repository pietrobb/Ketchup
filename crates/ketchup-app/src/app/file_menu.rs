//! The File menu (#2689): documents and Open Recent on top, the formats in
//! submenus whose items carry short names, as the submenu already says what
//! they do. Screen readers and the command search keep the full names.

use crate::recent_files::Presence;
use crate::*;

const IMPORTS: [(AppCommand, &str); 7] = [
    (AppCommand::ImportMeshStl, "file-short-stl"),
    (AppCommand::ImportDrawingDxf, "file-short-dxf-2d"),
    (AppCommand::ImportDrawingUnderlay, "file-short-dxf-underlay"),
    (AppCommand::ImportExactStep, "file-short-step"),
    (AppCommand::ImportExactIges, "file-short-iges"),
    (AppCommand::ImportSketchupScene, "file-short-sketchup"),
    (AppCommand::ImportBlenderGlb, "file-short-glb"),
];

const EXPORTS: [(AppCommand, &str); 7] = [
    (AppCommand::ExportExactStep, "file-short-step"),
    (AppCommand::ExportExactIges, "file-short-iges"),
    (AppCommand::ExportMeshStl, "file-short-stl"),
    (AppCommand::ExportPrintThreeMf, "file-short-3mf"),
    (AppCommand::ExportBlenderGlb, "file-short-glb"),
    (AppCommand::ExportSketchupDae, "file-short-dae"),
    (AppCommand::ExportDrawingDxf, "file-short-dxf-profiles"),
];

const VIEWER: [(AppCommand, &str); 2] = [
    (AppCommand::ExportViewer, "file-short-viewer-export"),
    (AppCommand::OpenViewerComments, "file-short-viewer-comments"),
];

const MANUFACTURING: [(AppCommand, &str); 5] = [
    (
        AppCommand::ExportGeneralFabrication,
        "file-short-general-fabrication",
    ),
    (AppCommand::ExportWeldmentCutList, "file-short-cut-list"),
    (
        AppCommand::ExportSheetMetalManufacturing,
        "file-short-sheet-metal",
    ),
    (AppCommand::ExportHomagMpr, "file-short-homag-mpr"),
    (AppCommand::ReviewCamExport, "file-short-cam"),
];

const ANALYSIS: [(AppCommand, &str); 2] = [
    (AppCommand::ReviewStaticFea, "file-short-static-fea"),
    (AppCommand::ReviewLocalPdm, "file-short-local-pdm"),
];

impl KetchupApp {
    pub(crate) fn file_menu(&mut self, ui: &mut egui::Ui) {
        self.menu_command(ui, AppCommand::New);
        self.menu_command(ui, AppCommand::Open);
        ui.menu_button(self.catalog.text("file-open-recent"), |ui| {
            self.open_recent_menu(ui);
        });
        self.menu_command(ui, AppCommand::Save);
        self.menu_command(ui, AppCommand::SaveAs);
        ui.separator();
        self.short_command_submenu(ui, "menu-file-import", &IMPORTS);
        self.short_command_submenu(ui, "menu-file-export", &EXPORTS);
        self.menu_command(ui, AppCommand::ExportProjectDrawings);
        self.short_command_submenu(ui, "menu-file-viewer", &VIEWER);
        ui.menu_button(self.catalog.text("menu-file-manufacturing"), |ui| {
            for (command, key) in MANUFACTURING {
                self.menu_command_as(ui, command, key);
            }
            ui.separator();
            self.menu_command_as(ui, AppCommand::ExportHundeggerBtlx, "file-short-btlx");
            self.btlx_options_menu(ui);
        });
    }

    /// Tools > Analysis: the checks that read the model without changing it.
    pub(crate) fn analysis_menu(&mut self, ui: &mut egui::Ui) {
        self.short_command_submenu(ui, "menu-tools-analysis", &ANALYSIS);
    }

    fn short_command_submenu(
        &mut self,
        ui: &mut egui::Ui,
        title_key: &str,
        commands: &[(AppCommand, &str)],
    ) {
        ui.menu_button(self.catalog.text(title_key), |ui| {
            for &(command, key) in commands {
                self.menu_command_as(ui, command, key);
            }
        });
    }

    fn btlx_options_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(self.catalog.text("file-short-btlx-options"), |ui| {
            ui.selectable_value(
                &mut self.btlx_profile_strategy,
                BtlxProfileStrategy::PortableFreeContour,
                self.catalog.text("file-export-btlx-portable-contour"),
            );
            ui.selectable_value(
                &mut self.btlx_profile_strategy,
                BtlxProfileStrategy::EdgeSawCutsThenMillContour,
                self.catalog.text("file-export-btlx-saw-then-mill"),
            );
            ui.add_enabled(
                self.btlx_profile_strategy == BtlxProfileStrategy::EdgeSawCutsThenMillContour,
                egui::Slider::new(&mut self.btlx_intermediate_saw_cuts, 0..=32)
                    .text(self.catalog.text("file-export-btlx-intermediate-saw-cuts")),
            );
        });
    }

    /// The documents of Open Recent, newest first: the name to read, the
    /// whole path on hover and for screen readers. A document no longer on
    /// disk stays listed, greyed, until it is clicked; the disk is asked once
    /// when the menu opens, off the UI thread (review 2026-11-04 UI-141).
    fn open_recent_menu(&mut self, ui: &mut egui::Ui) {
        self.window
            .recent
            .shown(ui.ctx().cumulative_pass_nr(), ui.ctx());
        if self.window.recent.files().is_empty() {
            ui.add_enabled(
                false,
                egui::Button::new(self.catalog.text("file-recent-empty")),
            );
            return;
        }
        let mut chosen = None;
        for path in self.window.recent.files() {
            let full = path.display().to_string();
            let name = path
                .file_name()
                .map_or_else(|| full.clone(), |name| name.to_string_lossy().into_owned());
            let missing = match self.window.recent.on_disk(path) {
                None | Some(Presence::Here) => None,
                Some(Presence::Gone) => Some("file-recent-missing-name"),
                Some(Presence::Unavailable) => Some("file-recent-unavailable-name"),
            };
            let (text, accessible) = match missing {
                None => (egui::RichText::new(name), full),
                Some(key) => (
                    egui::RichText::new(name).color(ui.visuals().weak_text_color()),
                    self.catalog.format(key, &BTreeMap::from([("path", full)])),
                ),
            };
            let response = ui.button(text).on_hover_text(&accessible);
            name_widget(&response, true, &accessible);
            if response.clicked() {
                chosen = Some(path.clone());
            }
        }
        ui.separator();
        if ui.button(self.catalog.text("file-recent-clear")).clicked() {
            let result = self.window.recent.clear();
            self.report_recent_files(result);
            ui.close();
        }
        if let Some(path) = chosen {
            ui.close();
            self.open_recent(&path);
        }
    }

    /// Open a document of Open Recent as Open would, after the same question
    /// about unsaved changes. One that is gone is said so and dropped; one
    /// whose folder does not answer, as on a server that is offline, is
    /// said so and kept for when it is back (review 2026-11-04 P3 GUI). The
    /// disk is asked off the window; while it does not answer, the document
    /// opens when it does (review 2026-11-04 r19 UI-161).
    pub(crate) fn open_recent(&mut self, path: &Path) {
        let context = self.file.activity.context.clone();
        match self.window.recent.pick(path, context, self.file.generation) {
            Some(presence) => self.open_recent_found(path, presence),
            None => {
                self.digest = self.catalog.format(
                    "digest-recent-checking",
                    &BTreeMap::from([("path", path.display().to_string())]),
                );
            }
        }
    }

    /// The answer for a picked recent document that came after the frame it
    /// was picked in. Once New, Open or a client brought another document to
    /// the window, the answer opens nothing (review 2026-11-06 UI-180).
    pub(crate) fn poll_recent_pick(&mut self) {
        if let Some((path, presence, generation)) = self.window.recent.picked(None)
            && generation == self.file.generation
        {
            self.open_recent_found(&path, presence);
        }
    }

    fn open_recent_found(&mut self, path: &Path, presence: Presence) {
        let shown = BTreeMap::from([("path", path.display().to_string())]);
        match presence {
            Presence::Here => {}
            Presence::Gone => {
                let result = self.window.recent.remove(path);
                self.digest = self.catalog.format("digest-recent-missing", &shown);
                self.report_recent_files(result);
                return;
            }
            Presence::Unavailable => {
                self.digest = self.catalog.format("digest-recent-unavailable", &shown);
                return;
            }
        }
        if !self.defer_open_of(path) && self.confirm_discard_if_dirty() {
            self.open_document_with_discard(path, true);
        }
    }

    /// Put `path` first in Open Recent: it was just opened, or saved under a
    /// new name.
    pub(crate) fn remember_recent_file(&mut self, path: &Path) {
        let result = self.window.recent.add(path);
        self.report_recent_files(result);
    }

    /// A list that could not be stored is said after what was just done.
    fn report_recent_files(&mut self, result: std::io::Result<()>) {
        if let Err(error) = result {
            let notice = self.catalog.format(
                "digest-recent-not-remembered",
                &BTreeMap::from([("error", error.to_string())]),
            );
            self.digest = if self.digest.is_empty() {
                notice
            } else {
                format!("{} {notice}", self.digest)
            };
        }
    }
}
