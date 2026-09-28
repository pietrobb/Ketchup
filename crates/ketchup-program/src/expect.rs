//! Declared intent: conditions a program states about its own geometry
//! ("the back is flush with the seat", "3 mm between the doors"). Each is a
//! comparison of a sum of generic measurements on the final model, so the
//! library states new kinds of intent without Rust changes. A condition that
//! does not hold is an error with the measured and required numbers.

use crate::eval::{TOLERANCE_MM, contact};
use crate::frame;
use crate::model::{Face, Part, ProgramModel};
use crate::relations::polygon_area;
use crate::validate::{Issue, Severity};
use serde::Serialize;

/// A world direction, fixed or following a face of a part wherever the part
/// ends up.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    World([f64; 3]),
    Face { part: String, face: Face },
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
        face: Option<Face>,
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
            let axis = frame::axis(&find(model, part)?.rotation, face.axis());
            Some(if face.is_max() {
                axis
            } else {
                axis.map(|value| -value)
            })
        }
    }
}

fn measure(model: &ProgramModel, measure: &Measure) -> Option<f64> {
    Some(match measure {
        Measure::Reach { part, direction: d } => find(model, part)?.reach(direction(model, d)?),
        Measure::Distance { a, b } => {
            let (a, b) = (find(model, a)?.obb(), find(model, b)?.obb());
            let separation = a.separation(&b);
            if separation > TOLERANCE_MM {
                a.distance(&b)
            } else {
                separation.min(0.0)
            }
        }
        Measure::ContactArea { a, b, face } => match contact(find(model, a)?, find(model, b)?) {
            Some(patch) if face.is_none_or(|face| patch.face_a == face) => {
                polygon_area(&patch.points_mm)
            }
            _ => 0.0,
        },
    })
}

fn round(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0 + 0.0
}

/// One error per condition that does not hold.
pub fn check(model: &ProgramModel, issues: &mut Vec<Issue>) {
    for expectation in &model.expectations {
        let measured: Option<f64> = expectation
            .terms
            .iter()
            .map(|(coefficient, term)| measure(model, term).map(|value| coefficient * value))
            .sum();
        let Some(measured) = measured else {
            continue;
        };
        if expectation
            .comparison
            .holds(measured, expectation.value, expectation.tolerance)
        {
            continue;
        }
        let unit = &expectation.unit;
        issues.push(Issue {
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
