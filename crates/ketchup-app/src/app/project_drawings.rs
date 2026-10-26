//! Project drawings of what is visible as a numbered set of sheets: a floor
//! plan of each storey, the sections and the elevations, each sheet an ISO A
//! format with a frame and a title block, drawn from the exact solids. The set
//! is written as one vector PDF, or each sheet as a DXF or a DWG of its
//! visible lines. Hidden layers are left out, so turning off the concept
//! layer draws the construction. The title block and the sheet format are kept
//! in the document.

use crate::app::drawing_export_activity::DrawingExport;
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::*;
use ketchup_application::threads::{CosmeticThreadBodyError, ThreadCylinder, cosmetic_thread_body};
use ketchup_manufacturing::project_drawings::{
    DrawingSet, DrawingSetOptions, DrawingSolid, DrawingThread, PlanSheet, ProjectDrawingError,
    ProjectView, SCALES, SheetTitles, Storey, drawing_set, storeys,
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
    /// The named part's bodies do not resolve to one exact output each (two
    /// extrusions ending in one body); evaluating again does not help.
    UnresolvedBodies(String, ketchup_model::exact_product::ExactProductError),
    /// The program is still being evaluated: the threads of its parts are
    /// not known yet, and the sheet would leave them out.
    ProgramPlanning,
    /// The title block and format stored in the document do not read; they
    /// are left as they are rather than replaced by the defaults.
    SettingsUnreadable(serde_json::Error),
    /// The named part's modelled threads could not be made cosmetic for the
    /// sheet, which would otherwise draw every edge of their helices.
    CosmeticThreads(String, CosmeticThreadBodyError),
    /// A sheet's DXF did not convert to DWG.
    Dwg(ketchup_dwg::DwgError),
    Write(std::io::Error),
}

/// What a client asks of the set beyond the settings the document keeps: the
/// plans alone, and a fixed scale 1:N.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SheetRequest {
    pub(crate) plan_only: bool,
    pub(crate) scale: Option<u32>,
}

/// What the set is written as: one PDF of all sheets, or a DXF or a DWG
/// (AutoCAD 2018) of each sheet.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SetFileFormat {
    Pdf,
    Dxf,
    Dwg,
}

impl SetFileFormat {
    pub(crate) const ALL: [Self; 3] = [Self::Pdf, Self::Dxf, Self::Dwg];

    pub(crate) const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Dxf => "dxf",
            Self::Dwg => "dwg",
        }
    }

    /// The format a path's extension names, in any letter case.
    pub(crate) fn of_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?;
        Self::ALL
            .into_iter()
            .find(|format| extension.eq_ignore_ascii_case(format.extension()))
    }
}

/// The files a set of `sheets` sheets is written to for `path`: the PDF at
/// `path` itself; a DXF or DWG per sheet beside it, numbered like the sheets,
/// `plan.dxf` as `plan-01.dxf`, `plan-02.dxf`, …
pub(crate) fn set_file_paths(path: &Path, format: SetFileFormat, sheets: usize) -> Vec<PathBuf> {
    if format == SetFileFormat::Pdf {
        return vec![path.to_path_buf()];
    }
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let digits = sheets.to_string().len().max(2);
    (1..=sheets)
        .map(|number| {
            path.with_file_name(format!("{stem}-{number:0digits$}.{}", format.extension()))
        })
        .collect()
}

/// The visible solids, the set's options and the PDF's document information:
/// everything the drawing set is drawn from.
pub(crate) struct ProjectDrawingsJob {
    solids: Vec<DrawingSolid>,
    options: DrawingSetOptions,
    info: PdfInfo,
}

impl ProjectDrawingsJob {
    /// Projects the solids and removes the hidden lines.
    fn set(&self) -> Result<DrawingSet, ProjectDrawingsError> {
        drawing_set(&self.solids, &self.options).map_err(ProjectDrawingsError::Drawing)
    }

    /// The set and the bytes of its files: one PDF, or a DXF or DWG per sheet.
    pub(crate) fn render(
        &self,
        format: SetFileFormat,
    ) -> Result<(DrawingSet, Vec<Vec<u8>>), ProjectDrawingsError> {
        let set = self.set()?;
        let files = match format {
            SetFileFormat::Pdf => vec![set.pdf(&self.info)],
            SetFileFormat::Dxf => set.dxf(),
            SetFileFormat::Dwg => set
                .dxf()
                .iter()
                .map(|dxf| ketchup_dwg::dxf_to_dwg(dxf).map_err(ProjectDrawingsError::Dwg))
                .collect::<Result<_, _>>()?,
        };
        Ok((set, files))
    }
}

/// What was written: the set and the files, in sheet order.
pub(crate) struct ExportedSet {
    pub(crate) set: DrawingSet,
    pub(crate) paths: Vec<PathBuf>,
}

/// The SHA-256 of each file of a set an export may replace, by its path.
pub(crate) type SetTargets = BTreeMap<PathBuf, String>;

/// Writes the set's `files` for `path`, each through a temporary file and a
/// rename, so a failed write never leaves a half-written sheet and a link is
/// replaced, not followed. A file there may be replaced only when its SHA-256
/// is the one `targets` holds for it (none: no file may be there), so a file
/// saved there while the set was drawn is never overwritten. When one file
/// cannot be written, those this export wrote are removed again and those it
/// replaced are written back: a set is written whole or not at all. The old
/// sheets in `targets` beyond the new set's last are removed with it, each only
/// while it still holds the SHA-256 `targets` has for it, so no stale sheet is
/// left beside the set as one of it.
pub(crate) fn publish_project_drawings(
    path: &Path,
    format: SetFileFormat,
    files: &[Vec<u8>],
    targets: &SetTargets,
) -> Result<Vec<PathBuf>, ProjectDrawingsError> {
    use crate::export_bundle::{ExportTransactionError, write_export_artifact_if_unchanged};
    let paths = set_file_paths(path, format, files.len());
    let mut written = Vec::<(&Path, &[u8], Option<Vec<u8>>)>::with_capacity(paths.len());
    let mut removed = Vec::<(&Path, Vec<u8>)>::new();
    let mut publish = || -> Result<(), ExportTransactionError> {
        for (target, bytes) in paths.iter().zip(files) {
            let replacing = targets.get(target).map(String::as_str);
            let previous = replacing
                .map(|_| std::fs::read(target))
                .transpose()
                .map_err(ExportTransactionError::io(target))?;
            write_export_artifact_if_unchanged(target, bytes, replacing)?;
            written.push((target, bytes, previous));
        }
        for (stale, sha256) in targets.iter().filter(|(stale, _)| !paths.contains(stale)) {
            let previous = std::fs::read(stale).map_err(ExportTransactionError::io(stale))?;
            if ketchup_model::graph::sha256_hex(&previous) != *sha256 {
                return Err(ExportTransactionError::changed(stale));
            }
            std::fs::remove_file(stale).map_err(ExportTransactionError::io(stale))?;
            removed.push((stale, previous));
        }
        Ok(())
    };
    if let Err(error) = publish() {
        for (stale, previous) in removed {
            let _ = write_export_artifact_if_unchanged(stale, &previous, None);
        }
        for (done, bytes, previous) in written {
            match previous {
                Some(previous) => {
                    let _ = write_export_artifact_if_unchanged(
                        done,
                        &previous,
                        Some(&ketchup_model::graph::sha256_hex(bytes)),
                    );
                }
                None => {
                    let _ = std::fs::remove_file(done);
                }
            }
        }
        return Err(ProjectDrawingsError::Write(std::io::Error::other(error)));
    }
    Ok(paths)
}

/// The files of a set as `format` for `path` an export started now may
/// replace, with their SHA-256: the PDF at `path`, or the numbered DXF or DWG
/// sheets an earlier export wrote beside it, so the set exports again under
/// the same name (review 2026-10-24 MF-27).
pub(crate) fn project_drawings_targets(
    path: &Path,
    format: SetFileFormat,
) -> Result<SetTargets, ProjectDrawingsError> {
    let candidates = if format == SetFileFormat::Pdf {
        vec![path.to_path_buf()]
    } else {
        numbered_sheet_files(path, format)
    };
    let mut targets = SetTargets::new();
    for candidate in candidates {
        if let Some(sha256) = crate::export_bundle::export_target_sha256(&candidate)
            .map_err(|error| ProjectDrawingsError::Write(std::io::Error::other(error)))?
        {
            targets.insert(candidate, sha256);
        }
    }
    Ok(targets)
}

/// The files beside `path` named as [`set_file_paths`] names a set's sheets.
fn numbered_sheet_files(path: &Path, format: SetFileFormat) -> Vec<PathBuf> {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    let prefix = format!("{stem}-");
    let suffix = format!(".{}", format.extension());
    let directory = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let Ok(entries) = std::fs::read_dir(directory) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            let number = name.strip_prefix(&prefix)?.strip_suffix(&suffix)?;
            (number.len() >= 2 && number.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| path.with_file_name(&name))
        })
        .collect()
}

/// The key of the window's button that exports the set as `format`.
pub(crate) const fn set_export_key(format: SetFileFormat) -> &'static str {
    match format {
        SetFileFormat::Pdf => "drawings-export-pdf",
        SetFileFormat::Dxf => "drawings-export-set-dxf",
        SetFileFormat::Dwg => "drawings-export-set-dwg",
    }
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

/// Today's date where the user is, as d. m. yyyy.
fn today() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
    date_at(now, super::local_time::utc_offset_seconds())
}

/// The date, as d. m. yyyy, at `unix_seconds` on a clock `offset_seconds` ahead of UTC.
pub(crate) fn date_at(unix_seconds: i64, offset_seconds: i64) -> String {
    let days = (unix_seconds + offset_seconds).div_euclid(86_400) + 719_468;
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
            let terminal_bodies = ketchup_model::exact_product::exact_body_terminal_features(
                snapshot,
                occurrence.definition_id,
            )
            .map_err(|cause| {
                ProjectDrawingsError::UnresolvedBodies(occurrence.occurrence_name.clone(), cause)
            })?;
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
        self.stored_sheet_bytes()
            .map_or_else(|| Ok(SheetSettings::default()), serde_json::from_slice)
    }

    /// The stored sheet settings as the document keeps them, unread.
    fn stored_sheet_bytes(&self) -> Option<&[u8]> {
        self.file
            .container_data
            .extensions()
            .find(|entry| {
                entry.namespace() == SHEET_NAMESPACE && entry.path() == SHEET_SETTINGS_PATH
            })
            .map(|entry| entry.bytes())
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
    fn without_title_defaults(&self, settings: SheetSettings) -> SheetSettings {
        Self::without_these_defaults(settings, &self.title_defaults())
    }

    fn without_these_defaults(
        mut settings: SheetSettings,
        defaults: &BTreeMap<TitleField, String>,
    ) -> SheetSettings {
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
            .chain([TitleField::Scale, TitleField::Format, TitleField::Sheet])
            .map(|field| {
                (
                    field,
                    self.catalog.text(&format!("title-field-{}", field.key())),
                )
            })
            .collect()
    }

    /// The set of the visible exact solids with the document's title block.
    #[cfg(test)]
    pub(crate) fn project_drawings(&self) -> Result<DrawingSet, ProjectDrawingsError> {
        self.project_drawings_job()?.set()
    }

    /// What the drawing set is drawn from, taken from the window; the
    /// projection itself needs nothing of it.
    pub(crate) fn project_drawings_job(&self) -> Result<ProjectDrawingsJob, ProjectDrawingsError> {
        self.project_drawings_job_for(SheetRequest::default())
    }

    /// The plan sheet of each storey, titled in the window's language: by
    /// the height of its cut, and by its storey when there are several.
    fn plan_sheets(&self, storeys: Vec<Storey>) -> Vec<PlanSheet> {
        let several = storeys.len() > 1;
        storeys
            .into_iter()
            .enumerate()
            .map(|(index, storey)| {
                let arguments = BTreeMap::from([
                    ("height", format!("{:.0}", storey.cut_mm)),
                    ("storey", (index + 1).to_string()),
                ]);
                let (view, sheet) = if several {
                    ("drawings-view-plan-storey", "drawings-sheet-plan-storey")
                } else {
                    ("drawings-view-plan", "drawings-sheet-plan")
                };
                PlanSheet {
                    storey,
                    view_title: self.catalog.format(view, &arguments),
                    sheet_title: self.catalog.format(sheet, &arguments),
                }
            })
            .collect()
    }

    /// The job of the set `request` asks for: all sheets or the plans alone,
    /// at the scale it fixes or the one the set picks.
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
        let mut storeys = storeys(&solids, settings.plan_cut_above_floor_mm());
        // A plan of annotations alone cuts nothing; its cut is at the floor.
        if storeys.is_empty() && !annotations.is_empty() {
            storeys.push(Storey {
                floor_mm: None,
                cut_mm: 0.0,
            });
        }
        if storeys.is_empty() {
            return Err(ProjectDrawingsError::Drawing(ProjectDrawingError::Empty));
        }
        let view_titles = ProjectView::ALL
            .into_iter()
            .filter(|view| *view != ProjectView::Plan)
            .map(|view| {
                let title = match view {
                    ProjectView::Plan => String::new(),
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
            options: DrawingSetOptions {
                plans: self.plan_sheets(storeys),
                sheet_titles: SheetTitles {
                    sections: self.catalog.text("drawings-sheet-sections"),
                    elevations: self.catalog.text("drawings-sheet-elevations"),
                },
                view_titles,
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
            ProjectDrawingsError::Drawing(ProjectDrawingError::TitleValueDoesNotFit(error)) => {
                self.catalog.format(
                    "drawings-title-value-does-not-fit",
                    &BTreeMap::from([
                        (
                            "field",
                            self.catalog
                                .text(&format!("title-field-{}", error.field.key())),
                        ),
                        ("lines", error.lines.to_string()),
                        ("characters", error.characters.to_string()),
                        ("length", error.length.to_string()),
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
            ProjectDrawingsError::UnresolvedBodies(part, cause) => self.catalog.format(
                "drawings-part-bodies-unresolved",
                &BTreeMap::from([("part", part.clone()), ("reason", cause.to_string())]),
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
            ProjectDrawingsError::Dwg(error) => error.to_string(),
            ProjectDrawingsError::Write(error) => error.to_string(),
        }
    }

    /// Writes the drawing set of what is visible as `format` for `path`, each
    /// file replacing only one whose SHA-256 `targets` holds (none: only a new
    /// file).
    pub(crate) fn write_project_drawings(
        &self,
        path: &Path,
        format: SetFileFormat,
        request: SheetRequest,
        targets: &SetTargets,
    ) -> Result<ExportedSet, ProjectDrawingsError> {
        let (set, files) = self.project_drawings_job_for(request)?.render(format)?;
        let paths = publish_project_drawings(path, format, &files, targets)?;
        Ok(ExportedSet { set, paths })
    }

    /// Writes the drawing set as the format `path` names (PDF when it names
    /// none) and reports it in the digest.
    pub fn export_project_drawings_to(&mut self, path: &Path) -> bool {
        let format = SetFileFormat::of_path(path).unwrap_or(SetFileFormat::Pdf);
        let result = project_drawings_targets(path, format).and_then(|targets| {
            self.write_project_drawings(path, format, SheetRequest::default(), &targets)
        });
        self.report_project_drawings(path, &result);
        result.is_ok()
    }

    pub(crate) fn report_project_drawings(
        &mut self,
        path: &Path,
        result: &Result<ExportedSet, ProjectDrawingsError>,
    ) {
        let shown = |paths: &[PathBuf]| match paths {
            [one] => one.display().to_string(),
            [first, .., last] => format!("{} … {}", first.display(), last.display()),
            [] => path.display().to_string(),
        };
        self.digest = match result {
            Ok(ExportedSet { set, paths }) => {
                let missing = set.printed_as_question_mark();
                let mut arguments = BTreeMap::from([
                    ("path", shown(paths)),
                    ("scale", set.scale.to_string()),
                    ("sheets", set.sheets.len().to_string()),
                ]);
                let key = if missing.is_empty() {
                    "digest-exported-project-drawings"
                } else {
                    arguments.insert("characters", missing.iter().collect());
                    "digest-exported-project-drawings-question-marks"
                };
                self.catalog.format(key, &arguments)
            }
            Err(error) => self.catalog.format(
                "error-export-project-drawings",
                &BTreeMap::from([
                    ("path", path.display().to_string()),
                    ("reason", self.project_drawings_error_text(error)),
                ]),
            ),
        };
    }

    pub(crate) fn open_project_drawings_window(&mut self) {
        self.read_sheet_settings_into_window();
        self.drawings.open = true;
    }

    /// The window's fields from the document's stored settings, read once.
    fn read_sheet_settings_into_window(&mut self) {
        self.drawings.editing = self.sheet_settings();
        self.drawings.shown_defaults = self.title_defaults();
        self.drawings.kept = self.without_title_defaults(self.drawings.editing.clone());
        self.note_seen_sheet_settings();
    }

    /// Remembers the stored settings the window has seen, and why they do
    /// not read; called when the window reads or writes them.
    fn note_seen_sheet_settings(&mut self) {
        self.drawings.seen_bytes = self.stored_sheet_bytes().map(<[u8]>::to_vec);
        self.drawings.unreadable = self
            .stored_sheet_settings()
            .err()
            .map(|reason| reason.to_string());
    }

    pub(crate) fn show_project_drawings_window(&mut self, context: &egui::Context) {
        if !self.drawings.open {
            return;
        }
        // Settings the AI wrote, or another document's, replace the fields.
        if self.stored_sheet_bytes() != self.drawings.seen_bytes.as_deref() {
            self.read_sheet_settings_into_window();
        }
        let mut open = true;
        let mut export = None;
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
        let unreadable = self.drawings.unreadable.clone();
        egui::Window::new(self.catalog.text("window-project-drawings"))
            .open(&mut open)
            .default_width(460.0)
            .show(context, |ui| {
                ui.label(self.catalog.text("drawings-window-hint"));
                if let Some(reason) = unreadable {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        self.catalog.format(
                            "drawings-settings-unreadable-edit",
                            &BTreeMap::from([("reason", reason)]),
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
                ui.horizontal(|ui| {
                    for format in SetFileFormat::ALL {
                        if ui
                            .button(self.catalog.text(set_export_key(format)))
                            .clicked()
                        {
                            export = Some(format);
                        }
                    }
                });
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
        let kept = Self::without_these_defaults(editing.clone(), &defaults);
        self.drawings.shown_defaults = defaults;
        if kept != self.drawings.kept {
            self.store_sheet_settings(editing.clone());
            self.note_seen_sheet_settings();
            self.drawings.kept = kept;
        }
        self.drawings.editing = editing;
        if !open {
            self.drawings.open = false;
        }
        // Projected and hidden lines removed on a thread behind the file
        // activity indication, which can cancel it (review 2026-10-18 UI-11).
        if let Some(format) = export
            && let Some(path) = self.choose_export_path(format.extension())
        {
            self.start_drawing_export(DrawingExport::Project(format), path, context);
        }
        if let Some((id, format)) = export_sheet
            && let Some(path) = self.choose_export_path(format.extension())
        {
            self.start_drawing_export(DrawingExport::Sheet(id, format), path, context);
        }
    }
}
