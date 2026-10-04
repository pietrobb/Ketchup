//! Project drawings of what is visible: floor plan, sections and elevations on
//! one ISO A sheet with a frame and a title block, written as a vector PDF and
//! drawn from the exact solids. Hidden layers are left out, so turning off the
//! concept layer draws the construction. The title block and the sheet format
//! are kept in the document.

use crate::*;
use ketchup_manufacturing::project_drawings::{
    DrawingSolid, ProjectDrawingError, ProjectSheet, ProjectSheetOptions, ProjectView,
    default_plan_cut_z, project_sheet,
};
use ketchup_manufacturing::sheet_pdf::PdfInfo;
use ketchup_manufacturing::title_block::{SheetFormat, SheetSettings, TitleField};

/// Colour of a solid with no colour of its own.
const DEFAULT_SOLID_COLOR: [u8; 3] = [190, 190, 190];
const SHEET_NAMESPACE: &str = "org.ketchup.drawings";
const SHEET_SETTINGS_PATH: &str = "sheet-v1.json";

/// Why the project drawings could not be made or written.
#[derive(Debug)]
pub(crate) enum ProjectDrawingsError {
    Drawing(ProjectDrawingError),
    Write(std::io::Error),
}

/// Today's date as d. m. yyyy (UTC).
fn today() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() / 86_400);
    let days = i64::try_from(days).unwrap_or(0) + 719_468;
    // Civil date from days since 0000-03-01 (H. Hinnant's algorithm).
    let era = days.div_euclid(146_097);
    let day_of_era = days.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{day}. {month}. {year}")
}

impl KetchupApp {
    /// Every visible occurrence with a published exact solid, in world millimetres.
    fn visible_drawing_solids(&self, snapshot: &Snapshot) -> Vec<DrawingSolid> {
        let exact = self.exact.results.render_by_definition(snapshot);
        snapshot
            .scene_query()
            .into_iter()
            .filter(|occurrence| occurrence.visible)
            .filter_map(|occurrence| {
                let package = exact.get(&occurrence.definition_id)?;
                let vertices = package.vertices();
                let world = |index: u32| {
                    let vertex = vertices.get(usize::try_from(index).ok()?)?;
                    Some(occurrence.transform.transform_point(vertex.position_mm))
                };
                let triangles = package
                    .triangles()
                    .iter()
                    .filter_map(|triangle| {
                        let [a, b, c] = triangle.vertex_indices;
                        Some([world(a)?, world(b)?, world(c)?])
                    })
                    .collect();
                Some(DrawingSolid {
                    name: occurrence.occurrence_name.clone(),
                    color: occurrence.color.unwrap_or(DEFAULT_SOLID_COLOR),
                    triangles,
                })
            })
            .collect()
    }

    /// The document name: its file name, else the program's.
    fn drawing_document_name(&self) -> String {
        self.file
            .path
            .as_deref()
            .and_then(Path::file_stem)
            .and_then(|stem| stem.to_str())
            .map_or_else(
                || {
                    self.document
                        .current_rule_program()
                        .map_or_else(|| "Kečup".to_owned(), |program| program.file_name.clone())
                },
                str::to_owned,
            )
    }

    /// The sheet settings kept in the document, as stored.
    pub(crate) fn stored_sheet_settings(&self) -> SheetSettings {
        self.file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == SHEET_NAMESPACE && entry.path() == SHEET_SETTINGS_PATH
            })
            .and_then(|entry| serde_json::from_slice(entry.bytes()).ok())
            .unwrap_or_default()
    }

    /// The stored settings with the project, sheet title and date filled in
    /// where the user left them empty.
    pub(crate) fn sheet_settings(&self) -> SheetSettings {
        let mut settings = self.stored_sheet_settings();
        let defaults = [
            (TitleField::Project, self.drawing_document_name()),
            (
                TitleField::Drawing,
                self.catalog.text("drawings-default-sheet"),
            ),
            (TitleField::Date, today()),
        ];
        for (field, value) in defaults {
            let entry = settings.title_block.entry(field).or_default();
            if entry.trim().is_empty() {
                *entry = value;
            }
        }
        settings
    }

    /// Keeps `settings` in the document; empty fields are dropped.
    pub(crate) fn store_sheet_settings(&mut self, mut settings: SheetSettings) {
        settings.title_block.retain(|field, value| {
            !value.trim().is_empty() && TitleField::EDITABLE.contains(field)
        });
        if settings == self.stored_sheet_settings() {
            return;
        }
        let Ok(bytes) = serde_json::to_vec(&settings) else {
            return;
        };
        let Ok(entry) = ketchup_model::persistence::ExtensionEntry::new(
            SHEET_NAMESPACE,
            SHEET_SETTINGS_PATH,
            false,
            bytes,
        ) else {
            return;
        };
        self.file.container_data.set_extension(entry);
        self.drawings.unsaved = true;
    }

    fn title_labels(&self) -> BTreeMap<TitleField, String> {
        TitleField::EDITABLE
            .into_iter()
            .chain([TitleField::Scale, TitleField::Format])
            .map(|field| {
                (
                    field,
                    self.catalog.text(&format!("title-field-{}", field.key())),
                )
            })
            .collect()
    }

    /// The sheet of the visible exact solids with the document's title block.
    pub(crate) fn project_drawings(&self) -> Result<ProjectSheet, ProjectDrawingError> {
        let snapshot = self.document.current();
        let solids = self.visible_drawing_solids(&snapshot);
        let cut = default_plan_cut_z(&solids).ok_or(ProjectDrawingError::Empty)?;
        let view_titles = ProjectView::ALL
            .into_iter()
            .map(|view| {
                let title = match view {
                    ProjectView::Plan => self.catalog.format(
                        "drawings-view-plan",
                        &BTreeMap::from([("height", format!("{cut:.0}"))]),
                    ),
                    ProjectView::LongitudinalSection => {
                        self.catalog.text("drawings-view-longitudinal")
                    }
                    ProjectView::CrossSection => self.catalog.text("drawings-view-cross"),
                    ProjectView::Front => self.catalog.text("drawings-view-front"),
                    ProjectView::Back => self.catalog.text("drawings-view-back"),
                    ProjectView::Left => self.catalog.text("drawings-view-left"),
                    ProjectView::Right => self.catalog.text("drawings-view-right"),
                };
                (view, title)
            })
            .collect();
        let settings = self.sheet_settings();
        project_sheet(
            &solids,
            &ProjectSheetOptions {
                view_titles,
                plan_cut_z_mm: Some(cut),
                format: settings.format,
                title_labels: self.title_labels(),
                title_block: settings.title_block,
            },
        )
    }

    pub(crate) fn project_drawings_error_text(&self, error: &ProjectDrawingsError) -> String {
        match error {
            ProjectDrawingsError::Drawing(ProjectDrawingError::Empty) => {
                self.catalog.text("drawings-unavailable")
            }
            ProjectDrawingsError::Write(error) => error.to_string(),
        }
    }

    /// Writes the project drawings of what is visible as a one-sheet PDF.
    pub(crate) fn write_project_drawings(
        &self,
        path: &Path,
    ) -> Result<ProjectSheet, ProjectDrawingsError> {
        let sheet = self
            .project_drawings()
            .map_err(ProjectDrawingsError::Drawing)?;
        let block = &self.sheet_settings().title_block;
        let field = |field| block.get(&field).cloned().unwrap_or_default();
        let info = PdfInfo {
            title: format!(
                "{} – {}",
                field(TitleField::Project),
                field(TitleField::Drawing)
            ),
            author: field(TitleField::Author),
            subject: field(TitleField::Stage),
        };
        std::fs::write(path, sheet.pdf(&info)).map_err(ProjectDrawingsError::Write)?;
        Ok(sheet)
    }

    /// Writes the project drawings as PDF and reports it in the digest.
    pub fn export_project_drawings_to(&mut self, path: &Path) -> bool {
        let result = self.write_project_drawings(path);
        let path = path.display().to_string();
        self.digest = match &result {
            Ok(sheet) => self.catalog.format(
                "digest-exported-project-drawings",
                &BTreeMap::from([
                    ("path", path),
                    ("scale", sheet.scale.to_string()),
                    ("format", sheet.format.name().to_owned()),
                ]),
            ),
            Err(error) => self.catalog.format(
                "error-export-project-drawings",
                &BTreeMap::from([
                    ("path", path),
                    ("reason", self.project_drawings_error_text(error)),
                ]),
            ),
        };
        result.is_ok()
    }

    pub(crate) fn open_project_drawings_window(&mut self) {
        self.drawings.editing = self.sheet_settings();
        self.drawings.open = true;
    }

    pub(crate) fn show_project_drawings_window(&mut self, context: &egui::Context) {
        if !self.drawings.open {
            return;
        }
        let mut open = true;
        let mut export = false;
        let mut editing = std::mem::take(&mut self.drawings.editing);
        egui::Window::new(self.catalog.text("window-project-drawings"))
            .open(&mut open)
            .default_width(460.0)
            .show(context, |ui| {
                ui.label(self.catalog.text("drawings-window-hint"));
                egui::Grid::new("project-drawings-title-block")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(self.catalog.text("drawings-format"));
                        let name = |format: Option<SheetFormat>| {
                            format.map_or_else(
                                || self.catalog.text("drawings-format-auto"),
                                |format| format.name().to_owned(),
                            )
                        };
                        egui::ComboBox::from_id_salt("project-drawings-format")
                            .selected_text(name(editing.format))
                            .show_ui(ui, |ui| {
                                for format in std::iter::once(None)
                                    .chain(SheetFormat::ALL.into_iter().map(Some))
                                {
                                    ui.selectable_value(&mut editing.format, format, name(format));
                                }
                            });
                        ui.end_row();
                        for field in TitleField::EDITABLE {
                            ui.label(self.catalog.text(&format!("title-field-{}", field.key())));
                            ui.add(
                                egui::TextEdit::singleline(
                                    editing.title_block.entry(field).or_default(),
                                )
                                .desired_width(300.0),
                            );
                            ui.end_row();
                        }
                    });
                ui.separator();
                if ui
                    .button(self.catalog.text("drawings-export-pdf"))
                    .clicked()
                {
                    export = true;
                }
            });
        // Merely opening the window with the filled-in defaults changes nothing.
        if editing != self.sheet_settings() {
            self.store_sheet_settings(editing.clone());
        }
        self.drawings.editing = editing;
        if !open {
            self.drawings.open = false;
        }
        if export && let Some(path) = self.choose_export_path("pdf") {
            self.export_project_drawings_to(&path);
        }
    }
}
