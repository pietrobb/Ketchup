//! Answers of the exact solids for pairs of parts whose boxes misstate them.
//!
//! The program checks measure boxes. A profile body, a moved face or a
//! boolean makes a part's solid smaller than its box, so two boxes can touch
//! or overlap while the solids stay apart. The application measures such
//! pairs on the built solids and hands the answers back here; collisions,
//! floating parts, joints, relations and expectations then use them instead
//! of the boxes.

use crate::expect::Measure;
use crate::model::{Part, ProgramModel};
use crate::validate;
use ketchup_model::tolerance::CONTACT_FIT_MM;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What the exact solids of two parts share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize, Serialize)]
pub struct ExactPair {
    pub common_volume_mm3: f64,
    /// How deep the solids overlap: the thickness of the thickest solid they
    /// share, as twice its volume over its surface area. Zero when unknown.
    pub common_thickness_mm: f64,
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
        !self.penetrating() && self.distance_mm.is_some_and(|d| d <= CONTACT_FIT_MM)
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
/// measured, unless one of its parts was checked against every other part.
#[derive(Clone, Debug, Default, PartialEq, Deserialize, Serialize)]
#[serde(from = "ShapesWire", into = "ShapesWire")]
pub struct ExactShapes {
    pairs: BTreeMap<(String, String), ExactPair>,
    /// Parts checked against every part of `against`: their pairs among
    /// those without an answer are exactly apart. One name each instead of
    /// one answer per pair, which for a model of 750 parts did not fit a
    /// worker request.
    checked: BTreeSet<String>,
    against: BTreeSet<String>,
}

#[derive(Deserialize, Serialize)]
struct ShapesWire {
    pairs: Vec<(String, String, ExactPair)>,
    checked: BTreeSet<String>,
    against: BTreeSet<String>,
}

impl From<ShapesWire> for ExactShapes {
    fn from(wire: ShapesWire) -> Self {
        let mut shapes = Self {
            checked: wire.checked,
            against: wire.against,
            ..Self::default()
        };
        for (a, b, pair) in wire.pairs {
            shapes.insert(&a, &b, pair);
        }
        shapes
    }
}

impl From<ExactShapes> for ShapesWire {
    fn from(shapes: ExactShapes) -> Self {
        Self {
            pairs: shapes
                .pairs
                .into_iter()
                .map(|((a, b), pair)| (a, b, pair))
                .collect(),
            checked: shapes.checked,
            against: shapes.against,
        }
    }
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

    /// Records that every part of `checked` was checked against every part
    /// of `against`: their pairs without an answer are apart.
    pub fn checked_against(
        &mut self,
        checked: impl IntoIterator<Item = String>,
        against: impl IntoIterator<Item = String>,
    ) {
        self.checked.extend(checked);
        self.against.extend(against);
    }

    /// The exact answer for a pair, or `None` when it was not measured.
    #[must_use]
    pub fn pair(&self, a: &str, b: &str) -> Option<ExactPair> {
        let apart = |a: &str, b: &str| self.checked.contains(a) && self.against.contains(b);
        self.pairs
            .get(&key(a, b))
            .copied()
            .or_else(|| (a != b && (apart(a, b) || apart(b, a))).then(ExactPair::default))
    }

    /// The exact answer for two parts under the program's contact rule: see
    /// [`within_contact`].
    #[must_use]
    pub fn between(&self, a: &Part, b: &Part) -> Option<ExactPair> {
        self.pair(&a.name, &b.name).map(within_contact)
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
/// overlap by no more than [`CONTACT_FIT_MM`] touch, so solids whose common
/// solid is no thicker than that touch too. The thickness is the common
/// solid's own, not the face where the parts' boxes overlap: a box holding
/// the other part whole (an open box, a frame) makes that face large while a
/// corner cuts deep into a wall (review status N-1).
fn within_contact(pair: ExactPair) -> ExactPair {
    if pair.common_volume_mm3 <= 0.0 {
        return pair;
    }
    if pair.common_thickness_mm > 0.0 && pair.common_thickness_mm <= CONTACT_FIT_MM {
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
            if bounds[right].0[0] > bounds[left].1[0] + CONTACT_FIT_MM {
                break;
            }
            let (a, b) = (&parts[left], &parts[right]);
            if box_is_solid(a) && box_is_solid(b) {
                continue;
            }
            if a.obb().separation(&b.obb()) > CONTACT_FIT_MM {
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
