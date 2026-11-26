//! A raster image under the model — an orthophoto, a scanned map — as an underlay
//! (Měchenice MECH-10(a)). Its file is a blob of the document's container, kept as it was
//! read; the underlay holds where its pixels lie in the underlay's own coordinates, as a
//! world file (`.pgw`, `.jgw`, `.wld`) beside it says: so an orthophoto in S-JTSK lies on
//! the cadastral drawing in the same coordinates, moved near the origin by the same offset
//! ([`super::UnderlayLines::move_near_origin`]). Its placement, view and tag are the
//! underlay's, as for lines; it has no layers.

use std::fmt;

use super::UNDERLAY_EXTENT_MM;
use crate::tolerance::MAX_COORDINATE_MM;

/// The encoding of a raster underlay's file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RasterFormat {
    Png,
    Jpeg,
}

impl RasterFormat {
    /// The format a file of this extension is read as, any case.
    #[must_use]
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.to_ascii_lowercase().as_str() {
            "png" => Some(Self::Png),
            "jpg" | "jpeg" => Some(Self::Jpeg),
            _ => None,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpeg",
        }
    }

    /// The extensions of the world file beside an image of this format, the format's own
    /// first (`ortofoto.pgw` beside `ortofoto.png`), then the generic `.wld`.
    #[must_use]
    pub const fn world_file_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Png => &["pgw", "pngw", "wld"],
            Self::Jpeg => &["jgw", "jpgw", "jpegw", "wld"],
        }
    }
}

/// The most pixels a raster underlay may have: 8 192 × 8 192, a quarter of a gigabyte
/// decoded.
pub const MAX_RASTER_PIXELS: u64 = 8_192 * 8_192;

/// The largest file read as a raster underlay.
pub const MAX_RASTER_FILE_BYTES: u64 = 256 * 1024 * 1024;

/// Where the pixels of a raster underlay lie in its own coordinates: the outer corner of
/// its upper-left pixel, and the steps of one pixel to the right (a column) and one down
/// (a row), in millimetres. A world file's map, with its origin moved from the middle of
/// that pixel to its corner.
#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnderlayRaster {
    /// SHA-256 of the image file, a blob of the document's container.
    pub image_sha256: [u8; 32],
    pub format: RasterFormat,
    pub width_px: u32,
    pub height_px: u32,
    pub origin_mm: [f64; 2],
    pub column_mm: [f64; 2],
    pub row_mm: [f64; 2],
}

impl UnderlayRaster {
    /// The hex name of the container blob that holds its file.
    #[must_use]
    pub fn image_blob(&self) -> String {
        self.image_sha256
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// Its corners in its own coordinates: upper left, upper right, lower right, lower
    /// left, as the image is read.
    #[must_use]
    pub fn corners_mm(&self) -> [[f64; 2]; 4] {
        let [width, height] = [f64::from(self.width_px), f64::from(self.height_px)];
        let at = |columns: f64, rows: f64| {
            [0, 1].map(|axis| {
                self.origin_mm[axis] + self.column_mm[axis] * columns + self.row_mm[axis] * rows
            })
        };
        [
            at(0.0, 0.0),
            at(width, 0.0),
            at(width, height),
            at(0.0, height),
        ]
    }

    /// How long one pixel is across and down, in millimetres.
    #[must_use]
    pub fn pixel_mm(&self) -> [f64; 2] {
        [self.column_mm, self.row_mm].map(|[x, y]| x.hypot(y))
    }

    /// Whether a document can keep it: pixels within [`MAX_RASTER_PIXELS`], a map that does
    /// not fold the image flat, and corners within the extent an underlay keeps.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        let pixels = u64::from(self.width_px) * u64::from(self.height_px);
        let numbers = self
            .origin_mm
            .iter()
            .chain(&self.column_mm)
            .chain(&self.row_mm);
        let area = self.column_mm[0] * self.row_mm[1] - self.column_mm[1] * self.row_mm[0];
        (1..=MAX_RASTER_PIXELS).contains(&pixels)
            && numbers.clone().all(|value| value.is_finite())
            && area.abs() > 0.0
            && self
                .corners_mm()
                .iter()
                .flatten()
                .all(|value| value.abs() <= UNDERLAY_EXTENT_MM)
    }
}

/// The six numbers of a world file in millimetres: the step of one pixel to the right
/// `[A, D]`, of one down `[B, E]`, and the middle of the upper-left pixel `[C, F]`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorldFile {
    pub column_mm: [f64; 2],
    pub row_mm: [f64; 2],
    pub first_pixel_mm: [f64; 2],
}

impl WorldFile {
    /// Reads the text of a world file, its numbers in units of `unit_mm` millimetres:
    /// six lines A, D, B, E, C, F, blank lines and spaces around them aside.
    ///
    /// # Errors
    ///
    /// [`RasterError::WorldFile`] when it holds other than six finite numbers or its map
    /// folds the image flat.
    pub fn parse(text: &str, unit_mm: f64) -> Result<Self, RasterError> {
        let numbers = text
            .split_whitespace()
            .map(|word| word.parse::<f64>().ok().filter(|value| value.is_finite()))
            .collect::<Option<Vec<_>>>()
            .ok_or(RasterError::WorldFile("a value is not a number"))?;
        let [a, d, b, e, c, f] = numbers[..] else {
            return Err(RasterError::WorldFile("it holds other than six numbers"));
        };
        if (a * e - b * d).abs() == 0.0 {
            return Err(RasterError::WorldFile("its pixels have no area"));
        }
        let scaled = |pair: [f64; 2]| pair.map(|value| value * unit_mm);
        Ok(Self {
            column_mm: scaled([a, d]),
            row_mm: scaled([b, e]),
            first_pixel_mm: scaled([c, f]),
        })
    }
}

/// Why an image is not taken as a raster underlay.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RasterError {
    /// The world file beside it cannot be read as one.
    WorldFile(&'static str),
    /// It has more pixels than [`MAX_RASTER_PIXELS`] or none.
    Pixels { width_px: u32, height_px: u32 },
    /// It reaches further than an underlay keeps around its middle.
    TooLarge,
}

impl fmt::Display for RasterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WorldFile(reason) => write!(formatter, "the world file is not one: {reason}"),
            Self::Pixels {
                width_px,
                height_px,
            } => write!(
                formatter,
                "the image has {width_px} × {height_px} pixels; an underlay takes 1 to \
                 {MAX_RASTER_PIXELS}"
            ),
            Self::TooLarge => write!(
                formatter,
                "the image reaches further than {} km",
                2.0 * UNDERLAY_EXTENT_MM / 1_000_000.0
            ),
        }
    }
}

impl std::error::Error for RasterError {}

/// An image of `width_px` × `height_px` pixels laid out by `world`, or without a world
/// file one pixel a millimetre with its lower-left corner at the origin, and the offset it
/// was moved by: none while it lies within the model, else `geo_origin_mm`, the offset of
/// a drawing of the document in the same grid ([`super::placement::geo_origin`]), when that
/// keeps it within the extent an underlay keeps, else its middle rounded to whole metres —
/// as [`super::UnderlayLines::move_near_origin`] moves lines.
///
/// # Errors
///
/// [`RasterError::Pixels`] or [`RasterError::TooLarge`] for an image a document cannot
/// keep.
pub fn lay_out_raster(
    (image_sha256, format): ([u8; 32], RasterFormat),
    [width_px, height_px]: [u32; 2],
    world: Option<WorldFile>,
    geo_origin_mm: Option<[f64; 2]>,
) -> Result<(UnderlayRaster, [f64; 2]), RasterError> {
    let pixels = u64::from(width_px) * u64::from(height_px);
    if !(1..=MAX_RASTER_PIXELS).contains(&pixels) {
        return Err(RasterError::Pixels {
            width_px,
            height_px,
        });
    }
    let world = world.unwrap_or(WorldFile {
        column_mm: [1.0, 0.0],
        row_mm: [0.0, -1.0],
        first_pixel_mm: [0.5, f64::from(height_px) - 0.5],
    });
    let origin_mm = [0, 1].map(|axis| {
        world.first_pixel_mm[axis] - (world.column_mm[axis] + world.row_mm[axis]) / 2.0
    });
    let mut raster = UnderlayRaster {
        image_sha256,
        format,
        width_px,
        height_px,
        origin_mm,
        column_mm: world.column_mm,
        row_mm: world.row_mm,
    };
    let corners = raster.corners_mm();
    let within = |offset: [f64; 2], extent: f64| {
        corners
            .iter()
            .all(|corner| (0..2).all(|axis| (corner[axis] - offset[axis]).abs() <= extent))
    };
    let middle = [0, 1].map(|axis| {
        let sum: f64 = corners.iter().map(|corner| corner[axis]).sum();
        // `+ 0.0` makes a -0.0 0.0.
        (sum / 4.0 / 1000.0).round() * 1000.0 + 0.0
    });
    let offset_mm = if within([0.0; 2], MAX_COORDINATE_MM) {
        [0.0; 2]
    } else {
        match geo_origin_mm {
            Some(origin)
                if origin.iter().all(|value| value.is_finite())
                    && within(origin, UNDERLAY_EXTENT_MM) =>
            {
                origin
            }
            _ => middle,
        }
    };
    raster.origin_mm = [0, 1].map(|axis| raster.origin_mm[axis] - offset_mm[axis]);
    if !raster.is_valid() {
        return Err(RasterError::TooLarge);
    }
    Ok((raster, offset_mm))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_world_file_puts_the_corner_half_a_pixel_before_the_middle_of_the_first_pixel() {
        let world =
            WorldFile::parse(" 2.5\n0\n0\n-2.5\n\n-499998.75\n-1199996.25\n", 1_000.0).unwrap();
        let (raster, offset) =
            lay_out_raster(([1; 32], RasterFormat::Png), [4, 2], Some(world), None).unwrap();
        assert_eq!(offset, [-499_995_000.0, -1_199_998_000.0]);
        let drawing = raster
            .corners_mm()
            .map(|corner| [corner[0] + offset[0], corner[1] + offset[1]]);
        assert_eq!(
            drawing,
            [
                [-500_000_000.0, -1_199_995_000.0],
                [-499_990_000.0, -1_199_995_000.0],
                [-499_990_000.0, -1_200_000_000.0],
                [-500_000_000.0, -1_200_000_000.0],
            ]
        );
        assert_eq!(raster.pixel_mm(), [2_500.0, 2_500.0]);
    }

    #[test]
    fn an_image_in_the_grid_of_a_drawing_moves_by_its_offset() {
        let world = WorldFile::parse("1\n0\n0\n-1\n-500000.5\n-1200000.5", 1_000.0).unwrap();
        let origin = [-499_000_000.0, -1_201_000_000.0];
        let (_, offset) = lay_out_raster(
            ([1; 32], RasterFormat::Jpeg),
            [8, 8],
            Some(world),
            Some(origin),
        )
        .unwrap();
        assert_eq!(offset, origin);
    }

    #[test]
    fn without_a_world_file_a_pixel_is_a_millimetre_from_the_origin() {
        let (raster, offset) =
            lay_out_raster(([1; 32], RasterFormat::Png), [3, 2], None, None).unwrap();
        assert_eq!(offset, [0.0; 2]);
        assert_eq!(raster.corners_mm()[3], [0.0, 0.0]);
        assert_eq!(raster.corners_mm()[1], [3.0, 2.0]);
    }

    #[test]
    fn a_world_file_of_other_than_six_numbers_or_of_flat_pixels_is_refused() {
        for text in ["1\n0\n0\n-1\n5", "1\n0\n0\n-1\n5\nx", "1\n2\n2\n4\n0\n0"] {
            assert!(
                matches!(WorldFile::parse(text, 1.0), Err(RasterError::WorldFile(_))),
                "{text:?}"
            );
        }
    }

    #[test]
    fn an_image_too_large_or_of_no_pixels_is_refused() {
        assert!(matches!(
            lay_out_raster(([1; 32], RasterFormat::Png), [0, 5], None, None),
            Err(RasterError::Pixels { .. })
        ));
        let world = WorldFile::parse("10\n0\n0\n-10\n0\n0", 1_000.0).unwrap();
        assert_eq!(
            lay_out_raster(([1; 32], RasterFormat::Png), [4_000, 10], Some(world), None),
            Err(RasterError::TooLarge)
        );
    }
}
