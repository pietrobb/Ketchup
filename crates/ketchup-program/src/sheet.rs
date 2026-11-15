//! Sheet declarations and geometric selectors, resolved against exact projections at publication.
use ketchup_model::drawing::{
    DrawingGeometricCharacteristic, DrawingMaterialCondition, OrthographicView,
    annotation::{AnnotationTolerance, DrawingAnnotation},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramSheet {
    pub name: String,
    pub source: String,
    pub hole_callouts: bool,
    pub annotations: Vec<ProgramSheetAnnotation>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LineSelector {
    pub direction: [f64; 2],
    #[serde(default)]
    pub through_mm: Option<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CircleSelector {
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ProgramSheetAnnotation {
    pub name: String,
    pub view: String,
    pub mark: SheetMark,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SheetMark {
    Angular {
        lines: [LineSelector; 2],
        arc_radius_page_mm: f64,
        tolerance: Option<f64>,
    },
    Radius {
        circle: CircleSelector,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        tolerance: Option<f64>,
    },
    Diameter {
        circle: CircleSelector,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        tolerance: Option<f64>,
    },
    Datum {
        line: LineSelector,
        label: String,
        offset_page_mm: [f64; 2],
    },
    FeatureControlFrame {
        line: LineSelector,
        characteristic: DrawingGeometricCharacteristic,
        tolerance_mm: f64,
        diameter_zone: bool,
        material_condition: DrawingMaterialCondition,
        datum_labels: Vec<String>,
        offset_page_mm: [f64; 2],
    },
}

impl LineSelector {
    fn resolve(&self, view: &OrthographicView) -> anyhow::Result<String> {
        let [dx, dy] = self.direction;
        let length = dx.hypot(dy);
        anyhow::ensure!(
            length.is_finite() && length > 0.0,
            "line direction must be finite and nonzero"
        );
        anyhow::ensure!(
            self.through_mm
                .is_none_or(|p| p.iter().all(|v| v.is_finite())),
            "line through_mm must be finite"
        );
        let direction = [dx / length, dy / length];
        let tolerance = ketchup_model::tolerance::CONTACT_FIT_MM;
        let mut matches = view
            .visible_lines
            .iter()
            .filter(|line| {
                let delta = [
                    line.end_mm[0] - line.start_mm[0],
                    line.end_mm[1] - line.start_mm[1],
                ];
                let size = delta[0].hypot(delta[1]);
                size > 0.0
                    && (direction[0] * delta[1] - direction[1] * delta[0]).abs() <= tolerance * size
                    && self.through_mm.is_none_or(|point| {
                        let along = (point[0] - line.start_mm[0]) * delta[0] / size
                            + (point[1] - line.start_mm[1]) * delta[1] / size;
                        let across = ((point[0] - line.start_mm[0]) * delta[1]
                            - (point[1] - line.start_mm[1]) * delta[0])
                            .abs()
                            / size;
                        across <= tolerance && along >= -tolerance && along <= size + tolerance
                    })
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| a.stable_line_id.cmp(&b.stable_line_id));
        let same = |a: [f64; 2], b: [f64; 2]| {
            (a[0] - b[0]).hypot(a[1] - b[1]) <= ketchup_model::tolerance::ROUNDING
        };
        match matches.as_slice() {
            [first, rest @ ..]
                if rest.iter().all(|line| {
                    line.foreshortened == first.foreshortened
                        && line.clipped == first.clipped
                        && ((same(first.start_mm, line.start_mm)
                            && same(first.end_mm, line.end_mm))
                            || (same(first.start_mm, line.end_mm)
                                && same(first.end_mm, line.start_mm)))
                }) =>
            {
                Ok(first.stable_line_id.clone())
            }
            [] => anyhow::bail!(
                "no visible line matches direction/through_mm; choose a line in this view"
            ),
            _ => anyhow::bail!(
                "ambiguous line selector; specify through_mm on exactly one visible segment"
            ),
        }
    }
}

impl CircleSelector {
    fn resolve(&self, view: &OrthographicView) -> anyhow::Result<String> {
        anyhow::ensure!(
            self.center_mm.iter().all(|v| v.is_finite())
                && self.radius_mm.is_finite()
                && self.radius_mm > 0.0,
            "circle center_mm must be finite and radius_mm positive"
        );
        let tolerance = ketchup_model::tolerance::CONTACT_FIT_MM;
        let mut matches = view
            .circles
            .iter()
            .filter(|circle| {
                (circle.center_mm[0] - self.center_mm[0])
                    .hypot(circle.center_mm[1] - self.center_mm[1])
                    <= tolerance
                    && (circle.radius_mm - self.radius_mm).abs() <= tolerance
            })
            .collect::<Vec<_>>();
        matches.sort_by(|a, b| a.stable_circle_id.cmp(&b.stable_circle_id));
        let same = |a: f64, b: f64| (a - b).abs() <= ketchup_model::tolerance::ROUNDING;
        match matches.as_slice() {
            [first, rest @ ..]
                if rest.iter().all(|circle| {
                    same(first.center_mm[0], circle.center_mm[0])
                        && same(first.center_mm[1], circle.center_mm[1])
                        && same(first.radius_mm, circle.radius_mm)
                        && same(first.span_rad, circle.span_rad)
                        && (same(first.span_rad, std::f64::consts::TAU)
                            || same(first.start_rad, circle.start_rad))
                }) =>
            {
                Ok(first.stable_circle_id.clone())
            }
            [] => anyhow::bail!("no exact circle matches center_mm/radius_mm in this view"),
            _ => anyhow::bail!(
                "ambiguous circle selector; use a view with one matching exact circle"
            ),
        }
    }
}

impl ProgramSheetAnnotation {
    pub fn resolve(&self, view: &OrthographicView) -> anyhow::Result<DrawingAnnotation> {
        let view_name = self.view.clone();
        let tolerance = |value: Option<f64>| {
            value.map_or(AnnotationTolerance::None, |deviation| {
                AnnotationTolerance::Symmetric { deviation }
            })
        };
        Ok(match &self.mark {
            SheetMark::Angular {
                lines,
                arc_radius_page_mm,
                tolerance: value,
            } => DrawingAnnotation::Angular {
                view: view_name,
                source_line_ids: [lines[0].resolve(view)?, lines[1].resolve(view)?],
                arc_radius_page_mm: *arc_radius_page_mm,
                tolerance: tolerance(*value),
            },
            SheetMark::Radius {
                circle,
                leader_angle_degrees,
                offset_page_mm,
                tolerance: value,
            } => DrawingAnnotation::Radius {
                view: view_name,
                source_circle_id: circle.resolve(view)?,
                leader_angle_degrees: *leader_angle_degrees,
                offset_page_mm: *offset_page_mm,
                tolerance: tolerance(*value),
            },
            SheetMark::Diameter {
                circle,
                leader_angle_degrees,
                offset_page_mm,
                tolerance: value,
            } => DrawingAnnotation::Diameter {
                view: view_name,
                source_circle_id: circle.resolve(view)?,
                leader_angle_degrees: *leader_angle_degrees,
                offset_page_mm: *offset_page_mm,
                tolerance: tolerance(*value),
            },
            SheetMark::Datum {
                line,
                label,
                offset_page_mm,
            } => DrawingAnnotation::Datum {
                view: view_name,
                source_line_id: line.resolve(view)?,
                label: label.clone(),
                offset_page_mm: *offset_page_mm,
            },
            SheetMark::FeatureControlFrame {
                line,
                characteristic,
                tolerance_mm,
                diameter_zone,
                material_condition,
                datum_labels,
                offset_page_mm,
            } => DrawingAnnotation::FeatureControlFrame {
                view: view_name,
                source_line_id: line.resolve(view)?,
                characteristic: *characteristic,
                tolerance_mm: *tolerance_mm,
                diameter_zone: *diameter_zone,
                material_condition: *material_condition,
                datum_labels: datum_labels.clone(),
                offset_page_mm: *offset_page_mm,
            },
        })
    }
}
