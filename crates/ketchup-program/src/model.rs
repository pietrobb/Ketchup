//! Plain data produced by evaluating a rule program.
//!
//! Every part has a rigid frame: its local origin `at_mm` in world millimetres
//! and a rotation (identity for axis-aligned parts). A panel occupies
//! `0..size_mm` in its own frame. Holes and pockets are operations machined
//! into a named face (see `crate::faces`), kept where the part is when the
//! program writes them.

use crate::frame::{self, Mat3, Obb};
use ketchup_geometry::linalg::{CubicBezier, dot};
use ketchup_model::tolerance::ROUNDING;
use serde::Serialize;
use std::collections::BTreeMap;

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
    Sweep,
    Loft,
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

/// A cylindrical hole drilled into a face along its inward normal at face
/// coordinates `at_mm` (u, v on a plane; angle in degrees, v on a cylinder).
/// `entry_mm` and `inward` are in the part's frame as the operations written
/// before the hole left it; the operations after it carry it on.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hole {
    pub id: String,
    pub face: String,
    pub at_mm: [f64; 2],
    pub entry_mm: [f64; 3],
    pub inward: [f64; 3],
    pub diameter_mm: f64,
    pub depth_mm: f64,
}

/// A rectangular pocket milled into a flat face (grooves, rabbets, notches):
/// `rect_mm` = (u_min, v_min, u_max, v_max) in face coordinates, which may run
/// off the face edges. `corner_mm` (the rectangle's (u_min, v_min) corner),
/// `u`, `v` and `inward` are in the part's frame as the operations written
/// before the pocket left it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Pocket {
    pub id: String,
    pub face: String,
    pub rect_mm: [f64; 4],
    pub corner_mm: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    pub inward: [f64; 3],
    pub depth_mm: f64,
}

impl Pocket {
    /// Width and height of the rectangle along `u` and `v`.
    #[must_use]
    pub fn extent_mm(&self) -> [f64; 2] {
        [
            self.rect_mm[2] - self.rect_mm[0],
            self.rect_mm[3] - self.rect_mm[1],
        ]
    }

    /// The eight corners of the removed block, in the body's frame.
    #[must_use]
    pub fn corners(&self) -> [[f64; 3]; 8] {
        let [du, dv] = self.extent_mm();
        std::array::from_fn(|index| {
            let along = |axis: [f64; 3], bit: usize, length: f64| {
                if index >> bit & 1 == 1 {
                    axis.map(|value| value * length)
                } else {
                    [0.0; 3]
                }
            };
            let (a, b, c) = (
                along(self.u, 0, du),
                along(self.v, 1, dv),
                along(self.inward, 2, self.depth_mm),
            );
            std::array::from_fn(|i| self.corner_mm[i] + a[i] + b[i] + c[i])
        })
    }

    /// Bounds of the removed block in the body's frame as (min, max); the
    /// block itself on a box's faces, which lie along its axes.
    #[must_use]
    pub fn local_box(&self) -> ([f64; 3], [f64; 3]) {
        self.corners().iter().fold(
            ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
            |(min, max), point| {
                (
                    std::array::from_fn(|i| min[i].min(point[i])),
                    std::array::from_fn(|i| max[i].max(point[i])),
                )
            },
        )
    }
}

/// One named edge of a closed profile: a straight line, a circular arc
/// around `arc.center_mm` from `start_mm` to `end_mm`, or a cubic Bezier
/// curve with the two inner control points `bezier`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramProfileSegment {
    pub name: String,
    pub start_mm: [f64; 2],
    pub end_mm: [f64; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arc: Option<ProgramArc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[[f64; 2]; 2]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProgramArc {
    pub center_mm: [f64; 2],
    pub clockwise: bool,
}

/// The exact kernel's label of profile segment `index`. A full circle drawn
/// as two half arcs becomes one cylindrical face, `face`, so both halves
/// name it.
fn segment_label(segments: &[ProgramProfileSegment], index: usize) -> String {
    if let [first, second] = segments
        && let (Some(a), Some(b)) = (first.arc, second.arc)
        && a == b
        && first.start_mm == second.end_mm
        && first.end_mm == second.start_mm
    {
        let start = [
            first.start_mm[0] - a.center_mm[0],
            first.start_mm[1] - a.center_mm[1],
        ];
        let end = [
            first.end_mm[0] - a.center_mm[0],
            first.end_mm[1] - a.center_mm[1],
        ];
        let radius = start[0].hypot(start[1]);
        if (start[0] + end[0]).hypot(start[1] + end[1]) <= ROUNDING * radius.max(1.0) {
            return "face".to_owned();
        }
    }
    format!("segment_{}", index + 1)
}

impl ProgramProfileSegment {
    #[must_use]
    pub fn line(name: impl Into<String>, start_mm: [f64; 2], end_mm: [f64; 2]) -> Self {
        Self {
            name: name.into(),
            start_mm,
            end_mm,
            arc: None,
            bezier: None,
        }
    }

    #[must_use]
    pub fn is_line(&self) -> bool {
        self.arc.is_none() && self.bezier.is_none()
    }

    /// The point at `t` in 0..=1 of a Bezier segment (or of the chord).
    #[must_use]
    pub fn bezier_point(&self, t: f64) -> [f64; 2] {
        let [c1, c2] = self.bezier.unwrap_or([self.start_mm, self.end_mm]);
        CubicBezier::new([self.start_mm, c1, c2, self.end_mm]).eval(t)
    }

    /// The largest `direction · p` over the segment's points.
    #[must_use]
    pub fn support(&self, direction: [f64; 2]) -> f64 {
        let dot = |p: [f64; 2]| ketchup_geometry::linalg::dot2(p, direction);
        let ends = dot(self.start_mm).max(dot(self.end_mm));
        if let Some([c1, c2]) = self.bezier {
            // d/dt of the cubic's projection is a t^2 + b t + c; its roots
            // in (0, 1) are the only other candidates.
            let (p0, p1, p2, p3) = (dot(self.start_mm), dot(c1), dot(c2), dot(self.end_mm));
            let a = -p0 + 3.0 * p1 - 3.0 * p2 + p3;
            let b = 2.0 * (p0 - 2.0 * p1 + p2);
            let c = p1 - p0;
            let mut roots = Vec::new();
            if a.abs() <= ROUNDING {
                if b.abs() > ROUNDING {
                    roots.push(-c / b);
                }
            } else {
                let discriminant = b * b - 4.0 * a * c;
                if discriminant >= 0.0 {
                    let root = discriminant.sqrt();
                    roots.extend([(-b + root) / (2.0 * a), (-b - root) / (2.0 * a)]);
                }
            }
            return roots
                .into_iter()
                .filter(|t| (0.0..=1.0).contains(t))
                .map(|t| dot(self.bezier_point(t)))
                .fold(ends, f64::max);
        }
        let Some(arc) = self.arc else {
            return ends;
        };
        let length = direction[0].hypot(direction[1]);
        if length <= f64::EPSILON {
            return ends;
        }
        let c = arc.center_mm;
        let radius = (self.start_mm[0] - c[0]).hypot(self.start_mm[1] - c[1]);
        let angle = |p: [f64; 2]| (p[1] - c[1]).atan2(p[0] - c[0]);
        let turn = |from: f64, to: f64| (to - from).rem_euclid(std::f64::consts::TAU);
        let (start, end, target) = (
            angle(self.start_mm),
            angle(self.end_mm),
            direction[1].atan2(direction[0]),
        );
        // Counter-clockwise sweep from `from` to `to` covering the arc.
        let (from, to) = if arc.clockwise {
            (end, start)
        } else {
            (start, end)
        };
        let sweep = match turn(from, to) {
            0.0 => std::f64::consts::TAU,
            sweep => sweep,
        };
        if turn(from, target) <= sweep {
            ends.max(dot(c) + radius * length)
        } else {
            ends
        }
    }
}

/// `(min, max)` of a closed profile in its plane.
#[must_use]
pub fn profile_bounds(segments: &[ProgramProfileSegment]) -> ([f64; 2], [f64; 2]) {
    let reach = |direction: [f64; 2]| {
        segments
            .iter()
            .map(|segment| segment.support(direction))
            .fold(f64::NEG_INFINITY, f64::max)
    };
    (
        [-reach([-1.0, 0.0]), -reach([0.0, -1.0])],
        [reach([1.0, 0.0]), reach([0.0, 1.0])],
    )
}

/// Ends of the straight side `index` of a closed profile after it moves by
/// `shift` along its right normal (outward for a counter-clockwise loop),
/// where its straight neighbours, extended, meet the moved line. `None` next
/// to an arc or a parallel neighbour.
fn moved_side(
    segments: &[ProgramProfileSegment],
    index: usize,
    shift: f64,
) -> Option<[[f64; 2]; 2]> {
    let cross = |a: [f64; 2], b: [f64; 2]| a[0] * b[1] - a[1] * b[0];
    let minus = |a: [f64; 2], b: [f64; 2]| [a[0] - b[0], a[1] - b[1]];
    let count = segments.len();
    let side = &segments[index];
    let direction = minus(side.end_mm, side.start_mm);
    let length = direction[0].hypot(direction[1]);
    if !side.is_line() || length <= f64::EPSILON {
        return None;
    }
    let normal = [direction[1] / length, -direction[0] / length];
    let moved_start = [
        side.start_mm[0] + normal[0] * shift,
        side.start_mm[1] + normal[1] * shift,
    ];
    // The neighbour's line through `vertex` meets the moved line where
    // `vertex + along * t` lies on it.
    let meet = |neighbour: &ProgramProfileSegment, vertex: [f64; 2]| {
        let along = minus(neighbour.end_mm, neighbour.start_mm);
        let denominator = cross(along, direction);
        if !neighbour.is_line() || denominator.abs() <= ROUNDING * length * along[0].hypot(along[1])
        {
            return None;
        }
        let t = cross(minus(moved_start, vertex), direction) / denominator;
        Some([vertex[0] + along[0] * t, vertex[1] + along[1] * t])
    };
    Some([
        meet(&segments[(index + count - 1) % count], side.start_mm)?,
        meet(&segments[(index + 1) % count], side.end_mm)?,
    ])
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
    /// Adds the tool's volume to the part, making one solid.
    Union,
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

/// The solid reflected in its own frame across the plane where coordinate
/// `axis` equals `center_mm`. Every face keeps the name of the face it mirrors.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramMirror {
    pub name: String,
    pub axis: usize,
    pub center_mm: f64,
}

impl ProgramMirror {
    /// Row-major 4 x 4 matrix of the reflection in the part's frame.
    #[must_use]
    pub fn matrix(&self) -> [f64; 16] {
        let mut matrix = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
        ];
        matrix[self.axis * 5] = -1.0;
        matrix[self.axis * 4 + 3] = 2.0 * self.center_mm;
        matrix
    }
}

/// The solid hollowed to walls `thickness_mm` thick, measured inwards; the
/// `open` faces are removed so the hollow opens there (none: a closed void).
/// The inner walls are "<name>.<face they follow>".
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramShell {
    pub name: String,
    pub thickness_mm: f64,
    pub open: Vec<String>,
}

/// One shaping step on a part's solid. A part applies its operations in the
/// order the program wrote them, each to the result of the one before.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ProgramOperation {
    Cut(ProgramCut),
    Finish(ProgramEdgeFillet),
    FaceOffset(ProgramFaceOffset),
    Boolean(Box<ProgramBoolean>),
    Mirror(ProgramMirror),
    Shell(ProgramShell),
    /// Drilled where the part is when the program writes it.
    Hole(Hole),
    /// Milled where the part is when the program writes it.
    Pocket(Pocket),
}

impl ProgramOperation {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Hole(hole) => &hole.id,
            Self::Pocket(pocket) => &pocket.id,
            Self::Cut(cut) => &cut.name,
            Self::Finish(finish) => &finish.name,
            Self::FaceOffset(offset) => &offset.name,
            Self::Boolean(boolean) => &boolean.name,
            Self::Mirror(mirror) => &mirror.name,
            Self::Shell(shell) => &shell.name,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProgramPartBody {
    /// The closed profile padded along local +z by `distance_mm`; `caps`
    /// name the faces at z = 0 and z = distance.
    Extrusion {
        segments: Vec<ProgramProfileSegment>,
        distance_mm: f64,
        caps: [String; 2],
    },
    Revolve {
        segments: Vec<ProgramProfileSegment>,
        axis_start_mm: [f64; 2],
        axis_end_mm: [f64; 2],
        angle_degrees: f64,
    },
    /// The closed profile carried along a tangent-continuous path; see
    /// `crate::path` for the section frame.
    Sweep {
        segments: Vec<ProgramProfileSegment>,
        path: Vec<ProgramPathSegment>,
        /// A fixed unit direction the profile's v keeps; see `crate::path`.
        up: Option<[f64; 3]>,
    },
    /// A solid through closed profiles lying in local XY planes at strictly
    /// increasing heights.
    Loft { sections: Vec<ProgramLoftSection> },
}

impl ProgramPartBody {
    /// A profile extruded from its "start" cap to its "end" cap.
    #[must_use]
    pub fn extrusion(segments: Vec<ProgramProfileSegment>, distance_mm: f64) -> Self {
        Self::Extrusion {
            segments,
            distance_mm,
            caps: ["start".to_owned(), "end".to_owned()],
        }
    }

    /// The cuboid `0..size_mm`: the rectangle y-, x+, y+, x- extruded from
    /// cap z- to cap z+, so every face is named by its outward axis.
    #[must_use]
    pub fn cuboid(size_mm: [f64; 3]) -> Self {
        let [x, y, z] = size_mm;
        Self::Extrusion {
            segments: vec![
                ProgramProfileSegment::line("y-", [0.0, 0.0], [x, 0.0]),
                ProgramProfileSegment::line("x+", [x, 0.0], [x, y]),
                ProgramProfileSegment::line("y+", [x, y], [0.0, y]),
                ProgramProfileSegment::line("x-", [0.0, y], [0.0, 0.0]),
            ],
            distance_mm: z,
            caps: ["z-".to_owned(), "z+".to_owned()],
        }
    }

    /// The size of the cuboid this body is: the rectangle of `cuboid(size)`,
    /// corner for corner, extruded by its distance (under any face names).
    #[must_use]
    pub fn cuboid_size(&self) -> Option<[f64; 3]> {
        let Self::Extrusion {
            segments,
            distance_mm,
            ..
        } = self
        else {
            return None;
        };
        let size = [
            segments.first()?.end_mm[0],
            segments.get(2)?.start_mm[1],
            *distance_mm,
        ];
        let Self::Extrusion { segments: own, .. } = Self::cuboid(size) else {
            unreachable!("a cuboid is an extrusion")
        };
        (segments.len() == own.len()
            && segments.iter().zip(&own).all(|(segment, corner)| {
                segment.is_line()
                    && segment.start_mm == corner.start_mm
                    && segment.end_mm == corner.end_mm
            }))
        .then_some(size)
    }
}

/// A circular path arc, counter-clockwise about `normal` seen from its tip.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ProgramPathArc {
    pub center_mm: [f64; 3],
    pub normal: [f64; 3],
}

/// One piece of a sweep path in the part's frame: a line, an arc, or a
/// cubic Bezier curve with the two inner control points `bezier`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramPathSegment {
    pub start_mm: [f64; 3],
    pub end_mm: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arc: Option<ProgramPathArc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bezier: Option<[[f64; 3]; 2]>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramLoftSection {
    pub segments: Vec<ProgramProfileSegment>,
    pub elevation_mm: f64,
}

/// The finish a `fillet(a,b)` or `chamfer(a,b)` face name comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FinishKind {
    Fillet,
    Chamfer,
}

impl FinishKind {
    /// The name prefix up to the opening parenthesis.
    #[must_use]
    pub const fn prefix(self) -> &'static str {
        match self {
            Self::Fillet => "fillet(",
            Self::Chamfer => "chamfer(",
        }
    }
}

/// Why a program face name names no face of a part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FaceNameError {
    /// `<face>#<n>` without a number after `#`.
    SplitSuffix { name: String },
    /// A finish face name without its closing parenthesis.
    FinishForm { name: String, kind: FinishKind },
    UnknownCutFace {
        name: String,
        cut: String,
        faces: Vec<String>,
    },
    /// The tool face of a boolean does not exist.
    ToolFace {
        name: String,
        operation: String,
        cause: Box<FaceNameError>,
    },
    /// The face a shell's inner wall follows does not exist.
    InnerWall {
        name: String,
        cause: Box<FaceNameError>,
    },
    /// `<operation>.<face>` names an operation that leaves no faces of its own.
    NotAFaceOperation {
        name: String,
        operation: String,
        part: String,
    },
    /// The part is swept or lofted: its own faces have no names.
    Unnamed { part: String },
    Unknown {
        name: String,
        part: String,
        faces: Vec<String>,
    },
    /// The part has no flat or cylindrical named face to take a frame of.
    NoFramedFaces { part: String },
    /// The face exists but is neither flat nor cylindrical.
    NotFramed {
        name: String,
        part: String,
        faces: Vec<String>,
    },
}

impl std::fmt::Display for FaceNameError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SplitSuffix { name } => write!(
                f,
                "face {name:?}: a split face is <face>#<number>, e.g. \"z+#2\""
            ),
            Self::FinishForm { name, kind } => {
                write!(f, "face {name:?}: a finish face is {}a,b)", kind.prefix())
            }
            Self::UnknownCutFace { name, cut, faces } => write!(
                f,
                "face {name:?}: cut {cut:?} has faces start, end, {}",
                faces.join(", ")
            ),
            Self::ToolFace {
                name,
                operation,
                cause,
            } => write!(f, "face {name:?}: tool of {operation:?}: {cause}"),
            Self::InnerWall { name, cause } => write!(f, "inner wall {name:?}: {cause}"),
            Self::NotAFaceOperation {
                name,
                operation,
                part,
            } => write!(
                f,
                "face {name:?}: {operation:?} is not a cut, boolean or shell on {part:?}; faces an operation leaves are <operation name>.<tool face>"
            ),
            Self::Unnamed { part } => write!(
                f,
                "{part:?} is swept or lofted and its faces have no names; faces left by subtract()/intersect() are <operation name>.<tool face>"
            ),
            Self::Unknown { name, part, faces } => write!(
                f,
                "face {name:?} does not exist on {part:?}; faces are: {}",
                faces.join(", ")
            ),
            Self::NoFramedFaces { part } => write!(
                f,
                "{part:?} has no flat or cylindrical faces with names (swept and lofted bodies have none)"
            ),
            Self::NotFramed { name, part, faces } => write!(
                f,
                "face {name:?} is not a flat or cylindrical face on {part:?}; its faces are: {}",
                faces.join(", ")
            ),
        }
    }
}

impl std::error::Error for FaceNameError {}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Part {
    /// Stable identity: the path given in the program, e.g. `korpus/bok_lavy`.
    pub name: String,
    pub size_mm: [f64; 3],
    pub grounded: bool,
    /// World position of the local origin.
    pub at_mm: [f64; 3],
    /// Row-major rotation of the local frame; columns are the local axes in world.
    #[serde(skip_serializing_if = "frame::is_identity")]
    pub rotation: Mat3,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    /// Free named attributes the program attaches for its own use, e.g. a
    /// library's direction of a material's texture.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
    pub body: ProgramPartBody,
    /// Cuts, finishes, moved faces, booleans, holes and pockets in program order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ProgramOperation>,
    /// Stable, operation-class-level source tree used by incremental reconciliation.
    pub features: Vec<ProgramFeature>,
}

impl Part {
    pub fn holes(&self) -> impl Iterator<Item = &Hole> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Hole(hole) => Some(hole),
                _ => None,
            })
    }

    pub fn pockets(&self) -> impl Iterator<Item = &Pocket> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Pocket(pocket) => Some(pocket),
                _ => None,
            })
    }

    /// How many operations a cuboid's document panel drills itself: the holes
    /// and pockets written before any other operation (`crate::cad`). The
    /// rest, and all of a profile body's, are subtracted in program order.
    #[must_use]
    pub fn panel_machining(&self) -> usize {
        if self.body.cuboid_size().is_none() {
            return 0;
        }
        self.operations
            .iter()
            .take_while(|operation| {
                matches!(
                    operation,
                    ProgramOperation::Hole(_) | ProgramOperation::Pocket(_)
                )
            })
            .count()
    }

    pub fn cuts(&self) -> impl Iterator<Item = &ProgramCut> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Cut(cut) => Some(cut),
                _ => None,
            })
    }

    pub fn finishes(&self) -> impl Iterator<Item = &ProgramEdgeFillet> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Finish(finish) => Some(finish),
                _ => None,
            })
    }

    pub fn face_offsets(&self) -> impl Iterator<Item = &ProgramFaceOffset> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::FaceOffset(offset) => Some(offset),
                _ => None,
            })
    }

    pub fn booleans(&self) -> impl Iterator<Item = &ProgramBoolean> {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Boolean(boolean) => Some(&**boolean),
                _ => None,
            })
    }

    pub fn booleans_mut(&mut self) -> impl Iterator<Item = &mut ProgramBoolean> {
        self.operations
            .iter_mut()
            .filter_map(|operation| match operation {
                ProgramOperation::Boolean(boolean) => Some(&mut **boolean),
                _ => None,
            })
    }

    /// Names of the faces of the body before any operation: the caps and the
    /// segment names of an extruded profile ("z-", "z+", "y-", "x+", "y+",
    /// "x-" on a box); "start", "end" and the segment names of a revolved
    /// one. Swept and lofted bodies have none.
    #[must_use]
    pub fn body_face_names(&self) -> Vec<String> {
        match &self.body {
            ProgramPartBody::Extrusion { segments, caps, .. } => caps
                .iter()
                .cloned()
                .chain(segments.iter().map(|segment| segment.name.clone()))
                .collect(),
            ProgramPartBody::Revolve { segments, .. } => ["start", "end"]
                .into_iter()
                .map(str::to_owned)
                .chain(segments.iter().map(|segment| segment.name.clone()))
                .collect(),
            ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => Vec::new(),
        }
    }

    /// The exact kernel's label of a program face name. Body faces are
    /// `start`, `end` and `segment_<n>` (a box is the rectangle y-, x+, y+, x-
    /// padded from z- to z+); a face an operation left is `<operation>.<face>`
    /// with the tool's own label; a finish face is `fillet(a,b)` or
    /// `chamfer(a,b)`; a face an operation split keeps its `#<n>` suffix.
    ///
    /// # Errors
    /// Names the unknown face and lists the names that exist.
    pub fn exact_face_label(&self, name: &str) -> Result<String, FaceNameError> {
        if let Some((base, suffix)) = name.rsplit_once('#') {
            if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
                return Err(FaceNameError::SplitSuffix {
                    name: name.to_owned(),
                });
            }
            return Ok(format!("{}#{suffix}", self.exact_face_label(base)?));
        }
        if let Some((kind, rest)) = [FinishKind::Fillet, FinishKind::Chamfer]
            .into_iter()
            .find_map(|kind| name.strip_prefix(kind.prefix()).map(|rest| (kind, rest)))
        {
            let faces = rest
                .strip_suffix(')')
                .ok_or_else(|| FaceNameError::FinishForm {
                    name: name.to_owned(),
                    kind,
                })?;
            let mut labels = faces
                .split(',')
                .map(|face| self.exact_face_label(face.trim()))
                .collect::<Result<Vec<_>, _>>()?;
            labels.sort();
            return Ok(format!("{}{})", kind.prefix(), labels.join(",")));
        }
        if let Some((operation, face)) = name.split_once('.') {
            return match self.operations.iter().find(|op| op.name() == operation) {
                Some(ProgramOperation::Cut(cut)) => match face {
                    "start" | "end" => Ok(name.to_owned()),
                    _ => cut
                        .segments
                        .iter()
                        .position(|segment| segment.name == face)
                        .map(|index| format!("{operation}.{}", segment_label(&cut.segments, index)))
                        .ok_or_else(|| FaceNameError::UnknownCutFace {
                            name: name.to_owned(),
                            cut: operation.to_owned(),
                            faces: cut
                                .segments
                                .iter()
                                .map(|segment| segment.name.clone())
                                .collect(),
                        }),
                },
                Some(ProgramOperation::Boolean(boolean)) => Ok(format!(
                    "{operation}.{}",
                    boolean.tool.body_face_label(face).map_err(|cause| {
                        FaceNameError::ToolFace {
                            name: name.to_owned(),
                            operation: operation.to_owned(),
                            cause: Box::new(cause),
                        }
                    })?
                )),
                Some(ProgramOperation::Shell(_)) => Ok(format!(
                    "{operation}.{}",
                    self.exact_face_label(face)
                        .map_err(|cause| FaceNameError::InnerWall {
                            name: name.to_owned(),
                            cause: Box::new(cause),
                        })?
                )),
                _ => Err(FaceNameError::NotAFaceOperation {
                    name: name.to_owned(),
                    operation: operation.to_owned(),
                    part: self.name.clone(),
                }),
            };
        }
        self.body_face_label(name)
    }

    fn body_face_label(&self, name: &str) -> Result<String, FaceNameError> {
        let unknown = || {
            let faces = self.body_face_names();
            if faces.is_empty() {
                FaceNameError::Unnamed {
                    part: self.name.clone(),
                }
            } else {
                FaceNameError::Unknown {
                    name: name.to_owned(),
                    part: self.name.clone(),
                    faces,
                }
            }
        };
        match &self.body {
            ProgramPartBody::Extrusion { segments, caps, .. } => {
                if let Some(index) = caps.iter().position(|cap| cap == name) {
                    return Ok(["start", "end"][index].to_owned());
                }
                segments
                    .iter()
                    .position(|segment| segment.name == name)
                    .map(|index| segment_label(segments, index))
                    .ok_or_else(unknown)
            }
            ProgramPartBody::Revolve { segments, .. } => {
                if name == "start" || name == "end" {
                    return Ok(name.to_owned());
                }
                segments
                    .iter()
                    .position(|segment| segment.name == name)
                    .map(|index| segment_label(segments, index))
                    .ok_or_else(unknown)
            }
            ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => Err(unknown()),
        }
    }

    /// Whether any operation other than a boolean, hole or pocket shapes the body.
    #[must_use]
    pub fn has_shaping(&self) -> bool {
        self.operations.iter().any(|operation| {
            !matches!(
                operation,
                ProgramOperation::Boolean(_)
                    | ProgramOperation::Hole(_)
                    | ProgramOperation::Pocket(_)
            )
        })
    }

    /// Bounds of the body in its own frame, before cuts and finishes, grown
    /// by every face a push_pull moves outward and every body joined in.
    #[must_use]
    pub fn local_bounds(&self) -> ([f64; 3], [f64; 3]) {
        let (mut min, mut max) = self.body_bounds();
        for tool in self.joined() {
            let (tool_min, tool_max) = tool.local_bounds();
            for corner in 0..8 {
                let local: [f64; 3] = std::array::from_fn(|axis| {
                    if corner >> axis & 1 == 0 {
                        tool_min[axis]
                    } else {
                        tool_max[axis]
                    }
                });
                let point = self.to_local(tool.to_world(local));
                for axis in 0..3 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
        }
        (min, max)
    }

    /// Bodies union() joined into this part.
    fn joined(&self) -> impl Iterator<Item = &Part> {
        self.booleans()
            .filter(|boolean| boolean.kind == ProgramBooleanKind::Union)
            .map(|boolean| &boolean.tool)
    }

    fn body_bounds(&self) -> ([f64; 3], [f64; 3]) {
        match &self.body {
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
                caps,
            } => {
                let (min, max) = profile_bounds(segments);
                self.grown_by_pushed_faces(
                    [min[0], min[1], 0.0],
                    [max[0], max[1], *distance_mm],
                    segments,
                    Some(caps),
                )
            }
            ProgramPartBody::Revolve { .. } => {
                let support = |axis: usize, sign: f64| {
                    let direction = std::array::from_fn(|i| if i == axis { sign } else { 0.0 });
                    sign * self.revolve_support(direction)
                };
                self.grown_by_pushed_faces(
                    std::array::from_fn(|i| support(i, -1.0)),
                    std::array::from_fn(|i| support(i, 1.0)),
                    &[],
                    None,
                )
            }
            ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => {
                let axis = |index: usize, sign: f64| {
                    let mut direction = [0.0; 3];
                    direction[index] = sign;
                    sign * self.local_reach(direction)
                };
                (
                    std::array::from_fn(|index| axis(index, -1.0)),
                    std::array::from_fn(|index| axis(index, 1.0)),
                )
            }
        }
    }

    /// Exact support of the swept profile, including offset axes and partial turns.
    /// For each signed radial coordinate only the extrema of A cos(t) + B sin(t)
    /// matter. The remaining maximization is the profile's existing support query.
    fn revolve_support(&self, direction: [f64; 3]) -> f64 {
        let ProgramPartBody::Revolve {
            segments,
            axis_start_mm: origin,
            axis_end_mm: end,
            angle_degrees,
        } = &self.body
        else {
            return f64::NEG_INFINITY;
        };
        let delta = [end[0] - origin[0], end[1] - origin[1]];
        let length = delta[0].hypot(delta[1]);
        let axis = delta.map(|v| v / length);
        let radial = [-axis[1], axis[0]];
        let axial = dot(direction, [axis[0], axis[1], 0.0]);
        let a = dot(direction, [radial[0], radial[1], 0.0]);
        let b = direction[2]; // axis × radial = +Z
        let sweep = angle_degrees.to_radians().min(std::f64::consts::TAU);
        let mut values = vec![a, a * sweep.cos() + b * sweep.sin()];
        let extremum = b.atan2(a).rem_euclid(std::f64::consts::TAU);
        for angle in [
            extremum,
            (extremum + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU),
        ] {
            if angle <= sweep {
                values.push(a * angle.cos() + b * angle.sin());
            }
        }
        values
            .into_iter()
            .map(|radial_support| {
                let d = std::array::from_fn(|i| axial * axis[i] + radial_support * radial[i]);
                let translation = dot(
                    [direction[0] - d[0], direction[1] - d[1], 0.0],
                    [origin[0], origin[1], 0.0],
                );
                segments
                    .iter()
                    .map(|segment| segment.support(d) + translation)
                    .fold(f64::NEG_INFINITY, f64::max)
            })
            .fold(f64::NEG_INFINITY, f64::max)
    }

    fn pushes_faces_out(&self) -> bool {
        self.face_offsets().any(|offset| offset.distance_mm > 0.0)
    }

    /// `min`/`max` grown by each face moved outward. Extrusion caps move
    /// along z; a straight profile side moves to its offset line, which its
    /// straight neighbours extend to meet. Any other face (a revolved or cut
    /// face, a side next to an arc) grows the bounds by its distance on every
    /// side.
    fn grown_by_pushed_faces(
        &self,
        mut min: [f64; 3],
        mut max: [f64; 3],
        segments: &[ProgramProfileSegment],
        caps: Option<&[String; 2]>,
    ) -> ([f64; 3], [f64; 3]) {
        let doubled_area: f64 = segments
            .iter()
            .map(|s| s.start_mm[0] * s.end_mm[1] - s.end_mm[0] * s.start_mm[1])
            .sum();
        let turn = if doubled_area < 0.0 { -1.0 } else { 1.0 };
        for offset in self.face_offsets().filter(|o| o.distance_mm > 0.0) {
            let d = offset.distance_mm;
            let face = offset.face.split('#').next().unwrap_or_default();
            match caps.and_then(|caps| caps.iter().position(|cap| cap == face)) {
                Some(0) => {
                    min[2] -= d;
                    continue;
                }
                Some(_) => {
                    max[2] += d;
                    continue;
                }
                None => {}
            }
            let moved = segments
                .iter()
                .position(|s| s.name == face)
                .and_then(|index| moved_side(segments, index, d * turn));
            let Some(points) = moved else {
                for axis in 0..3 {
                    min[axis] -= d;
                    max[axis] += d;
                }
                continue;
            };
            for point in points {
                for axis in 0..2 {
                    min[axis] = min[axis].min(point[axis]);
                    max[axis] = max[axis].max(point[axis]);
                }
            }
        }
        (min, max)
    }

    /// `reach` of a swept or lofted body along a direction in its own frame.
    fn local_reach(&self, local: [f64; 3]) -> f64 {
        match &self.body {
            ProgramPartBody::Sweep { segments, path, up } => {
                crate::path::sweep_support(segments, path, *up, local)
            }
            ProgramPartBody::Loft { sections } => sections
                .iter()
                .map(|section| {
                    section
                        .segments
                        .iter()
                        .map(|segment| segment.support([local[0], local[1]]))
                        .fold(f64::NEG_INFINITY, f64::max)
                        + local[2] * section.elevation_mm
                })
                .fold(f64::NEG_INFINITY, f64::max),
            _ => unreachable!("only swept and lofted bodies have a local reach"),
        }
    }

    /// How far the body (before cuts, finishes and booleans) reaches along a
    /// world `direction`: the largest `direction · p` over its points. Exact
    /// for boxes, extruded profiles, sweeps and revolutions about any in-plane
    /// axis, including partial turns. Lofts use their sections (exact for two;
    /// a smooth loft through more may bulge slightly past them).
    #[must_use]
    pub fn reach(&self, direction: [f64; 3]) -> f64 {
        let local = frame::apply_transposed(&self.rotation, direction);
        let bounds_reach = || {
            let (min, max) = self.body_bounds();
            (0..3)
                .map(|i| (local[i] * min[i]).max(local[i] * max[i]))
                .sum::<f64>()
        };
        let largest =
            |values: &mut dyn Iterator<Item = f64>| values.fold(f64::NEG_INFINITY, f64::max);
        let body = match &self.body {
            ProgramPartBody::Extrusion { .. } | ProgramPartBody::Revolve { .. }
                if self.pushes_faces_out() =>
            {
                bounds_reach()
            }
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
                ..
            } => {
                largest(&mut segments.iter().map(|s| s.support([local[0], local[1]])))
                    + (local[2] * distance_mm).max(0.0)
            }
            ProgramPartBody::Revolve { .. } => self.revolve_support(local),
            ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => self.local_reach(local),
        };
        self.joined()
            .map(|tool| tool.reach(direction))
            .fold(dot(self.at_mm, direction) + body, f64::max)
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
        self.booleans().any(|boolean| {
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
        if matches!(self.body, ProgramPartBody::Revolve { .. }) {
            self.size_mm = std::array::from_fn(|i| {
                let axis = std::array::from_fn(|j| if i == j { 1.0 } else { 0.0 });
                self.revolve_support(axis) + self.revolve_support(axis.map(|v| -v))
            });
        }
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
                let center = segment.arc.map(|arc| ("center", arc.center_mm));
                let controls = segment
                    .bezier
                    .into_iter()
                    .flat_map(|[c1, c2]| [("control_1", c1), ("control_2", c2)]);
                for (point, coordinates) in [("start", segment.start_mm), ("end", segment.end_mm)]
                    .into_iter()
                    .chain(center)
                    .chain(controls)
                {
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
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
                ..
            } => {
                // A cuboid's sketch is the document's rectangle (`crate::cad`).
                let sketch = match self.body.cuboid_size() {
                    Some([width, height, _]) => vec![
                        length("bounds.width", width),
                        length("bounds.height", height),
                    ],
                    None => profile_parameters(segments),
                };
                features.push(feature(
                    format!("{} sketch", self.name),
                    ProgramFeatureKind::Sketch,
                    sketch,
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
            // Any change rebuilds a swept or lofted part, so its tree names
            // the body only.
            ProgramPartBody::Sweep { .. } => features.push(feature(
                format!("{} feature", self.name),
                ProgramFeatureKind::Sweep,
                Vec::new(),
            )),
            ProgramPartBody::Loft { .. } => features.push(feature(
                format!("{} feature", self.name),
                ProgramFeatureKind::Loft,
                Vec::new(),
            )),
        }
        for operation in &self.operations {
            let cut = match operation {
                ProgramOperation::Cut(cut) => cut,
                ProgramOperation::Finish(fillet) => {
                    features.push(feature(
                        fillet.name.clone(),
                        match fillet.kind {
                            ProgramEdgeFinishKind::Fillet => ProgramFeatureKind::Fillet,
                            ProgramEdgeFinishKind::Chamfer => ProgramFeatureKind::Chamfer,
                        },
                        vec![length("radius", fillet.radius_mm)],
                    ));
                    continue;
                }
                ProgramOperation::FaceOffset(offset) => {
                    features.push(feature(
                        offset.name.clone(),
                        ProgramFeatureKind::FaceOffset,
                        vec![length("distance", offset.distance_mm)],
                    ));
                    continue;
                }
                // A changed boolean, mirror or shell rebuilds the part; they
                // have no parameters here.
                ProgramOperation::Boolean(_)
                | ProgramOperation::Mirror(_)
                | ProgramOperation::Shell(_) => continue,
                // Listed after the operations, as the document panel drills them.
                ProgramOperation::Hole(_) | ProgramOperation::Pocket(_) => continue,
            };
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
                vec![length("extent.distance", cut.depth_mm)],
            ));
        }
        for hole in self.holes() {
            let prefix = format!("{} hole {}", self.name, hole.id);
            let origin = hole.entry_mm;
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
                vec![length("extent.distance", hole.depth_mm)],
            ));
        }
        for pocket in self.pockets() {
            let prefix = format!("{} pocket {}", self.name, pocket.id);
            let [du, dv] = pocket.extent_mm();
            let origin: [f64; 3] = std::array::from_fn(|i| {
                pocket.corner_mm[i] + 0.5 * (pocket.u[i] * du + pocket.v[i] * dv)
            });
            features.push(feature(
                format!("{prefix} workplane"),
                ProgramFeatureKind::Workplane,
                ["frame.origin.x", "frame.origin.y", "frame.origin.z"]
                    .into_iter()
                    .zip(origin)
                    .map(|(path, value)| length(path, value))
                    .collect(),
            ));
            // The planner's sketch x axis follows the next axis after the
            // face normal's (x -> y -> z -> x); on a y face that is v.
            let (u, v) = (du * 0.5, dv * 0.5);
            let (x, y) = if pocket.inward[1].abs() > 0.5 {
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
                vec![length("extent.distance", pocket.depth_mm)],
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

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramGroup {
    pub name: String,
    pub grounded: bool,
    /// Direct members: parts or previously declared groups, each with one parent.
    pub members: Vec<String>,
}

/// One shared assembly definition; its original group is also its first instance.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramComponent {
    pub name: String,
    pub parts: Vec<Part>,
    pub groups: Vec<ProgramGroup>,
    pub joints: Vec<Joint>,
    pub motions: Vec<crate::motion::ProgramMotion>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ProgramInstance {
    pub name: String,
    pub component: String,
    pub at_mm: [f64; 3],
    pub rotation: Mat3,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ProgramModel {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub floor_z_mm: Option<f64>,
    pub params: Vec<Param>,
    pub parts: Vec<Part>,
    /// Explicit new-name to previous-name identity claims, independent of geometry.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub continuations: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub groups: Vec<ProgramGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<ProgramComponent>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub instances: Vec<ProgramInstance>,
    /// Helper bodies used only as boolean tools: never built, listed or validated.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<Part>,
    pub joints: Vec<Joint>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub motions: Vec<crate::motion::ProgramMotion>,
    /// Conditions the program states about its geometry (`expect()`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub expectations: Vec<crate::expect::Expectation>,
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
