//! Reads the STEP files a program's bought parts name (`purchased()`,
//! `imported()`) from the folder of the program's file, with the exact
//! worker, and keeps their bytes for the document to seal.

use ketchup_application::program_imports::{ImportFiles, placed_part_name, unreadable};
use ketchup_model::document::{FeatureKind, Snapshot, Transform};
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{
    ImportFormat, STEP_PARSER_VERSION, STEP_XDE_PARSER_VERSION, StepImportEvidence,
    StepXdeImportEvidence, StepXdeNodeEvidence,
};
use ketchup_program::imported::ImportedBody;
use ketchup_rejection::Rejection;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

/// The largest STEP file a program reads; a bought part's file is a few MiB.
const MAX_STEP_BYTES: u64 = 256 * 1024 * 1024;
/// The most bytes of STEP files the window keeps read between applies.
const MAX_CACHED_BYTES: u64 = 512 * 1024 * 1024;

/// The folder a program's file was last applied from, and what the exact
/// worker read of the STEP files in it, kept for the window's later applies
/// (a Push/Pull, set_params): an unchanged file is not read again.
#[derive(Default)]
pub(crate) struct ProgramFolder {
    /// The program file's name and its folder, as an apply last gave them.
    given: Option<(String, PathBuf)>,
    cache: Arc<Mutex<StepCache>>,
}

impl ProgramFolder {
    /// The reader of the STEP files of the program `file_name`: from the
    /// folder `given`, kept for later applies of the same file, or the one an
    /// earlier apply gave for it.
    pub(crate) fn files(
        &mut self,
        given: Option<PathBuf>,
        file_name: &str,
        worker_path: Option<PathBuf>,
        cancelled: Arc<AtomicBool>,
        snapshot: &Snapshot,
    ) -> StepFiles {
        if let Some(folder) = given {
            self.given = Some((file_name.to_owned(), folder));
        }
        let folder = self
            .given
            .as_ref()
            .filter(|(name, _)| name == file_name)
            .map(|(_, folder)| folder.clone());
        StepFiles::new(
            folder,
            worker_path,
            cancelled,
            snapshot,
            Arc::clone(&self.cache),
        )
    }

    /// How many times the exact worker read a STEP file for a program.
    #[cfg(test)]
    pub(crate) fn worker_reads(&self) -> usize {
        lock(&self.cache).worker_reads
    }
}

/// The STEP files read, by their canonical path.
#[derive(Default)]
struct StepCache {
    files: BTreeMap<PathBuf, CachedFile>,
    worker_reads: usize,
}

/// A file as last read: its size and time of change to see it unchanged
/// without reading it, its SHA-256 to see it unchanged when only its time
/// changed, and what the worker read of it.
#[derive(Clone)]
struct CachedFile {
    length: u64,
    modified: Option<SystemTime>,
    sha: [u8; 32],
    bytes: Arc<Vec<u8>>,
    /// The file as one body.
    whole: Option<StepImportEvidence>,
    /// The file as an assembly, its parts read one by one.
    assembly: Option<StepXdeImportEvidence>,
}

fn lock(cache: &Mutex<StepCache>) -> std::sync::MutexGuard<'_, StepCache> {
    cache.lock().unwrap_or_else(PoisonError::into_inner)
}

impl StepCache {
    fn keep(&mut self, path: PathBuf, file: CachedFile) {
        let held: u64 = self
            .files
            .iter()
            .filter(|(kept, _)| **kept != path)
            .map(|(_, kept)| kept.length)
            .sum();
        if held + file.length > MAX_CACHED_BYTES {
            self.files.clear();
        }
        self.files.insert(path, file);
    }
}

pub(crate) struct StepFiles {
    /// The folder of the program's file, none when the program came without it.
    folder: Option<PathBuf>,
    worker_path: Option<PathBuf>,
    worker: Option<ExactWorkerSupervisor>,
    cancelled: Arc<AtomicBool>,
    /// The bytes of each file read, by their SHA-256, for the document to seal.
    sources: BTreeMap<[u8; 32], Arc<Vec<u8>>>,
    /// Whole-file bodies the document already holds, by the SHA-256 of their
    /// file: a file read again unchanged is not inspected again.
    known: BTreeMap<[u8; 32], StepImportEvidence>,
    /// Assembly parts the document already places, by the SHA-256 of their
    /// file and the name `part=` gave: an unchanged file's assembly is not
    /// read again for them, in a window that has not read it yet either.
    known_parts: BTreeMap<([u8; 32], String), Placed>,
    cache: Arc<Mutex<StepCache>>,
}

impl StepFiles {
    fn new(
        folder: Option<PathBuf>,
        worker_path: Option<PathBuf>,
        cancelled: Arc<AtomicBool>,
        snapshot: &Snapshot,
        cache: Arc<Mutex<StepCache>>,
    ) -> Self {
        let (mut known, mut known_parts) = (BTreeMap::new(), BTreeMap::new());
        for feature in snapshot.features() {
            let FeatureKind::ImportedExactBody(spec) = feature.kind() else {
                continue;
            };
            let Some(receipt) = snapshot.import_receipt(spec.import_id) else {
                continue;
            };
            let Some(topology_counts) = spec.topology_counts else {
                continue;
            };
            let version = match spec.source_part_index {
                None => STEP_PARSER_VERSION,
                Some(_) => STEP_XDE_PARSER_VERSION,
            };
            if receipt.format() != ImportFormat::Step || receipt.parser_version() != version {
                continue;
            }
            let evidence = StepImportEvidence {
                source_sha256: spec.source_sha256,
                source_byte_len: spec.source_byte_len,
                source_unit: receipt.units().source_unit(),
                result_fingerprint: spec.result_fingerprint.clone(),
                body_kind: spec.body_kind,
                solid_count: spec.solid_count,
                topology_counts,
                area_mm2: spec.area_mm2,
                volume_mm3: spec.volume_mm3,
                bounds_mm: spec.bounds_mm,
                backend: spec.backend.clone(),
                tolerance: spec.tolerance.clone(),
                enclosing_bounds_mm: spec.enclosing_bounds_mm,
            };
            let Some(index) = spec.source_part_index else {
                known.insert(spec.source_sha256, evidence);
                continue;
            };
            let named = snapshot
                .definition(feature.definition_id())
                .and_then(|definition| {
                    placed_part_name(feature.name(), definition.name(), receipt.source_name())
                });
            if let Some(named) = named {
                known_parts.insert(
                    (spec.source_sha256, named.to_owned()),
                    (index, evidence, spec.source_placement),
                );
            }
        }
        Self {
            folder,
            worker_path,
            worker: None,
            cancelled,
            sources: BTreeMap::new(),
            known,
            known_parts,
            cache,
        }
    }

    /// The bytes of every file read.
    pub(crate) fn into_sources(self) -> Vec<Vec<u8>> {
        self.sources
            .into_values()
            .map(Arc::unwrap_or_clone)
            .collect()
    }

    /// The file at `path` as the cache holds it when its size and time of
    /// change, or else its bytes, are unchanged; otherwise as read now.
    fn cached(&self, path: &Path) -> Result<CachedFile, Rejection> {
        let metadata = std::fs::metadata(path)
            .map_err(|error| unreadable("its size cannot be read").caused_by(error))?;
        let length = metadata.len();
        if length > MAX_STEP_BYTES {
            return Err(unreadable(format!(
                "it holds {length} bytes, more than the {MAX_STEP_BYTES} a bought part's file may"
            )));
        }
        let modified = metadata.modified().ok();
        let kept = lock(&self.cache).files.get(path).cloned();
        if let Some(kept) = &kept
            && kept.length == length
            && modified.is_some()
            && kept.modified == modified
        {
            return Ok(kept.clone());
        }
        let bytes = std::fs::read(path)
            .map_err(|io| unreadable("its bytes cannot be read").caused_by(io))?;
        if bytes.is_empty() {
            return Err(unreadable("the file is empty"));
        }
        let sha = sha256_bytes(&bytes);
        Ok(match kept.filter(|kept| kept.sha == sha) {
            Some(kept) => CachedFile { modified, ..kept },
            None => CachedFile {
                length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
                modified,
                sha,
                bytes: Arc::new(bytes),
                whole: self.known.get(&sha).cloned(),
                assembly: None,
            },
        })
    }

    /// A copy of `file`'s bytes for the worker to read, not the file someone
    /// may change meanwhile, and their SHA-256 in hex.
    fn copy(&self, file: &CachedFile) -> Result<(tempfile::NamedTempFile, String), Rejection> {
        lock(&self.cache).worker_reads += 1;
        let mut copy = tempfile::Builder::new()
            .prefix("ketchup-program-step-")
            .suffix(".step")
            .tempfile()
            .map_err(|io| unreadable("no copy of it could be made for the worker").caused_by(io))?;
        copy.write_all(&file.bytes)
            .and_then(|()| copy.flush())
            .map_err(|io| unreadable("no copy of it could be made for the worker").caused_by(io))?;
        let sha_hex = file.sha.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok((copy, sha_hex))
    }

    /// `file` as an assembly, its parts read one by one.
    fn assembly(
        &mut self,
        file: &mut CachedFile,
    ) -> Result<Result<StepXdeImportEvidence, ketchup_scheduler::WorkerError>, Rejection> {
        if let Some(assembly) = &file.assembly {
            return Ok(Ok(assembly.clone()));
        }
        let (copy, sha_hex) = self.copy(file)?;
        let cancelled = Arc::clone(&self.cancelled);
        let read = self.worker()?.inspect_step_xde_import_with_cancellation(
            copy.path(),
            &sha_hex,
            &cancelled,
        );
        if let Ok(assembly) = &read {
            file.assembly = Some(assembly.clone());
        }
        Ok(read)
    }

    /// `file` as one body.
    fn whole(&mut self, file: &mut CachedFile) -> Result<StepImportEvidence, Rejection> {
        if let Some(whole) = &file.whole {
            return Ok(whole.clone());
        }
        let (copy, sha_hex) = self.copy(file)?;
        let cancelled = Arc::clone(&self.cancelled);
        let read =
            self.worker()?
                .inspect_step_import_with_cancellation(copy.path(), &sha_hex, &cancelled);
        let whole = match read {
            Ok(whole) => whole,
            Err(error) => {
                // An assembly whose parts are valid one by one, but not as
                // one body: say which part to take. The parts only name what
                // to choose: when they cannot be read either, the whole
                // file's error says why.
                let Some(parts) = self
                    .assembly(file)?
                    .ok()
                    .map(|assembly| assembly.parts)
                    .filter(|parts| parts.len() >= 2)
                else {
                    return Err(not_step(error));
                };
                return Err(unreadable(format!(
                    "the exact kernel cannot read it as one STEP body; it is an assembly \
                     of {} parts, so name the one to place with part=, one of {}",
                    parts.len(),
                    part_names(&parts)
                ))
                .caused_by(error));
            }
        };
        file.whole = Some(whole.clone());
        self.known.insert(file.sha, whole.clone());
        Ok(whole)
    }

    /// The part `name` of `file`, its index, body and placement.
    fn part(&mut self, file: &mut CachedFile, name: &str) -> Result<Placed, Rejection> {
        if file.assembly.is_none()
            && let Some(placed) = self.known_parts.get(&(file.sha, name.to_owned()))
        {
            return Ok(placed.clone());
        }
        let assembly = self.assembly(file)?.map_err(not_step)?;
        let mut found = assembly
            .parts
            .iter()
            .filter(|candidate| candidate.name == name);
        let Some(first) = found.next() else {
            if let Some(rejected) = assembly
                .rejected_parts
                .iter()
                .find(|rejected| rejected.name == name)
            {
                return Err(unreadable(format!(
                    "its part {name:?} cannot be read: {}",
                    rejected.reason
                )));
            }
            return placed_part(&assembly, name);
        };
        let more = found.count();
        if more > 0 {
            return Err(unreadable(format!(
                "{} of its parts are named {name:?}, so the name does not say which \
                 one; give the part its own name in the file, or export it alone",
                more + 1
            )));
        }
        let placements = assembly
            .nodes
            .iter()
            .filter(|node| node.part_index == Some(first.index))
            .map(|node| placement(&assembly, node.id))
            .collect::<Vec<_>>();
        if placements.windows(2).any(|pair| pair[0] != pair[1]) {
            return Err(unreadable(format!(
                "it places its part {name:?} {} times, so the name does not say \
                 which one; name its placement in the assembly",
                placements.len()
            )));
        }
        Ok((
            first.index,
            first.exact.clone(),
            placements.first().copied().flatten(),
        ))
    }

    /// `file` in the folder, its real path inside the real folder: a link
    /// out of it is refused like a `..` (`check_inside_program_folder`).
    fn path(folder: &Path, file: &str) -> Result<PathBuf, Rejection> {
        let real_folder = std::fs::canonicalize(folder).map_err(|error| {
            unreadable(format!(
                "the program's folder {} cannot be read",
                folder.display()
            ))
            .caused_by(error)
        })?;
        let path = std::fs::canonicalize(folder.join(file)).map_err(|error| {
            unreadable(format!("there is no such file in {}", folder.display())).caused_by(error)
        })?;
        if !path.starts_with(&real_folder) {
            return Err(unreadable(format!(
                "it leads out of the program's folder {}; copy the file into it",
                folder.display()
            )));
        }
        if !path.is_file() {
            return Err(unreadable("it is not a file"));
        }
        Ok(path)
    }

    fn worker(&mut self) -> Result<&mut ExactWorkerSupervisor, Rejection> {
        let worker = match self.worker.take() {
            Some(worker) => worker,
            None => {
                let path = self.worker_path.clone().ok_or_else(|| {
                    unreadable(
                        "the exact worker that reads STEP files is not installed next to Kečup",
                    )
                })?;
                ExactWorkerSupervisor::spawn_with_cancellation(&path, &self.cancelled).map_err(
                    |error| unreadable("the exact worker did not start").caused_by(error),
                )?
            }
        };
        Ok(self.worker.insert(worker))
    }
}

/// The exact kernel could not read the file as a STEP body.
fn not_step(error: ketchup_scheduler::WorkerError) -> Rejection {
    unreadable("the exact kernel cannot read it as a STEP body").caused_by(error)
}

/// An assembly part read: its index among the file's parts, its body in its
/// own frame, and where the assembly places it (none when nowhere else).
type Placed = (u32, StepImportEvidence, Option<Transform>);

/// The nodes from the assembly's root down to `id`, the root first.
fn path_to(assembly: &StepXdeImportEvidence, id: u32) -> Vec<&StepXdeNodeEvidence> {
    let mut path = Vec::new();
    let mut at = Some(id);
    while let Some(node) = at
        .filter(|_| path.len() <= assembly.nodes.len())
        .and_then(|id| assembly.nodes.iter().find(|node| node.id == id))
    {
        path.push(node);
        at = node.parent_id;
    }
    path.reverse();
    path
}

/// Where the assembly places the node `id`: each placement from its root
/// down to it, the root's outermost; none when that is no move.
fn placement(assembly: &StepXdeImportEvidence, id: u32) -> Option<Transform> {
    let placed = path_to(assembly, id)
        .into_iter()
        .fold(Transform::identity(), |placed, node| {
            placed.compose(node.transform)
        });
    (placed != Transform::identity()).then_some(placed)
}

/// The one part placed under the assembly node `name`, where it is placed.
/// Many CAD systems name each part's product after its shape ("SOLID") and
/// only its placement after the article, so `part=` names either.
fn placed_part(assembly: &StepXdeImportEvidence, name: &str) -> Result<Placed, Rejection> {
    let mut indices = Vec::new();
    let mut leaves = Vec::new();
    for node in &assembly.nodes {
        let Some(index) = node.part_index else {
            continue;
        };
        if path_to(assembly, node.id)
            .iter()
            .any(|above| above.name == name)
        {
            leaves.push(node.id);
            if !indices.contains(&index) {
                indices.push(index);
            }
        }
    }
    match indices.as_slice() {
        [] => {
            // A refused part's placements are left out of `nodes`, so its
            // name may be the one asked for: say which were refused and why.
            let refused = assembly
                .rejected_parts
                .iter()
                .map(|rejected| format!("{:?}: {}", rejected.name, rejected.reason))
                .collect::<Vec<_>>();
            let refused = if refused.is_empty() {
                String::new()
            } else {
                format!("; it refused {}", refused.join("; "))
            };
            Err(unreadable(format!(
                "it has no part {name:?}; its parts are {}{refused}",
                names(
                    assembly
                        .nodes
                        .iter()
                        .filter(|node| node.name_from_source)
                        .map(|node| node.name.as_str())
                        .chain(assembly.parts.iter().map(|part| part.name.as_str()))
                )
            )))
        }
        [index] => {
            if let Some(part) = assembly.parts.iter().find(|part| part.index == *index) {
                let [leaf] = leaves.as_slice() else {
                    return Err(unreadable(format!(
                        "{name:?} in it places its part {} times, so the name does not say \
                         which one; name one placement under it, or export it alone",
                        leaves.len()
                    )));
                };
                return Ok((part.index, part.exact.clone(), placement(assembly, *leaf)));
            }
            let reason = assembly
                .rejected_parts
                .iter()
                .find(|rejected| rejected.index == *index)
                .map_or("the exact kernel refused it", |rejected| {
                    rejected.reason.as_str()
                });
            Err(unreadable(format!(
                "its part {name:?} cannot be read: {reason}"
            )))
        }
        more => Err(unreadable(format!(
            "{name:?} in it holds {} parts, so the name does not say which one; \
             name one placed under it, or export it alone",
            more.len()
        ))),
    }
}

/// The distinct names of `parts`, quoted, the first few of a long list.
fn part_names(parts: &[ketchup_model::import::StepXdePartEvidence]) -> String {
    names(parts.iter().map(|part| part.name.as_str()))
}

/// The distinct `all`, quoted, the first few of a long list.
fn names<'a>(all: impl Iterator<Item = &'a str>) -> String {
    const SHOWN: usize = 20;
    let mut names = Vec::<&str>::new();
    for name in all {
        if !names.contains(&name) {
            names.push(name);
        }
    }
    let mut text = names
        .iter()
        .take(SHOWN)
        .map(|name| format!("{name:?}"))
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > SHOWN {
        text.push_str(&format!(" and {} more", names.len() - SHOWN));
    }
    text
}

impl ImportFiles for StepFiles {
    fn read(&mut self, file: &str, part: Option<&str>) -> Result<Option<ImportedBody>, Rejection> {
        let Some(folder) = self.folder.clone() else {
            return Ok(None);
        };
        let path = Self::path(&folder, file)?;
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| unreadable("the file's name is not UTF-8"))?
            .to_owned();
        let mut cached = self.cached(&path)?;
        let read = match part {
            None => self.whole(&mut cached).map(|whole| (None, whole, None)),
            Some(name) => self
                .part(&mut cached, name)
                .map(|(index, body, placement)| (Some(index), body, placement)),
        };
        // What the worker read stays known, a file it refused too.
        let (sha, bytes) = (cached.sha, Arc::clone(&cached.bytes));
        lock(&self.cache).keep(path, cached);
        let (part_index, evidence, placement) = read?;
        self.sources.insert(sha, bytes);
        Ok(Some(ImportedBody {
            file_name,
            part_index,
            evidence,
            placement,
        }))
    }
}
