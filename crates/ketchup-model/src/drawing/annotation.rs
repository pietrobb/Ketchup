//! Annotation inputs shared by drawing tools and serialized clients. Values are
//! paper millimetres, not the bit encodings used by the persisted drawing model.

use super::*;

impl DrawingNote {
    pub fn new(
        id: DrawingNoteId,
        position_page_mm: [f64; 2],
        text_template: impl Into<String>,
    ) -> Result<Self, DrawingError> {
        let text_template = text_template.into();
        if id.0 == 0
            || position_page_mm.into_iter().any(|coordinate| {
                !coordinate.is_finite() || !(0.0..=MAX_COORDINATE_MM).contains(&coordinate)
            })
            || text_template.trim().is_empty()
            || !valid_drawing_template(&text_template)
        {
            return Err(DrawingError::InvalidAnnotation);
        }
        Ok(Self {
            id,
            position_bits: position_page_mm
                .map(canonical_view_component)
                .map(f64::to_bits),
            text_template,
            generated_hole_callout: None,
        })
    }

    #[must_use]
    pub const fn id(&self) -> DrawingNoteId {
        self.id
    }

    #[must_use]
    pub fn position_page_mm(&self) -> [f64; 2] {
        self.position_bits.map(f64::from_bits)
    }

    #[must_use]
    pub fn text_template(&self) -> &str {
        &self.text_template
    }
}

#[derive(Clone, Debug, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum AnnotationTolerance {
    #[default]
    None,
    Symmetric {
        deviation: f64,
    },
    Bilateral {
        upper: f64,
        lower: f64,
    },
}

impl AnnotationTolerance {
    fn dimension(&self) -> Result<DrawingDimensionTolerance, DrawingError> {
        match *self {
            Self::None => Ok(DrawingDimensionTolerance::None),
            Self::Symmetric { deviation } => DrawingDimensionTolerance::symmetric(deviation),
            Self::Bilateral { upper, lower } => DrawingDimensionTolerance::bilateral(upper, lower),
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum DrawingAnnotation {
    Angular {
        view: String,
        source_line_ids: [String; 2],
        arc_radius_page_mm: f64,
        #[serde(default)]
        tolerance: AnnotationTolerance,
    },
    Radius {
        view: String,
        source_circle_id: String,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        #[serde(default)]
        tolerance: AnnotationTolerance,
    },
    Diameter {
        view: String,
        source_circle_id: String,
        leader_angle_degrees: f64,
        offset_page_mm: f64,
        #[serde(default)]
        tolerance: AnnotationTolerance,
    },
    Datum {
        view: String,
        source_line_id: String,
        label: String,
        offset_page_mm: [f64; 2],
    },
    FeatureControlFrame {
        view: String,
        source_line_id: String,
        characteristic: DrawingGeometricCharacteristic,
        tolerance_mm: f64,
        #[serde(default)]
        diameter_zone: bool,
        #[serde(default = "no_material_condition")]
        material_condition: DrawingMaterialCondition,
        #[serde(default)]
        datum_labels: Vec<String>,
        offset_page_mm: [f64; 2],
    },
}

fn no_material_condition() -> DrawingMaterialCondition {
    DrawingMaterialCondition::None
}

fn next_id(ids: impl Iterator<Item = u64>) -> Result<u64, DrawingError> {
    ids.max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or(DrawingError::ResourceLimit)
}

fn dimension_id(annotations: &DrawingAnnotations) -> Result<DrawingDimensionId, DrawingError> {
    next_id(
        annotations
            .linear_dimensions
            .iter()
            .map(|d| d.id().0)
            .chain(annotations.angular_dimensions.iter().map(|d| d.id().0))
            .chain(annotations.circular_dimensions.iter().map(|d| d.id().0)),
    )
    .map(DrawingDimensionId)
}

impl DrawingAnnotation {
    fn view(&self) -> &str {
        match self {
            Self::Angular { view, .. }
            | Self::Radius { view, .. }
            | Self::Diameter { view, .. }
            | Self::Datum { view, .. }
            | Self::FeatureControlFrame { view, .. } => view,
        }
    }

    fn append(
        &self,
        view: OrthographicViewKind,
        annotations: &mut DrawingAnnotations,
    ) -> Result<(), DrawingError> {
        match self {
            Self::Angular {
                source_line_ids,
                arc_radius_page_mm,
                tolerance,
                ..
            } => {
                let dimension = DrawingAngularDimension::new(
                    dimension_id(annotations)?,
                    view,
                    source_line_ids.clone(),
                    *arc_radius_page_mm,
                    tolerance.dimension()?,
                )?;
                annotations.angular_dimensions.push(dimension);
            }
            Self::Radius {
                source_circle_id,
                leader_angle_degrees,
                offset_page_mm,
                tolerance,
                ..
            }
            | Self::Diameter {
                source_circle_id,
                leader_angle_degrees,
                offset_page_mm,
                tolerance,
                ..
            } => {
                let kind = if matches!(self, Self::Radius { .. }) {
                    DrawingCircularDimensionKind::Radius
                } else {
                    DrawingCircularDimensionKind::Diameter
                };
                let dimension = DrawingCircularDimension::new(
                    dimension_id(annotations)?,
                    view,
                    source_circle_id.clone(),
                    kind,
                    *leader_angle_degrees,
                    *offset_page_mm,
                    tolerance.dimension()?,
                )?;
                annotations.circular_dimensions.push(dimension);
            }
            Self::Datum {
                source_line_id,
                label,
                offset_page_mm,
                ..
            } => {
                let id =
                    DrawingDatumId(next_id(annotations.datum_symbols.iter().map(|d| d.id().0))?);
                annotations.datum_symbols.push(DrawingDatumSymbol::new(
                    id,
                    view,
                    source_line_id.clone(),
                    label.clone(),
                    *offset_page_mm,
                )?);
            }
            Self::FeatureControlFrame {
                source_line_id,
                characteristic,
                tolerance_mm,
                diameter_zone,
                material_condition,
                datum_labels,
                offset_page_mm,
                ..
            } => {
                let id = DrawingFeatureControlFrameId(next_id(
                    annotations.feature_control_frames.iter().map(|f| f.id().0),
                )?);
                let datums = datum_labels
                    .iter()
                    .map(|label| DrawingDatumReference::new(label, DrawingMaterialCondition::None))
                    .collect::<Result<Vec<_>, _>>()?;
                annotations
                    .feature_control_frames
                    .push(DrawingFeatureControlFrame::new(
                        id,
                        view,
                        source_line_id.clone(),
                        *characteristic,
                        *tolerance_mm,
                        *diameter_zone,
                        *material_condition,
                        datums,
                        *offset_page_mm,
                    )?);
            }
        }
        Ok(())
    }
}

/// Builds a replacement sheet without changing the original. Publish it through
/// `prepare_edit_drawing_sheet`, which checks references against the current exact
/// projection and makes the complete batch one Undo step.
pub fn annotate_sheet(
    sheet: &DrawingSheet,
    additions: &[DrawingAnnotation],
) -> Result<DrawingSheet, DrawingError> {
    if additions.is_empty() {
        return Err(DrawingError::InvalidAnnotation);
    }
    if additions.len() > MAX_DRAWING_DIMENSIONS + 2 * MAX_DRAWING_NOTES {
        return Err(DrawingError::ResourceLimit);
    }
    let mut annotations = DrawingAnnotations::with_bom_annotations(
        sheet.linear_dimensions().to_vec(),
        sheet.angular_dimensions().to_vec(),
        sheet.circular_dimensions().to_vec(),
        sheet.datum_symbols().to_vec(),
        sheet.feature_control_frames().to_vec(),
        sheet.bom_balloons().to_vec(),
        sheet.notes().to_vec(),
    );
    for addition in additions {
        let view = sheet
            .views()
            .iter()
            .copied()
            .find(|view| view.stable_name() == addition.view())
            .ok_or(DrawingError::InvalidView)?;
        addition.append(view, &mut annotations)?;
    }
    DrawingSheet::with_contract_views_and_annotations(
        sheet.id(),
        sheet.name(),
        sheet.source().clone(),
        sheet.page(),
        sheet.title_block().clone(),
        sheet.views().to_vec(),
        annotations,
    )
}
