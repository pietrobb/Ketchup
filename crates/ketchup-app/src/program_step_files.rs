//! Reads the STEP files a program's bought parts name (`purchased()`,
//! `imported()`) from the folder of the program's file, with the exact
//! worker, and keeps their bytes for the document to seal.

use ketchup_application::program_imports::ImportFiles;
use ketchup_model::document::{FeatureKind, Snapshot};
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{ImportFormat, STEP_PARSER_VERSION, StepImportEvidence};
use ketchup_program::imported::ImportedBody;
use ketchup_scheduler::ExactWorkerSupervisor;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// The largest STEP file a program reads; a bought part's file is a few MiB.
const MAX_STEP_BYTES: u64 = 256 * 1024 * 1024;

pub(crate) struct StepFiles {
    /// The folder of the program's file, none when the program came without it.
    folder: Option<PathBuf>,
    worker_path: Option<PathBuf>,
    worker: Option<ExactWorkerSupervisor>,
    cancelled: Arc<AtomicBool>,
    /// The bytes of each file read, by their SHA-256, for the document to seal.
    sources: BTreeMap<[u8; 32], Vec<u8>>,
    /// Whole-file bodies the document already holds, by the SHA-256 of their
    /// file: a file read again unchanged is not inspected again.
    known: BTreeMap<[u8; 32], StepImportEvidence>,
}

impl StepFiles {
    pub(crate) fn new(
        folder: Option<PathBuf>,
        worker_path: Option<PathBuf>,
        cancelled: Arc<AtomicBool>,
        snapshot: &Snapshot,
    ) -> Self {
        let known = snapshot
            .features()
            .filter_map(|feature| match feature.kind() {
                FeatureKind::ImportedExactBody(spec) if spec.source_part_index.is_none() => {
                    let receipt = snapshot.import_receipt(spec.import_id)?;
                    let topology_counts = spec.topology_counts?;
                    if receipt.format() != ImportFormat::Step
                        || receipt.parser_version() != STEP_PARSER_VERSION
                    {
                        return None;
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
                    };
                    Some((spec.source_sha256, evidence))
                }
                _ => None,
            })
            .collect();
        Self {
            folder,
            worker_path,
            worker: None,
            cancelled,
            sources: BTreeMap::new(),
            known,
        }
    }

    /// The bytes of every file read.
    pub(crate) fn into_sources(self) -> Vec<Vec<u8>> {
        self.sources.into_values().collect()
    }

    /// `file` in the folder, its real path inside the real folder: a link
    /// out of it is refused like a `..` (`check_inside_program_folder`).
    fn path(folder: &Path, file: &str) -> Result<PathBuf, String> {
        let real_folder = std::fs::canonicalize(folder).map_err(|error| {
            format!(
                "the program's folder {} cannot be read: {error}",
                folder.display()
            )
        })?;
        let path = std::fs::canonicalize(folder.join(file))
            .map_err(|error| format!("there is no such file in {} ({error})", folder.display()))?;
        if !path.starts_with(&real_folder) {
            return Err(format!(
                "it leads out of the program's folder {}; copy the file into it",
                folder.display()
            ));
        }
        if !path.is_file() {
            return Err("it is not a file".to_owned());
        }
        Ok(path)
    }

    fn worker(&mut self) -> Result<&mut ExactWorkerSupervisor, String> {
        if self.worker.is_none() {
            let path = self
                .worker_path
                .clone()
                .ok_or("the exact worker that reads STEP files is not installed next to Kečup")?;
            let worker = ExactWorkerSupervisor::spawn_with_cancellation(&path, &self.cancelled)
                .map_err(|error| format!("the exact worker did not start: {error}"))?;
            self.worker = Some(worker);
        }
        Ok(self.worker.as_mut().expect("started above"))
    }
}

impl ImportFiles for StepFiles {
    fn read(&mut self, file: &str, part: Option<&str>) -> Result<Option<ImportedBody>, String> {
        let Some(folder) = self.folder.clone() else {
            return Ok(None);
        };
        let path = Self::path(&folder, file)?;
        let length = std::fs::metadata(&path)
            .map_err(|error| error.to_string())?
            .len();
        if length > MAX_STEP_BYTES {
            return Err(format!(
                "it holds {length} bytes, more than the {MAX_STEP_BYTES} a bought part's file may"
            ));
        }
        let bytes = std::fs::read(&path).map_err(|error| error.to_string())?;
        if bytes.is_empty() {
            return Err("the file is empty".to_owned());
        }
        let sha = sha256_bytes(&bytes);
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or("the file's name is not UTF-8")?
            .to_owned();
        if part.is_none()
            && let Some(evidence) = self.known.get(&sha).cloned()
        {
            self.sources.insert(sha, bytes);
            return Ok(Some(ImportedBody {
                file_name,
                part_index: None,
                evidence,
            }));
        }
        let sha_hex = sha
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        // The worker reads a copy of the bytes hashed, not the file someone
        // may change meanwhile.
        let mut copy = tempfile::Builder::new()
            .prefix("ketchup-program-step-")
            .suffix(".step")
            .tempfile()
            .map_err(|error| error.to_string())?;
        copy.write_all(&bytes)
            .and_then(|()| copy.flush())
            .map_err(|error| error.to_string())?;
        let cancelled = Arc::clone(&self.cancelled);
        let worker = self.worker()?;
        let not_step = |error: ketchup_scheduler::WorkerError| {
            format!("the exact kernel cannot read it as a STEP body: {error}")
        };
        let (part_index, evidence) = match part {
            None => (
                None,
                worker
                    .inspect_step_import_with_cancellation(copy.path(), &sha_hex, &cancelled)
                    .map_err(not_step)?,
            ),
            Some(name) => {
                let assembly = worker
                    .inspect_step_xde_import_with_cancellation(copy.path(), &sha_hex, &cancelled)
                    .map_err(not_step)?;
                let found = assembly
                    .parts
                    .iter()
                    .find(|candidate| candidate.name == name)
                    .ok_or_else(|| {
                        let names = assembly
                            .parts
                            .iter()
                            .map(|candidate| format!("{:?}", candidate.name))
                            .collect::<Vec<_>>();
                        format!(
                            "it has no part {name:?}; its parts are {}",
                            names.join(", ")
                        )
                    })?;
                (Some(found.index), found.exact.clone())
            }
        };
        self.sources.insert(sha, bytes);
        if part.is_none() {
            self.known.insert(sha, evidence.clone());
        }
        Ok(Some(ImportedBody {
            file_name,
            part_index,
            evidence,
        }))
    }
}
