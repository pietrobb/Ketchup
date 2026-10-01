//! Size limits on input, one name per quantity.
//!
//! A limit keeps a request bounded: it caps how much work or memory one input can
//! demand. Every crate checks against these names; a crate never restates a value.
//! A limit that only one crate knows about stays in that crate. The largest length is
//! [`crate::MAX_COORDINATE_MM`]: a distance never exceeds the model space it lives in.

/// Shortest edge, distance, radius, thickness or extent a feature may have, in mm:
/// 1e5 linear tolerances, so a feature this small is still unambiguous geometry.
pub const MIN_LENGTH_MM: f64 = 0.01;

/// Most segments in one curve chain: a sweep or slot path, one loop of a planar
/// profile, a program path, the quarter turns of a helix.
pub const PATH_SEGMENTS: usize = 64;

/// Most holes in one planar region.
pub const REGION_HOLES: usize = 64;

/// Most segments across all loops of one planar region.
pub const REGION_SEGMENTS: usize = PATH_SEGMENTS * REGION_HOLES;

/// Most vertices in one mesh: a mesh body, one imported STL, glTF primitive or scene
/// definition.
pub const MESH_VERTICES: usize = 100_000;

/// Most triangles in one mesh, including a tessellated display mesh.
pub const MESH_TRIANGLES: usize = 2 * MESH_VERTICES;

/// Most steps in one instance path (nesting depth of definitions).
pub const INSTANCE_PATH_STEPS: usize = 256;

/// Most placed instances one export or drawing writes.
pub const EXPORT_INSTANCES: usize = 8_000;

/// Most vertices one mesh export writes.
pub const EXPORT_VERTICES: usize = 2_000_000;

/// Most triangles one mesh export writes.
pub const EXPORT_TRIANGLES: usize = 2 * EXPORT_VERTICES;

/// Largest serialized report one query returns (a scene, index, workset or
/// validation report), in bytes.
pub const REPORT_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// Most batch jobs one host keeps open at a time.
pub const BATCH_JOBS: usize = 16;

/// Longest name, in bytes: a part, feature, parameter path, case or fingerprint label.
pub const NAME_BYTES: usize = 128;

/// Longest free-text field, in bytes: a backend, tolerance description, imported label
/// or plan name.
pub const TEXT_BYTES: usize = 1_024;

/// Most topology references (edges or faces) one feature or analysis case names: a
/// fillet, chamfer, shell opening or constrained faces.
pub const FEATURE_REFERENCES: usize = 64;

/// Most radius stations along one variable fillet.
pub const FILLET_RADIUS_STATIONS: usize = 32;

/// Most sections one loft passes through.
pub const LOFT_SECTIONS: usize = 16;

/// Most surfaces one knit joins.
pub const KNIT_SURFACES: usize = 256;
