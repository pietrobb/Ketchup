//! A later revision of an underlay's drawing compared with it by handle (review 2026-11-08
//! §5.3): an architect sends revision B of the plan the model was traced over. What B
//! changed, added and removed is added as an underlay of its own lying on the first, on
//! three layers in their own colors, as one Undo step, and the comparison is reported.

use super::{ReadUnderlay, UnderlayGeometry, UnderlaySnapIndex};
use crate::*;
use ketchup_model::import::{
    OverlayLayers, Revision, RevisionComparison, compare_revisions, revision_overlay,
};
use ketchup_model::underlay::{UnderlayId, UnderlayLines, UnderlayPlanError, plan_underlay_over};

/// Why a revision was not compared with an underlay.
#[derive(Debug)]
pub(crate) enum RevisionOverlayError {
    /// The document has no underlay of this id.
    NotFound(UnderlayId),
    /// The document holds no readable lines of the underlay.
    LinesUnreadable(UnderlayId),
    /// One drawing, or both, has no entity with a handle, so nothing tells which entity of
    /// one is which of the other.
    NoHandles,
    Plan(UnderlayPlanError),
    Import(ImportError),
    /// The overlay's lines are more than an underlay keeps.
    TooLarge(ketchup_model::underlay::UnderlayLinesError),
}

impl std::fmt::Display for RevisionOverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound(id) => write!(formatter, "the document has no underlay {}", id.0),
            Self::LinesUnreadable(id) => write!(
                formatter,
                "the document holds no readable lines of underlay {}",
                id.0
            ),
            Self::NoHandles => formatter.write_str(
                "one of the drawings has no entities with handles, so they cannot be compared entity by entity",
            ),
            Self::Plan(_) => formatter.write_str("the overlay of the changes was not planned"),
            Self::Import(_) => formatter.write_str("the overlay of the changes was not kept"),
            Self::TooLarge(_) => formatter.write_str("the overlay of the changes is too large"),
        }
    }
}

impl std::error::Error for RevisionOverlayError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Plan(error) => Some(error),
            Self::Import(error) => Some(error),
            Self::TooLarge(error) => Some(error),
            _ => None,
        }
    }
}

/// The underlay a drawing being read is a later revision of, handed to the thread that
/// reads it: its lines as read, without edits, and the names of the overlay's layers.
pub(crate) struct EarlierRevision {
    lines: Arc<UnderlayLines>,
    /// The blob the lines were decoded from, to tell whether the underlay still has them
    /// when the revision is added.
    lines_sha256: [u8; 32],
    offset_mm: [f64; 2],
    names: OverlayLayers,
}

/// What comparing a revision with an underlay makes, on the thread that read the revision
/// (review 2026-11-10 og:R1110-UL-402: the overlay of a plan of 100 000 lines froze the
/// window for 22.7 s): the comparison and, where anything changed, the overlay.
pub(crate) struct RevisionWork {
    comparison: RevisionComparison,
    lines_sha256: [u8; 32],
    overlay: Option<RevisionOverlay>,
}

/// The overlay of the changes as the document keeps it, with its snap index and GPU lines.
struct RevisionOverlay {
    layers: Vec<String>,
    lines: UnderlayLines,
    bytes: Vec<u8>,
    snaps: Arc<UnderlaySnapIndex>,
    geometry: Arc<UnderlayGeometry>,
}

/// Compares `later`, read, with `earlier` and makes the overlay of what changed.
pub(crate) fn revision_work(
    earlier: &EarlierRevision,
    later: &ReadUnderlay,
) -> Result<RevisionWork, RevisionOverlayError> {
    let (earlier_revision, later_revision) = (
        Revision {
            lines: &earlier.lines,
            offset_mm: earlier.offset_mm,
        },
        Revision {
            lines: &later.lines,
            offset_mm: later.read.offset_mm,
        },
    );
    let comparison = compare_revisions(earlier_revision, later_revision);
    if comparison.lacks_handles() {
        return Err(RevisionOverlayError::NoHandles);
    }
    let changed = !(comparison.changed.is_empty()
        && comparison.added.is_empty()
        && comparison.removed.is_empty());
    let overlay = changed
        .then(|| {
            let (layers, lines) = revision_overlay(
                earlier_revision,
                later_revision,
                &comparison,
                &earlier.names,
            );
            let bytes = lines
                .encode_readable()
                .map_err(RevisionOverlayError::TooLarge)?;
            let snaps = Arc::new(UnderlaySnapIndex::new(&lines));
            let geometry = Arc::new(UnderlayGeometry::new(sha256_bytes(&bytes), &lines));
            Ok(RevisionOverlay {
                layers,
                lines,
                bytes,
                snaps,
                geometry,
            })
        })
        .transpose()?;
    Ok(RevisionWork {
        comparison,
        lines_sha256: earlier.lines_sha256,
        overlay,
    })
}

impl KetchupApp {
    /// Underlay `of` as the earlier revision of a drawing to be read.
    pub(crate) fn earlier_revision(
        &self,
        of: UnderlayId,
    ) -> Result<EarlierRevision, RevisionOverlayError> {
        let snapshot = self.document.current();
        let under = snapshot
            .underlay(of)
            .ok_or(RevisionOverlayError::NotFound(of))?;
        let lines = self
            .cached_decoded_underlay_lines(under)
            .ok_or(RevisionOverlayError::LinesUnreadable(of))?;
        Ok(EarlierRevision {
            lines,
            lines_sha256: under.lines_sha256,
            offset_mm: under.offset_mm(),
            names: OverlayLayers {
                changed: self.catalog.text("underlay-revision-changed"),
                added: self.catalog.text("underlay-revision-added"),
                removed: self.catalog.text("underlay-revision-removed"),
            },
        })
    }

    /// File ▸ Import ▸ revision of the underlay: the drawing chosen is compared with the
    /// underlay selected, or with the only one the document has.
    pub(crate) fn begin_underlay_revision(&mut self) {
        let snapshot = self.document.current();
        let only = {
            let mut underlays = snapshot.underlays();
            match (underlays.next(), underlays.next()) {
                (Some(underlay), None) => Some(underlay.id),
                _ => None,
            }
        };
        let Some(of) = self.selected_underlay().or(only) else {
            self.digest = self.catalog.text("digest-underlay-revision-select");
            return;
        };
        if let Some(path) = self.choose_dxf_import_path() {
            self.file.activity.underlay.revision_of = Some(of);
            let code_page = snapshot.underlay(of).and_then(|under| under.code_page);
            self.start_underlay_read(path, None, code_page);
        }
    }

    /// Compares `read`, the drawing at `path`, with underlay `of` as its later revision and,
    /// where anything changed, adds the overlay of the changes on `of` as one Undo step.
    /// The comparison the reading thread made is taken while the underlay still has the
    /// lines it compared; it is made here otherwise.
    pub(crate) fn add_revision_overlay(
        &mut self,
        path: &Path,
        mut read: ReadUnderlay,
        of: UnderlayId,
    ) -> Result<(Option<UnderlayId>, RevisionComparison), RevisionOverlayError> {
        let snapshot = self.document.current();
        let under = snapshot
            .underlay(of)
            .ok_or(RevisionOverlayError::NotFound(of))?;
        let work = match read.revision.take() {
            Some(Err(error)) => return Err(error),
            Some(Ok(work)) if work.lines_sha256 == under.lines_sha256 => work,
            _ => revision_work(&self.earlier_revision(of)?, &read)?,
        };
        let comparison = work.comparison;
        // A renumbered entity draws alike, under the handle the later export gave it.
        let alike = comparison.unchanged + comparison.renumbered;
        let counts = BTreeMap::from([
            ("changed", comparison.changed.len().to_string()),
            ("added", comparison.added.len().to_string()),
            ("removed", comparison.removed.len().to_string()),
            ("unchanged", alike.to_string()),
        ]);
        let Some(overlay) = work.overlay else {
            self.digest = self
                .catalog
                .format("digest-underlay-revision-same", &counts);
            return Ok((None, comparison));
        };
        let revision = path
            .file_stem()
            .map_or_else(|| "revision".into(), |stem| stem.to_string_lossy());
        let name = self.catalog.format(
            "underlay-revision-name",
            &BTreeMap::from([
                ("underlay", under.name.clone()),
                ("revision", revision.into_owned()),
            ]),
        );
        let RevisionOverlay {
            layers,
            lines,
            bytes,
            snaps,
            geometry,
        } = overlay;
        let key = sha256_bytes(&bytes);
        let (batch, id) =
            plan_underlay_over(&snapshot, of, &name, &layers, key, read.read.offset_mm)
                .map_err(RevisionOverlayError::Plan)?;
        self.keep_underlay_lines(
            (batch, ImportFormat::Dxf.into()),
            bytes,
            &lines,
            (snaps, geometry),
            None,
        )
        .map_err(RevisionOverlayError::Import)?;
        self.digest = self.catalog.format("digest-underlay-revision", &counts);
        Ok((Some(id), comparison))
    }
}
