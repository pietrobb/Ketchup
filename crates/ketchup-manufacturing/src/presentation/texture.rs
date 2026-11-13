//! The pictures textured faces show in a presentation image (U4e): a SketchUp or glTF texture,
//! decoded once, averaged down to at most [`MAX_TEXTURE_SIDE`] pixels a side and read with
//! repetition and bilinear smoothing.

use std::fmt;

use super::raster::{index, real};

/// Longest side of a picture as the renderer keeps it; a larger one is averaged down.
pub const MAX_TEXTURE_SIDE: usize = 1024;
/// Longest side of a picture the renderer reads at all.
pub const MAX_SOURCE_SIDE: usize = 16_384;
/// Most pixels of a picture the renderer reads at all (64 MP): a picture is read a row at a
/// time, but its size, not its file, decides how long that takes (review 2026-11-12
/// R1112-SKP-500: a 5 MB PNG of 16 384² took 1.8 GB and 2 s).
pub const MAX_SOURCE_PIXELS: usize = 64 << 20;

/// A decoded picture: sRGB pixels, rows from the top. Its transparency is not kept.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationTexture {
    width: usize,
    height: usize,
    rgb: Vec<[u8; 3]>,
}

/// Why a picture cannot be read.
#[derive(Debug)]
pub enum TextureError {
    /// The document has no picture under the name a face gives.
    Missing,
    /// The picture is neither `image/png` nor `image/jpeg`.
    MediaType(String),
    /// The picture is more than [`MAX_SOURCE_SIDE`] pixels on a side or has more than
    /// [`MAX_SOURCE_PIXELS`].
    TooLarge {
        width: u64,
        height: u64,
    },
    /// The picture has no pixels or fewer than its size says.
    Empty,
    Png(png::DecodingError),
    Jpeg(zune_jpeg::errors::DecodeErrors),
}

impl fmt::Display for TextureError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => write!(formatter, "the document has no such picture"),
            Self::MediaType(mime_type) => {
                write!(formatter, "{mime_type} is not image/png or image/jpeg")
            }
            Self::TooLarge { width, height } => write!(
                formatter,
                "{width} × {height} px is more than {MAX_SOURCE_SIDE} px per side or {} MP",
                MAX_SOURCE_PIXELS >> 20
            ),
            Self::Empty => write!(formatter, "the picture has no pixels"),
            Self::Png(error) => write!(formatter, "the PNG cannot be read: {error}"),
            Self::Jpeg(error) => write!(formatter, "the JPEG cannot be read: {error}"),
        }
    }
}

impl std::error::Error for TextureError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Png(error) => Some(error),
            Self::Jpeg(error) => Some(error),
            _ => None,
        }
    }
}

impl PresentationTexture {
    /// The PNG or JPEG `bytes` as `mime_type` names them, unless they cannot be read or are
    /// more than [`MAX_SOURCE_SIDE`] pixels on a side or [`MAX_SOURCE_PIXELS`] in all.
    pub fn decode(bytes: &[u8], mime_type: &str) -> Result<Self, TextureError> {
        match mime_type {
            "image/png" => decode_png(bytes),
            "image/jpeg" => decode_jpeg(bytes),
            _ => Err(TextureError::MediaType(mime_type.to_owned())),
        }
    }

    /// Width and height in pixels, at most [`MAX_TEXTURE_SIDE`] each.
    #[must_use]
    pub fn size(&self) -> [usize; 2] {
        [self.width, self.height]
    }

    /// The pixels as opaque sRGB RGBA bytes, rows from the top.
    #[must_use]
    pub fn rgba(&self) -> Vec<u8> {
        self.rgb
            .iter()
            .flat_map(|[r, g, b]| [*r, *g, *b, u8::MAX])
            .collect()
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
                .map_or([0.0; 3], |pixel| pixel.map(f64::from))
        };
        let [a, b, c, d] = [pixel(x0, y0), pixel(x1, y0), pixel(x0, y1), pixel(x1, y1)];
        [0, 1, 2].map(|channel| {
            let top = a[channel] + (b[channel] - a[channel]) * fx;
            let bottom = c[channel] + (d[channel] - c[channel]) * fx;
            (top + (bottom - top) * fy) / 255.0
        })
    }
}

/// The size of a picture whose header says `width` × `height`, unless it is empty or too
/// large to read.
fn checked_size(width: u64, height: u64) -> Result<(usize, usize), TextureError> {
    if width == 0 || height == 0 {
        return Err(TextureError::Empty);
    }
    let fits = |side: u64| {
        usize::try_from(side)
            .ok()
            .filter(|side| *side <= MAX_SOURCE_SIDE)
    };
    match (fits(width), fits(height)) {
        (Some(columns), Some(rows))
            if columns
                .checked_mul(rows)
                .is_some_and(|pixels| pixels <= MAX_SOURCE_PIXELS) =>
        {
            Ok((columns, rows))
        }
        _ => Err(TextureError::TooLarge { width, height }),
    }
}

/// The colour of a pixel of grey, grey and alpha, RGB or RGBA.
fn rgb_of(pixel: &[u8]) -> [u8; 3] {
    match pixel {
        [grey] | [grey, _] => [*grey; 3],
        [r, g, b, ..] => [*r, *g, *b],
        [] => [0; 3],
    }
}

/// Averages a picture fed a row at a time from the top down by a whole factor to fit
/// [`MAX_TEXTURE_SIDE`], keeping only one row of sums besides the result: a PNG is never
/// held whole (review 2026-11-12 R1112-SKP-500; an interlaced one, see
/// [`reduce_interlaced`]). A JPEG is: its decoder gives no rows before the last, so one of
/// [`MAX_SOURCE_PIXELS`] takes 3 bytes a pixel (192 MiB) while it is read, on the thread
/// that reads pictures one at a time.
struct Reducer {
    width: usize,
    height: usize,
    factor: usize,
    rows: usize,
    short: bool,
    /// Red, green, blue and count of each pixel of the result row being summed.
    sums: Vec<[u64; 4]>,
    small: Vec<[u8; 3]>,
}

impl Reducer {
    /// For a picture of `width` × `height`, both from [`checked_size`].
    fn new(width: usize, height: usize) -> Self {
        let factor = width.max(height).div_ceil(MAX_TEXTURE_SIDE).max(1);
        let small_width = width.div_ceil(factor);
        Self {
            width,
            height,
            factor,
            rows: 0,
            short: false,
            sums: vec![[0; 4]; small_width],
            small: Vec::with_capacity(small_width * height.div_ceil(factor)),
        }
    }

    /// The next row's pixels, `channels` bytes each: grey, grey and alpha, RGB or RGBA.
    fn push(&mut self, row: &[u8], channels: usize) {
        if self.rows == self.height {
            return;
        }
        let mut columns = 0;
        for (column, pixel) in row
            .chunks_exact(channels.max(1))
            .take(self.width)
            .enumerate()
        {
            if let Some(sum) = self.sums.get_mut(column / self.factor) {
                for (total, value) in sum.iter_mut().zip(rgb_of(pixel)) {
                    *total += u64::from(value);
                }
                sum[3] += 1;
            }
            columns += 1;
        }
        self.short |= columns < self.width;
        self.rows += 1;
        if self.rows.is_multiple_of(self.factor) || self.rows == self.height {
            for sum in &mut self.sums {
                let [red, green, blue, count] = *sum;
                self.small.push(
                    [red, green, blue]
                        .map(|total| u8::try_from(total / count.max(1)).unwrap_or(u8::MAX)),
                );
                *sum = [0; 4];
            }
        }
    }

    fn finish(self) -> Result<PresentationTexture, TextureError> {
        if self.short || self.rows < self.height {
            return Err(TextureError::Empty);
        }
        Ok(PresentationTexture {
            width: self.sums.len(),
            height: self.small.len() / self.sums.len().max(1),
            rgb: self.small,
        })
    }
}

fn decode_png(bytes: &[u8]) -> Result<PresentationTexture, TextureError> {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(TextureError::Png)?;
    let (width, height) = reader.info().size();
    let (width, height) = checked_size(u64::from(width), u64::from(height))?;
    let channels = reader.output_color_type().0.samples();
    if reader.info().interlaced {
        return reduce_interlaced(&mut reader, width, height, channels);
    }
    let mut reducer = Reducer::new(width, height);
    while let Some(row) = reader.next_row().map_err(TextureError::Png)? {
        reducer.push(row.data(), channels);
    }
    reducer.finish()
}

/// The first column, first row, column step and row step of each Adam7 pass.
const ADAM7_PASSES: [(usize, usize, usize, usize); 7] = [
    (0, 0, 8, 8),
    (4, 0, 8, 8),
    (0, 4, 4, 8),
    (2, 0, 4, 4),
    (0, 2, 2, 4),
    (1, 0, 2, 2),
    (0, 1, 1, 2),
];

/// Averages an interlaced (Adam7) PNG of `width` × `height` into the blocks [`Reducer`]
/// averages. Its rows come in seven passes, each of every so many pixels of every so many
/// rows, so the sums of every block of the result are held rather than the picture
/// (review 2026-11-13 R1113 P3: it was read whole, 200–400 MB at 8 192²).
fn reduce_interlaced(
    reader: &mut png::Reader<std::io::Cursor<&[u8]>>,
    width: usize,
    height: usize,
    channels: usize,
) -> Result<PresentationTexture, TextureError> {
    let factor = width.max(height).div_ceil(MAX_TEXTURE_SIDE).max(1);
    let columns = width.div_ceil(factor);
    // Red, green, blue and count of each pixel of the result: at most 255 × 16² each.
    let mut sums = vec![[0_u32; 4]; columns * height.div_ceil(factor)];
    let full_width = reader.info().width;
    // The passes that hold pixels of the picture, in the order its rows come.
    let mut passes = (1_u8..)
        .zip(ADAM7_PASSES)
        .filter(|(_, (x, y, _, _))| *x < width && *y < height);
    let mut pass = passes.next();
    // The row within its pass, counted as the PNG counts it and as an index.
    let (mut line, mut line_number, mut read) = (0, 0_u32, 0_usize);
    while let Some(row) = reader.next_interlaced_row().map_err(TextureError::Png)? {
        while let Some((_, (_, first_row, _, row_step))) = pass
            && first_row + line * row_step >= height
        {
            pass = passes.next();
            (line, line_number) = (0, 0);
        }
        let Some((number, (first_column, first_row, column_step, row_step))) = pass else {
            return Err(TextureError::Empty);
        };
        let expected = png::Adam7Info::new(number, line_number, full_width);
        if !matches!(row.interlace(), png::InterlaceInfo::Adam7(info) if *info == expected) {
            return Err(TextureError::Empty);
        }
        let y = first_row + line * row_step;
        for (index, pixel) in row.data().chunks_exact(channels.max(1)).enumerate() {
            let x = first_column + index * column_step;
            if x >= width {
                break;
            }
            if let Some(sum) = sums.get_mut(y / factor * columns + x / factor) {
                for (total, value) in sum.iter_mut().zip(rgb_of(pixel)) {
                    *total += u32::from(value);
                }
                sum[3] += 1;
                read += 1;
            }
        }
        line += 1;
        line_number += 1;
    }
    if read != width * height {
        return Err(TextureError::Empty);
    }
    Ok(PresentationTexture {
        width: columns,
        height: sums.len() / columns.max(1),
        rgb: sums
            .iter()
            .map(|[red, green, blue, count]| {
                [red, green, blue]
                    .map(|total| u8::try_from(total / (*count).max(1)).unwrap_or(u8::MAX))
            })
            .collect(),
    })
}

fn decode_jpeg(bytes: &[u8]) -> Result<PresentationTexture, TextureError> {
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
    decoder.decode_headers().map_err(TextureError::Jpeg)?;
    let (width, height) = decoder.dimensions().ok_or(TextureError::Empty)?;
    let (width, height) = checked_size(
        u64::try_from(width).unwrap_or(u64::MAX),
        u64::try_from(height).unwrap_or(u64::MAX),
    )?;
    let pixels = decoder.decode().map_err(TextureError::Jpeg)?;
    let mut reducer = Reducer::new(width, height);
    for row in pixels.chunks(width * 3) {
        reducer.push(row, 3);
    }
    reducer.finish()
}

#[cfg(test)]
mod tests;
