//! Plain data produced by evaluating a rule program.
//!
//! Every part has a rigid frame: its local origin `at_mm` in world millimetres
//! and a rotation (identity for axis-aligned parts). A panel occupies
//! `0..size_mm` in its own frame. Holes and pockets are machined into one of
//! the six local faces and use face-local coordinates.

use crate::frame::{self, Mat3, Obb};
use serde::Serialize;

/// One of the six faces of an axis-aligned part.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub enum Face {
    #[serde(rename = "x-")]
    XMin,
    #[serde(rename = "x+")]
    XMax,
    #[serde(rename = "y-")]
    YMin,
    #[serde(rename = "y+")]
    YMax,
    #[serde(rename = "z-")]
    ZMin,
    #[serde(rename = "z+")]
    ZMax,
}

impl Face {
    pub const ALL: [Self; 6] = [
        Self::XMin,
        Self::XMax,
        Self::YMin,
        Self::YMax,
        Self::ZMin,
        Self::ZMax,
    ];

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::XMin => "x-",
            Self::XMax => "x+",
            Self::YMin => "y-",
            Self::YMax => "y+",
            Self::ZMin => "z-",
            Self::ZMax => "z+",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|face| face.name() == text)
    }

    /// Axis index (0 = x, 1 = y, 2 = z) the face is perpendicular to.
    #[must_use]
    pub const fn axis(self) -> usize {
        match self {
            Self::XMin | Self::XMax => 0,
            Self::YMin | Self::YMax => 1,
            Self::ZMin | Self::ZMax => 2,
        }
    }

    #[must_use]
    pub const fn is_max(self) -> bool {
        matches!(self, Self::XMax | Self::YMax | Self::ZMax)
    }

    #[must_use]
    pub const fn from_axis(axis: usize, max: bool) -> Self {
        match (axis, max) {
            (0, false) => Self::XMin,
            (0, true) => Self::XMax,
            (1, false) => Self::YMin,
            (1, true) => Self::YMax,
            (_, false) => Self::ZMin,
            (_, true) => Self::ZMax,
        }
    }

    #[must_use]
    pub const fn opposite(self) -> Self {
        Self::from_axis(self.axis(), !self.is_max())
    }

    /// Face-local (u, v) axes: z faces use (x, y), x faces (y, z), y faces (x, z).
    #[must_use]
    pub const fn uv_axes(self) -> (usize, usize) {
        match self.axis() {
            0 => (1, 2),
            1 => (0, 2),
            _ => (0, 1),
        }
    }

    /// Unit vector pointing from the face into the material.
    #[must_use]
    pub fn inward(self) -> [f64; 3] {
        let mut normal = [0.0; 3];
        normal[self.axis()] = if self.is_max() { -1.0 } else { 1.0 };
        normal
    }

    /// Part-local point on this face for face coordinates (u, v).
    #[must_use]
    pub fn local_point(self, size_mm: [f64; 3], u: f64, v: f64) -> [f64; 3] {
        let (u_axis, v_axis) = self.uv_axes();
        let mut point = [0.0; 3];
        point[self.axis()] = if self.is_max() {
            size_mm[self.axis()]
        } else {
            0.0
        };
        point[u_axis] = u;
        point[v_axis] = v;
        point
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Param {
    pub name: String,
    pub value: f64,
    pub default: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub doc: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramFeatureKind {
    Transform,
    Workplane,
    Sketch,
    Pad,
    Cut,
    Revolve,
    Fillet,
    Chamfer,
    FaceOffset,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramParameterValueType {
    Length,
    Angle,
    Scalar,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramFeatureParameter {
    pub path: String,
    pub value_type: ProgramParameterValueType,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramFeature {
    /// Stable identity inside one part. Reordering program statements does not change it.
    pub name: String,
    pub kind: ProgramFeatureKind,
    pub parameters: Vec<ProgramFeatureParameter>,
}

/// A cylindrical hole drilled perpendicular to a face.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hole {
    pub id: String,
    pub face: Face,
    pub u_mm: f64,
    pub v_mm: f64,
    pub diameter_mm: f64,
    pub depth_mm: f64,
}

/// A rectangular pocket milled perpendicular to a face (grooves, rabbets,
/// notches). The rectangle may run off the face edges.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pocket {
    pub id: String,
    pub face: Face,
    pub u_min_mm: f64,
    pub v_min_mm: f64,
    pub u_max_mm: f64,
    pub v_max_mm: f64,
    pub depth_mm: f64,
}

impl Pocket {
    /// Part-local removed box as (min, max).
    #[must_use]
    pub fn local_box(&self, size_mm: [f64; 3]) -> ([f64; 3], [f64; 3]) {
        let axis = self.face.axis();
        let (u_axis, v_axis) = self.face.uv_axes();
        let mut min = [0.0; 3];
        let mut max = [0.0; 3];
        min[u_axis] = self.u_min_mm;
        max[u_axis] = self.u_max_mm;
        min[v_axis] = self.v_min_mm;
        max[v_axis] = self.v_max_mm;
        if self.face.is_max() {
            min[axis] = size_mm[axis] - self.depth_mm;
            max[axis] = size_mm[axis];
        } else {
            min[axis] = 0.0;
            max[axis] = self.depth_mm;
        }
        (min, max)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramProfileSegment {
    pub name: String,
    pub start_mm: [f64; 2],
    pub end_mm: [f64; 2],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramEdgeFinishKind {
    Fillet,
    Chamfer,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramEdgeFillet {
    pub name: String,
    pub kind: ProgramEdgeFinishKind,
    pub edges: Vec<[String; 2]>,
    pub radius_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramCut {
    pub name: String,
    pub segments: Vec<ProgramProfileSegment>,
    pub depth_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramBooleanKind {
    /// Removes the tool's volume from the part.
    Subtract,
    /// Keeps only the volume the part shares with the tool.
    Intersect,
}

/// A boolean between a part and another body (a tool or a real part). The
/// tool is recorded as it was when the operation was declared: its body shape
/// and its own booleans; its holes, pockets, cuts and finishes are not part of
/// the removed or kept volume.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramBoolean {
    pub name: String,
    pub kind: ProgramBooleanKind,
    pub tool: Part,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramFaceOffset {
    pub name: String,
    pub face: String,
    pub distance_mm: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgramPartBody {
    Panel,
    Extrusion {
        segments: Vec<ProgramProfileSegment>,
        distance_mm: f64,
    },
    Revolve {
        segments: Vec<ProgramProfileSegment>,
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Part {
    /// Stable identity: the path given in the program, e.g. `korpus/bok_lavy`.
    pub name: String,
    pub size_mm: [f64; 3],
    /// World position of the local origin.
    pub at_mm: [f64; 3],
    /// Row-major rotation of the local frame; columns are the local axes in world.
    #[serde(skip_serializing_if = "frame::is_identity")]
    pub rotation: Mat3,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Axis index of the grain direction, if the material has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain_axis: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    pub body: ProgramPartBody,
    pub fillets: Vec<ProgramEdgeFillet>,
    pub cuts: Vec<ProgramCut>,
    pub face_offsets: Vec<ProgramFaceOffset>,
    /// Applied after every other feature, in program order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub booleans: Vec<ProgramBoolean>,
    /// Stable, operation-class-level source tree used by incremental reconciliation.
    pub features: Vec<ProgramFeature>,
    pub holes: Vec<Hole>,
    pub pockets: Vec<Pocket>,
}

impl Part {
    /// Bounds of the body in its own frame, before cuts and finishes.
    #[must_use]
    pub fn local_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let profile_bounds = |segments: &[ProgramProfileSegment]| {
            let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            for point in segments.iter().flat_map(|s| [s.start_mm, s.end_mm]) {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
            (min, max)
        };
        match &self.body {
            ProgramPartBody::Panel => ([0.0; 3], self.size_mm),
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
            } => {
                let (min, max) = profile_bounds(segments);
                ([min[0], min[1], 0.0], [max[0], max[1], *distance_mm])
            }
            ProgramPartBody::Revolve { segments, .. } => {
                let (min, max) = profile_bounds(segments);
                let radius = self.size_mm[0] * 0.5;
                ([-radius, min[1], -radius], [radius, max[1], radius])
            }
        }
    }

    #[must_use]
    pub fn obb(&self) -> Obb {
        let (min, max) = self.local_bounds();
        Obb::new(self.at_mm, &self.rotation, min, max)
    }

    /// World axis-aligned bounds as (min, max).
    #[must_use]
    pub fn world_bounds(&self) -> ([f64; 3], [f64; 3]) {
        self.obb().world_bounds()
    }

    #[must_use]
    pub fn is_rotated(&self) -> bool {
        !frame::is_identity(&self.rotation)
    }

    /// Row-major 4x4 local-to-world matrix.
    #[must_use]
    pub fn transform_matrix(&self) -> [f64; 16] {
        let mut matrix = [0.0; 16];
        for row in 0..3 {
            matrix[row * 4..row * 4 + 3].copy_from_slice(&self.rotation[row]);
            matrix[row * 4 + 3] = self.at_mm[row];
        }
        matrix[15] = 1.0;
        matrix
    }

    /// Row-major 4x4 matrix placing `other`'s frame inside this part's frame.
    #[must_use]
    pub fn frame_of(&self, other: &Self) -> [f64; 16] {
        let rotation = frame::multiply(&frame::transposed(&self.rotation), &other.rotation);
        let origin = self.to_local(other.at_mm);
        let mut matrix = [0.0; 16];
        for row in 0..3 {
            matrix[row * 4..row * 4 + 3].copy_from_slice(&rotation[row]);
            matrix[row * 4 + 3] = origin[row];
        }
        matrix[15] = 1.0;
        matrix
    }

    /// Whether the volume `other` occupies now was already subtracted from this part.
    #[must_use]
    pub fn subtracts(&self, other: &Self) -> bool {
        self.booleans.iter().any(|boolean| {
            boolean.kind == ProgramBooleanKind::Subtract
                && boolean.tool.name == other.name
                && boolean.tool.body == other.body
                && boolean.tool.size_mm == other.size_mm
                && boolean.tool.transform_matrix() == other.transform_matrix()
        })
    }

    /// World point for a part-local point.
    #[must_use]
    pub fn to_world(&self, local: [f64; 3]) -> [f64; 3] {
        let rotated = frame::apply(&self.rotation, local);
        std::array::from_fn(|axis| self.at_mm[axis] + rotated[axis])
    }

    /// Part-local point for a world point.
    #[must_use]
    pub fn to_local(&self, world: [f64; 3]) -> [f64; 3] {
        frame::apply_transposed(
            &self.rotation,
            std::array::from_fn(|axis| world[axis] - self.at_mm[axis]),
        )
    }

    pub fn refresh_feature_tree(&mut self) {
        let length = |path: &str, value| ProgramFeatureParameter {
            path: path.to_owned(),
            value_type: ProgramParameterValueType::Length,
            value,
        };
        let feature =
            |name: String, kind: ProgramFeatureKind, parameters: Vec<ProgramFeatureParameter>| {
                ProgramFeature {
                    name,
                    kind,
                    parameters,
                }
            };
        let mut features = vec![feature(
            format!("{} transform", self.name),
            ProgramFeatureKind::Transform,
            ["translation.x", "translation.y", "translation.z"]
                .into_iter()
                .zip(self.at_mm)
                .map(|(path, value)| length(path, value))
                .collect(),
        )];
        let profile_parameters = |segments: &[ProgramProfileSegment]| {
            let mut parameters = Vec::new();
            for (index, segment) in segments.iter().enumerate() {
                for (point, coordinates) in [("start", segment.start_mm), ("end", segment.end_mm)] {
                    parameters.push(length(
                        &format!("entities.{}.{point}.x", index + 1),
                        coordinates[0],
                    ));
                    parameters.push(length(
                        &format!("entities.{}.{point}.y", index + 1),
                        coordinates[1],
                    ));
                }
            }
            parameters
        };
        match &self.body {
            ProgramPartBody::Panel => {
                features.push(feature(
                    format!("{} sketch", self.name),
                    ProgramFeatureKind::Sketch,
                    vec![
                        length("bounds.width", self.size_mm[0]),
                        length("bounds.height", self.size_mm[1]),
                    ],
                ));
                features.push(feature(
                    format!("{} feature", self.name),
                    ProgramFeatureKind::Pad,
                    vec![length("extent.distance", self.size_mm[2])],
                ));
            }
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
            } => {
                features.push(feature(
                    format!("{} sketch", self.name),
                    ProgramFeatureKind::Sketch,
                    profile_parameters(segments),
                ));
                features.push(feature(
                    format!("{} feature", self.name),
                    ProgramFeatureKind::Pad,
                    vec![length("extent.distance", *distance_mm)],
                ));
            }
            ProgramPartBody::Revolve {
                segments,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => {
                features.push(feature(
                    format!("{} sketch", self.name),
                    ProgramFeatureKind::Sketch,
                    profile_parameters(segments),
                ));
                let mut parameters = vec![
                    length("axis_start.x", axis_start_mm[0]),
                    length("axis_start.y", axis_start_mm[1]),
                    length("axis_end.x", axis_end_mm[0]),
                    length("axis_end.y", axis_end_mm[1]),
                ];
                parameters.push(ProgramFeatureParameter {
                    path: "angle".to_owned(),
                    value_type: ProgramParameterValueType::Angle,
                    value: *angle_degrees,
                });
                features.push(feature(
                    format!("{} feature", self.name),
                    ProgramFeatureKind::Revolve,
                    parameters,
                ));
            }
        }
        for cut in &self.cuts {
            features.push(feature(
                format!("{} sketch", cut.name),
                ProgramFeatureKind::Sketch,
                profile_parameters(&cut.segments)
                    .into_iter()
                    .map(|mut parameter| {
                        parameter.path = parameter.path.replacen("entities.", "segments.", 1);
                        let index = parameter.path["segments.".len()..]
                            .split('.')
                            .next()
                            .and_then(|index| index.parse::<usize>().ok())
                            .expect("generated segment path");
                        parameter.path = parameter.path.replacen(
                            &format!("segments.{index}."),
                            &format!("segments.{}.", index - 1),
                            1,
                        );
                        parameter
                    })
                    .collect(),
            ));
            features.push(feature(
                cut.name.clone(),
                ProgramFeatureKind::Cut,
                vec![length("depth", cut.depth_mm)],
            ));
        }
        for fillet in &self.fillets {
            features.push(feature(
                fillet.name.clone(),
                match fillet.kind {
                    ProgramEdgeFinishKind::Fillet => ProgramFeatureKind::Fillet,
                    ProgramEdgeFinishKind::Chamfer => ProgramFeatureKind::Chamfer,
                },
                vec![length("radius", fillet.radius_mm)],
            ));
        }
        for offset in &self.face_offsets {
            features.push(feature(
                offset.name.clone(),
                ProgramFeatureKind::FaceOffset,
                vec![length("distance", offset.distance_mm)],
            ));
        }
        for hole in &self.holes {
            let prefix = format!("{} hole {}", self.name, hole.id);
            let origin = hole.face.local_point(self.size_mm, hole.u_mm, hole.v_mm);
            features.push(feature(
                format!("{prefix} workplane"),
                ProgramFeatureKind::Workplane,
                ["frame.origin.x", "frame.origin.y", "frame.origin.z"]
                    .into_iter()
                    .zip(origin)
                    .map(|(path, value)| length(path, value))
                    .collect(),
            ));
            features.push(feature(
                prefix.clone(),
                ProgramFeatureKind::Sketch,
                vec![length("entities.1.radius", hole.diameter_mm * 0.5)],
            ));
            features.push(feature(
                format!("{prefix} pocket"),
                ProgramFeatureKind::Cut,
                vec![length("depth", hole.depth_mm)],
            ));
        }
        for pocket in &self.pockets {
            let prefix = format!("{} pocket {}", self.name, pocket.id);
            let origin = pocket.face.local_point(
                self.size_mm,
                (pocket.u_min_mm + pocket.u_max_mm) * 0.5,
                (pocket.v_min_mm + pocket.v_max_mm) * 0.5,
            );
            features.push(feature(
                format!("{prefix} workplane"),
                ProgramFeatureKind::Workplane,
                ["frame.origin.x", "frame.origin.y", "frame.origin.z"]
                    .into_iter()
                    .zip(origin)
                    .map(|(path, value)| length(path, value))
                    .collect(),
            ));
            let u = (pocket.u_max_mm - pocket.u_min_mm) * 0.5;
            let v = (pocket.v_max_mm - pocket.v_min_mm) * 0.5;
            let (x, y) = if pocket.face.axis() == 1 {
                (v, u)
            } else {
                (u, v)
            };
            let corners = [[-x, -y], [x, -y], [x, y], [-x, y]];
            let mut parameters = Vec::new();
            for index in 0..4 {
                for (point, corner) in
                    [("start", corners[index]), ("end", corners[(index + 1) % 4])]
                {
                    parameters.push(length(
                        &format!("entities.{}.{point}.x", index + 1),
                        corner[0],
                    ));
                    parameters.push(length(
                        &format!("entities.{}.{point}.y", index + 1),
                        corner[1],
                    ));
                }
            }
            features.push(feature(
                prefix.clone(),
                ProgramFeatureKind::Sketch,
                parameters,
            ));
            features.push(feature(
                format!("{prefix} cut"),
                ProgramFeatureKind::Cut,
                vec![length("depth", pocket.depth_mm)],
            ));
        }
        self.features = features;
    }
}

/// A declared connection between two parts. Overlap inside `volume` is
/// expected; a joint whose parts do not touch is an error.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Joint {
    pub name: String,
    pub kind: String,
    pub parts: [String; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub volume_mm: Option<([f64; 3], [f64; 3])>,
    /// World positions of fasteners (dowel centres, screws, ...).
    pub fasteners_mm: Vec<[f64; 3]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastener: Option<String>,
    /// Largest gap between the parts that still counts as connected (shelf
    /// pins, clearance fits). Zero means the parts must touch.
    pub max_gap_mm: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProgramModel {
    pub params: Vec<Param>,
    pub parts: Vec<Part>,
    /// Helper bodies used only as boolean tools: never built, listed or validated.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Part>,
    pub joints: Vec<Joint>,
}

impl ProgramModel {
    #[must_use]
    pub fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.name == name)
    }

    #[must_use]
    pub fn tool(&self, name: &str) -> Option<&Part> {
        self.tools.iter().find(|part| part.name == name)
    }
}
