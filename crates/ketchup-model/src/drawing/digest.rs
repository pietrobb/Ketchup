//! Digests that pin a drawing's projected linework and sheet layout.

use super::*;
use sha2::{Digest as _, Sha256};

#[allow(clippy::too_many_arguments)]
pub(crate) fn drawing_layout_digest(
    page: DrawingPageTemplate,
    title_block: &DrawingTitleBlock,
    border_bounds_mm: [[f64; 2]; 2],
    title_block_bounds_mm: [[f64; 2]; 2],
    placements: &[DrawingViewPlacement],
    dimensions: &[DrawingLinearDimensionLayout],
    angular_dimensions: &[DrawingAngularDimensionLayout],
    circular_dimensions: &[DrawingCircularDimensionLayout],
    datum_symbols: &[DrawingDatumSymbolLayout],
    feature_control_frames: &[DrawingFeatureControlFrameLayout],
    bom_balloons: &[DrawingBomBalloonLayout],
    bom_rows: &[DrawingBomRowLayout],
    notes: &[DrawingNoteLayout],
) -> String {
    let mut digest = Sha256::new();
    push_digest(&mut digest, DRAWING_SHEET_LAYOUT_SCHEMA_V2.as_bytes());
    push_digest(&mut digest, page.size().stable_name().as_bytes());
    push_digest(&mut digest, page.orientation().stable_name().as_bytes());
    digest.update(page.scale().numerator().to_le_bytes());
    digest.update(page.scale().denominator().to_le_bytes());
    for margin in page.margins().values_mm() {
        digest.update(margin.to_le_bytes());
    }
    for field in [
        title_block.title(),
        title_block.drawing_number(),
        title_block.revision(),
        title_block.author(),
    ] {
        push_digest(&mut digest, field.as_bytes());
    }
    for coordinate in border_bounds_mm
        .into_iter()
        .flatten()
        .chain(title_block_bounds_mm.into_iter().flatten())
    {
        digest.update(coordinate.to_bits().to_le_bytes());
    }
    for placement in placements {
        push_digest(&mut digest, placement.kind.stable_name().as_bytes());
        push_digest(&mut digest, placement.stable_view_id.as_bytes());
        for coordinate in placement
            .origin_mm
            .into_iter()
            .chain(placement.page_bounds_mm.into_iter().flatten())
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        push_digest(&mut digest, dimension.source_line_id.as_bytes());
        digest.update(dimension.value_mm.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .extension_lines_mm
            .into_iter()
            .flatten()
            .flatten()
            .chain(dimension.dimension_line_mm.into_iter().flatten())
            .chain(dimension.text_position_mm)
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in angular_dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        for source in &dimension.source_line_ids {
            push_digest(&mut digest, source.as_bytes());
        }
        digest.update(dimension.value_degrees.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .extension_lines_mm
            .iter()
            .flatten()
            .chain(&dimension.arc_points_mm)
            .chain([&dimension.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for dimension in circular_dimensions {
        push_digest(&mut digest, dimension.stable_dimension_id.as_bytes());
        push_digest(&mut digest, dimension.source_circle_id.as_bytes());
        digest.update(match dimension.kind {
            DrawingCircularDimensionKind::Radius => [1],
            DrawingCircularDimensionKind::Diameter => [2],
        });
        digest.update(dimension.value_mm.to_bits().to_le_bytes());
        push_digest(&mut digest, dimension.label.as_bytes());
        for coordinate in dimension
            .leader_line_mm
            .iter()
            .chain([&dimension.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for datum in datum_symbols {
        push_digest(&mut digest, datum.stable_datum_id.as_bytes());
        push_digest(&mut digest, datum.source_line_id.as_bytes());
        push_digest(&mut digest, datum.label.as_bytes());
        for coordinate in datum
            .leader_line_mm
            .iter()
            .chain(&datum.triangle_mm)
            .chain(datum.frame_bounds_mm.iter())
            .chain([&datum.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for frame in feature_control_frames {
        push_digest(&mut digest, frame.stable_frame_id.as_bytes());
        push_digest(&mut digest, frame.source_line_id.as_bytes());
        push_digest(&mut digest, frame.characteristic.stable_name().as_bytes());
        digest.update(frame.tolerance_mm.to_bits().to_le_bytes());
        digest.update([u8::from(frame.diameter_zone)]);
        push_digest(
            &mut digest,
            frame.material_condition.stable_name().as_bytes(),
        );
        for reference in &frame.datum_references {
            push_digest(&mut digest, reference.label().as_bytes());
            push_digest(
                &mut digest,
                reference.material_condition().stable_name().as_bytes(),
            );
        }
        push_digest(&mut digest, frame.label.as_bytes());
        for coordinate in frame
            .leader_line_mm
            .iter()
            .chain(frame.frame_bounds_mm.iter())
            .chain([&frame.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        for separator in &frame.separator_x_mm {
            digest.update(separator.to_bits().to_le_bytes());
        }
    }
    for balloon in bom_balloons {
        push_digest(&mut digest, balloon.stable_balloon_id.as_bytes());
        push_digest(
            &mut digest,
            stable_instance_path(&balloon.instance_path).as_bytes(),
        );
        digest.update(balloon.position.to_le_bytes());
        push_digest(&mut digest, balloon.label.as_bytes());
        digest.update(balloon.circle_radius_mm.to_bits().to_le_bytes());
        for coordinate in balloon
            .leader_line_mm
            .iter()
            .chain([&balloon.circle_center_mm, &balloon.text_position_mm])
            .flatten()
        {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for row in bom_rows {
        digest.update(row.position.to_le_bytes());
        digest.update(row.definition_id.0.to_le_bytes());
        push_digest(&mut digest, row.part_name.as_bytes());
        push_digest(
            &mut digest,
            row.material.as_deref().unwrap_or("").as_bytes(),
        );
        digest.update(row.quantity.to_le_bytes());
        push_digest(&mut digest, row.label.as_bytes());
        for coordinate in row.text_position_mm {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
    }
    for note in notes {
        push_digest(&mut digest, note.stable_note_id.as_bytes());
        for coordinate in note.position_mm {
            digest.update(coordinate.to_bits().to_le_bytes());
        }
        push_digest(&mut digest, note.text.as_bytes());
        // Absent for plain notes, so their layout digests are unchanged.
        if let (Some(leader), Some(target)) = (note.leader_line_mm, &note.target_circle_id) {
            for coordinate in leader.into_iter().flatten() {
                digest.update(coordinate.to_bits().to_le_bytes());
            }
            push_digest(&mut digest, target.as_bytes());
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn drawing_result_digest(
    stable_source_identity: &str,
    views: &[OrthographicView],
) -> String {
    let mut digest = Sha256::new();
    push_digest(&mut digest, ORTHOGRAPHIC_LINEWORK_SCHEMA_V2.as_bytes());
    push_digest(&mut digest, stable_source_identity.as_bytes());
    for view in views {
        push_digest(&mut digest, view.kind.stable_name().as_bytes());
        for bound in view.bounds_mm.iter().flatten() {
            digest.update(bound.to_bits().to_le_bytes());
        }
        for (classification, lines) in [
            (b"visible".as_slice(), view.visible_lines.as_slice()),
            (b"hidden".as_slice(), view.hidden_lines.as_slice()),
        ] {
            push_digest(&mut digest, classification);
            for line in lines {
                push_digest(&mut digest, line.stable_line_id.as_bytes());
                for coordinate in line.start_mm.into_iter().chain(line.end_mm) {
                    digest.update(coordinate.to_bits().to_le_bytes());
                }
            }
        }
        for circle in &view.circles {
            push_digest(&mut digest, circle.stable_circle_id.as_bytes());
            for coordinate in circle.center_mm {
                digest.update(coordinate.to_bits().to_le_bytes());
            }
            digest.update(circle.radius_mm.to_bits().to_le_bytes());
            digest.update(circle.start_rad.to_bits().to_le_bytes());
            digest.update(circle.span_rad.to_bits().to_le_bytes());
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn push_digest(digest: &mut Sha256, value: &[u8]) {
    digest.update((value.len() as u64).to_le_bytes());
    digest.update(value);
}
