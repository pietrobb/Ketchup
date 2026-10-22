//! Drawing annotations: a text at a point, a dimension between two points whose
//! value the document measures itself, a leader from a point to its text, and a
//! drawn line. An annotation is no part: it has no body, so it is never in the
//! bill of materials, a collision, a mass or a takeoff. Tags show and hide it like
//! a part. Text height is millimetres on paper, every point is in world
//! millimetres.

use crate::document::TagId;
use crate::tolerance::{MAX_COORDINATE_MM, MIN_DIMENSION_MM, ROUNDING};
use std::collections::BTreeSet;
use std::fmt;

/// The tallest text, in millimetres on paper.
pub const MAX_TEXT_HEIGHT_MM: f64 = 100.0;
/// The longest text, in characters.
pub const MAX_TEXT_CHARS: usize = 1000;
/// The most points of one drawn line.
pub const MAX_LINE_POINTS: usize = 100_000;
/// The most annotations of one document.
pub const MAX_ANNOTATIONS: usize = 100_000;

/// Whether a document has no annotations, so its file leaves the field out.
pub(crate) fn none(annotations: &std::sync::Arc<Vec<Annotation>>) -> bool {
    annotations.is_empty()
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Annotation {
    pub name: String,
    pub tags: BTreeSet<TagId>,
    pub mark: AnnotationMark,
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AnnotationMark {
    /// `text` with its insertion point at `at_mm`.
    Text {
        at_mm: [f64; 3],
        text: String,
        height_mm: f64,
    },
    /// The distance from `from_mm` to `to_mm`, drawn `offset_mm` to the left of
    /// the way from one to the other in plan (see [`DimensionLayout`]).
    Dimension {
        from_mm: [f64; 3],
        to_mm: [f64; 3],
        offset_mm: f64,
        height_mm: f64,
    },
    /// An arrow at `point_mm` and a line to `text`, which starts at `at_mm`.
    Leader {
        point_mm: [f64; 3],
        at_mm: [f64; 3],
        text: String,
        height_mm: f64,
    },
    /// Straight lines through `points_mm`, back to the first one when `closed`.
    Line {
        points_mm: Vec<[f64; 3]>,
        closed: bool,
    },
}

impl Annotation {
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self.mark {
            AnnotationMark::Text { .. } => "text",
            AnnotationMark::Dimension { .. } => "dimension",
            AnnotationMark::Leader { .. } => "leader",
            AnnotationMark::Line { .. } => "line",
        }
    }

    /// The text it writes: its own, or the measured value of a dimension.
    #[must_use]
    pub fn text(&self) -> Option<String> {
        match &self.mark {
            AnnotationMark::Text { text, .. } | AnnotationMark::Leader { text, .. } => {
                Some(text.clone())
            }
            AnnotationMark::Dimension { from_mm, to_mm, .. } => {
                Some(dimension_text(distance(*from_mm, *to_mm)))
            }
            AnnotationMark::Line { .. } => None,
        }
    }

    #[must_use]
    pub const fn height_mm(&self) -> Option<f64> {
        match self.mark {
            AnnotationMark::Text { height_mm, .. }
            | AnnotationMark::Dimension { height_mm, .. }
            | AnnotationMark::Leader { height_mm, .. } => Some(height_mm),
            AnnotationMark::Line { .. } => None,
        }
    }

    /// Every point it is drawn through, for bounds and snapping.
    #[must_use]
    pub fn points(&self) -> Vec<[f64; 3]> {
        match &self.mark {
            AnnotationMark::Text { at_mm, .. } => vec![*at_mm],
            AnnotationMark::Dimension {
                from_mm,
                to_mm,
                offset_mm,
                ..
            } => {
                let layout = DimensionLayout::new(*from_mm, *to_mm, *offset_mm);
                vec![*from_mm, *to_mm, layout.line[0], layout.line[1]]
            }
            AnnotationMark::Leader {
                point_mm, at_mm, ..
            } => vec![*point_mm, *at_mm],
            AnnotationMark::Line { points_mm, .. } => points_mm.clone(),
        }
    }

    /// The layout of a dimension; `None` for the other kinds.
    #[must_use]
    pub fn dimension(&self) -> Option<DimensionLayout> {
        match self.mark {
            AnnotationMark::Dimension {
                from_mm,
                to_mm,
                offset_mm,
                ..
            } => Some(DimensionLayout::new(from_mm, to_mm, offset_mm)),
            _ => None,
        }
    }

    fn problem(&self) -> Option<AnnotationProblem> {
        if self.name.trim().is_empty() {
            return Some(AnnotationProblem::EmptyName);
        }
        let mut points = self.points();
        if let AnnotationMark::Dimension { offset_mm, .. } = self.mark {
            points.push([offset_mm, 0.0, 0.0]);
        }
        if points
            .iter()
            .flatten()
            .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
        {
            return Some(AnnotationProblem::PointOutsideModel);
        }
        if let Some(height) = self.height_mm()
            && !(height > 0.0 && height <= MAX_TEXT_HEIGHT_MM)
        {
            return Some(AnnotationProblem::TextHeight);
        }
        match &self.mark {
            AnnotationMark::Text { text, .. } | AnnotationMark::Leader { text, .. } => {
                text_problem(text)
            }
            AnnotationMark::Dimension { from_mm, to_mm, .. } => (distance(*from_mm, *to_mm)
                < MIN_DIMENSION_MM)
                .then_some(AnnotationProblem::DimensionTooShort),
            AnnotationMark::Line { points_mm, closed } => {
                let least = if *closed { 3 } else { 2 };
                (points_mm.len() < least || points_mm.len() > MAX_LINE_POINTS)
                    .then_some(AnnotationProblem::LinePoints)
            }
        }
    }
}

fn text_problem(text: &str) -> Option<AnnotationProblem> {
    if text.trim().is_empty() || text.chars().count() > MAX_TEXT_CHARS {
        Some(AnnotationProblem::TextLength)
    } else if text.chars().any(|c| c.is_control() && c != '\n') {
        Some(AnnotationProblem::ControlCharacter)
    } else {
        None
    }
}

fn distance(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3)
        .map(|axis| (b[axis] - a[axis]).powi(2))
        .sum::<f64>()
        .sqrt()
}

/// A measured length as a drawing writes it: millimetres to one decimal place,
/// without a trailing zero or a unit.
#[must_use]
pub fn dimension_text(length_mm: f64) -> String {
    let tenths = (length_mm * 10.0).round();
    if tenths % 10.0 == 0.0 {
        format!("{:.0}", tenths / 10.0)
    } else {
        format!("{:.1}", tenths / 10.0)
    }
}

/// Where a dimension is drawn. The dimension line runs parallel to the measured
/// points, moved by the offset to the left of the way from the first point to the
/// second as seen from above; a vertical dimension moves along −x. The value is the
/// straight distance between the points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DimensionLayout {
    pub value_mm: f64,
    /// Unit vector from the first point to the second.
    pub along: [f64; 3],
    /// Unit vector the offset moves the dimension line along.
    pub across: [f64; 3],
    /// The dimension line, from the first point's side to the second's.
    pub line: [[f64; 3]; 2],
}

impl DimensionLayout {
    #[must_use]
    pub fn new(from: [f64; 3], to: [f64; 3], offset_mm: f64) -> Self {
        let value_mm = distance(from, to);
        let along = if value_mm > 0.0 {
            std::array::from_fn(|axis| (to[axis] - from[axis]) / value_mm)
        } else {
            [1.0, 0.0, 0.0]
        };
        let plan = along[0].hypot(along[1]);
        let across = if plan > ROUNDING {
            [-along[1] / plan, along[0] / plan, 0.0]
        } else {
            [-1.0, 0.0, 0.0]
        };
        let moved =
            |point: [f64; 3]| std::array::from_fn(|axis| point[axis] + across[axis] * offset_mm);
        Self {
            value_mm,
            along,
            across,
            line: [moved(from), moved(to)],
        }
    }

    /// The middle of the dimension line, where its value is written.
    #[must_use]
    pub fn middle(&self) -> [f64; 3] {
        std::array::from_fn(|axis| (self.line[0][axis] + self.line[1][axis]) / 2.0)
    }
}

/// Why an annotation cannot be stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationProblem {
    EmptyName,
    /// Another annotation already has this name.
    NameTaken,
    PointOutsideModel,
    TextHeight,
    TextLength,
    ControlCharacter,
    DimensionTooShort,
    LinePoints,
    TooMany,
}

impl fmt::Display for AnnotationProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("needs a name"),
            Self::NameTaken => formatter.write_str("has the name of another annotation"),
            Self::PointOutsideModel => write!(
                formatter,
                "needs finite points and offset within ±{MAX_COORDINATE_MM} mm"
            ),
            Self::TextHeight => write!(
                formatter,
                "needs a text height above 0 and at most {MAX_TEXT_HEIGHT_MM} mm on paper"
            ),
            Self::TextLength => write!(
                formatter,
                "needs a text of 1 to {MAX_TEXT_CHARS} characters"
            ),
            Self::ControlCharacter => {
                formatter.write_str("has a control character in its text other than a line break")
            }
            Self::DimensionTooShort => write!(
                formatter,
                "measures two points less than {MIN_DIMENSION_MM} mm apart"
            ),
            Self::LinePoints => write!(
                formatter,
                "needs 2 to {MAX_LINE_POINTS} points, 3 when closed"
            ),
            Self::TooMany => write!(
                formatter,
                "is one more than the {MAX_ANNOTATIONS} annotations a document holds"
            ),
        }
    }
}

/// The first annotation that cannot be stored, by name, and why. Tags are checked
/// by the document, which knows them.
#[must_use]
pub fn first_problem(annotations: &[Annotation]) -> Option<(String, AnnotationProblem)> {
    if annotations.len() > MAX_ANNOTATIONS {
        let last = &annotations[MAX_ANNOTATIONS];
        return Some((last.name.clone(), AnnotationProblem::TooMany));
    }
    let mut names = BTreeSet::new();
    annotations.iter().find_map(|annotation| {
        annotation
            .problem()
            .or_else(|| {
                (!names.insert(annotation.name.as_str())).then_some(AnnotationProblem::NameTaken)
            })
            .map(|problem| (annotation.name.clone(), problem))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dimension(from: [f64; 3], to: [f64; 3], offset_mm: f64) -> Annotation {
        Annotation {
            name: "d".to_owned(),
            tags: BTreeSet::new(),
            mark: AnnotationMark::Dimension {
                from_mm: from,
                to_mm: to,
                offset_mm,
                height_mm: 2.5,
            },
        }
    }

    #[test]
    fn a_dimension_measures_its_points_and_is_drawn_to_their_left() {
        let annotation = dimension([0.0, 0.0, 0.0], [1944.44, 0.0, 0.0], 500.0);
        assert_eq!(annotation.text().as_deref(), Some("1944.4"));
        let layout = annotation.dimension().unwrap();
        assert_eq!(layout.line, [[0.0, 500.0, 0.0], [1944.44, 500.0, 0.0]]);
        assert_eq!(layout.middle(), [972.22, 500.0, 0.0]);
        let upward = dimension([0.0, 0.0, 0.0], [0.0, 0.0, 300.0], 50.0);
        assert_eq!(upward.dimension().unwrap().line[1], [-50.0, 0.0, 300.0]);
        assert_eq!(dimension_text(2000.04), "2000");
    }

    #[test]
    fn an_annotation_that_cannot_be_drawn_is_named_with_its_problem() {
        let short = dimension([1.0, 1.0, 0.0], [1.0, 1.0, 0.0], 0.0);
        assert_eq!(
            first_problem(&[short]),
            Some(("d".to_owned(), AnnotationProblem::DimensionTooShort))
        );
        let good = dimension([0.0; 3], [10.0, 0.0, 0.0], 0.0);
        assert_eq!(
            first_problem(&[good.clone(), good.clone()]),
            Some(("d".to_owned(), AnnotationProblem::NameTaken))
        );
        let text = |text: &str, height_mm: f64| Annotation {
            name: "t".to_owned(),
            tags: BTreeSet::new(),
            mark: AnnotationMark::Text {
                at_mm: [0.0; 3],
                text: text.to_owned(),
                height_mm,
            },
        };
        assert_eq!(first_problem(&[text("S-05", 3.5)]), None);
        assert_eq!(
            first_problem(&[text("S-05", 0.0)]).map(|(_, problem)| problem),
            Some(AnnotationProblem::TextHeight)
        );
        assert_eq!(
            first_problem(&[text(" ", 3.5)]).map(|(_, problem)| problem),
            Some(AnnotationProblem::TextLength)
        );
    }
}
