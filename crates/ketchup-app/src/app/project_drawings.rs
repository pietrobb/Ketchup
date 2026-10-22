//! Project drawings of what is visible: floor plan, sections and elevations on
//! one ISO A sheet with a frame and a title block, written as a vector PDF and
//! drawn from the exact solids. Hidden layers are left out, so turning off the
//! concept layer draws the construction. The title block and the sheet format
//! are kept in the document.

use crate::app::drawing_export_activity::DrawingExport;
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::*;
use ketchup_application::threads::{CosmeticThreadBodyError, ThreadCylinder, cosmetic_thread_body};
use ketchup_manufacturing::project_drawings::{
    DrawingSolid, DrawingThread, ProjectDrawingError, ProjectSheet, ProjectSheetOptions,
    ProjectView, SCALES, default_plan_cut_z, project_sheet,
};
use ketchup_manufacturing::sheet_pdf::PdfInfo;
use ketchup_manufacturing::title_block::{
    DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM, SheetFormat, SheetSettings, TitleField,
};

/// Colour of a solid with no colour of its own.
const DEFAULT_SOLID_COLOR: [u8; 3] = [190, 190, 190];
/// The highest plan cut the window offers: above a storey, a plan stops being a floor plan.
const MAX_PLAN_CUT_ABOVE_FLOOR_MM: u32 = 10_000;
const SHEET_NAMESPACE: &str = "org.ketchup.drawings";
const SHEET_SETTINGS_PATH: &str = "sheet-v1.json";

/// Why the project drawings could not be made or written.
#[derive(Debug)]
pub(crate) enum ProjectDrawingsError {
    Drawing(ProjectDrawingError),
    /// Visible parts with no current exact solid, by name: the sheet would
    /// leave them out.
    NotEvaluated(Vec<String>),
    /// The program is still being evaluated: the threads of its parts are
    /// not known yet, and the sheet would leave them out.
    ProgramPlanning,
    /// The title block and format stored in the document do not read; they
    /// are left as they are rather than replaced by the defaults.
    SettingsUnreadable(serde_json::Error),
    /// The named part's modelled threads could not be made cosmetic for the
    /// sheet, which would otherwise draw every edge of their helices.
    CosmeticThreads(String, CosmeticThreadBodyError),
    Write(std::io::Error),
}

/// What a client asks of one project sheet beyond the settings the document
/// keeps: the plan alone, and a fixed scale 1:N.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SheetRequest {
    pub(crate) plan_only: bool,
    pub(crate) scale: Option<u32>,
}

/// The visible solids, the sheet options and the PDF's document information:
/// everything the project sheet is drawn from.
pub(crate) struct ProjectDrawingsJob {
    solids: Vec<DrawingSolid>,
    options: ProjectSheetOptions,
    info: PdfInfo,
}

impl ProjectDrawingsJob {
    /// Projects the solids and removes the hidden lines.
    fn sheet(&self) -> Result<ProjectSheet, ProjectDrawingsError> {
        project_sheet(&self.solids, &self.options).map_err(ProjectDrawingsError::Drawing)
    }

    /// The sheet and its PDF.
    pub(crate) fn render(&self) -> Result<(ProjectSheet, Vec<u8>), ProjectDrawingsError> {
        let sheet = self.sheet()?;
        let pdf = sheet.pdf(&self.info);
        Ok((sheet, pdf))
    }
}

/// Writes `pdf` to `path` through a temporary file and a rename: a failed
/// write never leaves a half-written sheet, and a link at `path` is replaced,
/// not followed.
pub(crate) fn publish_project_drawings(
    path: &Path,
    pdf: &[u8],
) -> Result<(), ProjectDrawingsError> {
    let expected = crate::export_bundle::export_target_sha256(path)
        .map_err(|error| ProjectDrawingsError::Write(std::io::Error::other(error)))?;
    crate::export_bundle::write_export_artifact_if_unchanged(path, pdf, expected.as_deref())
        .map_err(|error| ProjectDrawingsError::Write(std::io::Error::other(error)))
}

/// The first few names of parts left out of the sheet, then how many more.
pub(crate) fn missing_parts_list(parts: &[String]) -> String {
    const SHOWN: usize = 5;
    let mut list = parts
        .iter()
        .take(SHOWN)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if parts.len() > SHOWN {
        list.push_str(&format!(" … (+{})", parts.len() - SHOWN));
    }
    list
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

/// `thread` of a definition where `transform` places it in the world; its
/// sizes follow the placement's stretch along its axis.
fn placed_thread(transform: &Transform, thread: &ThreadCylinder) -> Option<DrawingThread> {
    let along = transform.transform_vector(thread.direction);
    let stretch = ketchup_geometry::linalg::length(along);
    Some(DrawingThread {
        start_mm: transform.transform_point(thread.start_mm),
        direction: ketchup_geometry::linalg::normalize(along)?,
        length_mm: thread.length_mm * stretch,
        major_diameter_mm: thread.nominal_diameter_mm * stretch,
        root_diameter_mm: thread.root_diameter_mm() * stretch,
        internal: thread.internal,
    })
}

impl KetchupApp {
    /// Every visible body of every visible occurrence as its exact solid, in
    /// world millimetres, the part's threads on its first; a visible part with
    /// a body but no current exact solid is an error naming it, never a sheet
    /// that silently leaves it out.
    pub(crate) fn visible_drawing_solids(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<DrawingSolid>, ProjectDrawingsError> {
        let mut threads = BTreeMap::<DefinitionId, Vec<ThreadCylinder>>::new();
        for thread in self
            .document_threads(snapshot)
            .ok_or(ProjectDrawingsError::ProgramPlanning)?
        {
            threads
                .entry(thread.definition_id)
                .or_default()
                .push(thread);
        }
        let exact = self
            .exact
            .results
            .body_values(snapshot)
            .unwrap_or_default()
            .into_iter()
            .map(|(key, package)| ((key.definition_id, key.body_id), package))
            .collect::<BTreeMap<_, _>>();
        let mut missing = Vec::new();
        let mut drawn = Vec::new();
        for occurrence in snapshot.scene_query() {
            let Some(definition) = snapshot
                .definition(occurrence.definition_id)
                .filter(|_| occurrence.visible)
            else {
                continue;
            };
            let Ok(terminal_bodies) = ketchup_model::exact_product::exact_body_terminal_features(
                snapshot,
                occurrence.definition_id,
            ) else {
                missing.push(occurrence.occurrence_name.clone());
                continue;
            };
            let terminal_bodies = terminal_bodies.into_keys().collect::<Vec<_>>();
            let mut packages = Vec::new();
            for body_id in terminal_bodies {
                if !definition.body(body_id).is_some_and(|body| body.visible()) {
                    continue;
                }
                match exact.get(&(occurrence.definition_id, body_id)) {
                    Some(package) => packages.push(*package),
                    None => {
                        missing.push(occurrence.occurrence_name.clone());
                        break;
                    }
                }
            }
            let placed = threads
                .get(&occurrence.definition_id)
                .into_iter()
                .flatten()
                .filter_map(|thread| placed_thread(&occurrence.transform, thread))
                .collect::<Vec<_>>();
            drawn.extend(packages.into_iter().enumerate().map(|(index, package)| {
                let threads = if index == 0 {
                    placed.clone()
                } else {
                    Vec::new()
                };
                (occurrence.clone(), package, threads)
            }));
        }
        if !missing.is_empty() {
            return Err(ProjectDrawingsError::NotEvaluated(missing));
        }
        let executable = self.exact.worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|candidate| candidate.is_file())
        });
        let mut solids = Vec::with_capacity(drawn.len());
        for (occurrence, package, threads) in drawn {
            // Modelled threads are drawn by ISO 6410 like cosmetic ones: from
            // the body with them cosmetic, made only now, never the helix.
            let package: &ExactBodyPackage = package;
            let cosmetic = match package {
                ExactBodyPackage::Graph(package) => cosmetic_thread_body(
                    executable.as_deref(),
                    package,
                    self.file.container_data.blobs(),
                )
                .map_err(|cause| {
                    ProjectDrawingsError::CosmeticThreads(occurrence.occurrence_name.clone(), cause)
                })?
                .map(ExactBodyPackage::Graph),
                ExactBodyPackage::Imported(_) => None,
            };
            let package = cosmetic.as_ref().unwrap_or(package);
            let vertices = package.vertices();
            let world = |index: u32| {
                let vertex = vertices.get(usize::try_from(index).ok()?)?;
                Some(occurrence.transform.transform_point(vertex.position_mm))
            };
            let (triangles, faces) = package
                .triangles()
                .iter()
                .enumerate()
                .filter_map(|(index, triangle)| {
                    let [a, b, c] = triangle.vertex_indices;
                    let face = package.triangle_face_ordinal(index)?;
                    Some(([world(a)?, world(b)?, world(c)?], face))
                })
                .unzip();
            solids.push(DrawingSolid {
                name: occurrence.occurrence_name,
                color: occurrence.color.unwrap_or(DEFAULT_SOLID_COLOR),
                triangles,
                faces,
                threads,
            });
        }
        Ok(solids)
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

    /// The sheet settings kept in the document, as stored (the defaults when
    /// none are); stored settings that do not read are an error, never
    /// quietly the defaults.
    pub(crate) fn stored_sheet_settings(&self) -> Result<SheetSettings, serde_json::Error> {
        self.file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == SHEET_NAMESPACE && entry.path() == SHEET_SETTINGS_PATH
            })
            .map_or_else(
                || Ok(SheetSettings::default()),
                |entry| serde_json::from_slice(entry.bytes()),
            )
    }

    /// The stored settings with the project, sheet title and date filled in
    /// where the user left them empty; unreadable stored settings show as the
    /// defaults (the export refuses them, see `project_drawings`).
    pub(crate) fn sheet_settings(&self) -> SheetSettings {
        let mut settings = self.stored_sheet_settings().unwrap_or_default();
        for (field, value) in self.title_defaults() {
            let entry = settings.title_block.entry(field).or_default();
            if entry.trim().is_empty() {
                *entry = value;
            }
        }
        settings
    }

    /// What the title block shows where the user wrote nothing: the document
    /// name, the sheet title in the language of the window, today's date.
    fn title_defaults(&self) -> BTreeMap<TitleField, String> {
        BTreeMap::from([
            (TitleField::Project, self.drawing_document_name()),
            (
                TitleField::Drawing,
                self.catalog.text("drawings-default-sheet"),
            ),
            (TitleField::Date, today()),
        ])
    }

    /// `settings` as the document keeps them: empty fields and fields that
    /// only show their default are left out, so the default is filled in
    /// when a sheet is drawn and never frozen in the document (review
    /// 2026-10-18 UI-12).
    fn without_title_defaults(&self, mut settings: SheetSettings) -> SheetSettings {
        let defaults = self.title_defaults();
        settings.title_block.retain(|field, value| {
            !value.trim().is_empty()
                && TitleField::EDITABLE.contains(field)
                && defaults
                    .get(field)
                    .is_none_or(|default| default.as_str() != value.trim())
        });
        settings
    }

    /// Keeps `settings` in the document, without empty or default fields.
    pub(crate) fn store_sheet_settings(&mut self, settings: SheetSettings) {
        let settings = self.without_title_defaults(settings);
        if self.stored_sheet_settings().ok().as_ref() == Some(&settings) {
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
    #[cfg(test)]
    pub(crate) fn project_drawings(&self) -> Result<ProjectSheet, ProjectDrawingsError> {
        self.project_drawings_job()?.sheet()
    }

    /// What the project sheet is drawn from, taken from the window; the
    /// projection itself needs nothing of it.
    pub(crate) fn project_drawings_job(&self) -> Result<ProjectDrawingsJob, ProjectDrawingsError> {
        self.project_drawings_job_for(SheetRequest::default())
    }

    /// The job of the sheet `request` asks for: the whole sheet or the plan
    /// alone, at the scale it fixes or the one the sheet picks.
    pub(crate) fn project_drawings_job_for(
        &self,
        request: SheetRequest,
    ) -> Result<ProjectDrawingsJob, ProjectDrawingsError> {
        self.stored_sheet_settings()
            .map_err(ProjectDrawingsError::SettingsUnreadable)?;
        let snapshot = self.document.current();
        let solids = self.visible_drawing_solids(&snapshot)?;
        let annotations = snapshot
            .annotations()
            .iter()
            .filter(|annotation| snapshot.tags_visible(&annotation.tags))
            .cloned()
            .collect::<Vec<_>>();
        let settings = self.sheet_settings();
        // A plan of annotations alone cuts nothing; its cut is at the floor.
        let cut = default_plan_cut_z(&solids, settings.plan_cut_above_floor_mm())
            .or_else(|| (solids.is_empty() && !annotations.is_empty()).then_some(0.0))
            .ok_or(ProjectDrawingsError::Drawing(ProjectDrawingError::Empty))?;
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
        let field = |field| {
            settings
                .title_block
                .get(&field)
                .cloned()
                .unwrap_or_default()
        };
        let info = PdfInfo {
            title: format!(
                "{} – {}",
                field(TitleField::Project),
                field(TitleField::Drawing)
            ),
            author: field(TitleField::Author),
            subject: field(TitleField::Stage),
        };
        Ok(ProjectDrawingsJob {
            solids,
            options: ProjectSheetOptions {
                view_titles,
                plan_cut_z_mm: Some(cut),
                format: settings.format,
                coarsest_scale: Some(settings.coarsest_scale()),
                title_labels: self.title_labels(),
                title_block: settings.title_block,
                plan_only: request.plan_only,
                scale: request.scale,
                annotations,
            },
            info,
        })
    }

    pub(crate) fn project_drawings_error_text(&self, error: &ProjectDrawingsError) -> String {
        match error {
            ProjectDrawingsError::Drawing(ProjectDrawingError::Empty) => {
                self.catalog.text("drawings-unavailable")
            }
            ProjectDrawingsError::Drawing(ProjectDrawingError::DoesNotFit { scale, format }) => {
                self.catalog.format(
                    "drawings-do-not-fit",
                    &BTreeMap::from([
                        ("scale", scale.to_string()),
                        ("format", format.map_or("A0", SheetFormat::name).to_owned()),
                    ]),
                )
            }
            ProjectDrawingsError::NotEvaluated(parts) => self.catalog.format(
                "drawings-parts-not-evaluated",
                &BTreeMap::from([
                    ("count", parts.len().to_string()),
                    ("parts", missing_parts_list(parts)),
                ]),
            ),
            ProjectDrawingsError::ProgramPlanning => self.catalog.text("status-program-planning"),
            ProjectDrawingsError::SettingsUnreadable(reason) => self.catalog.format(
                "drawings-settings-unreadable",
                &BTreeMap::from([("reason", reason.to_string())]),
            ),
            ProjectDrawingsError::CosmeticThreads(part, cause) => self.catalog.format(
                "drawings-cosmetic-threads-failed",
                &BTreeMap::from([("part", part.clone()), ("reason", cause.to_string())]),
            ),
            ProjectDrawingsError::Write(error) => error.to_string(),
        }
    }

    /// Writes the project drawings of what is visible as a one-sheet PDF.
    pub(crate) fn write_project_drawings(
        &self,
        path: &Path,
        request: SheetRequest,
    ) -> Result<ProjectSheet, ProjectDrawingsError> {
        let (sheet, pdf) = self.project_drawings_job_for(request)?.render()?;
        publish_project_drawings(path, &pdf)?;
        Ok(sheet)
    }

    /// Writes the project drawings as PDF and reports it in the digest.
    pub fn export_project_drawings_to(&mut self, path: &Path) -> bool {
        let result = self.write_project_drawings(path, SheetRequest::default());
        self.report_project_drawings(path, &result);
        result.is_ok()
    }

    pub(crate) fn report_project_drawings(
        &mut self,
        path: &Path,
        result: &Result<ProjectSheet, ProjectDrawingsError>,
    ) {
        let path = path.display().to_string();
        self.digest = match result {
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
    }

    pub(crate) fn open_project_drawings_window(&mut self) {
        self.drawings.editing = self.sheet_settings();
        self.drawings.shown_defaults = self.title_defaults();
        self.drawings.open = true;
    }

    pub(crate) fn show_project_drawings_window(&mut self, context: &egui::Context) {
        if !self.drawings.open {
            return;
        }
        let mut open = true;
        let mut export = false;
        let mut export_sheet = None;
        let mut editing = std::mem::take(&mut self.drawings.editing);
        // A field still showing its default follows it when the default
        // changes: past midnight, after Save As, in another language.
        let defaults = self.title_defaults();
        for (field, shown) in &self.drawings.shown_defaults {
            if let Some(value) = editing.title_block.get_mut(field)
                && value == shown
                && let Some(default) = defaults.get(field)
            {
                value.clone_from(default);
            }
        }
        self.drawings.shown_defaults = defaults;
        egui::Window::new(self.catalog.text("window-project-drawings"))
            .open(&mut open)
            .default_width(460.0)
            .show(context, |ui| {
                ui.label(self.catalog.text("drawings-window-hint"));
                if let Err(reason) = self.stored_sheet_settings() {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        self.catalog.format(
                            "drawings-settings-unreadable-edit",
                            &BTreeMap::from([("reason", reason.to_string())]),
                        ),
                    );
                }
                egui::Grid::new("project-drawings-title-block")
                    .num_columns(2)
                    .show(ui, |ui| {
                        ui.label(self.catalog.text("drawings-format"));
                        let coarsest = editing.coarsest_scale();
                        let name = |format: Option<SheetFormat>| {
                            format.map_or_else(
                                || {
                                    self.catalog.format(
                                        "drawings-format-auto",
                                        &BTreeMap::from([("scale", coarsest.to_string())]),
                                    )
                                },
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
                        ui.label(self.catalog.text("drawings-coarsest-scale"));
                        let mut scale = coarsest;
                        egui::ComboBox::from_id_salt("project-drawings-scale")
                            .selected_text(format!("1:{scale}"))
                            .show_ui(ui, |ui| {
                                for option in SCALES {
                                    ui.selectable_value(&mut scale, option, format!("1:{option}"));
                                }
                            });
                        if scale != coarsest {
                            editing.coarsest_scale = Some(scale);
                        }
                        ui.end_row();
                        ui.label(self.catalog.text("drawings-plan-cut-height"));
                        let mut height = editing
                            .plan_cut_above_floor_mm
                            .unwrap_or(DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM);
                        if ui
                            .add(
                                egui::DragValue::new(&mut height)
                                    .range(0..=MAX_PLAN_CUT_ABOVE_FLOOR_MM)
                                    .suffix(" mm"),
                            )
                            .changed()
                        {
                            editing.plan_cut_above_floor_mm = Some(height);
                        }
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
                ui.separator();
                ui.strong(self.catalog.text("drawings-document-sheets"));
                ui.label(self.catalog.text("drawings-document-sheets-hint"));
                let snapshot = self.document.current();
                let mut sheets = snapshot.drawing_sheets().peekable();
                if sheets.peek().is_none() {
                    ui.label(self.catalog.text("drawings-document-sheets-none"));
                }
                for sheet in sheets {
                    ui.horizontal(|ui| {
                        ui.label(sheet.name());
                        for format in SheetFileFormat::ALL {
                            let label = match format {
                                SheetFileFormat::Dxf => "drawings-export-sheet-dxf",
                                SheetFileFormat::Dwg => "drawings-export-sheet-dwg",
                            };
                            if ui.button(self.catalog.text(label)).clicked() {
                                export_sheet = Some((sheet.id(), format));
                            }
                        }
                    });
                }
            });
        // Merely opening the window with the filled-in defaults changes
        // nothing, nor does a default that changes; unreadable stored
        // settings are replaced only by an edit.
        if self.without_title_defaults(editing.clone())
            != self.stored_sheet_settings().unwrap_or_default()
        {
            self.store_sheet_settings(editing.clone());
        }
        self.drawings.editing = editing;
        if !open {
            self.drawings.open = false;
        }
        // Projected and hidden lines removed on a thread behind the file
        // activity indication, which can cancel it (review 2026-10-18 UI-11).
        if export && let Some(path) = self.choose_export_path("pdf") {
            self.start_drawing_export(DrawingExport::Project, path, context);
        }
        if let Some((id, format)) = export_sheet
            && let Some(path) = self.choose_export_path(format.extension())
        {
            self.start_drawing_export(DrawingExport::Sheet(id, format), path, context);
        }
    }
}
