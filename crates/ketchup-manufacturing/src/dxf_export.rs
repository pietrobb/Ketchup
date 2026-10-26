#![forbid(unsafe_code)]

use ketchup_geometry::linalg::{cross2, dot2};
use ketchup_model::tolerance::{MAX_COORDINATE_MM, ROUNDING};
use std::collections::BTreeSet;
use std::fmt::{self, Write as _};

use ketchup_geometry::sketch::{
    SketchError, SketchSpec, SolvedSketchRegionEdge, SolvedSketchRegionProfile, WorkplaneFrame,
};
use ketchup_model::document::{
    CanonicalError, Definition, Feature, FeatureKind, ProfileSegment, Snapshot, Transform,
};
pub(crate) use ketchup_model::drawing_export::layers::R12Layers;
use ketchup_model::import::{DxfImportError, DxfImportOptions, inspect_dxf, read_dxf_underlay};

use crate::dxf_annotations::{
    DEFAULT_PLAN_SCALE, MAX_PLAN_SCALE, PlanAnnotations, plan_annotations,
};

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
    /// The profiles of `part` do not lie in a plane parallel to world XY, or
    /// are scaled unevenly or sheared.
    UnsupportedTransform {
        part: String,
    },
    Transform(CanonicalError),
    /// The scale is not 1:N with N from 1 to [`MAX_PLAN_SCALE`].
    InvalidScale,
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
            Self::UnsupportedTransform { part } => {
                return write!(
                    formatter,
                    "the profiles of {part:?} do not lie in a plane parallel to XY under a \
                     uniform scale; hide that part or turn it flat before exporting the plan"
                );
            }
            Self::InvalidScale => {
                return write!(
                    formatter,
                    "the plan scale is 1:N with N from 1 to {MAX_PLAN_SCALE}"
                );
            }
            Self::Empty => {
                "the visible model contains no exportable planar profiles or annotations"
            }
            Self::TooManyProfiles => "DXF export exceeds the bounded 170 profile limit",
            Self::TooManySegments => "DXF export exceeds the bounded 10,000 segment limit",
            Self::InvalidGeometry => "DXF profile contains invalid or out-of-envelope geometry",
            Self::UnsupportedCurve => "DXF export does not approximate cubic profile curves",
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
/// The app writes the same drawing as DWG through `ketchup-dwg`, which the
/// report calls out. The visible annotations are drawn
/// at 1:100, see [`export_visible_plan_dxf`].
pub fn export_visible_profiles_dxf(
    snapshot: &Snapshot,
) -> Result<DxfProfileExport, DxfProfileExportError> {
    export_visible_plan_dxf(snapshot, DEFAULT_PLAN_SCALE)
}

/// [`export_visible_profiles_dxf`] with the visible annotations drawn at `1:scale`:
/// their texts as TEXT, their dimensions as DIMENSION, each on the layer of its tag.
pub fn export_visible_plan_dxf(
    snapshot: &Snapshot,
    scale: f64,
) -> Result<DxfProfileExport, DxfProfileExportError> {
    if !(1.0..=MAX_PLAN_SCALE).contains(&scale) {
        return Err(DxfProfileExportError::InvalidScale);
    }
    let mut profiles = Vec::new();
    let mut segment_count = 0_usize;
    let mut elevated_profile_count = 0_usize;
    let mut renamed_layers = BTreeSet::new();
    let mut r12_layers = R12Layers::default();
    let source_layer = |feature: &Feature, definition: &Definition| -> String {
        feature
            .name()
            .strip_prefix(IMPORTED_LAYER_PREFIX)
            .unwrap_or_else(|| definition.name())
            .to_owned()
    };
    // Parts take their layers before annotations, each group by name.
    let part_sources = snapshot
        .scene_query()
        .into_iter()
        .filter(|item| item.visible)
        .filter_map(|occurrence| snapshot.definition(occurrence.definition_id))
        .flat_map(|definition| {
            definition
                .feature_ids()
                .iter()
                .filter_map(|id| snapshot.feature(*id))
                .filter(|feature| {
                    matches!(
                        feature.kind(),
                        FeatureKind::Profile { .. } | FeatureKind::Sketch(_)
                    )
                })
                .map(|feature| source_layer(feature, definition))
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    r12_layers.claim(part_sources.iter().map(String::as_str));
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
            let source_layer = source_layer(feature, definition);
            let source_layer = source_layer.as_str();
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
            let layer = r12_layers.layer(source_layer);
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
                let vertices =
                    polyline_vertices(&source_segments, closed, transform).map_err(|error| {
                        match error {
                            DxfProfileExportError::UnsupportedTransform { .. } => {
                                DxfProfileExportError::UnsupportedTransform {
                                    part: definition.name().to_owned(),
                                }
                            }
                            other => other,
                        }
                    })?;
                if transform.matrix()[11].abs() > EPSILON {
                    elevated_profile_count += 1;
                }
                profiles.push(ExportProfile {
                    layer: layer.clone(),
                    closed,
                    vertices,
                });
                if profiles.len() > MAX_EXPORT_PROFILES {
                    return Err(DxfProfileExportError::TooManyProfiles);
                }
            }
        }
    }
    let annotations = plan_annotations(snapshot, scale, &mut r12_layers);
    if profiles.is_empty() && annotations.is_empty() {
        return Err(DxfProfileExportError::Empty);
    }

    let layers = profiles
        .iter()
        .map(|profile| profile.layer.clone())
        .collect::<BTreeSet<_>>();
    let profiles_only = encode_dxf(&profiles, &PlanAnnotations::default());
    if !profiles.is_empty() {
        let inspected = inspect_dxf(&profiles_only, DxfImportOptions::new(None))
            .map_err(DxfProfileExportError::Reimport)?;
        if inspected.profiles().len() != profiles.len()
            || inspected.layers().iter().cloned().collect::<BTreeSet<_>>() != layers
        {
            return Err(DxfProfileExportError::VerificationFailed);
        }
    }
    let dxf = if annotations.is_empty() {
        profiles_only
    } else {
        let dxf = encode_dxf(&profiles, &annotations);
        let drawn = read_dxf_underlay(&dxf, DxfImportOptions::new(None))
            .map_err(DxfProfileExportError::Reimport)?;
        if drawn.lines.text_count() != annotations.drawn_texts
            || !annotations
                .layers
                .iter()
                .all(|layer| drawn.layers.contains(layer))
        {
            return Err(DxfProfileExportError::VerificationFailed);
        }
        dxf
    };

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
        "schema={DXF_PROFILE_EXPORT_SCHEMA_V1}\nauthority=current canonical snapshot\nformat=ASCII DXF R12 (AC1009)\nunit=millimetre\ngeometry=one 2D POLYLINE per profile, circular arcs as bulges\nlayers=imported DXF layer identity or canonical definition name, written in ASCII (accents dropped, other characters as _)\nrenamed_layer_count={}\nprofile_count={}\nsegment_count={segment_count}\nelevated_profile_count={elevated_profile_count}\nelevation=a profile in a plane parallel to XY above or below z = 0 is drawn where it lies in plan; its height is not written\nlayer_count={}\nsource_digest={}\neditability_loss=constraints, dimensions, feature dependencies, body topology, materials, colors, and Undo history are not preserved\nprofile_identity_loss=occurrence and feature names are not written; each profile is one polyline on its layer\ndwg=a .dwg destination gets this drawing as AutoCAD 2018 DWG, its dimensions still DIMENSION on the layers of their tags; a DWG drawing imports through its DXF conversion\nomitted_non_profile_feature_count={omitted_feature_count}\n{}",
        renamed_layers.len(),
        profiles.len(),
        layers.len(),
        snapshot.canonical_digest(),
        annotation_report(&annotations, scale),
    );
    Ok(DxfProfileExport { dxf, loss_report })
}

/// The lines of the loss report about the annotations.
fn annotation_report(annotations: &PlanAnnotations, scale: f64) -> String {
    format!(
        "scale=1:{scale}\ntext_height=height on paper times the scale, in model millimetres\n\
         annotation_text_count={}\nannotation_dimension_count={}\nannotation_leader_count={}\n\
         annotation_line_count={}\nhidden_annotation_count={}\n\
         dimensions=aligned DIMENSION drawn by its anonymous block *Dn (line, oblique ticks, \
         extension lines and the value Ketchup measured); a reader that redraws it measures in plan\n\
         dimension_value_override_count={}\ndimension_outside_plan_count={}\n\
         leaders=a LINE and a SOLID arrowhead, then the TEXT\n\
         annotation_layers=the name of the annotation's tag, or Annotations without one\n",
        annotations.texts,
        annotations.dimensions,
        annotations.leaders,
        annotations.lines,
        annotations.hidden,
        annotations.value_overrides,
        annotations.outside_plan,
    )
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
    // The profile's plane must be parallel to world XY (its axes have no z);
    // a height above or below z = 0 is left out, as a plan shows it.
    if matrix.iter().any(|value| !value.is_finite())
        || matrix[8].abs() > EPSILON
        || matrix[9].abs() > EPSILON
        || scale_x <= EPSILON
        || (scale_x - scale_y).abs() > EPSILON * scale
        || dot.abs() > EPSILON * scale * scale
        || determinant.abs() <= EPSILON
    {
        return Err(DxfProfileExportError::UnsupportedTransform {
            part: String::new(),
        });
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

fn encode_dxf(profiles: &[ExportProfile], annotations: &PlanAnnotations) -> Vec<u8> {
    let mut layers = profiles
        .iter()
        .map(|profile| profile.layer.as_str())
        .chain(annotations.layers.iter().map(String::as_str))
        .collect::<BTreeSet<_>>();
    let annotated = !annotations.is_empty();
    if annotated {
        // Dimension blocks draw on layer 0, so that they take their dimension's layer.
        layers.insert("0");
    }
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
    output.push_str("0\nENDTAB\n");
    if annotated {
        output.push_str(
            "0\nTABLE\n2\nSTYLE\n70\n1\n0\nSTYLE\n2\nSTANDARD\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n2.5\n3\ntxt\n4\n\n0\nENDTAB\n\
             0\nTABLE\n2\nDIMSTYLE\n70\n1\n0\nDIMSTYLE\n2\nSTANDARD\n70\n0\n0\nENDTAB\n",
        );
    }
    output.push_str("0\nENDSEC\n");
    if annotated {
        output.push_str("0\nSECTION\n2\nBLOCKS\n");
        output.push_str(&annotations.blocks);
        output.push_str("0\nENDSEC\n");
    }
    output.push_str("0\nSECTION\n2\nENTITIES\n");
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
    output.push_str(&annotations.entities);
    output.push_str("0\nENDSEC\n0\nEOF\n");
    output.into_bytes()
}

pub(crate) fn number(value: f64) -> String {
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
