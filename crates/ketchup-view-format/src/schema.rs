//! The JSON manifest of a `.ketchup-view` package: components, scenes and
//! dimensions. Every length is in source Z-up millimetres unless noted.

use serde::{Deserialize, Serialize};

/// Everything the Viewer shows besides the geometry itself.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub source: Source,
    pub definitions: Vec<Definition>,
    pub occurrences: Vec<Occurrence>,
    pub scenes: Vec<Scene>,
    pub dimensions: Vec<Dimension>,
    pub start_scene: Option<u64>,
}

/// Existing Snapshot identity, not a new Viewer fingerprint.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub document_id: u64,
    pub revision: u64,
    pub canonical_digest: String,
}

/// A component definition shared by all of its occurrences.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Definition {
    pub id: u64,
    pub name: String,
    pub note: Option<String>,
    pub material: Option<String>,
    pub color_srgb: Option<[u8; 3]>,
    pub local_dimensions: Option<LocalDimensions>,
}

/// Dimensions are in source definition axes, never world bounding-box extents.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "meaning", rename_all = "snake_case", deny_unknown_fields)]
pub enum LocalDimensions {
    LocalBounds { min_mm: [f64; 3], max_mm: [f64; 3] },
    AuthoredAxes { size_mm: [f64; 3] },
}

/// Stable path of an occurrence from its root component, as in the desktop model.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstancePath {
    pub root_occurrence_id: u64,
    pub steps: Vec<PathStep>,
}

/// One step below the root: into a group or a nested occurrence of a definition.
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PathStep {
    Group {
        owner_definition_id: u64,
        local_id: u64,
    },
    Occurrence {
        owner_definition_id: u64,
        local_id: u64,
    },
}

/// One placed component with its own name, note, material and attributes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Occurrence {
    pub name: String,
    pub path: InstancePath,
    pub definition_id: u64,
    /// GLB hierarchy node, whose matrix is glTF Y-up/metres. Multiple bodies
    /// may be children of this node; no flattened geometry or world AABB here.
    pub glb_node: usize,
    pub note: Option<String>,
    pub material: Option<String>,
    pub color_srgb: Option<[u8; 3]>,
    /// Inert author-supplied strings, never interpreted as programs or resources.
    pub attributes: std::collections::BTreeMap<String, String>,
}

/// Source coordinates are Z-up/mm. Span and FOV apply to the shorter viewport
/// axis, so a narrow phone preserves the scene intent without pixel pan/zoom.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub eye_mm: [f64; 3],
    pub target_mm: [f64; 3],
    pub up: [f64; 3],
    pub projection: Projection,
}

/// Parallel or perspective projection of a scene camera.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Projection {
    Orthographic {
        short_span_mm: f64,
    },
    Perspective {
        short_fov_radians: f64,
        /// Screen right/up displacement as a fraction of the short viewport
        /// extent. An off-axis lens shift, not an eye translation or pixel pan.
        lens_shift_short: [f64; 2],
    },
}

/// A saved view: camera, hidden components, display style, section and dimensions.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Scene {
    pub id: u64,
    pub name: String,
    pub camera: Camera,
    pub hidden: Vec<InstancePath>,
    pub style: DisplayStyle,
    pub section: Option<Section>,
    pub visible_dimensions: Vec<u64>,
}

/// How bodies are drawn: faces, faces with edges, or edges only.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayStyle {
    Shaded,
    ShadedEdges,
    Wireframe,
}

/// Hide the side normal points towards, matching the desktop section contract.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Section {
    pub normal: [f64; 3],
    pub offset_mm: f64,
}

/// A dimension that scenes can switch on, with its authoritative value.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dimension {
    pub id: u64,
    pub from: Anchor,
    pub to: Anchor,
    pub label_offset_mm: [f64; 3],
    /// Authoritative source value, not a measurement of the tessellated mesh.
    pub value_mm: f64,
    pub display_unit: LengthUnit,
}

/// One end of a dimension: a point on a component or in world space.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    pub occurrence: Option<InstancePath>,
    /// Local definition point when anchored, otherwise a world point.
    pub point_mm: [f64; 3],
}

/// Unit a dimension value is displayed in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    Millimetre,
    Centimetre,
    Metre,
    Inch,
}
