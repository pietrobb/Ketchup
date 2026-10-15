//! Answers of the exact solids for pairs of parts whose boxes misstate them.
//!
//! The program checks measure boxes. A profile body, a moved face or a
//! boolean makes a part's solid smaller than its box, so two boxes can touch
//! or overlap while the solids stay apart. The application measures such
//! pairs on the built solids and hands the answers back here; collisions,
//! floating parts, joints, relations and expectations then use them instead
//! of the boxes.

use crate::eval::TOLERANCE_MM;
use crate::expect::Measure;
use crate::frame;
use crate::model::{Part, ProgramModel};
use crate::validate;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// What the exact solids of two parts share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct ExactPair {
    pub common_volume_mm3: f64,
    /// Area of the faces the solids share without overlapping.
    pub contact_area_mm2: f64,
    /// Smallest distance between the solids; `None` when only known to be
    /// larger than the contact tolerance.
    pub distance_mm: Option<f64>,
}

impl ExactPair {
    #[must_use]
    pub fn penetrating(&self) -> bool {
        self.common_volume_mm3 > 0.0
    }

    /// No common volume, but no gap either (face, edge or point contact).
    #[must_use]
    pub fn touching(&self) -> bool {
        !self.penetrating() && self.distance_mm.is_some_and(|d| d <= TOLERANCE_MM)
    }

    /// Clearance between the solids, when they are apart and it is known.
    #[must_use]
    pub fn gap_mm(&self) -> Option<f64> {
        if self.penetrating() || self.touching() {
            return Some(0.0);
        }
        self.distance_mm
    }
}

/// Exact answers by the names of two parts; a pair without one was not
/// measured.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExactShapes {
    pairs: BTreeMap<(String, String), ExactPair>,
}

fn key(a: &str, b: &str) -> (String, String) {
    if a <= b {
        (a.to_owned(), b.to_owned())
    } else {
        (b.to_owned(), a.to_owned())
    }
}

impl ExactShapes {
    pub fn insert(&mut self, a: &str, b: &str, pair: ExactPair) {
        self.pairs.insert(key(a, b), pair);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.pairs.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pairs.is_empty()
    }

    /// The exact answer for a pair, or `None` when it was not measured.
    #[must_use]
    pub fn pair(&self, a: &str, b: &str) -> Option<ExactPair> {
        self.pairs.get(&key(a, b)).copied()
    }

    /// The exact answer for two parts under the program's contact rule: see
    /// [`within_contact`].
    #[must_use]
    pub fn between(&self, a: &Part, b: &Part) -> Option<ExactPair> {
        self.pair(&a.name, &b.name)
            .map(|pair| within_contact(pair, a, b))
    }

    /// The exact answer when the boxes of `a` and `b` misstate their solids.
    #[must_use]
    pub fn decides(&self, a: &Part, b: &Part) -> Option<ExactPair> {
        if box_is_solid(a) && box_is_solid(b) {
            return None;
        }
        self.between(a, b)
    }
}

/// One contact rule for boxes and exact solids (review PRG-5): boxes that
/// overlap by no more than [`TOLERANCE_MM`] touch, so solids sharing no more
/// volume than a slab that thick across the face where their boxes overlap
/// touch too. The face is measured in the frame of either part, whichever
/// fits the overlap tighter.
fn within_contact(pair: ExactPair, a: &Part, b: &Part) -> ExactPair {
    if pair.common_volume_mm3 <= 0.0 {
        return pair;
    }
    let shared = frame::intersection_vertices(&[a.obb(), b.obb()], TOLERANCE_MM);
    let face_mm2 = [a, b]
        .into_iter()
        .map(|part| {
            let mut extent: [f64; 3] = std::array::from_fn(|axis| {
                let along = shared.iter().map(|vertex| part.to_local(*vertex)[axis]);
                along.clone().fold(f64::NEG_INFINITY, f64::max)
                    - along.fold(f64::INFINITY, f64::min)
            });
            extent.sort_by(f64::total_cmp);
            extent[1] * extent[2]
        })
        .fold(f64::INFINITY, f64::min);
    if face_mm2.is_finite() && pair.common_volume_mm3 <= TOLERANCE_MM * face_mm2 {
        ExactPair {
            common_volume_mm3: 0.0,
            distance_mm: Some(0.0),
            ..pair
        }
    } else {
        pair
    }
}

/// Whether the part's solid is exactly its box.
#[must_use]
pub fn box_is_solid(part: &Part) -> bool {
    validate::is_box(part) && part.operations.is_empty()
}

/// Explicit distance expectations need native measurements regardless of machining,
/// even when bounds prove that the parts are separated.
#[must_use]
pub fn measured_pairs(model: &ProgramModel) -> BTreeSet<(String, String)> {
    model
        .expectations
        .iter()
        .flat_map(|expectation| &expectation.terms)
        .filter_map(|(_, term)| {
            let Measure::Distance { a, b } = term else {
                return None;
            };
            model.part(a).zip(model.part(b))?;
            Some(key(a, b))
        })
        .collect()
}

/// Parts requiring native collision or explicitly requested distance checks.
#[must_use]
pub fn exact_candidates(model: &ProgramModel) -> BTreeSet<String> {
    let parts = &model.parts;
    let bounds: Vec<_> = parts.iter().map(Part::world_bounds).collect();
    let mut order: Vec<usize> = (0..parts.len()).collect();
    order.sort_by(|left, right| bounds[*left].0[0].total_cmp(&bounds[*right].0[0]));
    let mut names: BTreeSet<String> = measured_pairs(model)
        .into_iter()
        .flat_map(|(a, b)| [a, b])
        .collect();
    for (position, &left) in order.iter().enumerate() {
        for &right in &order[position + 1..] {
            if bounds[right].0[0] > bounds[left].1[0] + TOLERANCE_MM {
                break;
            }
            let (a, b) = (&parts[left], &parts[right]);
            if box_is_solid(a) && box_is_solid(b) {
                continue;
            }
            if a.obb().separation(&b.obb()) > TOLERANCE_MM {
                continue;
            }
            for part in [a, b] {
                if !box_is_solid(part) {
                    names.insert(part.name.clone());
                }
            }
        }
    }
    names
}
