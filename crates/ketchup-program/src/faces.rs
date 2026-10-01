//! Faces of a part as frames: the plane or cylinder each named face lies on,
//! computed from the body and carried through the part's operations in
//! program order, so a program can address a face of any part by name, after
//! any mirror, rotation or boolean.
//!
//! Frames are in the part's own frame; [`FaceFrame::placed`] moves one into
//! world. A face that is neither flat nor cylindrical (a cone, a sphere, a
//! Bezier side) has no frame.

use crate::frame::{self, Mat3};
use crate::model::{
    Part, ProgramArc, ProgramBoolean, ProgramBooleanKind, ProgramOperation, ProgramPartBody,
    ProgramProfileSegment, profile_bounds,
};
use ketchup_geometry::linalg::{cross, dot, length};
use ketchup_model::tolerance::APPROXIMATION;
use serde::Serialize;
use std::collections::BTreeMap;
use std::f64::consts::TAU;

/// The surface a face lies on.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FaceKind {
    /// Points `origin + a·u + b·v` for face coordinates (a, b) in mm.
    Planar,
    /// Points `origin + b·v + radius·(cos a·u + sin a·(v × u))` for face
    /// coordinates (a in degrees about `v`, b in mm along `v`).
    Cylindrical { radius_mm: f64 },
}

/// One named face: its surface, outward normal and the face coordinates it
/// covers.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct FaceFrame {
    pub name: String,
    pub kind: FaceKind,
    pub origin_mm: [f64; 3],
    /// Outward unit normal; on a cylinder the normal at angle 0, which is
    /// `u` on a convex face and `-u` on a bore.
    pub normal: [f64; 3],
    pub u: [f64; 3],
    pub v: [f64; 3],
    /// Face coordinates the face spans (its bounding rectangle on a plane).
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// Panel faces: name, the local axis it is perpendicular to, whether it lies
/// at the far end of that axis. Face coordinates follow the remaining axes in
/// order: z faces use (x, y), x faces (y, z), y faces (x, z).
pub const PANEL_FACES: [(&str, usize, bool); 6] = [
    ("x-", 0, false),
    ("x+", 0, true),
    ("y-", 1, false),
    ("y+", 1, true),
    ("z-", 2, false),
    ("z+", 2, true),
];

fn unit(axis: usize) -> [f64; 3] {
    std::array::from_fn(|index| if index == axis { 1.0 } else { 0.0 })
}

fn scaled(vector: [f64; 3], factor: f64) -> [f64; 3] {
    vector.map(|value| value * factor)
}

fn plus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|index| a[index] + b[index])
}

fn minus(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|index| a[index] - b[index])
}

fn flat(point: [f64; 2]) -> [f64; 3] {
    [point[0], point[1], 0.0]
}

impl FaceFrame {
    fn planar(name: &str, origin_mm: [f64; 3], normal: [f64; 3], u: [f64; 3], v: [f64; 3]) -> Self {
        Self {
            name: name.to_owned(),
            kind: FaceKind::Planar,
            origin_mm,
            normal,
            u,
            v,
            min: [0.0; 2],
            max: [0.0; 2],
        }
    }

    fn spanning(mut self, min: [f64; 2], max: [f64; 2]) -> Self {
        self.min = min;
        self.max = max;
        self
    }

    /// The point at face coordinates `at`.
    #[must_use]
    pub fn point(&self, at: [f64; 2]) -> [f64; 3] {
        match self.kind {
            FaceKind::Planar => plus(
                self.origin_mm,
                plus(scaled(self.u, at[0]), scaled(self.v, at[1])),
            ),
            FaceKind::Cylindrical { radius_mm } => plus(
                plus(self.origin_mm, scaled(self.v, at[1])),
                scaled(self.radial(at[0]), radius_mm),
            ),
        }
    }

    /// Unit direction from the axis at `angle` degrees (cylinders).
    fn radial(&self, angle: f64) -> [f64; 3] {
        let (sin, cos) = angle.to_radians().sin_cos();
        plus(scaled(self.u, cos), scaled(cross(self.v, self.u), sin))
    }

    /// Outward unit normal at face coordinates `at`.
    #[must_use]
    pub fn normal_at(&self, at: [f64; 2]) -> [f64; 3] {
        match self.kind {
            FaceKind::Planar => self.normal,
            FaceKind::Cylindrical { .. } => {
                scaled(self.radial(at[0]), dot(self.normal, self.u).signum())
            }
        }
    }

    /// Face coordinates of the point on the face's surface nearest `point`.
    #[must_use]
    pub fn coordinates(&self, point: [f64; 3]) -> [f64; 2] {
        let offset = minus(point, self.origin_mm);
        let along_v = dot(offset, self.v);
        match self.kind {
            FaceKind::Planar => [dot(offset, self.u), along_v],
            FaceKind::Cylindrical { .. } => {
                let across = cross(self.v, self.u);
                let angle = dot(offset, across)
                    .atan2(dot(offset, self.u))
                    .rem_euclid(TAU);
                [angle.to_degrees(), along_v]
            }
        }
    }

    /// How far `point` lies from the face's surface.
    #[must_use]
    pub fn distance(&self, point: [f64; 3]) -> f64 {
        let offset = minus(point, self.origin_mm);
        match self.kind {
            FaceKind::Planar => dot(offset, self.normal).abs(),
            FaceKind::Cylindrical { radius_mm } => {
                let radial = minus(offset, scaled(self.v, dot(offset, self.v)));
                (length(radial) - radius_mm).abs()
            }
        }
    }

    /// Whether `point` lies on the face, within `tolerance` mm.
    #[must_use]
    pub fn contains(&self, point: [f64; 3], tolerance: f64) -> bool {
        if self.distance(point) > tolerance {
            return false;
        }
        let at = self.coordinates(point);
        let slack = match self.kind {
            FaceKind::Planar => [tolerance; 2],
            FaceKind::Cylindrical { radius_mm } => [
                (tolerance / radius_mm.max(tolerance)).to_degrees(),
                tolerance,
            ],
        };
        (0..2).all(|index| {
            at[index] >= self.min[index] - slack[index]
                && at[index] <= self.max[index] + slack[index]
        })
    }

    /// The middle of the face coordinates it spans, on its surface.
    #[must_use]
    pub fn center(&self) -> [f64; 3] {
        self.point(std::array::from_fn(|index| {
            0.5 * (self.min[index] + self.max[index])
        }))
    }

    /// The frame moved by rotation `rotation` then by `translation`; a
    /// reflection (`reflected`) keeps every face coordinate.
    #[must_use]
    pub fn placed(&self, rotation: &Mat3, translation: [f64; 3], reflected: bool) -> Self {
        let mut placed = Self {
            origin_mm: plus(frame::apply(rotation, self.origin_mm), translation),
            normal: frame::apply(rotation, self.normal),
            u: frame::apply(rotation, self.u),
            v: frame::apply(rotation, self.v),
            ..self.clone()
        };
        if reflected && matches!(self.kind, FaceKind::Cylindrical { .. }) {
            // A reflection turns angles the other way about `v`; turning
            // them about `-v` again keeps them, with `b` measured along `-v`.
            placed.v = scaled(placed.v, -1.0);
            placed.min[1] = -self.max[1];
            placed.max[1] = -self.min[1];
        }
        placed
    }

    /// The face moved `distance_mm` along its outward normal.
    fn offset(&mut self, distance_mm: f64) {
        match &mut self.kind {
            FaceKind::Planar => {
                self.origin_mm = plus(self.origin_mm, scaled(self.normal, distance_mm));
            }
            FaceKind::Cylindrical { radius_mm } => {
                *radius_mm += distance_mm * dot(self.normal, self.u).signum();
            }
        }
    }

    /// The same surface facing the other way (a subtracted tool's face, a
    /// shell's inner wall).
    fn reversed(mut self) -> Self {
        self.normal = scaled(self.normal, -1.0);
        self
    }
}

/// +1 for a counter-clockwise loop, -1 for a clockwise one.
fn turn(segments: &[ProgramProfileSegment]) -> f64 {
    let doubled_area: f64 = segments
        .iter()
        .map(|s| s.start_mm[0] * s.end_mm[1] - s.end_mm[0] * s.start_mm[1])
        .sum();
    if doubled_area < 0.0 { -1.0 } else { 1.0 }
}

/// Unit direction and length of a straight side.
fn side(segment: &ProgramProfileSegment) -> Option<([f64; 3], f64)> {
    let direction = minus(flat(segment.end_mm), flat(segment.start_mm));
    let length = length(direction);
    (segment.is_line() && length > 0.0).then(|| (scaled(direction, 1.0 / length), length))
}

/// Outward in-plane normal of a straight side of a loop turning `turn`.
fn side_normal(direction: [f64; 3], turn: f64) -> [f64; 3] {
    [direction[1] * turn, -direction[0] * turn, 0.0]
}

/// Counter-clockwise start (unit, from the centre), radius and span in
/// degrees of an arc side, and whether its face bulges outward.
fn arc_side(segment: &ProgramProfileSegment, turn: f64) -> Option<([f64; 3], f64, f64, bool)> {
    let arc = segment.arc?;
    let center = flat(arc.center_mm);
    let (from, to) = if arc.clockwise {
        (segment.end_mm, segment.start_mm)
    } else {
        (segment.start_mm, segment.end_mm)
    };
    let (from, to) = (minus(flat(from), center), minus(flat(to), center));
    let radius = length(from);
    let span = (to[1].atan2(to[0]) - from[1].atan2(from[0])).rem_euclid(TAU);
    let span = if span <= APPROXIMATION { TAU } else { span };
    // Travelling counter-clockwise the right-hand side faces away from the
    // centre, so the face bulges out when the arc turns with the loop.
    let convex = arc.clockwise == (turn < 0.0);
    Some((
        scaled(from, 1.0 / radius),
        radius,
        span.to_degrees(),
        convex,
    ))
}

fn extrusion_faces(segments: &[ProgramProfileSegment], distance_mm: f64) -> Vec<FaceFrame> {
    let (min, max) = profile_bounds(segments);
    let (x, y, z) = (unit(0), unit(1), unit(2));
    let mut faces = vec![
        FaceFrame::planar("start", [0.0; 3], scaled(z, -1.0), x, y).spanning(min, max),
        FaceFrame::planar("end", [0.0, 0.0, distance_mm], z, x, y).spanning(min, max),
    ];
    let turn = turn(segments);
    for segment in segments {
        if let Some((direction, length)) = side(segment) {
            faces.push(
                FaceFrame::planar(
                    &segment.name,
                    flat(segment.start_mm),
                    side_normal(direction, turn),
                    direction,
                    z,
                )
                .spanning([0.0; 2], [length, distance_mm]),
            );
        } else if let Some((from, radius, span, convex)) = arc_side(segment, turn) {
            faces.push(FaceFrame {
                name: segment.name.clone(),
                kind: FaceKind::Cylindrical { radius_mm: radius },
                origin_mm: flat(segment.arc.map_or([0.0; 2], |arc| arc.center_mm)),
                normal: scaled(from, if convex { 1.0 } else { -1.0 }),
                u: from,
                v: z,
                min: [0.0; 2],
                max: [span, distance_mm],
            });
        }
    }
    faces
}

fn revolve_faces(
    segments: &[ProgramProfileSegment],
    axis_start_mm: [f64; 2],
    axis_end_mm: [f64; 2],
    angle_degrees: f64,
) -> Vec<FaceFrame> {
    let origin = flat(axis_start_mm);
    let Some(axis) = frame::normalized(minus(flat(axis_end_mm), origin)) else {
        return Vec::new();
    };
    // `radial` points from the axis towards the profile, in its plane; the
    // solid turns from there towards `axis × radial`.
    let across = [-axis[1], axis[0], 0.0];
    let side_of_profile: f64 = segments
        .iter()
        .map(|segment| dot(minus(flat(segment.start_mm), origin), across))
        .sum();
    let radial = if side_of_profile < 0.0 {
        scaled(across, -1.0)
    } else {
        across
    };
    let tangent = cross(axis, radial);
    let full = angle_degrees >= 360.0;
    let mut faces = Vec::new();
    if !full {
        let (min, max) = profile_bounds(segments);
        let turned = frame::axis_angle(axis, angle_degrees).unwrap_or(frame::IDENTITY);
        let start = FaceFrame::planar("start", [0.0; 3], scaled(tangent, -1.0), unit(0), unit(1))
            .spanning(min, max);
        let mut end = start.placed(&turned, minus(origin, frame::apply(&turned, origin)), false);
        end.name = "end".to_owned();
        end.normal = frame::apply(&turned, tangent);
        faces.extend([start, end]);
    }
    let turn = turn(segments);
    let offset = |point: [f64; 2]| minus(flat(point), origin);
    for segment in segments {
        let Some((direction, _)) = side(segment) else {
            continue;
        };
        let outward = side_normal(direction, turn);
        let along = dot(direction, axis);
        let ends = [offset(segment.start_mm), offset(segment.end_mm)];
        let radius = dot(ends[0], radial).abs();
        if along.abs() >= 1.0 - APPROXIMATION {
            if radius <= APPROXIMATION {
                // A side on the axis sweeps no surface.
                continue;
            }
            let heights = ends.map(|end| dot(end, axis));
            faces.push(FaceFrame {
                name: segment.name.clone(),
                kind: FaceKind::Cylindrical { radius_mm: radius },
                origin_mm: origin,
                normal: scaled(radial, dot(outward, radial).signum()),
                u: radial,
                v: axis,
                min: [0.0, heights[0].min(heights[1])],
                max: [angle_degrees.min(360.0), heights[0].max(heights[1])],
            });
        } else if along.abs() <= APPROXIMATION {
            // A flat ring (or disc) across the axis; its rectangle bounds the
            // swept sector of radii between the side's ends.
            let height = dot(ends[0], axis);
            let radii = ends.map(|end| dot(end, radial).abs());
            let (inner, outer) = (radii[0].min(radii[1]), radii[0].max(radii[1]));
            let (mut min, mut max) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
            let sweep = angle_degrees.min(360.0);
            let angles = [0.0, 90.0, 180.0, 270.0, sweep]
                .into_iter()
                .filter(|angle| *angle <= sweep);
            for angle in angles {
                let (sin, cos) = f64::to_radians(angle).sin_cos();
                for radius in [inner, outer] {
                    let point = [radius * cos, radius * sin];
                    for index in 0..2 {
                        min[index] = min[index].min(point[index]);
                        max[index] = max[index].max(point[index]);
                    }
                }
            }
            faces.push(
                FaceFrame::planar(
                    &segment.name,
                    plus(origin, scaled(axis, height)),
                    outward,
                    radial,
                    tangent,
                )
                .spanning(min, max),
            );
        }
    }
    faces
}

impl Part {
    /// Frames of the body's faces before any operation.
    fn body_face_frames(&self) -> Vec<FaceFrame> {
        match &self.body {
            ProgramPartBody::Panel => PANEL_FACES
                .iter()
                .map(|&(name, axis, far)| {
                    let (u_axis, v_axis) = match axis {
                        0 => (1, 2),
                        1 => (0, 2),
                        _ => (0, 1),
                    };
                    let mut origin = [0.0; 3];
                    origin[axis] = if far { self.size_mm[axis] } else { 0.0 };
                    FaceFrame::planar(
                        name,
                        origin,
                        scaled(unit(axis), if far { 1.0 } else { -1.0 }),
                        unit(u_axis),
                        unit(v_axis),
                    )
                    .spanning([0.0; 2], [self.size_mm[u_axis], self.size_mm[v_axis]])
                })
                .collect(),
            ProgramPartBody::Extrusion {
                segments,
                distance_mm,
            } => extrusion_faces(segments, *distance_mm),
            ProgramPartBody::Revolve {
                segments,
                axis_start_mm,
                axis_end_mm,
                angle_degrees,
            } => revolve_faces(segments, *axis_start_mm, *axis_end_mm, *angle_degrees),
            ProgramPartBody::Sweep { .. } | ProgramPartBody::Loft { .. } => Vec::new(),
        }
    }

    /// Every flat or cylindrical face the part has now, in its own frame:
    /// the body's faces moved by push_pull, reflected by mirror, the faces a
    /// boolean tool leaves (`<operation>.<tool face>`) and a shell's inner
    /// walls (`<shell>.<face>`), in program order.
    #[must_use]
    pub fn face_frames(&self) -> Vec<FaceFrame> {
        let mut faces = self.body_face_frames();
        for operation in &self.operations {
            match operation {
                ProgramOperation::FaceOffset(offset) => {
                    for face in faces.iter_mut().filter(|face| face.name == offset.face) {
                        face.offset(offset.distance_mm);
                    }
                }
                ProgramOperation::Mirror(mirror) => {
                    let mut reflection = frame::IDENTITY;
                    reflection[mirror.axis][mirror.axis] = -1.0;
                    let shift = scaled(unit(mirror.axis), 2.0 * mirror.center_mm);
                    for face in &mut faces {
                        *face = face.placed(&reflection, shift, true);
                    }
                }
                ProgramOperation::Boolean(boolean) => {
                    let tool = &boolean.tool;
                    let rotation =
                        frame::multiply(&frame::transposed(&self.rotation), &tool.rotation);
                    let shift = self.to_local(tool.at_mm);
                    faces.extend(tool.face_frames().into_iter().map(|face| {
                        let mut placed = face.placed(&rotation, shift, false);
                        placed.name = format!("{}.{}", boolean.name, face.name);
                        if boolean.kind == ProgramBooleanKind::Subtract {
                            placed.reversed()
                        } else {
                            placed
                        }
                    }));
                }
                ProgramOperation::Shell(shell) => {
                    faces.retain(|face| !shell.open.contains(&face.name));
                    let inner = faces
                        .iter()
                        .map(|face| {
                            let mut wall = face.clone();
                            wall.offset(-shell.thickness_mm);
                            wall.name = format!("{}.{}", shell.name, face.name);
                            wall.reversed()
                        })
                        .collect::<Vec<_>>();
                    faces.extend(inner);
                }
                ProgramOperation::Cut(_) | ProgramOperation::Finish(_) => {}
            }
        }
        faces
    }

    /// The frame of face `name` in the part's own frame.
    ///
    /// # Errors
    /// Names the face and lists the faces the part has.
    pub fn face_frame(&self, name: &str) -> Result<FaceFrame, String> {
        let faces = self.face_frames();
        if let Some(face) = faces.iter().find(|face| face.name == name) {
            return Ok(face.clone());
        }
        if faces.is_empty() {
            return Err(format!(
                "{:?} has no flat or cylindrical faces with names (swept and lofted bodies have none)",
                self.name
            ));
        }
        let kind = if self.exact_face_label(name).is_ok() {
            "is not a flat or cylindrical face"
        } else {
            "does not exist"
        };
        Err(format!(
            "face {name:?} {kind} on {:?}; its faces are: {}",
            self.name,
            faces
                .iter()
                .map(|face| face.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))
    }

    /// The frame placed in world.
    #[must_use]
    pub fn world_face(&self, face: &FaceFrame) -> FaceFrame {
        face.placed(&self.rotation, self.at_mm, false)
    }

    /// The face nearest the part-local `point` among those it lies on within
    /// `tolerance` mm.
    #[must_use]
    pub fn face_frame_at(&self, point: [f64; 3], tolerance: f64) -> Option<FaceFrame> {
        self.face_frames()
            .into_iter()
            .filter(|face| face.contains(point, tolerance))
            .min_by(|a, b| a.distance(point).total_cmp(&b.distance(point)))
    }
}

/// A point and a direction in a part's frame.
type Placement = ([f64; 3], [f64; 3]);

impl Part {
    fn mirrors(&self) -> impl DoubleEndedIterator<Item = (usize, f64)> + '_ {
        self.operations
            .iter()
            .filter_map(|operation| match operation {
                ProgramOperation::Mirror(mirror) => Some((mirror.axis, mirror.center_mm)),
                _ => None,
            })
    }

    /// Where a point and direction of the body before its operations end up
    /// after them (its mirrors).
    #[must_use]
    pub fn after_operations(&self, (mut point, mut direction): Placement) -> Placement {
        for (axis, center) in self.mirrors() {
            point[axis] = 2.0 * center - point[axis];
            direction[axis] = -direction[axis];
        }
        (point, direction)
    }

    /// The body point and direction that the part's operations carry to
    /// `placement`: holes and pockets are drilled into the body before them.
    #[must_use]
    pub fn before_operations(&self, (mut point, mut direction): Placement) -> Placement {
        for (axis, center) in self.mirrors().rev() {
            point[axis] = 2.0 * center - point[axis];
            direction[axis] = -direction[axis];
        }
        (point, direction)
    }

    /// Holes and pockets as subtracted tools, in the body's frame, for a
    /// part built from a profile; a box carries them itself (`crate::cad`).
    #[must_use]
    pub fn machining_tools(&self) -> Vec<ProgramBoolean> {
        if matches!(self.body, ProgramPartBody::Panel) {
            return Vec::new();
        }
        let tool = |name: String, size_mm: [f64; 3], corner: [f64; 3], axes: Mat3, body| {
            let mut part = Part {
                name,
                size_mm,
                at_mm: self.to_world(corner),
                rotation: frame::multiply(&self.rotation, &axes),
                material: None,
                color: None,
                attributes: BTreeMap::new(),
                body,
                operations: Vec::new(),
                features: Vec::new(),
                holes: Vec::new(),
                pockets: Vec::new(),
            };
            part.refresh_feature_tree();
            ProgramBoolean {
                name: part.name.clone(),
                kind: ProgramBooleanKind::Subtract,
                tool: part,
            }
        };
        let columns = |x: [f64; 3], y: [f64; 3], z: [f64; 3]| -> Mat3 {
            std::array::from_fn(|row| [x[row], y[row], z[row]])
        };
        let holes = self.holes.iter().map(|hole| {
            let radius = hole.diameter_mm * 0.5;
            let arc = Some(ProgramArc {
                center_mm: [0.0; 2],
                clockwise: false,
            });
            let half = |name: &str, start_mm: [f64; 2], end_mm: [f64; 2]| ProgramProfileSegment {
                arc,
                ..ProgramProfileSegment::line(name, start_mm, end_mm)
            };
            let across = if hole.inward[0].abs() < 0.9 {
                unit(0)
            } else {
                unit(1)
            };
            let axes = frame::from_axes(across, hole.inward).unwrap_or(frame::IDENTITY);
            tool(
                format!("hole {}", hole.id),
                [hole.diameter_mm, hole.diameter_mm, hole.depth_mm],
                hole.entry_mm,
                axes,
                ProgramPartBody::Extrusion {
                    segments: vec![
                        half("wall_1", [radius, 0.0], [-radius, 0.0]),
                        half("wall_2", [-radius, 0.0], [radius, 0.0]),
                    ],
                    distance_mm: hole.depth_mm,
                },
            )
        });
        let pockets = self.pockets.iter().map(|pocket| {
            let [du, dv] = pocket.extent_mm();
            // A box frame is right-handed: u, v, inward or v, u, inward.
            let (axes, size) = if dot(cross(pocket.u, pocket.v), pocket.inward) > 0.0 {
                (
                    columns(pocket.u, pocket.v, pocket.inward),
                    [du, dv, pocket.depth_mm],
                )
            } else {
                (
                    columns(pocket.v, pocket.u, pocket.inward),
                    [dv, du, pocket.depth_mm],
                )
            };
            tool(
                format!("pocket {}", pocket.id),
                size,
                pocket.corner_mm,
                axes,
                ProgramPartBody::Panel,
            )
        });
        holes.chain(pockets).collect()
    }
}
