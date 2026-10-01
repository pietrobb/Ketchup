#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Point3 {
    pub const ORIGIN: Self = Self {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoxSpec {
    pub origin_mm: Point3,
    pub size_mm: Size3,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleExtrudeSpec {
    pub width_mm: f64,
    pub depth_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleOffsetSpec {
    pub min_mm: [f64; 2],
    pub max_mm: [f64; 2],
    pub distance_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RectangleSweepSpec {
    pub profile_min_mm: [f64; 2],
    pub profile_max_mm: [f64; 2],
    pub path_start_mm: [f64; 2],
    pub path_end_mm: [f64; 2],
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplineLoftSection {
    pub elevation_mm: f64,
    pub control_points_mm: Vec<[f64; 2]>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SplineLoftSpec {
    pub sections: Vec<SplineLoftSection>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanarLoftSection {
    pub elevation_mm: f64,
    pub profile: PlanarProfileLoop,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PlanarLoftSpec {
    pub sections: Vec<PlanarLoftSection>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum FramedLoftProfile {
    Planar(PlanarProfileLoop),
    Spline { control_points_mm: Vec<[f64; 2]> },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FramedLoftSection {
    pub elevation_mm: f64,
    pub frame: ketchup_geometry::linalg::Frame,
    pub profile: FramedLoftProfile,
}

#[derive(Clone, Debug, PartialEq)]
pub struct FramedLoftSpec {
    pub sections: Vec<FramedLoftSection>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LoftSurfaceContinuity {
    #[default]
    Position,
    Tangent,
    Curvature,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CircleExtrudeSpec {
    pub center_mm: [f64; 2],
    pub radius_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxialToolMotion {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    Arc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        clockwise: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxialToolSweepSpec {
    pub motion: AxialToolMotion,
    pub radius_mm: f64,
    pub axial_length_mm: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PlanarProfileSegment {
    Line {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
    },
    CircularArc {
        start_mm: [f64; 2],
        end_mm: [f64; 2],
        center_mm: [f64; 2],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 2],
        control_1_mm: [f64; 2],
        control_2_mm: [f64; 2],
        end_mm: [f64; 2],
    },
}

/// One exact three-dimensional segment of an open spatial sweep path.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpatialProfileSegment {
    Line {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
    },
    CircularArc {
        start_mm: [f64; 3],
        end_mm: [f64; 3],
        center_mm: [f64; 3],
        normal: [f64; 3],
        clockwise: bool,
    },
    CubicBezier {
        start_mm: [f64; 3],
        control_1_mm: [f64; 3],
        control_2_mm: [f64; 3],
        end_mm: [f64; 3],
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlanarProfileLoop {
    Segments(Vec<PlanarProfileSegment>),
    Circle { center_mm: [f64; 2], radius_mm: f64 },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CylinderToolSpec {
    pub center_mm: [f64; 2],
    pub origin_z_mm: f64,
    pub radius_mm: f64,
    pub height_mm: f64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EdgeFinish {
    Fillet,
    Chamfer,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AdvancedChamferMode {
    TwoDistance { second_distance_mm: f64 },
    DistanceAngle { angle_degrees: f64 },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShellDirection {
    Inward,
    Outward,
    Symmetric,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CutMode {
    ThroughAll,
    BlindPlanar,
}
