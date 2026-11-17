//! The Loft of the legacy formats, which knew no rail.

use super::{LegacyError, PersistenceError, Reader};
use crate::document::{FeatureId, LoftContinuity, LoftSection};

/// A Loft's sections then, when the format stores them, its guide and continuity.
pub(super) fn read(
    reader: &mut Reader<'_>,
    guide_and_continuity: bool,
) -> Result<(Vec<LoftSection>, Option<FeatureId>, LoftContinuity), PersistenceError> {
    let mut sections = Vec::new();
    for _ in 0..reader.count_with_limit(16)? {
        sections.push(LoftSection {
            profile: FeatureId(reader.u64()?),
            elevation_mm: f64::from_bits(reader.u64()?),
        });
    }
    if !guide_and_continuity {
        return Ok((sections, None, LoftContinuity::Position));
    }
    let guide = match reader.u8()? {
        0 => None,
        1 => Some(FeatureId(reader.u64()?)),
        value => return Err(PersistenceError::InvalidBoolean(value)),
    };
    let continuity = match reader.u8()? {
        1 => LoftContinuity::Position,
        2 => LoftContinuity::Tangent,
        3 => LoftContinuity::Curvature,
        value => {
            return Err(PersistenceError::Legacy(LegacyError::InvalidFeatureKind(
                value,
            )));
        }
    };
    Ok((sections, guide, continuity))
}
