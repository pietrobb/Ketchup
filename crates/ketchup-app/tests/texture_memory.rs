//! Review 2026-11-13 R1113 P3: an interlaced (Adam7) PNG was read whole before it was
//! averaged down, 200–400 MB for a picture of 8 192², though the comment on the averaging
//! said the whole picture is never held. The memory a decoding takes is counted here by
//! the allocator of this test, a binary of its own, as the counts are of the whole process.
// The allocator only counts what it passes on to the system's.
#![allow(unsafe_code)]
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use ketchup_manufacturing::presentation::PresentationTexture;

struct Counting;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let now = CURRENT.fetch_add(layout.size(), Ordering::Relaxed) + layout.size();
        PEAK.fetch_max(now, Ordering::Relaxed);
        // SAFETY: the caller's layout is passed on unchanged.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: `ptr` was allocated by `System` with this layout, in `alloc`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// The memory `run` takes above what is held before it, at most.
fn peak<T>(run: impl FnOnce() -> T) -> (T, usize) {
    let before = CURRENT.load(Ordering::Relaxed);
    PEAK.store(before, Ordering::Relaxed);
    let value = run();
    (value, PEAK.load(Ordering::Relaxed) - before)
}

fn crc(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
    let start = bytes.len();
    bytes.extend(kind);
    bytes.extend(data);
    let checked = crc(&bytes[start..]);
    bytes.extend(checked.to_be_bytes());
    bytes
}

/// `raw` in a zlib stream of stored blocks.
fn stored_zlib(raw: &[u8]) -> Vec<u8> {
    let mut bytes = vec![0x78, 0x01];
    let blocks = raw.chunks(0xFFFF).collect::<Vec<_>>();
    for (index, block) in blocks.iter().enumerate() {
        let length = u16::try_from(block.len()).unwrap();
        bytes.push(u8::from(index + 1 == blocks.len()));
        bytes.extend(length.to_le_bytes());
        bytes.extend((!length).to_le_bytes());
        bytes.extend(*block);
    }
    let (mut a, mut b) = (1_u32, 0_u32);
    for byte in raw {
        a = (a + u32::from(*byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    bytes.extend(((b << 16) | a).to_be_bytes());
    bytes
}

/// An RGB PNG of `pixel`, interlaced (Adam7) or not.
fn png_of(
    width: u32,
    height: u32,
    interlaced: bool,
    pixel: impl Fn(u32, u32) -> [u8; 3],
) -> Vec<u8> {
    // First column, first row, column step and row step of each pass.
    let passes: &[(u32, u32, u32, u32)] = if interlaced {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    } else {
        &[(0, 0, 1, 1)]
    };
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in passes {
        if *x0 >= width || *y0 >= height {
            continue;
        }
        for y in (*y0..height).step_by(*dy as usize) {
            raw.push(0);
            for x in (*x0..width).step_by(*dx as usize) {
                raw.extend(pixel(x, y));
            }
        }
    }
    let mut header = width.to_be_bytes().to_vec();
    header.extend(height.to_be_bytes());
    header.extend([8, 2, 0, 0, u8::from(interlaced)]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &header));
    bytes.extend(chunk(b"IDAT", &stored_zlib(&raw)));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

/// One test, as the counts are of the whole process: tests run side by side.
#[test]
fn an_interlaced_picture_is_averaged_down_without_holding_it_whole() {
    let pixel = |x: u32, y: u32| {
        [
            u8::try_from(x % 251).unwrap(),
            u8::try_from((y * 3) % 256).unwrap(),
            u8::try_from((x * 7 + y * 5) % 256).unwrap(),
        ]
    };
    // Odd sides, so the last blocks and Adam7 passes are partial.
    for (width, height) in [(3_001, 13), (17, 2_051), (5, 3)] {
        let plain = PresentationTexture::decode(&png_of(width, height, false, pixel), "image/png");
        let interlaced =
            PresentationTexture::decode(&png_of(width, height, true, pixel), "image/png");
        let (plain, interlaced) = (plain.unwrap(), interlaced.unwrap());
        assert_eq!(interlaced.size(), plain.size(), "{width} × {height}");
        assert!(interlaced.rgba() == plain.rgba(), "{width} × {height}");
    }

    // 16 384 × 256: 12.6 MB of pixels, averaged 16 into 1.
    let (width, height) = (16_384_u32, 256_u32);
    let whole = (width * height * 3) as usize;
    for interlaced in [false, true] {
        let source = png_of(width, height, interlaced, pixel);
        let (picture, taken) = peak(|| PresentationTexture::decode(&source, "image/png"));
        assert_eq!(picture.unwrap().size(), [1_024, 16]);
        assert!(
            taken < whole / 4,
            "interlaced {interlaced}: decoding took {taken} bytes, the whole picture is {whole}"
        );
    }
}
