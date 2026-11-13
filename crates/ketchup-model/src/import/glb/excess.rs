//! Refusals of a model larger than one import takes (R1110-SKP-401): each names what
//! is too many, how many, the most and what to do (§2).

use super::GlbImportError;
use std::fmt;

/// What a model holds more of than one import takes (R1110-SKP-401).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GlbMeasure {
    Meshes,
    MeshParts,
    BufferViews,
    Accessors,
    Nodes,
    Nesting,
    Vertices,
    Triangles,
    PlacedTriangles,
    Outputs,
    Commands,
    Tags,
    /// Values of the glTF JSON the import reads: past about 60 000 placements they run
    /// out before the count of any one thing does (review 2026-11-12 P3).
    Description,
}

impl GlbMeasure {
    fn name(self) -> &'static str {
        match self {
            Self::Meshes => "meshes",
            Self::MeshParts => "mesh parts",
            Self::BufferViews => "buffer views",
            Self::Accessors => "accessors",
            Self::Nodes => "groups and component placements",
            Self::Nesting => "levels of nested groups",
            Self::Vertices => "distinct vertices",
            Self::Triangles => "distinct triangles",
            Self::PlacedTriangles => "placed triangles",
            Self::Outputs => "objects to create",
            Self::Commands => "edits to make in one Undo step",
            Self::Tags => "tags",
            Self::Description => {
                "values describing its groups, component placements, meshes and materials"
            }
        }
    }

    fn advice(self) -> &'static str {
        match self {
            Self::Vertices | Self::Triangles | Self::PlacedTriangles => {
                "Lower the segments of arcs and circles, or delete detailed parts such as \
                 plants and 3D text, then import again."
            }
            Self::Nesting => "Explode some of the nested groups or components, then import again.",
            _ => {
                "Purge what the model does not use (in SketchUp: Window > Model Info > \
                 Statistics > Purge Unused) or save parts of it as separate files, then \
                 import them one by one."
            }
        }
    }
}

/// How much of `measure` a model holds, at least, and the most one import takes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GlbExcess {
    pub measure: GlbMeasure,
    /// The count found so far: the import stops counting once it is past `limit`.
    pub at_least: usize,
    pub limit: usize,
}

impl fmt::Display for GlbExcess {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "the model has at least {} {}; one import takes at most {}. {}",
            self.at_least,
            self.measure.name(),
            self.limit,
            self.measure.advice()
        )
    }
}

/// The refusal of `at_least` of `measure`, more than `limit`.
pub(super) fn too_large(measure: GlbMeasure, at_least: usize, limit: usize) -> GlbImportError {
    GlbImportError::TooLarge(GlbExcess {
        measure,
        at_least: at_least.max(limit.saturating_add(1)),
        limit,
    })
}
