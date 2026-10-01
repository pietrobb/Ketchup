//! Size limits on input, one name per quantity.
//!
//! A limit keeps a request bounded: it caps how much work or memory one input can
//! demand. Every crate checks against these names; a crate never restates a value.
//! A limit that only one crate knows about stays in that crate.

/// Most segments in one curve chain: a sweep or slot path, one loop of a planar
/// profile, a program path, the quarter turns of a helix.
pub const PATH_SEGMENTS: usize = 64;

/// Most holes in one planar region.
pub const REGION_HOLES: usize = 64;

/// Most segments across all loops of one planar region.
pub const REGION_SEGMENTS: usize = PATH_SEGMENTS * REGION_HOLES;
