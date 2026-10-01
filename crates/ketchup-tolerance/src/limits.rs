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
