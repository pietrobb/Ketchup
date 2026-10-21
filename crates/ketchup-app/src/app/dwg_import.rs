//! A `.dwg` drawing imports as the DXF that `ketchup-dwg` converts it to: the
//! review, the plan and the DXF count limits are those of a DXF import. The
//! sealed source stays the DWG file's own bytes, so publication still checks
//! that the file on disk is the one reviewed. What the DWG reader could not
//! decode is listed in the review, never dropped silently.
//!
//! The reader allocates what a file declares, so it runs in the program worker
//! (review SEC-5): a damaged file costs that process, not the window. The window
//! never waits for it (DR-3): the menu and a client's import read the file on a
//! thread of their own, and hold the conversion while its review is open, so the
//! review, its plan and the publication find it here instead of reading the file
//! again on the window's thread.

use crate::*;
use ketchup_dwg::DwgDrawing;
use ketchup_program::CancelToken;
use std::borrow::Cow;
use std::sync::{Arc, Mutex};

/// The DWG files read, by the SHA-256 of their bytes: every one a review or an
/// import still holds, and the last [`KEPT_READS`] besides.
static READ: Mutex<Vec<([u8; 32], Arc<DwgDrawing>)>> = Mutex::new(Vec::new());

/// Conversions nothing holds any more that are kept for a file read again.
const KEPT_READS: usize = 2;

/// The conversion of `source`, from [`READ`] or read now in the program worker.
pub(crate) fn read_dwg(
    source: &[u8],
    cancel: &CancelToken,
) -> Result<Arc<DwgDrawing>, ImportError> {
    let digest = sha256_bytes(source);
    let read = || {
        READ.lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    };
    if let Some((_, drawing)) = read().iter().find(|(kept, _)| *kept == digest) {
        return Ok(Arc::clone(drawing));
    }
    let drawing = Arc::new(
        ketchup_application::program_isolation::convert_dwg(source, cancel)
            .map_err(|error| ImportError::failed(ImportFormat::Dxf, error))?,
    );
    let mut kept = read();
    kept.retain(|(other, _)| *other != digest);
    kept.push((digest, Arc::clone(&drawing)));
    // Oldest first, and never one still held: publishing an open review must not
    // read its file again on the window's thread.
    let mut excess = kept.len().saturating_sub(KEPT_READS);
    kept.retain(|(_, kept)| {
        let evict = excess > 0 && Arc::strong_count(kept) == 1;
        excess -= usize::from(evict);
        !evict
    });
    Ok(drawing)
}

/// A DWG file's bytes and their conversion, or why it could not be read.
pub(crate) type DwgRead = Result<(Vec<u8>, Arc<DwgDrawing>), ImportError>;

/// The bytes of the DWG file at `path` and their conversion, for a thread that
/// reads it for the window.
pub(crate) fn read_dwg_file(path: &Path, cancel: &CancelToken) -> DwgRead {
    let source = KetchupApp::read_dxf_source(path)?;
    let drawing = read_dwg(&source, cancel)?;
    Ok((source, drawing))
}

/// A DWG file being read for the import review, off the window's thread.
pub(crate) struct DwgReadTask {
    path: PathBuf,
    cancel: CancelToken,
    receiver: std::sync::mpsc::Receiver<DwgRead>,
}

impl KetchupApp {
    /// Reads the DWG file at `path` on a thread; [`Self::poll_dwg_read`] opens
    /// the review when it is read.
    pub(crate) fn start_dwg_read(&mut self, path: PathBuf, context: &egui::Context) {
        let (sender, receiver) = std::sync::mpsc::channel();
        let cancel = CancelToken::default();
        let (thread_path, thread_cancel, repaint) = (path.clone(), cancel.clone(), context.clone());
        std::thread::spawn(move || {
            if sender
                .send(read_dwg_file(&thread_path, &thread_cancel))
                .is_ok()
            {
                repaint.request_repaint();
            }
        });
        self.file.activity.dwg_reading = Some(DwgReadTask {
            path,
            cancel,
            receiver,
        });
    }

    pub(super) fn poll_dwg_read(&mut self, context: &egui::Context) {
        let Some(task) = self.file.activity.dwg_reading.as_ref() else {
            return;
        };
        let result = match task.receiver.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Err(ImportError::failed(
                ImportFormat::Dxf,
                "the thread that reads the DWG file stopped",
            )),
        };
        let task = self.file.activity.dwg_reading.take().expect("polled task");
        match result {
            // Held until the review's plan holds it.
            Ok((source, _held)) => self.open_drawing_import(task.path, source),
            Err(error) => self.report_import_failure(&task.path, &error),
        }
        context.request_repaint();
    }

    /// Stops reading a DWG file; the worker reading it is killed.
    pub(super) fn cancel_dwg_read(&mut self) {
        if let Some(task) = self.file.activity.dwg_reading.take() {
            task.cancel.cancel();
        }
    }
}

/// What reading a DWG file as DXF reported.
#[derive(Clone, Debug)]
pub(crate) struct DwgConversion {
    /// The file's version code, for example `AC1032`.
    pub(crate) version: String,
    /// Records the DWG reader could not decode and left out, most frequent first.
    pub(crate) skipped: Vec<ketchup_dwg::SkippedRecords>,
    /// Keeps the conversion in [`READ`] while the plan that read it lives.
    drawing: Arc<DwgDrawing>,
}

impl PartialEq for DwgConversion {
    fn eq(&self, other: &Self) -> bool {
        self.version == other.version
            && self.skipped == other.skipped
            && (Arc::ptr_eq(&self.drawing, &other.drawing) || self.drawing.dxf == other.drawing.dxf)
    }
}

impl Eq for DwgConversion {}

pub(crate) fn is_dwg_path(path: &Path) -> bool {
    path.extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("dwg"))
}

/// The DXF a drawing import reads from `source`, the bytes of `path`: those
/// bytes, or for a `.dwg` file the DXF they convert to. The window has read the file on
/// a thread before ([`KetchupApp::start_dwg_read`], the bridge's DWG import) and
/// holds the conversion, so it is found here; only a caller that has not read it
/// (the in-process harness) waits for the program worker.
pub(crate) fn drawing_dxf<'a>(
    path: &Path,
    source: &'a [u8],
) -> Result<(Cow<'a, [u8]>, Option<DwgConversion>), ImportError> {
    if !is_dwg_path(path) {
        return Ok((Cow::Borrowed(source), None));
    }
    let drawing = read_dwg(source, &CancelToken::default())?;
    Ok((
        Cow::Owned(drawing.dxf.clone()),
        Some(DwgConversion {
            version: drawing.version.clone(),
            skipped: drawing.skipped.clone(),
            drawing: Arc::clone(&drawing),
        }),
    ))
}

/// A millimetre drawing on three declared layers: an open line and arc, a
/// closed polyline with a bulge and a circle.
#[cfg(test)]
pub(crate) const LAYERED_DRAWING_DXF: &str = concat!(
    "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
    "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n3\n",
    "0\nLAYER\n2\noutline\n70\n0\n62\n7\n6\nCONTINUOUS\n",
    "0\nLAYER\n2\ncut\n70\n0\n62\n1\n6\nCONTINUOUS\n",
    "0\nLAYER\n2\nholes\n70\n0\n62\n3\n6\nCONTINUOUS\n",
    "0\nENDTAB\n0\nENDSEC\n",
    "0\nSECTION\n2\nENTITIES\n",
    "0\nLINE\n8\noutline\n10\n0\n20\n0\n11\n10\n21\n0\n",
    "0\nARC\n8\noutline\n10\n10\n20\n10\n40\n10\n50\n270\n51\n0\n",
    "0\nLWPOLYLINE\n8\ncut\n90\n4\n70\n1\n",
    "10\n0\n20\n0\n42\n0\n10\n10\n20\n0\n42\n1\n10\n10\n20\n10\n42\n0\n10\n0\n20\n10\n42\n0\n",
    "0\nCIRCLE\n8\nholes\n10\n50\n20\n50\n40\n7.5\n",
    "0\nENDSEC\n0\nEOF\n",
);
