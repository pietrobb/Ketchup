//! Feature control frames laid out on a sheet: one box per value, each as
//! wide as the sheet font prints it, so every value stays inside its box.

use super::*;
use crate::drawing_export::GLYPH_WIDTH;

/// Text height of a feature control frame's values.
pub(crate) const FEATURE_CONTROL_FRAME_TEXT_HEIGHT_MM: f64 = 3.0;

/// One box of a feature control frame and where its value starts.
#[derive(Clone, Debug, PartialEq)]
pub struct DrawingFeatureControlFrameCell {
    pub text: String,
    pub text_position_mm: [f64; 2],
}

pub(super) fn layout_feature_control_frames(
    sheet: &DrawingSheet,
    views: &[OrthographicView],
    placements: &[DrawingViewPlacement],
    border_bounds_mm: [[f64; 2]; 2],
    drawing_min_y: f64,
) -> Result<Vec<DrawingFeatureControlFrameLayout>, DrawingError> {
    sheet
        .feature_control_frames()
        .iter()
        .map(|frame| {
            let anchor = annotation_source_anchor(
                sheet,
                views,
                placements,
                frame.view_stable_name(),
                frame.source_line_id(),
            )?;
            let offset = frame.offset_page_mm();
            let origin = [anchor[0] + offset[0], anchor[1] + offset[1]];
            let tolerance = format_decimal_mm(frame.tolerance_mm());
            let zone = if frame.diameter_zone() { "DIA " } else { "" };
            let condition = match frame.material_condition() {
                DrawingMaterialCondition::None => "",
                DrawingMaterialCondition::MaximumMaterial => " MMC",
                DrawingMaterialCondition::LeastMaterial => " LMC",
                DrawingMaterialCondition::RegardlessOfFeatureSize => " RFS",
            };
            let mut cells = vec![
                frame.characteristic().stable_name().to_ascii_uppercase(),
                format!("{zone}{tolerance}{condition}"),
            ];
            cells.extend(frame.datum_references().iter().map(|reference| {
                let suffix = match reference.material_condition() {
                    DrawingMaterialCondition::None => "",
                    DrawingMaterialCondition::MaximumMaterial => " MMC",
                    DrawingMaterialCondition::LeastMaterial => " LMC",
                    DrawingMaterialCondition::RegardlessOfFeatureSize => " RFS",
                };
                format!("{}{suffix}", reference.label())
            }));
            let advance = GLYPH_WIDTH * FEATURE_CONTROL_FRAME_TEXT_HEIGHT_MM;
            let mut left = origin[0];
            let mut separator_x_mm = Vec::with_capacity(cells.len());
            let label = cells.join(" | ");
            let cells = cells
                .into_iter()
                .map(|text| {
                    let width = (text.chars().count() as f64 * advance + 3.0).max(8.0);
                    let cell = DrawingFeatureControlFrameCell {
                        text,
                        text_position_mm: [left + 1.5, origin[1] + 2.0],
                    };
                    left += width;
                    separator_x_mm.push(left);
                    cell
                })
                .collect::<Vec<_>>();
            separator_x_mm.pop();
            let frame_bounds_mm = [origin, [left, origin[1] + 7.0]];
            let leader_line_mm = [anchor, [origin[0], origin[1] + 3.5]];
            if leader_line_mm
                .iter()
                .chain(frame_bounds_mm.iter())
                .chain(cells.iter().map(|cell| &cell.text_position_mm))
                .any(|point| !drawing_page_contains(*point, border_bounds_mm, drawing_min_y))
                || separator_x_mm.iter().any(|x| !x.is_finite())
            {
                return Err(DrawingError::LayoutOverflow);
            }
            Ok(DrawingFeatureControlFrameLayout {
                stable_frame_id: format!(
                    "sheet-{}/feature-control-frame-{}",
                    sheet.id().0,
                    frame.id().0
                ),
                source_line_id: frame.source_line_id().to_owned(),
                characteristic: frame.characteristic(),
                tolerance_mm: frame.tolerance_mm(),
                diameter_zone: frame.diameter_zone(),
                material_condition: frame.material_condition(),
                datum_references: frame.datum_references().to_vec(),
                label,
                cells,
                leader_line_mm,
                frame_bounds_mm,
                separator_x_mm,
            })
        })
        .collect()
}
