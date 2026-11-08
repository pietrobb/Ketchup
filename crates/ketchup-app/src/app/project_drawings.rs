//! Project drawings of what is visible as a numbered set of sheets: a floor
//! plan of each storey marking the program's windows and doors, the sections,
//! the elevations, a shop drawing of each part group the program names and the
//! schedule of the windows and doors, each sheet an ISO A
//! format with a frame and a title block, drawn from the exact solids. The set
//! is written as one vector PDF, or each sheet as a DXF or a DWG of its
//! visible lines. Hidden layers are left out, so turning off the concept
//! layer draws the construction. The title block and the sheet format are kept
//! in the document.

use crate::app::drawing_export_activity::DrawingExport;
use crate::app::drawing_sheet_export::SheetFileFormat;
use crate::*;
use ketchup_application::threads::{
    CosmeticThreadBodyError, CosmeticThreadGraph, ThreadCylinder, cosmetic_thread_graph,
};
use ketchup_manufacturing::project_drawings::{
    CuttingPiece, CuttingSheet, CuttingStock, DrawingOpening, DrawingSet, DrawingSetOptions,
    DrawingSolid, DrawingThread, OpeningSchedule, PlanSheet, ProjectDrawingError, ProjectView,
    SCALES, SheetTitles, ShopDrawings, ShopPosition, ShopSheet, Storey, drawing_set_until,
    ground_floor, storeys,
};
use ketchup_manufacturing::title_block::{
    DEFAULT_PLAN_CUT_ABOVE_FLOOR_MM, SheetFormat, SheetLanguage, SheetSettings, TitleField,
};
use ketchup_model::annotation::{DecimalMark, dimension_text_with};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::sheet_pdf::PdfInfo;
use ketchup_program::Evaluated;
use std::borrow::Cow;
use std::collections::BTreeSet;
use std::sync::Arc;

/// A sheet language as its speakers name it, whatever the window's language.
const fn sheet_language_name(language: SheetLanguage) -> &'static str {
    match language {
        SheetLanguage::Czech => "Čeština",
        SheetLanguage::Slovak => "Slovenčina",
        SheetLanguage::English => "English",
    }
}

/// Colour of a solid with no colour of its own.
const DEFAULT_SOLID_COLOR: [u8; 3] = [190, 190, 190];
/// The highest plan cut the window offers: above a storey, a plan stops being a floor plan.
const MAX_PLAN_CUT_ABOVE_FLOOR_MM: u32 = 10_000;
pub(super) const SHEET_NAMESPACE: &str = "org.ketchup.drawings";
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
    /// The document's exact results do not read; drawing on would draw no
    /// solid and find ±0,000 at 0 (review 2026-11-04 r19 MF-161).
    ExactResults(ketchup_model::exact_product::ExactProductError),
    /// The program is still being evaluated: the threads of its parts are
    /// not known yet, and the sheet would leave them out.
    ProgramPlanning,
    /// The title block and format stored in the document do not read; they
    /// are left as they are rather than replaced by the defaults.
    SettingsUnreadable(serde_json::Error),
    /// The named part's modelled threads could not be made cosmetic for the
    /// sheet, which would otherwise draw every edge of their helices.
    CosmeticThreads(String, CosmeticThreadBodyError),
    /// The named shown underlay's lines do not read, the plans would leave it
    /// out: `None` when the document holds no blob of them.
    UnderlayUnreadable(String, Option<ketchup_model::underlay::UnderlayLinesError>),
    /// A sheet could not be written as a DXF or DWG drawing.
    Dwg(ketchup_dwg::DwgError),
    /// The set at the path, listed at `list`, is another project's, or
    /// another copy's that wrote it since; `project` names the file it was
    /// exported from (review 2026-11-04 r19 MF-162), and `sheets` the files
    /// of its sheets, which taking it over replaces or deletes (review
    /// 2026-11-06 P3; r23 P3: the question named only how many).
    SetOfAnotherProject {
        list: PathBuf,
        project: Option<String>,
        sheets: Vec<String>,
    },
    Write(std::io::Error),
}

/// The characters of a sheet's name a question shows, the rest as "…": a
/// list on a share may hold names of any length (review 2026-11-07 P3).
pub(crate) const SHEET_NAME_CHARS: usize = 100;

/// The first [`ketchup_rejection::LISTED`] of `sheets`, each cut to
/// [`SHEET_NAME_CHARS`].
pub(crate) fn listed_sheets(sheets: &[String]) -> Vec<String> {
    sheets
        .iter()
        .take(ketchup_rejection::LISTED)
        .map(|name| match name.char_indices().nth(SHEET_NAME_CHARS) {
            Some((end, _)) => format!("{}…", &name[..end]),
            None => name.clone(),
        })
        .collect()
}

/// The first [`ketchup_rejection::LISTED`] of `sheets`, then "…" for the rest,
/// as a question names the sheets of a set it would replace.
pub(crate) fn sheet_names(sheets: &[String]) -> String {
    let mut names = listed_sheets(sheets).join(", ");
    if sheets.len() > ketchup_rejection::LISTED {
        names.push_str(", …");
    }
    names
}

/// Why a shown underlay's lines do not read, in words.
pub(crate) fn underlay_unreadable_reason(
    cause: Option<&ketchup_model::underlay::UnderlayLinesError>,
) -> String {
    cause.map_or_else(
        || "the document holds none of its lines".to_owned(),
        ToString::to_string,
    )
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

/// `package`'s triangles placed by `transform`, and the face each was meshed from.
fn world_mesh(package: &ExactBodyPackage, transform: &Transform) -> (Vec<[[f64; 3]; 3]>, Vec<u32>) {
    let vertices = package.vertices();
    let world = |index: u32| {
        let vertex = vertices.get(usize::try_from(index).ok()?)?;
        Some(transform.transform_point(vertex.position_mm))
    };
    package
        .triangles()
        .iter()
        .enumerate()
        .filter_map(|(index, triangle)| {
            let [a, b, c] = triangle.vertex_indices;
            let face = package.triangle_face_ordinal(index)?;
            Some(([world(a)?, world(b)?, world(c)?], face))
        })
        .unzip()
}

/// The visible solids as the window has them and, for each body that models
/// threads, what the body with them cosmetic is made from: a worker makes it
/// on the job's thread, never the window's (review 2026-11-01 UI-100, UI-11).
pub(crate) struct JobSolids {
    solids: Vec<DrawingSolid>,
    /// By the index of the solid it is drawn as.
    cosmetic: Vec<(usize, CosmeticThreadGraph, Transform)>,
    /// The bodies hidden one by one that ±0,000 is found among with the
    /// drawn ones, placed (review 2026-11-04 r19 MF-161).
    zero_among: Vec<(Arc<ExactBodyPackage>, Transform)>,
}

/// An occurrence the drawings take bodies of, with the exact solid of each
/// and whether the sheets draw it (see [`KetchupApp::job_bodies`]).
type JobOccurrence = (SceneOccurrence, Vec<(Arc<ExactBodyPackage>, bool)>);

/// The solid of `package` placed by `transform`, to find levels in.
fn level_solid(package: &ExactBodyPackage, transform: &Transform) -> DrawingSolid {
    let (triangles, faces) = world_mesh(package, transform);
    DrawingSolid {
        name: String::new(),
        color: DEFAULT_SOLID_COLOR,
        triangles,
        faces,
        threads: Vec::new(),
    }
}

/// Why no ground floor is shown for ±0,000.
#[derive(Debug)]
pub(crate) enum FoundZeroError {
    /// The bodies to find it among are not there.
    Bodies(ProjectDrawingsError),
    /// No worker could be started to find it.
    Worker(std::io::Error),
    /// The worker stopped before it found it.
    WorkerStopped,
}

/// The ground floor the drawings window shows for ±0,000 (review 2026-11-04
/// r19 MF-161): being found on a worker, or found, or why it cannot be.
pub(crate) enum FoundZero {
    Finding(std::sync::mpsc::Receiver<Result<f64, FoundZeroError>>),
    Found(Result<f64, Arc<FoundZeroError>>),
}

impl FoundZero {
    /// Finds the ground floor among the bodies of `snapshot` (see
    /// [`KetchupApp::job_bodies`]), placed, on a worker thread: which bodies
    /// those are is found there too (review 2026-11-06 P3).
    fn find(snapshot: Snapshot, results: ExactResultRegistry) -> Self {
        let (sender, receiver) = std::sync::mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("ketchup-ground-floor".to_owned())
            .spawn(move || {
                let found = KetchupApp::job_bodies(&results, &snapshot)
                    .map(|scoped| {
                        let solids = scoped
                            .iter()
                            .flat_map(|(occurrence, packages)| {
                                packages
                                    .iter()
                                    .map(|(package, _)| level_solid(package, &occurrence.transform))
                            })
                            .collect::<Vec<_>>();
                        ground_floor(&solids)
                    })
                    .map_err(FoundZeroError::Bodies);
                // Sending fails only when the window has gone: nobody to tell.
                crate::delivery::Deliver::deliver(&sender, found);
            });
        match spawned {
            Ok(_) => Self::Finding(receiver),
            Err(error) => Self::Found(Err(Arc::new(FoundZeroError::Worker(error)))),
        }
    }

    /// The ground floor once found; `None` while the worker is at it.
    fn poll(&mut self) -> Option<Result<f64, Arc<FoundZeroError>>> {
        if let Self::Finding(receiver) = self {
            *self = match receiver.try_recv() {
                Ok(found) => Self::Found(found.map_err(Arc::new)),
                Err(std::sync::mpsc::TryRecvError::Empty) => return None,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    Self::Found(Err(Arc::new(FoundZeroError::WorkerStopped)))
                }
            };
        }
        match self {
            Self::Found(found) => Some(found.clone()),
            Self::Finding(_) => None,
        }
    }
}

impl JobSolids {
    /// The solids as the sheet draws them; `cancelled` stops the worker.
    fn drawn(
        self,
        executable: Option<&Path>,
        cancelled: &AtomicBool,
    ) -> Result<Vec<DrawingSolid>, ProjectDrawingsError> {
        let Self {
            mut solids,
            cosmetic,
            ..
        } = self;
        for (index, graph, transform) in cosmetic {
            let solid = &mut solids[index];
            let body = graph.evaluate(executable, cancelled).map_err(|cause| {
                ProjectDrawingsError::CosmeticThreads(solid.name.clone(), cause)
            })?;
            (solid.triangles, solid.faces) = world_mesh(&ExactBodyPackage::Graph(body), &transform);
        }
        Ok(solids)
    }
}

/// The visible solids, the set's options and the PDF's document information:
/// everything the drawing set is drawn from.
pub(crate) struct ProjectDrawingsJob {
    solids: JobSolids,
    /// The exact worker that makes the cosmetic bodies of modelled threads.
    executable: Option<PathBuf>,
    options: DrawingSetOptions,
    info: PdfInfo,
}

impl ProjectDrawingsJob {
    /// Projects the solids and removes the hidden lines; `cancelled` stops
    /// the worker making cosmetic thread bodies and the hidden-line removal.
    fn set(self, cancelled: &AtomicBool) -> Result<(DrawingSet, PdfInfo), ProjectDrawingsError> {
        let Self {
            mut solids,
            executable,
            mut options,
            info,
        } = self;
        let zero_among = std::mem::take(&mut solids.zero_among);
        let solids = solids.drawn(executable.as_deref(), cancelled)?;
        if options.zero_level_mm.is_none() {
            // Found here, on the job's thread (review 2026-11-04 r19 MF-161).
            let hidden = zero_among
                .iter()
                .map(|(package, transform)| level_solid(package, transform))
                .collect::<Vec<_>>();
            options.zero_level_mm = Some(ground_floor(solids.iter().chain(&hidden)));
        }
        let set = drawing_set_until(&solids, &options, cancelled)
            .map_err(ProjectDrawingsError::Drawing)?;
        Ok((set, info))
    }

    /// The set and the bytes of its files: one PDF, or a DXF or DWG per sheet;
    /// `cancelled` stops the worker making cosmetic thread bodies.
    pub(crate) fn render(
        self,
        format: SetFileFormat,
        cancelled: &AtomicBool,
    ) -> Result<(DrawingSet, Vec<Vec<u8>>), ProjectDrawingsError> {
        let (set, info) = self.set(cancelled)?;
        let files = match format {
            SetFileFormat::Pdf => vec![set.pdf(&info)],
            SetFileFormat::Dxf => set.dxf().map_err(ProjectDrawingsError::Dwg)?,
            SetFileFormat::Dwg => set.dwg().map_err(ProjectDrawingsError::Dwg)?,
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

/// The first line of the list a DXF or DWG set was written with before the
/// list named the project's file.
const SET_LIST_HEADER: &str = "Ketchup drawing set";
/// The first line of the list a DXF or DWG set was written with by Kečup
/// 0.3.2, its third line naming the project's file by its absolute path.
const OWNED_SET_LIST_HEADER: &str = "Ketchup drawing set of a project file";
/// The first line of the list a DXF or DWG set is written with, its third
/// line naming the project's file by its name alone, so the list carries no
/// folder with the user's name. Whose the set is says the list each document
/// wrote, kept with the user's settings (review 2026-11-04 r19 MF-162,
/// 2026-11-06 SEC-182).
const DOCUMENT_SET_LIST_HEADER: &str = "Ketchup drawing set of a document";

/// The second line of a set's list: the token of the project's file, or the
/// document's id while it has none.
fn document_line(owner: &SetOwner) -> String {
    match &owner.token {
        Some(token) => format!("file {token}"),
        None => format!("document {}", owner.document.0),
    }
}

/// The line of a set's list naming the file the project is kept in by its
/// name, none for a project not saved.
fn project_name_line(owner: &SetOwner) -> String {
    let name = owner
        .file
        .as_deref()
        .and_then(Path::file_name)
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned());
    format!("project {name}")
}

/// The list written beside a DXF or DWG set for `path`, `plan.dxf` as
/// `plan.dxf.sheets.txt`: the project the set was drawn from and the name and
/// SHA-256 of each sheet. Only the sheets it lists are taken for the set's own
/// when it is exported again; a file merely named like a sheet is not
/// (review 2026-10-24 MF-27, run 325).
pub(crate) fn set_list_path(path: &Path, format: SetFileFormat) -> PathBuf {
    let stem = path
        .file_stem()
        .map_or_else(String::new, |stem| stem.to_string_lossy().into_owned());
    path.with_file_name(format!("{stem}.{}.sheets.txt", format.extension()))
}

/// Whose a set of drawings is: the project it is drawn from, the file the
/// project is kept in and the drawing files the document wrote. A set is the
/// document's while its list is the one the document last wrote there, so a
/// project renamed, moved or saved as "v2" exports its set again, and of two
/// copies only the one that wrote the set last replaces it (review 2026-11-04
/// P3, r19 MF-162).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SetOwner {
    pub(crate) document: DocumentId,
    /// None while the project has not been saved.
    pub(crate) file: Option<PathBuf>,
    /// The token of the project's file, new in a copy saved as another file:
    /// what a set's list names, not the document id a copy shares (review
    /// 2026-11-06 r23 MF-201). None while it has none.
    pub(crate) token: Option<String>,
    /// The SHA-256 of each drawing file the document wrote, by
    /// [`written_key`](super::drawing_sheet_export::written_key).
    pub(crate) written: BTreeMap<String, String>,
}

impl SetOwner {
    /// Whether the list at `list_path`, of SHA-256 `sha256`, is the one the
    /// document last wrote there.
    fn wrote(&self, list_path: &Path, sha256: &str) -> bool {
        super::drawing_sheet_export::written_key(list_path)
            .is_ok_and(|key| self.written.get(&key).is_some_and(|kept| kept == sha256))
    }

    /// Notes `written` files as the document's, as
    /// [`KetchupApp::wrote_drawings`] keeps them.
    #[cfg(test)]
    pub(crate) fn note(&mut self, written: &[(PathBuf, String)]) -> std::io::Result<()> {
        for (path, sha256) in written {
            let key = super::drawing_sheet_export::written_key(path)?;
            self.written.insert(key, sha256.clone());
        }
        Ok(())
    }
}

impl KetchupApp {
    /// The owner of the sets the window exports now. Drawing files kept in a
    /// list that does not read are none of its, so no set is taken unasked.
    /// A work recovery open is of the file it was found for (r19 MF-162).
    pub(crate) fn drawing_set_owner(&self) -> SetOwner {
        SetOwner {
            document: self.document.current().document_id(),
            file: self
                .file
                .path
                .clone()
                .or_else(|| self.recovery_requested_path().map(Path::to_path_buf)),
            token: self.file_token().map(str::to_owned),
            written: self.written_drawings(),
        }
    }
}

/// The list of a set of `owner` naming each of `sheets`, a sheet's file name
/// with a SHA-256 its file may hold; a name may come more than once.
fn set_list(owner: &SetOwner, sheets: &[(String, String)]) -> Vec<u8> {
    list_of(owner, None, sheets)
}

/// The line of a list in place while a set's sheets are written naming the
/// SHA-256 of the list it replaces.
const REPLACING: &str = "replacing ";

/// A set's list, `replacing` the list of that SHA-256 (`none`: no list) while
/// the sheets are written.
fn list_of(owner: &SetOwner, replacing: Option<&str>, sheets: &[(String, String)]) -> Vec<u8> {
    let mut list = format!(
        "{DOCUMENT_SET_LIST_HEADER}\n{}\n{}\n",
        document_line(owner),
        project_name_line(owner)
    );
    if let Some(replacing) = replacing {
        list.push_str(&format!("{REPLACING}{replacing}\n"));
    }
    for (name, sha256) in sheets {
        list.push_str(&format!("{name} {sha256}\n"));
    }
    list.into_bytes()
}

fn sheet_name(sheet: &Path) -> String {
    sheet
        .file_name()
        .map_or_else(String::new, |name| name.to_string_lossy().into_owned())
}

/// The list in place while a set's sheets are written: each sheet as it was
/// (`targets`, the list itself left out) and as it will be (`sheets`), so an
/// export stopped between the sheets and the list, the window closed or the
/// power cut, leaves a set the next export still takes for its own (review
/// 2026-11-04 P3). It names the list it replaces, the one the document wrote,
/// as the document never learns of it then (r19 MF-162).
fn pending_list(
    owner: &SetOwner,
    sheets: &[(String, String)],
    targets: &SetTargets,
    list_path: &Path,
) -> Vec<u8> {
    let was = targets
        .iter()
        .filter(|(target, _)| target.as_path() != list_path)
        .map(|(target, sha256)| (sheet_name(target), sha256.clone()));
    list_of(
        owner,
        Some(targets.get(list_path).map_or("none", String::as_str)),
        &sheets.iter().cloned().chain(was).collect::<Vec<_>>(),
    )
}

/// What a file at a set's list path is to the project exporting the set.
enum SetList {
    /// This document's set, or one it takes over: each sheet name with the
    /// SHA-256s its file may hold.
    Its(BTreeMap<String, BTreeSet<String>>),
    /// The set of another project, or another copy of this one that wrote it
    /// since: the name of the file it was exported from, none when unsaved or
    /// not named, and the files of the sheets it lists.
    Another {
        project: Option<String>,
        sheets: Vec<String>,
    },
    /// No list of a drawing set at all.
    NotAList,
}

/// Whether `project`, the project file a set's list names (by its absolute
/// path in a list of Kečup 0.3.2), has the name of the file `owner` is kept
/// in, as Windows compares names: a project moved with its drawings folder,
/// to an archive or a colleague's synced folder, or opened by another
/// spelling of its path, keeps its set (review 2026-11-06 MF-182, r19 MF-162,
/// capability review run 501). No file is looked at, so an offline share
/// does not hold the window (2026-11-06 P3).
fn names_the_project(project: &str, owner: &SetOwner) -> bool {
    let listed = project
        .rsplit_once(['/', '\\'])
        .map_or(project, |(_, name)| name);
    let name = owner
        .file
        .as_deref()
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    !listed.is_empty()
        && name.is_some_and(|name| {
            name == listed || cfg!(windows) && name.to_lowercase() == listed.to_lowercase()
        })
}

/// What `list`, at `list_path`, is to `owner`: its set when the document
/// last wrote it there or `take_over` says to take it, or when it is the same
/// document's list naming the project's file (see [`names_the_project`]), of
/// a file with a token in a list of this Kečup; a
/// list older than Kečup 0.3.2, naming no file, of the project's id. Any
/// other is another project's, or another copy's, never this one's by its
/// file name alone.
fn set_list_of(list: &[u8], list_path: &Path, owner: &SetOwner, take_over: bool) -> SetList {
    // A file that is not UTF-8 has no header of a list.
    let text = String::from_utf8_lossy(list);
    let mut lines = text.lines();
    let header = lines.next().unwrap_or_default();
    if ![
        SET_LIST_HEADER,
        OWNED_SET_LIST_HEADER,
        DOCUMENT_SET_LIST_HEADER,
    ]
    .contains(&header)
    {
        return SetList::NotAList;
    }
    // A list of this Kečup names the project's file by its token; an older
    // one, by the document id a copy shares too.
    let whose = if header == DOCUMENT_SET_LIST_HEADER {
        document_line(owner)
    } else {
        format!("document {}", owner.document.0)
    };
    let same_document = lines.next() == Some(whose.as_str());
    // A file without a token is named by the document id its copies share:
    // it takes no set by its name (review 2026-11-07 P3).
    let by_name = header != DOCUMENT_SET_LIST_HEADER || owner.token.is_some();
    let project = (header != SET_LIST_HEADER)
        .then(|| lines.next().unwrap_or_default())
        .and_then(|line| line.strip_prefix("project "));
    let mut lines = lines.peekable();
    // A list in place while the sheets were written is the document's when
    // the list it replaced was, or there was none and the project is its.
    let replacing = lines
        .next_if(|line| line.starts_with(REPLACING))
        .and_then(|line| line.strip_prefix(REPLACING));
    let wrote = |sha256: &str| owner.wrote(list_path, sha256);
    let its = take_over
        || wrote(&ketchup_model::graph::sha256_hex(list))
        || replacing.is_some_and(|replaced| wrote(replaced) || replaced == "none" && same_document)
        || same_document
            && by_name
            && (header == SET_LIST_HEADER
                || project.is_some_and(|project| names_the_project(project, owner)));
    let mut listed = BTreeMap::<String, BTreeSet<String>>::new();
    for (name, sha256) in lines.filter_map(|line| line.rsplit_once(' ')) {
        listed
            .entry(name.to_owned())
            .or_default()
            .insert(sha256.to_owned());
    }
    if !its {
        let project = project
            .filter(|project| !project.is_empty())
            .map(|project| {
                project
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(project)
                    .to_owned()
            });
        return SetList::Another {
            project,
            sheets: listed.into_keys().collect(),
        };
    }
    SetList::Its(listed)
}

/// Writes the set's `files` for `path`, each through a temporary file and a
/// rename, so a failed write never leaves a half-written sheet and a link is
/// replaced, not followed. A file there may be replaced only when its SHA-256
/// is the one `targets` holds for it (none: no file may be there), so a file
/// saved there while the set was drawn is never overwritten. When one file
/// cannot be written, those this export wrote are removed again and those it
/// replaced are written back: a set is written whole or not at all. The old
/// sheets in `targets` beyond the new set's last are removed with it, each only
/// while it still holds the SHA-256 `targets` has for it, so no stale sheet is
/// left beside the set as one of it; any other file named like a sheet beyond
/// the last refuses the export. A DXF or DWG set is written with its list
/// ([`set_list_path`]) naming `owner` and each sheet.
pub(crate) fn publish_project_drawings(
    path: &Path,
    format: SetFileFormat,
    files: &[Vec<u8>],
    targets: &SetTargets,
    owner: &SetOwner,
) -> Result<Vec<PathBuf>, ProjectDrawingsError> {
    use crate::export_bundle::ExportTransactionError;
    let paths = set_file_paths(path, format, files.len());
    // A DXF or DWG set's list: the one in place while the sheets are written,
    // then the one naming them as written.
    let lists = (format != SetFileFormat::Pdf).then(|| {
        let list_path = set_list_path(path, format);
        let sheets = paths
            .iter()
            .zip(files)
            .map(|(sheet, bytes)| (sheet_name(sheet), ketchup_model::graph::sha256_hex(bytes)))
            .collect::<Vec<_>>();
        let pending = pending_list(owner, &sheets, targets, &list_path);
        (list_path, pending, set_list(owner, &sheets))
    });
    let list_path = lists.as_ref().map(|(list_path, ..)| list_path);
    let mut written = Vec::<Written<'_>>::with_capacity(paths.len() + 2);
    let mut removed = Vec::<(&Path, Vec<u8>)>::new();
    let mut publish = || -> Result<(), ExportTransactionError> {
        // A file named like a sheet beyond the set's last that is not one the
        // set wrote would be left beside it as one of its sheets.
        if let Some(stray) = lists.as_ref().and_then(|_| {
            numbered_sheet_files(path, format)
                .into_iter()
                .find(|sheet| !paths.contains(sheet) && !targets.contains_key(sheet))
        }) {
            return Err(ExportTransactionError::changed(&stray));
        }
        if let Some((list_path, pending, _)) = &lists {
            let replacing = targets.get(list_path).map(String::as_str);
            write_one(&mut written, list_path, pending, replacing)?;
        }
        for (target, bytes) in paths.iter().zip(files) {
            write_one(
                &mut written,
                target,
                bytes,
                targets.get(target).map(String::as_str),
            )?;
        }
        let stale = targets
            .iter()
            .filter(|(stale, _)| !paths.contains(stale) && Some(*stale) != list_path);
        for (stale, sha256) in stale {
            let previous = std::fs::read(stale).map_err(ExportTransactionError::io(stale))?;
            if ketchup_model::graph::sha256_hex(&previous) != *sha256 {
                return Err(ExportTransactionError::changed(stale));
            }
            std::fs::remove_file(stale).map_err(ExportTransactionError::io(stale))?;
            removed.push((stale, previous));
        }
        if let Some((list_path, pending, list)) = &lists {
            let pending = ketchup_model::graph::sha256_hex(pending);
            write_one(&mut written, list_path, list, Some(&pending))?;
        }
        Ok(())
    };
    if let Err(error) = publish() {
        let error = match roll_back(removed, written) {
            Ok(()) => error,
            Err(recovery) => ExportTransactionError::unrecovered(error, recovery),
        };
        return Err(ProjectDrawingsError::Write(std::io::Error::other(error)));
    }
    Ok(paths)
}

/// A file a set export wrote: where, its bytes and what it held before (none:
/// it was new).
type Written<'a> = (&'a Path, &'a [u8], Option<Vec<u8>>);

/// Writes `bytes` to `target` if it still holds `replacing` (none: no file
/// there), noting in `written` what it held.
fn write_one<'a>(
    written: &mut Vec<Written<'a>>,
    target: &'a Path,
    bytes: &'a [u8],
    replacing: Option<&str>,
) -> Result<(), crate::export_bundle::ExportTransactionError> {
    use crate::export_bundle::{ExportTransactionError, write_export_artifact_if_unchanged};
    let previous = replacing
        .map(|_| std::fs::read(target))
        .transpose()
        .map_err(ExportTransactionError::io(target))?;
    write_export_artifact_if_unchanged(target, bytes, replacing)?;
    written.push((target, bytes, previous));
    Ok(())
}

/// Puts back what a failed set export did: the stale sheets it `removed` and
/// the files it `written` (each with what it replaced, none for a new file),
/// the last first, as the list is written twice. Every step is tried; the
/// first that fails is the answer, so a set left
/// half written is said to be, not reported as untouched (review 2026-11-04
/// P3).
fn roll_back(
    removed: Vec<(&Path, Vec<u8>)>,
    written: Vec<Written<'_>>,
) -> Result<(), crate::export_bundle::ExportTransactionError> {
    use crate::export_bundle::{ExportTransactionError, write_export_artifact_if_unchanged};
    let mut first = Ok(());
    let mut keep = |result: Result<(), ExportTransactionError>| {
        if first.is_ok() {
            first = result;
        }
    };
    for (stale, previous) in removed {
        keep(write_export_artifact_if_unchanged(stale, &previous, None));
    }
    for (done, bytes, previous) in written.into_iter().rev() {
        keep(match previous {
            Some(previous) => write_export_artifact_if_unchanged(
                done,
                &previous,
                Some(&ketchup_model::graph::sha256_hex(bytes)),
            ),
            None => std::fs::remove_file(done).map_err(ExportTransactionError::io(done)),
        });
    }
    first
}

/// The files a set of `owner` for `path` wrote at `paths`, each with the
/// SHA-256 of its bytes, a DXF or DWG set's list too.
pub(crate) fn written_files(
    path: &Path,
    format: SetFileFormat,
    owner: &SetOwner,
    paths: &[PathBuf],
    files: &[Vec<u8>],
) -> Vec<(PathBuf, String)> {
    let mut written = paths
        .iter()
        .zip(files)
        .map(|(path, bytes)| (path.clone(), ketchup_model::graph::sha256_hex(bytes)))
        .collect::<Vec<_>>();
    if format != SetFileFormat::Pdf {
        let sheets = written
            .iter()
            .map(|(sheet, sha256)| (sheet_name(sheet), sha256.clone()))
            .collect::<Vec<_>>();
        let list = set_list(owner, &sheets);
        written.push((
            set_list_path(path, format),
            ketchup_model::graph::sha256_hex(&list),
        ));
    }
    written
}

/// Writes the set's `files` for `path` as [`publish_project_drawings`] does,
/// after taking what it may replace, and notes what it wrote as `owner`'s.
#[cfg(test)]
pub(crate) fn publish_noted(
    path: &Path,
    format: SetFileFormat,
    files: &[Vec<u8>],
    owner: &mut SetOwner,
) -> Result<Vec<PathBuf>, ProjectDrawingsError> {
    let targets = project_drawings_targets(path, format, owner, false)?;
    let paths = publish_project_drawings(path, format, files, &targets, owner)?;
    owner
        .note(&written_files(path, format, owner, &paths, files))
        .map_err(ProjectDrawingsError::Write)?;
    Ok(paths)
}

/// The files of a set as `format` for `path` an export started now may
/// replace, with their SHA-256: the PDF at `path`, or the numbered DXF or DWG
/// sheets an earlier export of `owner` wrote beside it, so the set exports
/// again under the same name (review 2026-10-24 MF-27). A sheet is the set's
/// only while its list names it with the bytes it holds now; any other file
/// named like a sheet, such as another project's or another copy's, is not
/// replaced or removed (run 325). The set of another project, or of another
/// copy that wrote it since, refuses the export naming the project file it was
/// exported from, unless `take_over` says the user takes it for this one's
/// (review 2026-11-04 r19 MF-162).
pub(crate) fn project_drawings_targets(
    path: &Path,
    format: SetFileFormat,
    owner: &SetOwner,
    take_over: bool,
) -> Result<SetTargets, ProjectDrawingsError> {
    let sha256 = |candidate: &Path| {
        crate::export_bundle::export_target_sha256(candidate)
            .map_err(|error| ProjectDrawingsError::Write(std::io::Error::other(error)))
    };
    let mut targets = SetTargets::new();
    if format == SetFileFormat::Pdf {
        if let Some(sha256) = sha256(path)? {
            targets.insert(path.to_path_buf(), sha256);
        }
        return Ok(targets);
    }
    let list_path = set_list_path(path, format);
    let list = match std::fs::read(&list_path) {
        Ok(list) => list,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(targets),
        Err(error) => return Err(ProjectDrawingsError::Write(error)),
    };
    let listed = match set_list_of(&list, &list_path, owner, take_over) {
        SetList::Its(listed) => listed,
        SetList::Another { project, sheets } => {
            return Err(ProjectDrawingsError::SetOfAnotherProject {
                list: list_path,
                project,
                sheets,
            });
        }
        // Writing the list there is refused as for any file not the set's.
        SetList::NotAList => return Ok(targets),
    };
    targets.insert(list_path, ketchup_model::graph::sha256_hex(&list));
    for candidate in numbered_sheet_files(path, format) {
        let name = candidate.file_name().and_then(|name| name.to_str());
        let listed = name.and_then(|name| listed.get(name));
        if let Some(sha256) =
            sha256(&candidate)?.filter(|sha256| listed.is_some_and(|held| held.contains(sha256)))
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

/// What the sheets of `catalog` write before a decimal, read from its one
/// key for every number of a sheet, a plan DXF and a cutting list (review
/// 2026-11-04 r19 P3).
pub(crate) fn sheet_decimal_mark(catalog: &LocaleCatalog) -> DecimalMark {
    if catalog.text("drawings-sheet-decimal-separator") == "," {
        DecimalMark::Comma
    } else {
        DecimalMark::Point
    }
}

/// Today's date where the user is, as the sheets of `catalog` write it.
fn today(catalog: &LocaleCatalog) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0));
    date_at(catalog, now, super::local_time::utc_offset_seconds())
}

/// The date at `unix_seconds` on a clock `offset_seconds` ahead of UTC, as
/// the sheets of `catalog` write it: d. m. yyyy on a Czech or Slovak sheet,
/// yyyy-mm-dd on an English one (review 2026-11-04 r19 P3).
pub(crate) fn date_at(catalog: &LocaleCatalog, unix_seconds: i64, offset_seconds: i64) -> String {
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
    catalog.format(
        "drawings-sheet-date",
        &BTreeMap::from([
            ("day", day.to_string()),
            ("month", month.to_string()),
            ("year", year.to_string()),
            ("day2", format!("{day:02}")),
            ("month2", format!("{month:02}")),
        ]),
    )
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
    /// world millimetres, the part's threads on its first, each drawn as the
    /// job's thread makes it: here only the test waits for a worker.
    #[cfg(test)]
    pub(crate) fn visible_drawing_solids(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<DrawingSolid>, ProjectDrawingsError> {
        let executable = self.drawing_worker();
        self.visible_job_solids(snapshot)?
            .drawn(executable.as_deref(), &AtomicBool::new(false))
    }

    /// The exact worker that makes the cosmetic bodies of modelled threads.
    fn drawing_worker(&self) -> Option<PathBuf> {
        self.exact.worker_path.clone().or_else(|| {
            exact_worker_candidates()
                .into_iter()
                .find(|candidate| candidate.is_file())
        })
    }

    /// The bodies the sheets draw, every visible body of every visible
    /// occurrence, with the exact solid of each and whether it is drawn, and
    /// with them the bodies ±0,000 is found among: hidden one by one but on
    /// shown tags, so hiding the floor slab to look in does not move ±0,000
    /// (review 2026-11-04 MF-141), while a terrain, a neighbour or a variant
    /// on a hidden tag does not decide it (r19 MF-161). A drawn body with no
    /// current exact solid is an error naming its part, and exact results
    /// that do not read are an error, never no solid at all (r19 MF-161).
    fn job_bodies(
        results: &ExactResultRegistry,
        snapshot: &Snapshot,
    ) -> Result<Vec<JobOccurrence>, ProjectDrawingsError> {
        let exact = results
            .body_values(snapshot)
            .map_err(ProjectDrawingsError::ExactResults)?
            .into_iter()
            .map(|(key, package)| ((key.definition_id, key.body_id), package))
            .collect::<BTreeMap<_, _>>();
        // Each definition's bodies and whether each is shown, found once
        // however many times it is placed (r19 MF-161).
        let mut bodies_of = BTreeMap::<DefinitionId, Vec<(BodyId, bool)>>::new();
        let mut missing = Vec::new();
        let mut scoped = Vec::new();
        for occurrence in snapshot.scene_query() {
            let on_shown_tags = snapshot
                .occurrence(occurrence.occurrence_id)
                .is_none_or(|placed| snapshot.tags_visible(placed.tags()));
            let Some(definition) = snapshot
                .definition(occurrence.definition_id)
                .filter(|_| occurrence.visible || on_shown_tags)
            else {
                continue;
            };
            if let std::collections::btree_map::Entry::Vacant(entry) =
                bodies_of.entry(occurrence.definition_id)
            {
                let terminal = match ketchup_model::exact_product::exact_body_terminal_features(
                    snapshot,
                    occurrence.definition_id,
                ) {
                    Ok(terminal) => terminal,
                    Err(cause) => {
                        if occurrence.visible {
                            return Err(ProjectDrawingsError::UnresolvedBodies(
                                occurrence.occurrence_name.clone(),
                                cause,
                            ));
                        }
                        // Not drawn, it only helps find ±0,000: without one
                        // solid per body it gives that none. Not kept, so a
                        // drawn placement of it is refused by name.
                        continue;
                    }
                };
                let bodies = terminal
                    .into_keys()
                    .map(|body_id| {
                        let shown = definition.body(body_id).is_some_and(|body| body.visible());
                        (body_id, shown)
                    })
                    .collect();
                entry.insert(bodies);
            }
            let mut packages = Vec::new();
            for (body_id, shown) in &bodies_of[&occurrence.definition_id] {
                let drawn = occurrence.visible && *shown;
                match exact.get(&(occurrence.definition_id, *body_id)) {
                    Some(package) => packages.push((Arc::clone(package), drawn)),
                    None if drawn => {
                        missing.push(occurrence.occurrence_name.clone());
                        break;
                    }
                    None => {}
                }
            }
            scoped.push((occurrence, packages));
        }
        if !missing.is_empty() {
            return Err(ProjectDrawingsError::NotEvaluated(missing));
        }
        Ok(scoped)
    }

    /// Every visible body of every visible occurrence as its exact solid, in
    /// world millimetres, the part's threads on its first, and what a body
    /// with modelled threads is drawn from instead, with the hidden bodies
    /// ±0,000 is found among too (see [`Self::job_bodies`]).
    pub(crate) fn visible_job_solids(
        &self,
        snapshot: &Snapshot,
    ) -> Result<JobSolids, ProjectDrawingsError> {
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
        let mut drawn = Vec::new();
        let mut zero_among = Vec::new();
        for (occurrence, packages) in Self::job_bodies(&self.exact.results, snapshot)? {
            let placed = threads
                .get(&occurrence.definition_id)
                .into_iter()
                .flatten()
                .filter_map(|thread| placed_thread(&occurrence.transform, thread))
                .collect::<Vec<_>>();
            let mut first = true;
            for (package, is_drawn) in packages {
                if !is_drawn {
                    zero_among.push((package, occurrence.transform));
                    continue;
                }
                let threads = if first { placed.clone() } else { Vec::new() };
                first = false;
                drawn.push((occurrence.clone(), package, threads));
            }
        }
        let blobs = self.file.container_data.blobs();
        let mut solids = Vec::with_capacity(drawn.len());
        let mut cosmetic_graphs = Vec::new();
        for (occurrence, package, threads) in drawn {
            // Modelled threads are drawn by ISO 6410 like cosmetic ones: from
            // the body with them cosmetic, made only on the job's thread,
            // never the helix.
            let cosmetic = match &*package {
                ExactBodyPackage::Graph(package) => cosmetic_thread_graph(package, blobs)
                    .map_err(|cause| {
                        ProjectDrawingsError::CosmeticThreads(
                            occurrence.occurrence_name.clone(),
                            cause,
                        )
                    })?
                    .map(|graph| (solids.len(), graph, occurrence.transform)),
                ExactBodyPackage::Imported(_) => None,
            };
            cosmetic_graphs.extend(cosmetic);
            let (triangles, faces) = world_mesh(&package, &occurrence.transform);
            solids.push(DrawingSolid {
                name: occurrence.occurrence_name,
                color: occurrence.color.unwrap_or(DEFAULT_SOLID_COLOR),
                triangles,
                faces,
                threads,
            });
        }
        Ok(JobSolids {
            solids,
            cosmetic: cosmetic_graphs,
            zero_among,
        })
    }

    /// The ground floor ±0,000 is at when the settings give no height, as
    /// the drawings window shows it: found by a worker, never on the
    /// window's thread (review 2026-11-04 r19 MF-161), from the bodies
    /// [`Self::job_bodies`] finds it among, and kept while the document and
    /// its solids are unchanged. `None` while it is being found; why it
    /// cannot be when the bodies are not there to find it from.
    pub(crate) fn found_zero(
        &self,
        snapshot: &Snapshot,
    ) -> Option<Result<f64, Arc<FoundZeroError>>> {
        let key = (
            snapshot.document_id(),
            snapshot.revision_id(),
            self.exact.results.contents_stamp(),
        );
        let mut found = self.drawings.found_zero.borrow_mut();
        if found.as_ref().is_none_or(|(cached, _)| *cached != key) {
            let finding = FoundZero::find(snapshot.clone(), self.exact.results.clone());
            *found = Some((key, finding));
        }
        let (_, finding) = found.as_mut()?;
        finding.poll()
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
        for (field, value) in self.title_defaults(settings.language) {
            let entry = settings.title_block.entry(field).or_default();
            if entry.trim().is_empty() {
                *entry = value;
            }
        }
        settings
    }

    /// The texts the sheets are titled with: those of `language`, the
    /// window's when none (Měchenice MECH-5).
    pub(crate) fn sheet_catalog(&self, language: Option<SheetLanguage>) -> Cow<'_, LocaleCatalog> {
        match language {
            None => Cow::Borrowed(&self.catalog),
            Some(SheetLanguage::Czech) => Cow::Owned(LocaleCatalog::czech_sheets()),
            Some(SheetLanguage::Slovak) => Cow::Owned(LocaleCatalog::slovak()),
            Some(SheetLanguage::English) => Cow::Owned(LocaleCatalog::english()),
        }
    }

    /// What the title block shows where the user wrote nothing: the document
    /// name, the sheet title in the sheets' `language`, today's date.
    pub(crate) fn title_defaults(
        &self,
        language: Option<SheetLanguage>,
    ) -> BTreeMap<TitleField, String> {
        BTreeMap::from([
            (TitleField::Project, self.drawing_document_name()),
            (
                TitleField::Drawing,
                self.sheet_catalog(language).text("drawings-default-sheet"),
            ),
            (TitleField::Date, today(&self.sheet_catalog(language))),
        ])
    }

    /// `settings` as the document keeps them: empty fields and fields that
    /// only show their default are left out, so the default is filled in
    /// when a sheet is drawn and never frozen in the document (review
    /// 2026-10-18 UI-12).
    fn without_title_defaults(&self, settings: SheetSettings) -> SheetSettings {
        let defaults = self.title_defaults(settings.language);
        Self::without_these_defaults(settings, &defaults)
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

    fn title_labels(catalog: &LocaleCatalog) -> BTreeMap<TitleField, String> {
        TitleField::EDITABLE
            .into_iter()
            .chain([TitleField::Scale, TitleField::Format, TitleField::Sheet])
            .map(|field| (field, catalog.text(&format!("title-field-{}", field.key()))))
            .collect()
    }

    /// The set of the visible exact solids with the document's title block.
    #[cfg(test)]
    pub(crate) fn project_drawings(&self) -> Result<DrawingSet, ProjectDrawingsError> {
        let (set, _) = self.project_drawings_job()?.set(&AtomicBool::new(false))?;
        Ok(set)
    }

    /// What the drawing set is drawn from, taken from the window; the
    /// projection itself needs nothing of it.
    pub(crate) fn project_drawings_job(&self) -> Result<ProjectDrawingsJob, ProjectDrawingsError> {
        self.project_drawings_job_for(SheetRequest::default())
    }

    /// The plan sheet of each storey, titled from `catalog`: by the height
    /// of its cut, and by its storey when there are several.
    fn plan_sheets(catalog: &LocaleCatalog, storeys: Vec<Storey>) -> Vec<PlanSheet> {
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
                    view_title: catalog.format(view, &arguments),
                    sheet_title: catalog.format(sheet, &arguments),
                }
            })
            .collect()
    }

    /// The evaluation of the document's program, if it has one that runs;
    /// [`ProjectDrawingsError::ProgramPlanning`] while it is being evaluated.
    fn drawing_program(&self) -> Result<Option<Arc<Evaluated>>, ProjectDrawingsError> {
        match self
            .document
            .current_rule_program()
            .map(|program| self.program_evaluations.try_get(program))
        {
            None => Ok(None),
            Some(crate::program_evaluation::Lookup::Ready(evaluation)) => Ok(evaluation.ok()),
            Some(crate::program_evaluation::Lookup::Pending) => {
                Err(ProjectDrawingsError::ProgramPlanning)
            }
        }
    }

    /// The name of the solid each program part is drawn as.
    fn drawing_solid_names(snapshot: &Snapshot) -> BTreeMap<String, String> {
        snapshot
            .scene_query()
            .into_iter()
            .filter_map(|occurrence| {
                ketchup_application::rule_program_part_name(snapshot, &occurrence.instance_path)
                    .map(|name| (name, occurrence.occurrence_name))
            })
            .collect()
    }

    /// The program's shop drawings, titled from `catalog`, their parts named
    /// as the solids they are drawn as.
    fn drawing_shop(
        catalog: &LocaleCatalog,
        evaluated: Option<&Evaluated>,
        solid_names: &BTreeMap<String, String>,
    ) -> ShopDrawings {
        let sheets = evaluated
            .map(|evaluated| ketchup_program::shop_drawing::shop_drawings(&evaluated.model))
            .unwrap_or_default()
            .into_iter()
            .map(|drawing| ShopSheet {
                title: catalog.format(
                    "drawings-sheet-shop",
                    &BTreeMap::from([("name", drawing.name)]),
                ),
                right: drawing.right,
                up: drawing.up,
                positions: drawing
                    .positions
                    .into_iter()
                    .map(|position| ShopPosition {
                        material: position.material,
                        dimensions_mm: position.dimensions_mm,
                        count: position.parts.len(),
                        solids: position
                            .parts
                            .iter()
                            .filter_map(|part| solid_names.get(part).cloned())
                            .collect(),
                        mirror_of: position.mirror_of,
                    })
                    .collect(),
            })
            .collect();
        ShopDrawings {
            sheets,
            columns: ["position", "material", "size", "count"]
                .map(|column| catalog.text(&format!("drawings-shop-{column}"))),
            mirror_of: catalog.text("drawings-shop-mirror-of"),
        }
    }

    /// The program's cutting plans (the report's bom.cutting) as sheets of
    /// the set, titled and captioned from `catalog`.
    fn drawing_cutting(
        catalog: &LocaleCatalog,
        evaluated: Option<&Evaluated>,
    ) -> Vec<CuttingSheet> {
        let Some(evaluated) = evaluated else {
            return Vec::new();
        };
        // The numbers take the sheet's decimal separator, as its level marks
        // and dimensions do (review 2026-11-04 MF-140, r19 P3).
        let mark = sheet_decimal_mark(catalog);
        let number = |value: f64| mark.written(&value.to_string());
        let mm = |values: &[f64]| {
            values
                .iter()
                .map(|value| dimension_text_with(*value, mark))
                .collect::<Vec<_>>()
                .join(" x ")
        };
        ketchup_program::bom::bom(&evaluated.model)
            .cutting
            .into_iter()
            .filter(|plan| !plan.layouts.is_empty())
            .map(|plan| {
                let bars = plan.kind == ketchup_program::cutting::StockKind::Lengths;
                let stock = plan
                    .layouts
                    .iter()
                    .enumerate()
                    .map(|(index, layout)| CuttingStock {
                        caption: catalog.format(
                            "drawings-cutting-stock",
                            &BTreeMap::from([
                                ("number", (index + 1).to_string()),
                                ("size", mm(&layout.stock_mm)),
                                ("used", number(layout.used_percent)),
                            ]),
                        ),
                        size_mm: if bars {
                            [layout.stock_mm[0], plan.section_mm[0]]
                        } else {
                            [layout.stock_mm[0], layout.stock_mm[1]]
                        },
                        pieces: layout
                            .pieces
                            .iter()
                            .map(|piece| CuttingPiece {
                                label: format!("{} {}", piece.part, mm(&piece.size_mm)),
                                at_mm: if bars {
                                    [piece.at_mm[0], 0.0]
                                } else {
                                    [piece.at_mm[0], piece.at_mm[1]]
                                },
                                size_mm: if bars {
                                    [piece.size_mm[0], plan.section_mm[0]]
                                } else {
                                    [piece.size_mm[0], piece.size_mm[1]]
                                },
                            })
                            .collect(),
                    })
                    .collect();
                CuttingSheet {
                    title: catalog.format(
                        "drawings-sheet-cutting",
                        &BTreeMap::from([
                            ("material", plan.material.clone()),
                            ("section", mm(&plan.section_mm)),
                        ]),
                    ),
                    bars,
                    stock,
                }
            })
            .collect()
    }

    /// The program's windows and doors as the drawing set marks and lists
    /// them: one mark per kind and designation, numbered in the order the
    /// designations first come ("W1", "W2", "D1"), so each schedule row is a
    /// row of the hardware list; their pieces named as the solids they are
    /// drawn as.
    fn drawing_openings(
        catalog: &LocaleCatalog,
        evaluated: Option<&Evaluated>,
        solid_names: &BTreeMap<String, String>,
    ) -> OpeningSchedule {
        let mut schedule = OpeningSchedule {
            openings: Vec::new(),
            title: catalog.text("drawings-sheet-openings"),
            columns: ["mark", "designation", "count"]
                .map(|column| catalog.text(&format!("drawings-openings-{column}"))),
        };
        let Some(evaluated) = evaluated else {
            return schedule;
        };
        let mut types: Vec<(String, String)> = Vec::new();
        for opening in ketchup_program::bom::openings(&evaluated.model) {
            let key = (opening.kind.clone(), opening.designation.clone());
            if !types.contains(&key) {
                types.push(key.clone());
            }
            let number = types
                .iter()
                .take_while(|known| **known != key)
                .filter(|(kind, _)| *kind == opening.kind)
                .count()
                + 1;
            schedule.openings.push(DrawingOpening {
                mark: catalog.format(
                    &format!("drawings-mark-{}", opening.kind),
                    &BTreeMap::from([("number", number.to_string())]),
                ),
                designation: opening.designation,
                parts: opening
                    .parts
                    .iter()
                    .filter_map(|part| solid_names.get(part).cloned())
                    .collect(),
            });
        }
        schedule
    }

    /// The lines of every shown layer of every shown underlay lying flat, in
    /// the world, which the plans draw beneath the model (Měchenice MECH-10).
    /// A shown underlay whose lines do not read refuses the set: the plans
    /// would leave it out unsaid (review 2026-11-04 P3).
    fn drawing_underlay(
        &self,
        snapshot: &Snapshot,
    ) -> Result<Vec<[[f64; 3]; 2]>, ProjectDrawingsError> {
        let shown = |tag| snapshot.tag(tag).is_some_and(|tag| tag.visible());
        let mut lines = Vec::new();
        for underlay in snapshot
            .underlays()
            .filter(|underlay| underlay.lies_flat() && shown(underlay.tag))
        {
            let drawn = self.underlay_lines_or_reason(underlay).map_err(|cause| {
                ProjectDrawingsError::UnderlayUnreadable(underlay.name.clone(), cause)
            })?;
            let placement = underlay.world_transform();
            let world = |[x, y]: [f64; 2]| placement.transform_point([x, y, 0.0]);
            for (layer, layer_lines) in underlay.layers.iter().zip(drawn.layers()) {
                if shown(layer.tag) {
                    lines.extend(layer_lines.segments().map(|[a, b]| [world(a), world(b)]));
                }
            }
        }
        Ok(lines)
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
        let solids = self.visible_job_solids(&snapshot)?;
        let evaluated = self.drawing_program()?;
        let settings = self.sheet_settings();
        let catalog = self.sheet_catalog(settings.language);
        let solid_names = Self::drawing_solid_names(&snapshot);
        let openings = Self::drawing_openings(&catalog, evaluated.as_deref(), &solid_names);
        let shop = Self::drawing_shop(&catalog, evaluated.as_deref(), &solid_names);
        let cutting = Self::drawing_cutting(&catalog, evaluated.as_deref());
        let annotations = snapshot
            .annotations()
            .iter()
            .filter(|annotation| snapshot.tags_visible(&annotation.tags))
            .cloned()
            .collect::<Vec<_>>();
        // The storeys are found from the bodies as evaluated: a thread's
        // grooves never make a floor.
        let mut storeys = storeys(&solids.solids, settings.plan_cut_above_floor_mm());
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
                    ProjectView::Plan | ProjectView::Shop => String::new(),
                    ProjectView::LongitudinalSection => catalog.text("drawings-view-longitudinal"),
                    ProjectView::CrossSection => catalog.text("drawings-view-cross"),
                    ProjectView::Front => catalog.text("drawings-view-front"),
                    ProjectView::Back => catalog.text("drawings-view-back"),
                    ProjectView::Left => catalog.text("drawings-view-left"),
                    ProjectView::Right => catalog.text("drawings-view-right"),
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
        let zero_level_mm = settings.zero_level_mm.map(f64::from);
        Ok(ProjectDrawingsJob {
            solids,
            executable: self.drawing_worker(),
            options: DrawingSetOptions {
                plans: Self::plan_sheets(&catalog, storeys),
                sheet_titles: SheetTitles {
                    sections: catalog.text("drawings-sheet-sections"),
                    elevations: catalog.text("drawings-sheet-elevations"),
                },
                view_titles,
                format: settings.format,
                coarsest_scale: Some(settings.coarsest_scale()),
                title_labels: Self::title_labels(&catalog),
                title_block: settings.title_block,
                plan_only: request.plan_only,
                scale: request.scale,
                annotations,
                underlay: self.drawing_underlay(&snapshot)?,
                openings,
                shop,
                cutting,
                decimal_separator: sheet_decimal_mark(&catalog),
                zero_level_mm,
            },
            info,
        })
    }

    pub(crate) fn found_zero_error_text(&self, error: &FoundZeroError) -> String {
        match error {
            FoundZeroError::Bodies(error) => self.project_drawings_error_text(error),
            FoundZeroError::Worker(cause) => self.catalog.format(
                "drawings-zero-level-not-found",
                &BTreeMap::from([("reason", cause.to_string())]),
            ),
            FoundZeroError::WorkerStopped => self.catalog.format(
                "drawings-zero-level-not-found",
                &BTreeMap::from([(
                    "reason",
                    self.catalog.text("drawings-zero-level-worker-stopped"),
                )]),
            ),
        }
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
            ProjectDrawingsError::ExactResults(cause) => self.catalog.format(
                "drawings-exact-results-unreadable",
                &BTreeMap::from([("reason", cause.to_string())]),
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
            ProjectDrawingsError::UnderlayUnreadable(name, cause) => self.catalog.format(
                "drawings-underlay-unreadable",
                &BTreeMap::from([
                    ("name", name.clone()),
                    ("reason", underlay_unreadable_reason(cause.as_ref())),
                ]),
            ),
            ProjectDrawingsError::Drawing(error @ ProjectDrawingError::Cancelled) => {
                error.to_string()
            }
            ProjectDrawingsError::Dwg(error) => error.to_string(),
            ProjectDrawingsError::SetOfAnotherProject {
                list,
                project,
                sheets,
            } => self.catalog.format(
                "drawings-set-of-another-project",
                &BTreeMap::from([
                    ("list", list.display().to_string()),
                    ("sheets", sheets.len().to_string()),
                    ("names", sheet_names(sheets)),
                    (
                        "project",
                        project.clone().unwrap_or_else(|| {
                            self.catalog.text("drawings-set-of-an-unsaved-project")
                        }),
                    ),
                ]),
            ),
            ProjectDrawingsError::Write(error) => error.to_string(),
        }
    }

    /// Writes the drawing set of what is visible as `format` for `path`, each
    /// file replacing only one whose SHA-256 `targets` holds (none: only a new
    /// file). The caller waits: for tests only, as the window and the bridge
    /// draw on a thread.
    #[cfg(test)]
    pub(crate) fn write_project_drawings(
        &self,
        path: &Path,
        format: SetFileFormat,
        request: SheetRequest,
        targets: &SetTargets,
        cancelled: &AtomicBool,
    ) -> Result<ExportedSet, ProjectDrawingsError> {
        let (set, files) = self
            .project_drawings_job_for(request)?
            .render(format, cancelled)?;
        let owner = self.drawing_set_owner();
        let paths = publish_project_drawings(path, format, &files, targets, &owner)?;
        Ok(ExportedSet { set, paths })
    }

    /// Writes the drawing set as the format `path` names (PDF when it names
    /// none) and reports it in the digest, the caller waiting for it; the
    /// window's menu exports on a thread (`drawing_export_activity`).
    #[cfg(test)]
    pub fn export_project_drawings_to(&mut self, path: &Path) -> bool {
        let format = SetFileFormat::of_path(path).unwrap_or(SetFileFormat::Pdf);
        let owner = self.drawing_set_owner();
        let result = project_drawings_targets(path, format, &owner, false).and_then(|targets| {
            let cancelled = AtomicBool::new(false);
            self.write_project_drawings(path, format, SheetRequest::default(), &targets, &cancelled)
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
        self.drawings.another_projects_set = match result {
            Err(error @ ProjectDrawingsError::SetOfAnotherProject { .. }) => {
                SetFileFormat::of_path(path).map(|format| {
                    (
                        path.to_path_buf(),
                        format,
                        self.project_drawings_error_text(error),
                    )
                })
            }
            _ => None,
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
        self.drawings.shown_defaults = self.title_defaults(self.drawings.editing.language);
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
        let mut take_over = None;
        let mut editing = std::mem::take(&mut self.drawings.editing);
        // A field still showing its default follows it when the default
        // changes: past midnight, after Save As, in another language.
        let defaults = self.title_defaults(editing.language);
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
                        ui.label(self.catalog.text("drawings-language"));
                        let language_name = |language: Option<SheetLanguage>| {
                            language.map_or_else(
                                || self.catalog.text("drawings-language-auto"),
                                |language| sheet_language_name(language).to_owned(),
                            )
                        };
                        egui::ComboBox::from_id_salt("project-drawings-language")
                            .selected_text(language_name(editing.language))
                            .show_ui(ui, |ui| {
                                for language in std::iter::once(None)
                                    .chain(SheetLanguage::ALL.into_iter().map(Some))
                                {
                                    ui.selectable_value(
                                        &mut editing.language,
                                        language,
                                        language_name(language),
                                    );
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
                        // ±0,000: the ground floor found, shown, or a height the
                        // user sets from it, any height (review 2026-11-04 MF-141;
                        // r19 MF-160: a model in absolute heights was held to ±200 m).
                        ui.label(self.catalog.text("drawings-zero-level"));
                        // Found on a worker (r19 MF-161): "…" until then, "–"
                        // with the reason when it cannot be.
                        let finding = self.found_zero(&self.document.current());
                        if finding.is_none() {
                            ui.ctx()
                                .request_repaint_after(std::time::Duration::from_millis(100));
                        }
                        // Whole millimetres; none past the range of i32 (±2 147 km).
                        let found = finding
                            .as_ref()
                            .and_then(|found| found.as_ref().ok())
                            .and_then(|height| format!("{height:.0}").parse::<i32>().ok());
                        ui.horizontal(|ui| {
                            let mut ground = editing.zero_level_mm.is_none();
                            let ready = finding.is_some() || !ground;
                            let shown = match (&finding, found) {
                                (_, Some(found)) => found.to_string(),
                                (None, None) => "…".to_owned(),
                                (Some(_), None) => "–".to_owned(),
                            };
                            let why = finding
                                .and_then(Result::err)
                                .map(|error| self.found_zero_error_text(&error));
                            // Unticked only once found, so the height set starts
                            // from the floor, not from 0 (review 2026-11-06 P3).
                            let checkbox = egui::Checkbox::new(
                                &mut ground,
                                self.catalog.format(
                                    "drawings-zero-level-ground",
                                    &BTreeMap::from([("height", shown)]),
                                ),
                            );
                            if ui.add_enabled(ready, checkbox).changed() {
                                editing.zero_level_mm = (!ground).then(|| found.unwrap_or(0));
                            }
                            if let Some(why) = why {
                                ui.weak(why);
                            }
                            if let Some(height) = editing.zero_level_mm.as_mut() {
                                ui.add(egui::DragValue::new(height).suffix(" mm"));
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
                // Another project's set at the path chosen: the user may take
                // it over, its sheets then this project's (review 2026-11-04
                // r19 MF-162).
                if let Some((path, format, said)) = &self.drawings.another_projects_set {
                    ui.colored_label(ui.visuals().warn_fg_color, said);
                    if ui
                        .button(self.catalog.text("drawings-take-over-set"))
                        .clicked()
                    {
                        take_over = Some((path.clone(), *format));
                    }
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
        let kept = Self::without_these_defaults(editing.clone(), &defaults);
        self.drawings.shown_defaults = defaults;
        if kept != self.drawings.kept {
            // Without the defaults shown: a sheet title left at its default
            // follows a change of the sheet language, never kept in the old one.
            self.store_sheet_settings(kept.clone());
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
        if let Some((path, format)) = take_over {
            self.drawings.taking_over = Some(path.clone());
            self.start_drawing_export(DrawingExport::Project(format), path, context);
            // Only this export takes it over, also one that could not start.
            self.drawings.taking_over = None;
        }
        if let Some((id, format)) = export_sheet
            && let Some(path) = self.choose_export_path(format.extension())
        {
            self.start_drawing_export(DrawingExport::Sheet(id, format), path, context);
        }
    }
}

#[cfg(test)]
mod publish_tests;
