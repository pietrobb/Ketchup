//! A SketchUp `.skp` file imports as the GLB that `ketchup-skp` converts it to: the
//! review, the plan and the limits are those of a GLB import, its components and groups
//! arrive as the GLB's groups and placements. The sealed source stays the `.skp` file's
//! own bytes, so publication still checks that the file on disk is the one reviewed. The
//! SketchUp version that saved it, and what the conversion left out, are notes of the
//! review and of the import's receipt, never dropped silently.
//!
//! The reader trusts the sizes a file declares, so it runs in the program worker like a
//! DWG file's, and the window reads it on a thread of its own (see [`super::dwg_import`]).

use crate::*;
use ketchup_application::program_isolation::SkpModel;
use ketchup_model::import::{
    GlbNodeTags, GlbTag, ImportContractError, ImportDiagnostic, ImportDiagnosticSeverity,
};
use ketchup_program::CancelToken;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

/// The largest GLB a SketchUp file converts to: more than the GLB import's vertex and
/// triangle limits ever let through.
const MAX_SKP_GLB_BYTES: usize = 64 * 1024 * 1024;

/// The SketchUp files read, by the SHA-256 of their bytes, kept like the DWG files read.
static READ: Mutex<Vec<([u8; 32], Arc<SkpModel>)>> = Mutex::new(Vec::new());

/// Conversions nothing holds any more that are kept for a file read again.
const KEPT_READS: usize = 2;

pub(crate) fn is_skp_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("skp"))
}

/// The conversion of `source`, from [`READ`] or read now in the program worker.
pub(crate) fn read_skp(source: &[u8], cancel: &CancelToken) -> Result<Arc<SkpModel>, ImportError> {
    let digest = sha256_bytes(source);
    let read = || {
        READ.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    if let Some((_, model)) = read().iter().find(|(kept, _)| *kept == digest) {
        return Ok(Arc::clone(model));
    }
    let model = Arc::new(
        ketchup_application::program_isolation::convert_skp(source, MAX_SKP_GLB_BYTES, cancel)
            .map_err(|error| ImportError::failed(ImportFormat::Glb, error))?,
    );
    let mut kept = read();
    kept.retain(|(other, _)| *other != digest);
    kept.push((digest, Arc::clone(&model)));
    let mut excess = kept.len().saturating_sub(KEPT_READS);
    kept.retain(|(_, kept)| {
        let evict = excess > 0 && Arc::strong_count(kept) == 1;
        excess -= usize::from(evict);
        !evict
    });
    Ok(model)
}

/// A SketchUp file's bytes and their conversion, or why it could not be read.
pub(crate) type SkpRead = Result<(Vec<u8>, Arc<SkpModel>), ImportError>;

/// The bytes of the SketchUp file at `path` and their conversion, for a thread that
/// reads it for the window.
pub(crate) fn read_skp_file(path: &Path, cancel: &CancelToken) -> SkpRead {
    let source = read_import_source(ImportFormat::Glb, path)?;
    let model = read_skp(&source, cancel)?;
    Ok((source, model))
}

/// What reading a SketchUp file as GLB reported; holds the conversion in [`READ`] while
/// the plan that read it lives.
#[derive(Clone, Debug)]
pub(crate) struct SkpConversion(Arc<SkpModel>);

impl PartialEq for SkpConversion {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.glb == other.0.glb
    }
}

impl SkpConversion {
    /// The notes of the review and the receipt: the SketchUp version that saved the file,
    /// a warning when no test reads files of that version, and each kind of thing the GLB
    /// left out with its count.
    pub(crate) fn notes(&self) -> Result<Vec<ImportDiagnostic>, ImportContractError> {
        let model = &self.0;
        let (severity, code) = if model.verified {
            (ImportDiagnosticSeverity::Info, "skp_version_verified")
        } else {
            (ImportDiagnosticSeverity::Warning, "skp_version_unverified")
        };
        let mut notes = vec![ImportDiagnostic::new(
            severity,
            code,
            Some(model.version.clone()),
            1,
        )?];
        for left_out in &model.left_out {
            notes.push(ImportDiagnostic::new(
                ImportDiagnosticSeverity::Warning,
                left_out.code.clone(),
                None,
                u32::try_from(left_out.count).unwrap_or(u32::MAX),
            )?);
        }
        Ok(notes)
    }

    /// The SketchUp tags of the GLB's nodes, which its parts import with.
    pub(crate) fn node_tags(&self) -> GlbNodeTags {
        GlbNodeTags {
            tags: (self.0.tags.iter())
                .map(|tag| GlbTag {
                    name: tag.name.clone(),
                    visible: tag.visible,
                })
                .collect(),
            node_tags: self.0.node_tags.clone(),
        }
    }
}

/// The GLB an import of `source`, the bytes of `path`, reads: those bytes, or for a
/// `.skp` file the GLB they convert to and the conversion. A thread has read the file
/// before (the menu, the bridge's import job) and holds the conversion, so it is found
/// here; only a caller that has not read it (the in-process harness) waits for the
/// program worker.
pub(crate) fn model_glb<'a>(
    path: &Path,
    source: &'a [u8],
) -> Result<(Cow<'a, [u8]>, Option<SkpConversion>), ImportError> {
    if !is_skp_path(path) {
        return Ok((Cow::Borrowed(source), None));
    }
    let model = read_skp(source, &CancelToken::default())?;
    Ok((Cow::Owned(model.glb.clone()), Some(SkpConversion(model))))
}
