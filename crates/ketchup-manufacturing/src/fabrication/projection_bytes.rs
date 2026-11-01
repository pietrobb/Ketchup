//! The canonical bytes a fabrication projection digests: its BOM, drawing
//! and manufacturing.

use super::*;

pub(super) fn general_bom_bytes(
    rows: &[GeneralBomRow],
    evidence_counts: EvidenceCounts,
) -> Vec<u8> {
    let mut bytes = GENERAL_BOM_EXPORT_V3.as_bytes().to_vec();
    bytes.extend_from_slice(&(rows.len() as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.exact as u64).to_le_bytes());
    bytes.extend_from_slice(&(evidence_counts.tolerant as u64).to_le_bytes());
    for row in rows {
        push_projection_bytes(&mut bytes, row.stable_row_id.as_bytes());
        bytes.extend_from_slice(&(row.position as u64).to_le_bytes());
        bytes.extend_from_slice(&row.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &row.source);
        push_projection_bytes(&mut bytes, row.item_kind.token().as_bytes());
        push_projection_bytes(&mut bytes, row.material_key.as_bytes());
        bytes.extend_from_slice(&(row.quantity as u64).to_le_bytes());
        push_dimensions(&mut bytes, row.dimensions);
        bytes.extend_from_slice(&(row.instances.len() as u64).to_le_bytes());
        for instance in &row.instances {
            push_projection_path(&mut bytes, instance);
        }
        push_projection_evidence(&mut bytes, &row.evidence_class);
        push_validation_state(&mut bytes, row.validation_state);
    }
    bytes
}

pub(super) fn general_drawing_bytes(
    drawings: &[GeneralPieceDrawing],
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = GENERAL_DRAWING_SVG_V3.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(drawings.len() as u64).to_le_bytes());
    for drawing in drawings {
        push_projection_bytes(&mut bytes, drawing.stable_drawing_id.as_bytes());
        push_projection_bytes(&mut bytes, drawing.bom_row_id.as_bytes());
        bytes.extend_from_slice(&(drawing.position as u64).to_le_bytes());
        bytes.extend_from_slice(&drawing.definition_id.0.to_le_bytes());
        push_general_source(&mut bytes, &drawing.source);
        push_projection_bytes(&mut bytes, drawing.item_kind.token().as_bytes());
        push_projection_bytes(&mut bytes, drawing.material_key.as_bytes());
        bytes.extend_from_slice(&(drawing.quantity as u64).to_le_bytes());
        bytes.extend_from_slice(&(drawing.instances.len() as u64).to_le_bytes());
        for instance in &drawing.instances {
            push_projection_path(&mut bytes, instance);
        }
        push_projection_bytes(&mut bytes, drawing.projection_method.as_bytes());
        bytes.extend_from_slice(&(drawing.views.len() as u64).to_le_bytes());
        for view in &drawing.views {
            push_projection_bytes(&mut bytes, view.stable_view_id.as_bytes());
            push_projection_bytes(&mut bytes, view.name.as_bytes());
            push_projection_bytes(&mut bytes, view.horizontal_axis.as_bytes());
            push_projection_bytes(&mut bytes, view.vertical_axis.as_bytes());
            bytes.extend_from_slice(&view.width_mm.to_bits().to_le_bytes());
            bytes.extend_from_slice(&view.height_mm.to_bits().to_le_bytes());
        }
        bytes.extend_from_slice(&(drawing.dimensions.len() as u64).to_le_bytes());
        for dimension in &drawing.dimensions {
            push_projection_bytes(&mut bytes, dimension.stable_dimension_id.as_bytes());
            push_projection_bytes(&mut bytes, dimension.axis.as_bytes());
            bytes.extend_from_slice(&dimension.value_mm.to_bits().to_le_bytes());
        }
        push_projection_bytes(
            &mut bytes,
            &general_manufacturing_bytes(&drawing.machining_operations, &[], &[], validation_state),
        );
        push_projection_evidence(&mut bytes, &drawing.evidence_class);
    }
    bytes
}

pub(super) fn general_manufacturing_bytes(
    operations: &[GeneralManufacturingOperation],
    unresolved_sources: &[GeneralBodySource],
    undescribed_sources: &[GeneralBodySource],
    validation_state: ValidationState,
) -> Vec<u8> {
    let mut bytes = GENERAL_MANUFACTURING_EXPORT_V2.as_bytes().to_vec();
    push_validation_state(&mut bytes, validation_state);
    bytes.extend_from_slice(&(operations.len() as u64).to_le_bytes());
    for operation in operations {
        push_projection_bytes(&mut bytes, operation.stable_operation_id.as_bytes());
        bytes.extend_from_slice(&operation.definition_id.0.to_le_bytes());
        bytes.extend_from_slice(&operation.producer_feature_id.0.to_le_bytes());
        bytes.push(match operation.kind {
            GeneralManufacturingKind::Stock => 0,
            GeneralManufacturingKind::ThroughCut => 1,
            GeneralManufacturingKind::ProfileCut => 2,
            GeneralManufacturingKind::CircularDrill => 3,
            GeneralManufacturingKind::BooleanCut => 4,
        });
        bytes.extend_from_slice(&(operation.semantic_inputs.len() as u64).to_le_bytes());
        for input in &operation.semantic_inputs {
            bytes.extend_from_slice(&input.0.to_le_bytes());
        }
        push_projection_bytes(&mut bytes, operation.frame.as_bytes());
        push_dimensions(&mut bytes, operation.bounds);
        push_machining_geometry(&mut bytes, &operation.machining);
        push_general_source(
            &mut bytes,
            &GeneralBodySource::Exact(operation.source.clone()),
        );
        match &operation.thread {
            None => bytes.push(0),
            Some(thread) => {
                bytes.push(1);
                push_projection_bytes(&mut bytes, thread.size.as_bytes());
                bytes.extend_from_slice(&thread.pitch_mm.to_bits().to_le_bytes());
                bytes.extend_from_slice(&thread.depth_mm.to_bits().to_le_bytes());
            }
        }
    }
    bytes.extend_from_slice(&(unresolved_sources.len() as u64).to_le_bytes());
    for source in unresolved_sources {
        push_general_source(&mut bytes, source);
    }
    let undescribed = u64::try_from(undescribed_sources.len()).unwrap_or(u64::MAX);
    bytes.extend_from_slice(&undescribed.to_le_bytes());
    for source in undescribed_sources {
        push_general_source(&mut bytes, source);
    }
    bytes
}

pub(super) fn push_general_source(bytes: &mut Vec<u8>, source: &GeneralBodySource) {
    match source {
        GeneralBodySource::Exact(key) => {
            bytes.push(0);
            bytes.extend_from_slice(&key.document_id.0.to_le_bytes());
            bytes.extend_from_slice(&key.source_revision.to_le_bytes());
            push_projection_bytes(bytes, key.source_digest.as_bytes());
            bytes.extend_from_slice(&key.definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&key.producer_feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, key.canonical_input_digest.as_bytes());
            push_projection_bytes(bytes, key.exact_input_digest.as_bytes());
            push_projection_bytes(bytes, key.evaluator.as_bytes());
            push_projection_bytes(bytes, key.backend.as_bytes());
            push_projection_bytes(bytes, key.tolerance.as_bytes());
            push_projection_bytes(bytes, key.schema.as_bytes());
            push_projection_bytes(bytes, key.result_fingerprint.as_bytes());
        }
        GeneralBodySource::CanonicalMesh {
            definition_id,
            feature_id,
            geometry_digest,
        } => {
            bytes.push(1);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExtrusion {
            definition_id,
            profile_id,
            extrusion_id,
            geometry_digest,
        } => {
            bytes.push(2);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&profile_id.0.to_le_bytes());
            bytes.extend_from_slice(&extrusion_id.0.to_le_bytes());
            push_projection_bytes(bytes, geometry_digest.as_bytes());
        }
        GeneralBodySource::CanonicalExactGraph {
            definition_id,
            producer_feature_id,
            graph_digest,
        } => {
            bytes.push(3);
            bytes.extend_from_slice(&definition_id.0.to_le_bytes());
            bytes.extend_from_slice(&producer_feature_id.0.to_le_bytes());
            push_projection_bytes(bytes, graph_digest.as_bytes());
        }
    }
}

pub(super) fn push_dimensions(bytes: &mut Vec<u8>, dimensions: PieceDimensions) {
    bytes.extend_from_slice(&dimensions.length_mm.to_bits().to_le_bytes());
    bytes.extend_from_slice(&dimensions.width_mm.to_bits().to_le_bytes());
    bytes.extend_from_slice(&dimensions.height_mm.to_bits().to_le_bytes());
}

pub(super) fn push_validation_state(bytes: &mut Vec<u8>, state: ValidationState) {
    bytes.push(match state {
        ValidationState::Passed => 0,
        ValidationState::Failed => 1,
        ValidationState::NotEvaluated => 2,
        ValidationState::Unavailable => 3,
    });
}

pub(super) fn evidence_token(evidence: &EvidenceClass) -> &'static str {
    match evidence {
        EvidenceClass::Exact => "Exact",
        EvidenceClass::Tolerant(_) => "Tolerant",
    }
}

pub(super) fn format_number(value: f64) -> String {
    let formatted = format!("{value:.9}");
    formatted
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_owned()
}
