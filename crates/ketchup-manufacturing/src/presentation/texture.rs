//! The pictures textured faces show in a presentation image (U4e): a SketchUp or glTF texture,
//! decoded once, averaged down to at most [`MAX_TEXTURE_SIDE`] pixels a side and read with
//! repetition and bilinear smoothing.

use super::raster::{index, real};

/// Longest side of a picture as the renderer keeps it; a larger one is averaged down.
pub const MAX_TEXTURE_SIDE: usize = 1024;
/// Longest side of a picture the renderer reads at all.
pub const MAX_SOURCE_SIDE: usize = 16_384;

/// A decoded picture: sRGB pixels, rows from the top. Its transparency is not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationTexture {
    width: usize,
    height: usize,
    rgb: Vec<[u8; 3]>,
}

impl PresentationTexture {
    /// The PNG or JPEG `bytes` as `mime_type` names them; `None` when they cannot be read or
    /// are more than [`MAX_SOURCE_SIDE`] pixels on a side.
    #[must_use]
    pub fn decode(bytes: &[u8], mime_type: &str) -> Option<Self> {
        let (width, height, rgb) = match mime_type {
            "image/png" => decode_png(bytes)?,
            "image/jpeg" => decode_jpeg(bytes)?,
            _ => return None,
        };
        Self::reduced(width, height, &rgb)
    }

    /// `rgb` of `width` × `height`, averaged down by a whole factor to fit
    /// [`MAX_TEXTURE_SIDE`].
    fn reduced(width: usize, height: usize, rgb: &[[u8; 3]]) -> Option<Self> {
        if width == 0 || height == 0 || rgb.len() != width.checked_mul(height)? {
            return None;
        }
        let factor = width.max(height).div_ceil(MAX_TEXTURE_SIDE);
        let (small_width, small_height) = (width.div_ceil(factor), height.div_ceil(factor));
        let mut small = Vec::with_capacity(small_width * small_height);
        for y in 0..small_height {
            for x in 0..small_width {
                let (mut sum, mut count) = ([0_u32; 3], 0_u32);
                for row in y * factor..((y + 1) * factor).min(height) {
                    let columns = x * factor..((x + 1) * factor).min(width);
                    let pixels = rgb.get(row * width + columns.start..row * width + columns.end);
                    for pixel in pixels.unwrap_or_default() {
                        for (total, value) in sum.iter_mut().zip(pixel) {
                            *total += u32::from(*value);
                        }
                        count += 1;
                    }
                }
                small.push(sum.map(|total| u8::try_from(total / count.max(1)).unwrap_or(u8::MAX)));
            }
        }
        Some(Self {
            width: small_width,
            height: small_height,
            rgb: small,
        })
    }

    /// The colour, 0–1 per channel, at texture coordinates `uv` (glTF: `u` right, `v` down,
    /// one picture per unit, repeating), blended between the four nearest pixels.
    pub(super) fn sample(&self, uv: [f64; 2]) -> [f64; 3] {
        let axis = |coordinate: f64, size: usize| {
            let at = (coordinate - coordinate.floor()) * real(size) - 0.5;
            let low = at.floor();
            let first = if low < 0.0 {
                size - 1
            } else {
                index(low).min(size - 1)
            };
            (first, (first + 1) % size, at - low)
        };
        let (x0, x1, fx) = axis(uv[0], self.width);
        let (y0, y1, fy) = axis(uv[1], self.height);
        let pixel = |x: usize, y: usize| {
            self.rgb
                .get(y * self.width + x)
                .copied()
                .unwrap_or_default()
                .map(f64::from)
        };
        let [a, b, c, d] = [pixel(x0, y0), pixel(x1, y0), pixel(x0, y1), pixel(x1, y1)];
        [0, 1, 2].map(|channel| {
            let top = a[channel] + (b[channel] - a[channel]) * fx;
            let bottom = c[channel] + (d[channel] - c[channel]) * fx;
            (top + (bottom - top) * fy) / 255.0
        })
    }
}

fn decode_png(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 3]>)> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().ok()?;
    let (width, height) = reader.info().size();
    let (width, height) = (usize::try_from(width).ok()?, usize::try_from(height).ok()?);
    if width > MAX_SOURCE_SIDE || height > MAX_SOURCE_SIDE {
        return None;
    }
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let frame = reader.next_frame(&mut buffer).ok()?;
    let channels = frame.color_type.samples();
    let rgb = buffer
        .get(..frame.buffer_size())?
        .chunks_exact(channels)
        .map(|pixel| match pixel {
            [grey] | [grey, _] => [*grey; 3],
            [r, g, b, ..] => [*r, *g, *b],
            [] => [0; 3],
        })
        .collect();
    Some((width, height, rgb))
}

fn decode_jpeg(bytes: &[u8]) -> Option<(usize, usize, Vec<[u8; 3]>)> {
    use zune_jpeg::zune_core::{
        bytestream::ZCursor, colorspace::ColorSpace, options::DecoderOptions,
    };
    // The portable decoding paths, so every machine reads the same pixels.
    let options = DecoderOptions::default()
        .set_use_unsafe(false)
        .set_max_width(MAX_SOURCE_SIDE)
        .set_max_height(MAX_SOURCE_SIDE)
        .jpeg_set_out_colorspace(ColorSpace::RGB);
    let mut decoder = zune_jpeg::JpegDecoder::new_with_options(ZCursor::new(bytes), options);
    let pixels = decoder.decode().ok()?;
    let (width, height) = decoder.dimensions()?;
    let rgb = pixels
        .chunks_exact(3)
        .map(|pixel| match pixel {
            [r, g, b] => [*r, *g, *b],
            _ => [0; 3],
        })
        .collect();
    Some((width, height, rgb))
}
