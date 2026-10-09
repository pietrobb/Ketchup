//! Declared intent: conditions a program states about its own geometry
//! ("the back is flush with the seat", "3 mm between the doors"). Each is a
//! comparison of a sum of generic measurements on the final model, so the
//! library states new kinds of intent without Rust changes. A condition that
//! does not hold is an error with the measured and required numbers.

use crate::contact::contact;
use crate::eval::TOLERANCE_MM;
use crate::exact::ExactShapes;
use crate::frame;
use crate::model::{Part, ProgramModel};
use crate::relations::polygon_area;
use crate::validate::{Issue, Severity};
use serde::Serialize;

/// A world direction, fixed or following a face of a part wherever the part
/// ends up.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    World([f64; 3]),
    Face { part: String, face: String },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "measure", rename_all = "snake_case")]
pub enum Measure {
    /// How far `part` reaches along `direction` (see `reach()`).
    Reach { part: String, direction: Direction },
    /// Clearance between the parts; negative: how deep they overlap.
    Distance { a: String, b: String },
    /// Area over which the parts touch face to face (only through `face` of
    /// `a` when given).
    ContactArea {
        a: String,
        b: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        face: Option<String>,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum Comparison {
    #[serde(rename = "==")]
    Equal,
    #[serde(rename = "<=")]
    AtMost,
    #[serde(rename = ">=")]
    AtLeast,
    #[serde(rename = ">")]
    Above,
}

impl Comparison {
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "==" => Some(Self::Equal),
            "<=" => Some(Self::AtMost),
            ">=" => Some(Self::AtLeast),
            ">" => Some(Self::Above),
            _ => None,
        }
    }

    const fn symbol(self) -> &'static str {
        match self {
            Self::Equal => "==",
            Self::AtMost => "<=",
            Self::AtLeast => ">=",
            Self::Above => ">",
        }
    }

    fn holds(self, measured: f64, required: f64, tolerance: f64) -> bool {
        match self {
            Self::Equal => (measured - required).abs() <= tolerance,
            Self::AtMost => measured <= required + tolerance,
            Self::AtLeast => measured >= required - tolerance,
            Self::Above => measured > required + tolerance,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Expectation {
    pub name: String,
    /// `Σ coefficient · measure`.
    pub terms: Vec<(f64, Measure)>,
    pub comparison: Comparison,
    pub value: f64,
    pub tolerance: f64,
    /// Unit of the measured sum, for the message ("mm", "mm²").
    pub unit: String,
    pub parts: Vec<String>,
    pub hint: String,
}

fn find<'m>(model: &'m ProgramModel, name: &str) -> Option<&'m Part> {
    model.part(name).or_else(|| model.tool(name))
}

fn direction(model: &ProgramModel, direction: &Direction) -> Option<[f64; 3]> {
    match direction {
        Direction::World(vector) => frame::normalized(*vector),
        Direction::Face { part, face } => {
            let part = find(model, part)?;
            Some(part.world_face(&part.face_frame(face).ok()?).normal)
        }
    }
}

fn measure(model: &ProgramModel, exact: &ExactShapes, measure: &Measure) -> Option<f64> {
    Some(match measure {
        Measure::Reach { part, direction: d } => find(model, part)?.reach(direction(model, d)?),
        Measure::Distance { a, b } => {
            let (a, b) = (find(model, a)?, find(model, b)?);
            let (box_a, box_b) = (a.obb(), b.obb());
            let separation = box_a.separation(&box_b);
            match exact.decides(a, b) {
                // Apart by more than the tolerance, but by how much was not
                // measured: `check` reports the condition as unverified.
                Some(pair) if !pair.penetrating() => {
                    return Some(pair.gap_mm().unwrap_or(f64::NAN));
                }
                _ => {}
            }
            if separation > TOLERANCE_MM {
                box_a.distance(&box_b)
            } else {
                separation.min(0.0)
            }
        }
        Measure::ContactArea { a, b, face } => {
            let (a, b) = (find(model, a)?, find(model, b)?);
            let patch = contact(a, b);
            let on_face = face
                .as_ref()
                .is_none_or(|face| patch.as_ref().is_some_and(|p| &p.face_a == face));
            match (exact.decides(a, b), patch) {
                (Some(pair), _) if on_face => pair.contact_area_mm2,
                (Some(_), _) => 0.0,
                (None, Some(patch)) if on_face => polygon_area(&patch.points_mm),
                (None, _) => 0.0,
            }
        }
    })
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

/// One error per condition that does not hold.
pub fn check(model: &ProgramModel, exact: &ExactShapes, issues: &mut Vec<Issue>) {
    for expectation in &model.expectations {
        let measured: Option<f64> = expectation
            .terms
            .iter()
            .map(|(coefficient, term)| measure(model, exact, term).map(|value| coefficient * value))
            .sum();
        let Some(measured) = measured else {
            // A part or face the final model does not have: the condition
            // cannot be measured and must not look met.
            issues.push(Issue {
                source_lines: Vec::new(),
                severity: Severity::Error,
                kind: "expectation_unmeasurable",
                parts: expectation.parts.clone(),
                message: format!(
                    "{}: a measure names a part or face the model does not have",
                    expectation.name
                ),
                where_mm: None,
                hint: "Name an existing part and one of its faces.".to_owned(),
            });
            continue;
        };
        if measured.is_nan() {
            issues.push(Issue {
                source_lines: Vec::new(),
                severity: Severity::Warning,
                kind: "expectation_unverified",
                parts: expectation.parts.clone(),
                message: format!(
                    "{}: the exact solids are apart, but their distance was not measured",
                    expectation.name
                ),
                where_mm: None,
                hint: expectation.hint.clone(),
            });
            continue;
        }
        if expectation
            .comparison
            .holds(measured, expectation.value, expectation.tolerance)
        {
            continue;
        }
        let unit = &expectation.unit;
        issues.push(Issue {
            source_lines: Vec::new(),
            severity: Severity::Error,
            kind: "expectation_failed",
            parts: expectation.parts.clone(),
            message: format!(
                "{}: measured {} {unit}, required {} {} {unit} (tolerance {} {unit})",
                expectation.name,
                round(measured),
                expectation.comparison.symbol(),
                round(expectation.value),
                round(expectation.tolerance),
            ),
            where_mm: None,
            hint: expectation.hint.clone(),
        });
    }
}
