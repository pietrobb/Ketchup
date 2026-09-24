//! Plain data produced by evaluating a rule program.
//!
//! Every part is an axis-aligned cuboid in world millimetres, described by its
//! minimum corner (`at_mm`) and its extent (`size_mm`). Holes and pockets are
//! machined into one of the six faces and use face-local coordinates.

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
pub struct Part {
    /// Stable identity: the path given in the program, e.g. `korpus/bok_lavy`.
    pub name: String,
    pub size_mm: [f64; 3],
    pub at_mm: [f64; 3],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub material: Option<String>,
    /// Axis index of the grain direction, if the material has one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grain_axis: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
    pub holes: Vec<Hole>,
    pub pockets: Vec<Pocket>,
}

impl Part {
    #[must_use]
    pub fn max_mm(&self) -> [f64; 3] {
        std::array::from_fn(|axis| self.at_mm[axis] + self.size_mm[axis])
    }

    /// World point for a part-local point.
    #[must_use]
    pub fn to_world(&self, local: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| self.at_mm[axis] + local[axis])
    }

    /// Part-local point for a world point.
    #[must_use]
    pub fn to_local(&self, world: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|axis| world[axis] - self.at_mm[axis])
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
    pub joints: Vec<Joint>,
}

impl ProgramModel {
    #[must_use]
    pub fn part(&self, name: &str) -> Option<&Part> {
        self.parts.iter().find(|part| part.name == name)
    }
}
