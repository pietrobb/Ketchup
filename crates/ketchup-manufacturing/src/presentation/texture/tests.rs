//! Reading pictures a row at a time, within a limit on their pixels (review 2026-11-12
//! R1112-SKP-500).

use super::*;

/// A PNG of `width` × `height` whose pixels end at once: a header and one empty data chunk.
fn header_only(width: u32, height: u32) -> Vec<u8> {
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
    let chunk = |kind: &[u8; 4], data: &[u8]| {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        let start = chunk.len();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        let sum = crc(&chunk[start..]);
        chunk.extend_from_slice(&sum.to_be_bytes());
        chunk
    };
    let mut header = width.to_be_bytes().to_vec();
    header.extend_from_slice(&height.to_be_bytes());
    // 8 bits of RGB, deflate, adaptive filtering, not interlaced.
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(chunk(b"IHDR", &header));
    // An empty zlib stream.
    bytes.extend(chunk(
        b"IDAT",
        &[0x78, 0x9C, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
    ));
    bytes.extend(chunk(b"IEND", &[]));
    bytes
}

fn png(width: u32, height: u32, color: png::ColorType, data: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut encoder = png::Encoder::new(&mut bytes, width, height);
    encoder.set_color(color);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().unwrap();
    writer.write_image_data(data).unwrap();
    writer.finish().unwrap();
    bytes
}

/// A picture of more than 64 MP is refused from its header, before a pixel is read. Before,
/// a PNG of 16 384² (5 MB as a file) was read whole: 1.8 GB and 2 s in the window.
#[test]
fn a_picture_of_too_many_pixels_is_refused_from_its_header() {
    for (width, height) in [(16_384, 16_384), (8_193, 8_193), (16_385, 1), (1, 16_385)] {
        let refused = PresentationTexture::decode(&header_only(width, height), "image/png");
        assert!(
            matches!(
                refused,
                Err(TextureError::TooLarge { width: w, height: h })
                    if w == u64::from(width) && h == u64::from(height)
            ),
            "{width} × {height}: {refused:?}"
        );
    }
    // 8 192² is 64 MP: within the limit, so it is read, and these pixels are missing.
    let read = PresentationTexture::decode(&header_only(8_192, 8_192), "image/png");
    assert!(matches!(read, Err(TextureError::Png(_))), "{read:?}");
}

/// A picture wider than [`MAX_TEXTURE_SIDE`] is averaged down as a whole would be, though
/// only a row of it is held at a time.
#[test]
fn a_large_picture_is_averaged_down_a_row_at_a_time() {
    let (width, height) = (2_049_usize, 5_usize);
    let pixel = |x: usize, y: usize| {
        [
            u8::try_from(x % 256).unwrap(),
            u8::try_from(y * 40).unwrap(),
            u8::try_from((x * 7 + y * 3) % 256).unwrap(),
        ]
    };
    let data = (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| pixel(x, y)))
        .collect::<Vec<_>>();
    let picture =
        PresentationTexture::decode(&png(2_049, 5, png::ColorType::Rgb, &data), "image/png")
            .unwrap();

    // Three pixels a side into one: 683 × 2, the last column and row of fewer.
    assert_eq!(picture.size(), [683, 2]);
    let factor = 3;
    for (index, got) in picture.rgb.iter().enumerate() {
        let (x, y) = (index % 683, index / 683);
        let block = (y * factor..((y + 1) * factor).min(height))
            .flat_map(|row| (x * factor..((x + 1) * factor).min(width)).map(move |col| (col, row)))
            .map(|(col, row)| pixel(col, row))
            .collect::<Vec<_>>();
        let count = u32::try_from(block.len()).unwrap();
        let expected = [0, 1, 2].map(|channel| {
            let total: u32 = block.iter().map(|rgb| u32::from(rgb[channel])).sum();
            u8::try_from(total / count).unwrap()
        });
        assert_eq!(*got, expected, "pixel {x}, {y}");
    }
}

/// Grey pictures, with or without transparency, are read as grey.
#[test]
fn a_grey_picture_is_read_as_grey() {
    let grey = PresentationTexture::decode(
        &png(2, 1, png::ColorType::GrayscaleAlpha, &[10, 255, 200, 0]),
        "image/png",
    )
    .unwrap();
    assert_eq!(grey.rgb, [[10; 3], [200; 3]]);
    assert_eq!(grey.rgba(), [10, 10, 10, 255, 200, 200, 200, 255]);
}
