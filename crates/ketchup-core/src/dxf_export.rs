#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use crate::document::{FeatureKind, ProfileSegment, Snapshot, Transform};
use crate::import::{DxfImportOptions, inspect_dxf};

pub const DXF_PROFILE_EXPORT_SCHEMA_V1: &str = "ketchup.dxf-profile-export.v1";
const MAX_EXPORT_PROFILES: usize = 170;
const MAX_EXPORT_SEGMENTS: usize = 10_000;
const MAX_ABS_MM: f64 = 1_000_000.0;
const EPSILON: f64 = 1.0e-9;
const IMPORTED_LAYER_PREFIX: &str = "DXF profile · ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DxfProfileExport {
    pub dxf: Vec<u8>,
    pub loss_report: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DxfProfileExportError {
    Empty,
    TooManyProfiles,
    TooManySegments,
    InvalidLayer,
    InvalidGeometry,
    UnsupportedCurve,
    UnsupportedTransform,
    VerificationFailed,
}

impl fmt::Display for DxfProfileExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Empty => "the visible model contains no exportable planar profiles",
            Self::TooManyProfiles => "DXF export exceeds the bounded 170 profile limit",
            Self::TooManySegments => "DXF export exceeds the bounded 10,000 segment limit",
            Self::InvalidLayer => "DXF layer name is empty, unsafe, non-ASCII, or too long",
            Self::InvalidGeometry => "DXF profile contains invalid or out-of-envelope geometry",
            Self::UnsupportedCurve => "DXF export does not approximate cubic profile curves",
            Self::UnsupportedTransform => {
                "DXF export requires profiles to remain in world XY under a uniform planar transform"
            }
            Self::VerificationFailed => "generated DXF failed deterministic bounded reinspection",
        })
    }
}

impl std::error::Error for DxfProfileExportError {}

#[derive(Clone, Debug)]
struct ExportProfile {
    layer: String,
    block_name: String,
    entities: Vec<ExportEntity>,
}

#[derive(Clone, Copy, Debug)]
enum ExportEntity {
    Line {
        start: [f64; 2],
        end: [f64; 2],
    },
    Arc {
        center: [f64; 2],
        radius: f64,
        start_degrees: f64,
        end_degrees: f64,
    },
}

/// Export every visible canonical polygon/segment profile as a bounded ASCII DXF.
///
/// Each profile is isolated in a block so two profiles on the same layer cannot
/// be accidentally joined by a receiving importer. Native DWG is deliberately
/// outside this open, deterministic workflow and is called out in the report.
pub fn export_visible_profiles_dxf(
    snapshot: &Snapshot,
) -> Result<DxfProfileExport, DxfProfileExportError> {
    let mut profiles = Vec::new();
    let mut segment_count = 0_usize;
    for occurrence in snapshot
        .scene_query()
        .into_iter()
        .filter(|item| item.visible)
    {
        let definition = snapshot
            .definition(occurrence.definition_id)
            .ok_or(DxfProfileExportError::InvalidGeometry)?;
        for feature_id in definition.feature_ids() {
            let feature = snapshot
                .feature(*feature_id)
                .ok_or(DxfProfileExportError::InvalidGeometry)?;
            let layer = feature
                .name()
                .strip_prefix(IMPORTED_LAYER_PREFIX)
                .unwrap_or_else(|| definition.name());
            let source_segments = match feature.kind() {
                FeatureKind::Profile { points_mm } => polygon_segments(points_mm)?,
                FeatureKind::SegmentProfile { segments, .. } => segments.clone(),
                _ => continue,
            };
            validate_layer(layer)?;
            segment_count = segment_count
                .checked_add(source_segments.len())
                .ok_or(DxfProfileExportError::TooManySegments)?;
            if segment_count > MAX_EXPORT_SEGMENTS {
                return Err(DxfProfileExportError::TooManySegments);
            }
            let entities = transform_segments(&source_segments, occurrence.transform)?;
            let block_name = format!(
                "KETCHUP_P_{:016X}_{:016X}_{:04}",
                occurrence.occurrence_id.0,
                feature.id().0,
                profiles.len() + 1
            );
            profiles.push(ExportProfile {
                layer: layer.to_owned(),
                block_name,
                entities,
            });
            if profiles.len() > MAX_EXPORT_PROFILES {
                return Err(DxfProfileExportError::TooManyProfiles);
            }
        }
    }
    if profiles.is_empty() {
        return Err(DxfProfileExportError::Empty);
    }

    let layers = profiles
        .iter()
        .map(|profile| profile.layer.clone())
        .collect::<BTreeSet<_>>();
    let dxf = encode_dxf(&profiles);
    let inspected = inspect_dxf(&dxf, DxfImportOptions::new(None))
        .map_err(|_| DxfProfileExportError::VerificationFailed)?;
    if inspected.profiles().len() != profiles.len()
        || inspected.layers().iter().cloned().collect::<BTreeSet<_>>() != layers
    {
        return Err(DxfProfileExportError::VerificationFailed);
    }

    let omitted_feature_count = snapshot
        .features()
        .filter(|feature| {
            !matches!(
                feature.kind(),
                FeatureKind::Profile { .. } | FeatureKind::SegmentProfile { .. }
            )
        })
        .count();
    let loss_report = format!(
        "schema={DXF_PROFILE_EXPORT_SCHEMA_V1}\nauthority=current canonical snapshot\nformat=ASCII DXF AC1027\nunit=millimetre\ngeometry=LINE and circular ARC entities grouped into one block per profile\nlayers=preserved from imported DXF layer identity or canonical definition name\nprofile_count={}\nsegment_count={segment_count}\nlayer_count={}\nsource_digest={}\neditability_loss=constraints, dimensions, feature dependencies, body topology, materials, colors, and Undo history are not preserved\nprofile_identity_loss=occurrence and feature names are represented only by deterministic block identities\ndwg=unsupported; native DWG import/export is refused because no open deterministic DWG codec is present\nomitted_non_profile_feature_count={omitted_feature_count}\n",
        profiles.len(),
        layers.len(),
        snapshot.canonical_digest(),
    );
    Ok(DxfProfileExport { dxf, loss_report })
}

fn polygon_segments(points: &[[f64; 2]]) -> Result<Vec<ProfileSegment>, DxfProfileExportError> {
    if points.len() < 3 {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    Ok(points
        .iter()
        .copied()
        .zip(points.iter().copied().cycle().skip(1))
        .take(points.len())
        .map(|(start_mm, end_mm)| ProfileSegment::Line { start_mm, end_mm })
        .collect())
}

fn transform_segments(
    segments: &[ProfileSegment],
    transform: Transform,
) -> Result<Vec<ExportEntity>, DxfProfileExportError> {
    if segments.is_empty() {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    let matrix = transform.matrix();
    let scale_x = matrix[0].hypot(matrix[4]);
    let scale_y = matrix[1].hypot(matrix[5]);
    let dot = matrix[0] * matrix[1] + matrix[4] * matrix[5];
    let determinant = matrix[0] * matrix[5] - matrix[1] * matrix[4];
    let scale = scale_x.max(scale_y).max(1.0);
    if matrix.iter().any(|value| !value.is_finite())
        || matrix[8].abs() > EPSILON
        || matrix[9].abs() > EPSILON
        || matrix[11].abs() > EPSILON
        || scale_x <= EPSILON
        || (scale_x - scale_y).abs() > EPSILON * scale
        || dot.abs() > EPSILON * scale * scale
        || determinant.abs() <= EPSILON
    {
        return Err(DxfProfileExportError::UnsupportedTransform);
    }
    let reflected = determinant < 0.0;
    segments
        .iter()
        .map(|segment| match segment {
            ProfileSegment::Line { start_mm, end_mm } => {
                let start = transform_point(*start_mm, matrix)?;
                let end = transform_point(*end_mm, matrix)?;
                if distance(start, end) <= EPSILON {
                    return Err(DxfProfileExportError::InvalidGeometry);
                }
                Ok(ExportEntity::Line { start, end })
            }
            ProfileSegment::CircularArc {
                start_mm,
                end_mm,
                center_mm,
                clockwise,
            } => {
                let start = transform_point(*start_mm, matrix)?;
                let end = transform_point(*end_mm, matrix)?;
                let center = transform_point(*center_mm, matrix)?;
                let start_radius = distance(start, center);
                let end_radius = distance(end, center);
                if start_radius <= EPSILON
                    || distance(start, end) <= EPSILON
                    || (start_radius - end_radius).abs()
                        > EPSILON * start_radius.max(end_radius).max(1.0)
                {
                    return Err(DxfProfileExportError::InvalidGeometry);
                }
                let world_clockwise = *clockwise != reflected;
                let (dxf_start, dxf_end) = if world_clockwise {
                    (end, start)
                } else {
                    (start, end)
                };
                Ok(ExportEntity::Arc {
                    center,
                    radius: start_radius,
                    start_degrees: angle_degrees(dxf_start, center),
                    end_degrees: angle_degrees(dxf_end, center),
                })
            }
            ProfileSegment::CubicBezier { .. } => Err(DxfProfileExportError::UnsupportedCurve),
        })
        .collect()
}

fn transform_point(point: [f64; 2], matrix: &[f64; 16]) -> Result<[f64; 2], DxfProfileExportError> {
    if point.iter().any(|value| !value.is_finite()) {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    let transformed = [
        matrix[0] * point[0] + matrix[1] * point[1] + matrix[3],
        matrix[4] * point[0] + matrix[5] * point[1] + matrix[7],
    ];
    if transformed
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_ABS_MM)
    {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    Ok(transformed)
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

fn angle_degrees(point: [f64; 2], center: [f64; 2]) -> f64 {
    let angle = (point[1] - center[1])
        .atan2(point[0] - center[0])
        .to_degrees()
        .rem_euclid(360.0);
    if (360.0 - angle).abs() <= f64::EPSILON {
        0.0
    } else {
        angle
    }
}

fn validate_layer(layer: &str) -> Result<(), DxfProfileExportError> {
    if layer.is_empty()
        || layer.len() > 255
        || !layer.is_ascii()
        || layer.bytes().any(|byte| !(32..=126).contains(&byte))
        || layer.contains(['<', '>', '/', '\\', '"', ':', ';', '?', '*', '|', ',', '='])
    {
        Err(DxfProfileExportError::InvalidLayer)
    } else {
        Ok(())
    }
}

fn encode_dxf(profiles: &[ExportProfile]) -> Vec<u8> {
    let mut output = String::from(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1027\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n",
    );
    for profile in profiles {
        write!(
            output,
            "0\nBLOCK\n8\n0\n2\n{}\n3\n{}\n70\n0\n10\n0\n20\n0\n30\n0\n",
            profile.block_name, profile.block_name
        )
        .expect("writing to a String cannot fail");
        for entity in &profile.entities {
            encode_entity(&mut output, entity, "0");
        }
        output.push_str("0\nENDBLK\n8\n0\n");
    }
    output.push_str("0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for profile in profiles {
        write!(
            output,
            "0\nINSERT\n8\n{}\n2\n{}\n10\n0\n20\n0\n30\n0\n",
            profile.layer, profile.block_name
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("0\nENDSEC\n0\nEOF\n");
    output.into_bytes()
}

fn encode_entity(output: &mut String, entity: &ExportEntity, layer: &str) {
    match entity {
        ExportEntity::Line { start, end } => {
            write!(
                output,
                "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
                number(start[0]),
                number(start[1]),
                number(end[0]),
                number(end[1]),
            )
            .expect("writing to a String cannot fail");
        }
        ExportEntity::Arc {
            center,
            radius,
            start_degrees,
            end_degrees,
        } => {
            write!(
                output,
                "0\nARC\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n40\n{}\n50\n{}\n51\n{}\n",
                number(center[0]),
                number(center[1]),
                number(*radius),
                number(*start_degrees),
                number(*end_degrees),
            )
            .expect("writing to a String cannot fail");
        }
    }
}

fn number(value: f64) -> String {
    let canonical = if value.abs() <= f64::EPSILON {
        0.0
    } else {
        value
    };
    let mut output = format!("{canonical:.17}");
    while output.ends_with('0') {
        output.pop();
    }
    if output.ends_with('.') {
        output.pop();
    }
    output
}
