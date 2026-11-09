//! A later revision of an underlay's drawing compared with it by handle (review 2026-11-08
//! §5.3): an architect sends revision B of the plan the model was traced over. What B
//! changed, added and removed is added as an underlay of its own lying on the first, on
//! three layers in their own colors, as one Undo step, and the comparison is reported.

use super::{ReadUnderlay, UnderlayGeometry, UnderlaySnapIndex};
use crate::*;
use ketchup_model::import::{
    OverlayLayers, Revision, RevisionComparison, compare_revisions, revision_overlay,
};
use ketchup_model::underlay::{UnderlayId, UnderlayPlanError, plan_underlay_over};

/// Why a revision was not compared with an underlay.
#[derive(Debug)]
pub(crate) enum RevisionOverlayError {
    /// The document has no underlay of this id.
    NotFound(UnderlayId),
    /// The document holds no readable lines of the underlay.
    LinesUnreadable(UnderlayId),
    /// Neither drawing has an entity with a handle, so nothing tells which entity of one is
    /// which of the other.
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
                "neither drawing has entities with handles, so they cannot be compared entity by entity",
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

impl KetchupApp {
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
            self.start_underlay_read(path, None);
        }
    }

    /// Compares `read`, the drawing at `path`, with underlay `of` as its later revision and,
    /// where anything changed, adds the overlay of the changes on `of` as one Undo step.
    pub(crate) fn add_revision_overlay(
        &mut self,
        path: &Path,
        read: &ReadUnderlay,
        of: UnderlayId,
    ) -> Result<(Option<UnderlayId>, RevisionComparison), RevisionOverlayError> {
        let snapshot = self.document.current();
        let under = snapshot
            .underlay(of)
            .ok_or(RevisionOverlayError::NotFound(of))?;
        let earlier_lines = self
            .cached_decoded_underlay_lines(under)
            .ok_or(RevisionOverlayError::LinesUnreadable(of))?;
        let earlier = Revision {
            lines: &earlier_lines,
            offset_mm: under.offset_mm(),
        };
        let later = Revision {
            lines: &read.lines,
            offset_mm: read.read.offset_mm,
        };
        let comparison = compare_revisions(earlier, later);
        if comparison.compared_nothing() {
            return Err(RevisionOverlayError::NoHandles);
        }
        let counts = BTreeMap::from([
            ("changed", comparison.changed.len().to_string()),
            ("added", comparison.added.len().to_string()),
            ("removed", comparison.removed.len().to_string()),
            ("unchanged", comparison.unchanged.to_string()),
        ]);
        if comparison.changed.is_empty()
            && comparison.added.is_empty()
            && comparison.removed.is_empty()
        {
            self.digest = self
                .catalog
                .format("digest-underlay-revision-same", &counts);
            return Ok((None, comparison));
        }
        let names = OverlayLayers {
            changed: self.catalog.text("underlay-revision-changed"),
            added: self.catalog.text("underlay-revision-added"),
            removed: self.catalog.text("underlay-revision-removed"),
        };
        let (layers, lines) = revision_overlay(earlier, later, &comparison, &names);
        let bytes = lines
            .encode_readable()
            .map_err(RevisionOverlayError::TooLarge)?;
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
        let key = sha256_bytes(&bytes);
        let (batch, id) =
            plan_underlay_over(&snapshot, of, &name, &layers, key, read.read.offset_mm)
                .map_err(RevisionOverlayError::Plan)?;
        let snaps = Arc::new(UnderlaySnapIndex::new(&lines));
        let geometry = Arc::new(UnderlayGeometry::new(key, &lines));
        self.keep_underlay_lines(batch, bytes, &lines, snaps, geometry)
            .map_err(RevisionOverlayError::Import)?;
        self.digest = self.catalog.format("digest-underlay-revision", &counts);
        Ok((Some(id), comparison))
    }
}
