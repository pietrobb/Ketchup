//! Content-defined chunks of an encoded snapshot. Revisions of one document share
//! most of their bytes, so a revision history stores each distinct chunk once and a
//! revision as the list of chunks it is made of. Boundaries follow the content, so
//! an edit only changes the chunks around it. The file format does not depend on
//! where the boundaries fall: a reader only concatenates the listed chunks.

use std::ops::Range;

const MIN_CHUNK: usize = 2 * 1024;
const MAX_CHUNK: usize = 64 * 1024;
/// Top bits of the rolling hash that must be zero at a boundary: about 8 KiB apart.
const BOUNDARY_MASK: u64 = ((1 << 13) - 1) << (64 - 13);

const GEAR: [u64; 256] = gear_table();

/// Fixed pseudo-random byte weights (splitmix64), so boundaries are reproducible.
const fn gear_table() -> [u64; 256] {
    let mut table = [0_u64; 256];
    let mut state = 0_u64;
    let mut index = 0;
    while index < 256 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut value = state;
        value = (value ^ (value >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        table[index] = value ^ (value >> 31);
        index += 1;
    }
    table
}

pub(crate) struct Chunk {
    pub(crate) range: Range<usize>,
    pub(crate) sha256: [u8; 32],
}

pub(crate) fn chunks(bytes: &[u8]) -> Vec<Chunk> {
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let end = start + boundary(&bytes[start..]);
        chunks.push(Chunk {
            range: start..end,
            sha256: crate::graph::sha256_bytes(&bytes[start..end]),
        });
        start = end;
    }
    chunks
}

fn boundary(bytes: &[u8]) -> usize {
    let limit = bytes.len().min(MAX_CHUNK);
    if limit <= MIN_CHUNK {
        return limit;
    }
    let mut hash = 0_u64;
    for (index, &byte) in bytes.iter().enumerate().take(limit).skip(MIN_CHUNK) {
        hash = (hash << 1).wrapping_add(GEAR[usize::from(byte)]);
        if hash & BOUNDARY_MASK == 0 {
            return index + 1;
        }
    }
    limit
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noise(len: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 56) as u8
            })
            .collect()
    }

    #[test]
    fn chunks_cover_the_bytes_and_an_edit_changes_only_the_chunks_around_it() {
        let original = noise(1024 * 1024, 7);
        let original_chunks = chunks(&original);
        assert_eq!(original_chunks.first().unwrap().range.start, 0);
        assert_eq!(original_chunks.last().unwrap().range.end, original.len());
        assert!(
            original_chunks
                .windows(2)
                .all(|pair| pair[0].range.end == pair[1].range.start)
        );
        assert!(
            original_chunks
                .iter()
                .all(|chunk| chunk.range.len() <= MAX_CHUNK)
        );

        let mut edited = original.clone();
        edited.splice(500_000..500_010, *b"a toggled flag");
        let known = original_chunks
            .iter()
            .map(|chunk| chunk.sha256)
            .collect::<std::collections::BTreeSet<_>>();
        let new_bytes: usize = chunks(&edited)
            .iter()
            .filter(|chunk| !known.contains(&chunk.sha256))
            .map(|chunk| chunk.range.len())
            .sum();
        assert!(new_bytes <= 2 * MAX_CHUNK, "{new_bytes} new bytes");
    }
}
