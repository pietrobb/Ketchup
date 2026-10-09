#![forbid(unsafe_code)]

use ketchup_geometry::linalg::{cross2, dot2};
use ketchup_model::tolerance::{MAX_COORDINATE_MM, ROUNDING};
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use ketchup_geometry::sketch::{
    SketchError, SketchSpec, SolvedSketchRegionEdge, SolvedSketchRegionProfile, WorkplaneFrame,
};
use ketchup_model::document::{CanonicalError, FeatureKind, ProfileSegment, Snapshot, Transform};
use ketchup_model::import::{DxfImportError, DxfImportOptions, inspect_dxf};

pub const DXF_PROFILE_EXPORT_SCHEMA_V1: &str = "ketchup.dxf-profile-export.v1";
const MAX_EXPORT_PROFILES: usize = 170;
const MAX_EXPORT_SEGMENTS: usize = 10_000;
const EPSILON: f64 = ROUNDING;
const IMPORTED_LAYER_PREFIX: &str = "DXF profile · ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DxfProfileExport {
    pub dxf: Vec<u8>,
    pub loss_report: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DxfProfileExportError {
    Empty,
    TooManyProfiles,
    TooManySegments,
    InvalidGeometry,
    Sketch(SketchError),
    UnsupportedCurve,
    UnsupportedTransform,
    Transform(CanonicalError),
    VerificationFailed,
    Reimport(DxfImportError),
}

impl fmt::Display for DxfProfileExportError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Reimport(error) => {
                return write!(formatter, "exported DXF does not read back: {error}");
            }
            Self::Transform(error) => {
                return write!(formatter, "sketch workplane transform is invalid: {error}");
            }
            Self::Sketch(error) => {
                return write!(formatter, "sketch geometry cannot be exported: {error}");
            }
            Self::Empty => "the visible model contains no exportable planar profiles",
            Self::TooManyProfiles => "DXF export exceeds the bounded 170 profile limit",
            Self::TooManySegments => "DXF export exceeds the bounded 10,000 segment limit",
            Self::InvalidGeometry => "DXF profile contains invalid or out-of-envelope geometry",
            Self::UnsupportedCurve => "DXF export does not approximate cubic profile curves",
            Self::UnsupportedTransform => {
                "DXF export requires profiles to remain in world XY under a uniform planar transform"
            }
            Self::VerificationFailed => "generated DXF failed deterministic bounded reinspection",
        })
    }
}

impl std::error::Error for DxfProfileExportError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Reimport(error) => Some(error),
            Self::Transform(error) => Some(error),
            Self::Sketch(error) => Some(error),
            _ => None,
        }
    }
}

/// One profile as one 2D polyline: each vertex with the bulge of the segment
/// that starts there (0 for a line, tan(sweep / 4) for an arc, negative when
/// clockwise). An open profile ends with its last point and bulge 0.
#[derive(Clone, Debug)]
struct ExportProfile {
    layer: String,
    closed: bool,
    vertices: Vec<([f64; 2], f64)>,
}

/// Export every visible canonical polygon, segment, or solved Sketch profile as bounded ASCII DXF.
///
/// The file is AutoCAD R12 (AC1009), which needs no handles or subclass markers
/// and is read by AutoCAD and CAM software alike. Each profile is one POLYLINE,
/// so two profiles on the same layer cannot be joined by a receiving importer.
/// Native DWG is deliberately outside this open, deterministic workflow and is
/// called out in the report.
pub fn export_visible_profiles_dxf(
    snapshot: &Snapshot,
) -> Result<DxfProfileExport, DxfProfileExportError> {
    let mut profiles = Vec::new();
    let mut segment_count = 0_usize;
    let mut renamed_layers = BTreeSet::new();
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
            let source_layer = feature
                .name()
                .strip_prefix(IMPORTED_LAYER_PREFIX)
                .unwrap_or_else(|| definition.name());
            let (source_profiles, transform) = match feature.kind() {
                FeatureKind::Profile { segments, closed } => {
                    (vec![(segments.clone(), *closed)], occurrence.transform)
                }
                FeatureKind::Sketch(sketch) => {
                    let workplane = snapshot
                        .feature(sketch.workplane)
                        .ok_or(DxfProfileExportError::InvalidGeometry)?;
                    let FeatureKind::Workplane(workplane) = workplane.kind() else {
                        return Err(DxfProfileExportError::InvalidGeometry);
                    };
                    (
                        sketch_profiles(sketch)?,
                        occurrence
                            .transform
                            .compose(workplane_transform(workplane.frame)?),
                    )
                }
                _ => continue,
            };
            let layer = layer_name(source_layer);
            if layer != source_layer {
                renamed_layers.insert(source_layer.to_owned());
            }
            for (source_segments, closed) in source_profiles {
                segment_count = segment_count
                    .checked_add(source_segments.len())
                    .ok_or(DxfProfileExportError::TooManySegments)?;
                if segment_count > MAX_EXPORT_SEGMENTS {
                    return Err(DxfProfileExportError::TooManySegments);
                }
                profiles.push(ExportProfile {
                    layer: layer.clone(),
                    closed,
                    vertices: polyline_vertices(&source_segments, closed, transform)?,
                });
                if profiles.len() > MAX_EXPORT_PROFILES {
                    return Err(DxfProfileExportError::TooManyProfiles);
                }
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
    let inspected =
        inspect_dxf(&dxf, DxfImportOptions::new(None)).map_err(DxfProfileExportError::Reimport)?;
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
                FeatureKind::Profile { .. } | FeatureKind::Sketch(_)
            )
        })
        .count();
    let loss_report = format!(
        "schema={DXF_PROFILE_EXPORT_SCHEMA_V1}\nauthority=current canonical snapshot\nformat=ASCII DXF R12 (AC1009)\nunit=millimetre\ngeometry=one 2D POLYLINE per profile, circular arcs as bulges\nlayers=imported DXF layer identity or canonical definition name, written in ASCII (accents dropped, other characters as _)\nrenamed_layer_count={}\nprofile_count={}\nsegment_count={segment_count}\nlayer_count={}\nsource_digest={}\neditability_loss=constraints, dimensions, feature dependencies, body topology, materials, colors, and Undo history are not preserved\nprofile_identity_loss=occurrence and feature names are not written; each profile is one polyline on its layer\ndwg=unsupported; native DWG import/export is refused because no open deterministic DWG codec is present\nomitted_non_profile_feature_count={omitted_feature_count}\n",
        renamed_layers.len(),
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

fn sketch_profiles(
    sketch: &SketchSpec,
) -> Result<Vec<(Vec<ProfileSegment>, bool)>, DxfProfileExportError> {
    let regions = sketch
        .solved_regions()
        .map_err(DxfProfileExportError::Sketch)?;
    let mut profiles = Vec::new();
    for region in regions {
        profiles.push((solved_profile_segments(&region.outer)?, true));
        for hole in &region.holes {
            profiles.push((solved_profile_segments(hole)?, true));
        }
    }
    Ok(profiles)
}

fn solved_profile_segments(
    profile: &SolvedSketchRegionProfile,
) -> Result<Vec<ProfileSegment>, DxfProfileExportError> {
    match profile {
        SolvedSketchRegionProfile::Polyline(points) => polygon_segments(points),
        SolvedSketchRegionProfile::Boundary(edges) => Ok(edges
            .iter()
            .map(|edge| match edge {
                SolvedSketchRegionEdge::Line { start_mm, end_mm } => ProfileSegment::Line {
                    start_mm: *start_mm,
                    end_mm: *end_mm,
                },
                SolvedSketchRegionEdge::Arc {
                    start_mm,
                    end_mm,
                    center_mm,
                    clockwise,
                } => ProfileSegment::CircularArc {
                    start_mm: *start_mm,
                    end_mm: *end_mm,
                    center_mm: *center_mm,
                    clockwise: *clockwise,
                },
                SolvedSketchRegionEdge::CubicBezier {
                    start_mm,
                    control_1_mm,
                    control_2_mm,
                    end_mm,
                } => ProfileSegment::CubicBezier {
                    start_mm: *start_mm,
                    control_1_mm: *control_1_mm,
                    control_2_mm: *control_2_mm,
                    end_mm: *end_mm,
                },
            })
            .collect()),
        SolvedSketchRegionProfile::Circle {
            center_mm,
            radius_mm,
        } => {
            let right = [center_mm[0] + radius_mm, center_mm[1]];
            let left = [center_mm[0] - radius_mm, center_mm[1]];
            Ok(vec![
                ProfileSegment::CircularArc {
                    start_mm: right,
                    end_mm: left,
                    center_mm: *center_mm,
                    clockwise: false,
                },
                ProfileSegment::CircularArc {
                    start_mm: left,
                    end_mm: right,
                    center_mm: *center_mm,
                    clockwise: false,
                },
            ])
        }
    }
}

fn workplane_transform(frame: WorkplaneFrame) -> Result<Transform, DxfProfileExportError> {
    frame.validate().map_err(DxfProfileExportError::Sketch)?;
    Transform::from_matrix([
        frame.x_axis[0],
        frame.y_axis[0],
        frame.normal[0],
        frame.origin_mm[0],
        frame.x_axis[1],
        frame.y_axis[1],
        frame.normal[1],
        frame.origin_mm[1],
        frame.x_axis[2],
        frame.y_axis[2],
        frame.normal[2],
        frame.origin_mm[2],
        0.0,
        0.0,
        0.0,
        1.0,
    ])
    .map_err(DxfProfileExportError::Transform)
}

fn polyline_vertices(
    segments: &[ProfileSegment],
    closed: bool,
    transform: Transform,
) -> Result<Vec<([f64; 2], f64)>, DxfProfileExportError> {
    let pieces = transform_segments(segments, transform)?;
    let joined = |end: [f64; 2], start: [f64; 2]| {
        distance(end, start) <= EPSILON * end[0].abs().max(end[1].abs()).max(1.0)
    };
    if pieces
        .windows(2)
        .any(|pair| !joined(pair[0].end, pair[1].start))
        || (closed && !joined(pieces[pieces.len() - 1].end, pieces[0].start))
    {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    let mut vertices = pieces
        .iter()
        .map(|piece| (piece.start, piece.bulge))
        .collect::<Vec<_>>();
    if !closed {
        vertices.push((pieces[pieces.len() - 1].end, 0.0));
    }
    Ok(vertices)
}

/// One segment in world XY, with the bulge of the polyline vertex at its start.
struct ExportPiece {
    start: [f64; 2],
    end: [f64; 2],
    bulge: f64,
}

fn transform_segments(
    segments: &[ProfileSegment],
    transform: Transform,
) -> Result<Vec<ExportPiece>, DxfProfileExportError> {
    if segments.is_empty() {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    let matrix = transform.matrix();
    let scale_x = matrix[0].hypot(matrix[4]);
    let scale_y = matrix[1].hypot(matrix[5]);
    let (x_axis, y_axis) = ([matrix[0], matrix[4]], [matrix[1], matrix[5]]);
    let dot = dot2(x_axis, y_axis);
    let determinant = cross2(x_axis, y_axis);
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
                Ok(ExportPiece {
                    start,
                    end,
                    bulge: 0.0,
                })
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
                let (from, to) = if world_clockwise {
                    (end, start)
                } else {
                    (start, end)
                };
                // Counter-clockwise sweep from `from` to `to`, in (0, 360).
                let sweep = (angle_degrees(to, center) - angle_degrees(from, center))
                    .rem_euclid(360.0)
                    .to_radians();
                if sweep <= 0.0 {
                    return Err(DxfProfileExportError::InvalidGeometry);
                }
                let bulge = (sweep / 4.0).tan();
                Ok(ExportPiece {
                    start,
                    end,
                    bulge: if world_clockwise { -bulge } else { bulge },
                })
            }
            ProfileSegment::CubicBezier { .. } | ProfileSegment::Spline { .. } => {
                Err(DxfProfileExportError::UnsupportedCurve)
            }
        })
        .collect()
}

fn transform_point(point: [f64; 2], matrix: &[f64; 16]) -> Result<[f64; 2], DxfProfileExportError> {
    if point.iter().any(|value| !value.is_finite()) {
        return Err(DxfProfileExportError::InvalidGeometry);
    }
    let [x, y, _] = ketchup_geometry::linalg::Affine3::from_row_major(*matrix)
        .transform_point([point[0], point[1], 0.0].into())
        .to_array();
    let transformed = [x, y];
    if transformed
        .iter()
        .any(|value| !value.is_finite() || value.abs() > MAX_COORDINATE_MM)
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

/// A layer name every DXF reader accepts: R12 names are ASCII without the
/// characters AutoCAD reserves, so accents are dropped (`Pôdorys` → `Podorys`)
/// and anything else becomes `_`.
fn layer_name(source: &str) -> String {
    let mut name = String::new();
    for character in source.chars() {
        let lower = character.to_lowercase().next().unwrap_or(character);
        let folded = match lower {
            'á' | 'ä' | 'à' | 'â' | 'ã' | 'å' | 'ā' | 'ă' | 'ą' => "a",
            'č' | 'ć' | 'ç' => "c",
            'ď' | 'đ' => "d",
            'é' | 'ě' | 'ë' | 'è' | 'ê' | 'ē' | 'ę' => "e",
            'í' | 'ì' | 'î' | 'ï' => "i",
            'ľ' | 'ĺ' | 'ł' => "l",
            'ň' | 'ń' | 'ñ' => "n",
            'ó' | 'ô' | 'ö' | 'ò' | 'õ' | 'ő' | 'ø' => "o",
            'ŕ' | 'ř' => "r",
            'š' | 'ś' | 'ş' => "s",
            'ť' | 'ţ' => "t",
            'ú' | 'ů' | 'ü' | 'ù' | 'û' | 'ű' => "u",
            'ý' | 'ÿ' => "y",
            'ž' | 'ź' | 'ż' => "z",
            'ß' => "ss",
            '<' | '>' | '/' | '\\' | '"' | ':' | ';' | '?' | '*' | '|' | ',' | '=' | '`' => "_",
            other if other.is_ascii_graphic() || other == ' ' => {
                name.push(character);
                continue;
            }
            _ => "_",
        };
        if character.is_uppercase() {
            name.push_str(&folded.to_ascii_uppercase());
        } else {
            name.push_str(folded);
        }
    }
    let name = name.trim();
    if name.is_empty() {
        "0".to_owned()
    } else {
        name.chars().take(255).collect()
    }
}

fn encode_dxf(profiles: &[ExportProfile]) -> Vec<u8> {
    let layers = profiles
        .iter()
        .map(|profile| profile.layer.as_str())
        .collect::<BTreeSet<_>>();
    let mut output = format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n\
         0\nTABLE\n2\nLTYPE\n70\n1\n0\nLTYPE\n2\nCONTINUOUS\n70\n0\n3\nSolid line\n72\n65\n73\n0\n40\n0\n0\nENDTAB\n\
         0\nTABLE\n2\nLAYER\n70\n{}\n",
        layers.len()
    );
    for layer in layers {
        write!(
            output,
            "0\nLAYER\n2\n{layer}\n70\n0\n62\n7\n6\nCONTINUOUS\n"
        )
        .expect("writing to a String cannot fail");
    }
    output.push_str("0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n");
    for profile in profiles {
        let layer = &profile.layer;
        write!(
            output,
            "0\nPOLYLINE\n8\n{layer}\n66\n1\n10\n0\n20\n0\n30\n0\n70\n{}\n",
            u8::from(profile.closed)
        )
        .expect("writing to a String cannot fail");
        for (point, bulge) in &profile.vertices {
            write!(
                output,
                "0\nVERTEX\n8\n{layer}\n10\n{}\n20\n{}\n30\n0\n",
                number(point[0]),
                number(point[1]),
            )
            .expect("writing to a String cannot fail");
            if *bulge != 0.0 {
                write!(output, "42\n{}\n", number(*bulge))
                    .expect("writing to a String cannot fail");
            }
        }
        write!(output, "0\nSEQEND\n8\n{layer}\n").expect("writing to a String cannot fail");
    }
    output.push_str("0\nENDSEC\n0\nEOF\n");
    output.into_bytes()
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
