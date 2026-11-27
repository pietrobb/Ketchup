//! The orthophotos of a printed set cut to what each view shows: an orthophoto of
//! 8 192 × 8 192 px beneath a model of 20 × 20 m went whole into every PDF, though
//! the plan's clip showed a hundredth of it (review 2026-11-27 R1127-IMG-1).

use crate::renderer::underlay::raster::cropped_sheet_image;
use ketchup_manufacturing::project_drawings::DrawingSet;
use ketchup_model::sheet_pdf::{Mark, SheetImage};
use std::sync::Arc;

/// Pixels kept beyond the clip on each side, so the edge of the clip is never
/// the edge of the image.
const MARGIN_PX: f64 = 1.0;

/// Cuts each clipped image of the set's painted pages to the pixels its clip shows,
/// its corners moved with them; views showing the same part share one cut.
pub(super) fn crop_to_clips(set: &mut DrawingSet) {
    let mut cut: Vec<(Arc<SheetImage>, [u32; 4], Arc<SheetImage>)> = Vec::new();
    for sheet in &mut set.sheets {
        for mark in &mut sheet.page.marks {
            let Mark::Image {
                image,
                corners,
                clip: Some(clip),
            } = mark
            else {
                continue;
            };
            let Some((window, shown_corners)) = shown_pixels(image, corners, clip) else {
                continue;
            };
            let part = match cut
                .iter()
                .find(|(whole, at, _)| Arc::ptr_eq(whole, image) && *at == window)
            {
                Some((_, _, part)) => Arc::clone(part),
                None => {
                    let [column, row, end_column, end_row] = window;
                    let Some(part) = cropped_sheet_image(
                        image,
                        [column, row],
                        [end_column - column, end_row - row],
                    ) else {
                        continue;
                    };
                    let part = Arc::new(part);
                    cut.push((Arc::clone(image), window, Arc::clone(&part)));
                    part
                }
            };
            *image = part;
            *corners = shown_corners;
        }
    }
}

/// The pixels of `image` lying at upper-left, upper-right and lower-left `corners`
/// that `clip` shows, as first column, first row, end column and end row, with the
/// corners of that part; `None` when it shows all of them or none.
fn shown_pixels(
    image: &SheetImage,
    corners: &[[f64; 2]; 3],
    [least, greatest]: &[[f64; 2]; 2],
) -> Option<([u32; 4], [[f64; 2]; 3])> {
    let [upper_left, upper_right, lower_left] = *corners;
    let across = [0, 1].map(|axis| upper_right[axis] - upper_left[axis]);
    let down = [0, 1].map(|axis| lower_left[axis] - upper_left[axis]);
    let det = across[0] * down[1] - across[1] * down[0];
    if !det.is_finite() || det.abs() < f64::EPSILON {
        return None;
    }
    let size = [image.width_px, image.height_px].map(f64::from);
    // A page point in pixels across and down the image.
    let pixel = |point: [f64; 2]| {
        let from = [0, 1].map(|axis| point[axis] - upper_left[axis]);
        [
            (from[0] * down[1] - from[1] * down[0]) / det * size[0],
            (across[0] * from[1] - across[1] * from[0]) / det * size[1],
        ]
    };
    let (mut low, mut high) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
    for point in [
        *least,
        [greatest[0], least[1]],
        [least[0], greatest[1]],
        *greatest,
    ] {
        let at = pixel(point);
        for axis in 0..2 {
            low[axis] = low[axis].min(at[axis]);
            high[axis] = high[axis].max(at[axis]);
        }
    }
    let first = [0, 1].map(|axis| (low[axis] - MARGIN_PX).floor().clamp(0.0, size[axis]));
    let end = [0, 1].map(|axis| (high[axis] + MARGIN_PX).ceil().clamp(0.0, size[axis]));
    if (0..2).any(|axis| end[axis] <= first[axis] || !(end[axis] - first[axis]).is_finite())
        || (first == [0.0; 2] && end == size)
    {
        return None;
    }
    let point = |[column, row]: [f64; 2]| {
        let [u, v] = [column / size[0], row / size[1]];
        [0, 1].map(|axis| upper_left[axis] + u * across[axis] + v * down[axis])
    };
    // Whole pixels from 0 to the image's size, so written without a decimal they read back.
    let whole = |value: f64| format!("{:.0}", value + 0.0).parse::<u32>().ok();
    let window = [
        whole(first[0])?,
        whole(first[1])?,
        whole(end[0])?,
        whole(end[1])?,
    ];
    Some((
        window,
        [
            point(first),
            point([end[0], first[1]]),
            point([first[0], end[1]]),
        ],
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 10 × 4 px image, each pixel's red its column and green its row.
    fn image() -> SheetImage {
        let rgb = (0..4_u8)
            .flat_map(|row| (0..10_u8).flat_map(move |column| [column, row, 0]))
            .collect();
        SheetImage {
            width_px: 10,
            height_px: 4,
            rgb,
            jpeg: None,
        }
    }

    /// The image laid 1 mm a pixel from (100, 50), rotated a quarter turn: across
    /// runs down the page and down runs left, so the crop has to invert the corners.
    #[test]
    fn a_clip_keeps_the_pixels_it_shows_and_their_place() {
        let corners = [[100.0, 50.0], [100.0, 60.0], [96.0, 50.0]];
        // Within columns 3..5 (page y 53..55) and row 1 (page x 98..99).
        let clip = [[98.2, 53.2], [98.8, 54.8]];
        let (window, shown) = shown_pixels(&image(), &corners, &clip).unwrap();
        assert_eq!(window, [2, 0, 6, 3], "a pixel of margin, inside the image");
        let wanted = [[100.0, 52.0], [100.0, 56.0], [97.0, 52.0]];
        for (at, wanted) in shown.iter().zip(wanted) {
            assert!(
                (0..2).all(|axis| (at[axis] - wanted[axis]).abs() < 1e-9),
                "{shown:?}"
            );
        }
        let part = cropped_sheet_image(&image(), [2, 0], [4, 3]).unwrap();
        assert_eq!([part.width_px, part.height_px], [4, 3]);
        assert_eq!(&part.rgb[..3], &[2, 0, 0]);
        assert_eq!(&part.rgb[part.rgb.len() - 3..], &[5, 2, 0]);
        assert!(part.jpeg.is_some());
    }

    #[test]
    fn a_clip_showing_the_whole_image_or_none_of_it_leaves_it() {
        let corners = [[0.0, 0.0], [10.0, 0.0], [0.0, 4.0]];
        assert_eq!(
            shown_pixels(&image(), &corners, &[[-1.0, -1.0], [11.0, 5.0]]),
            None
        );
        assert_eq!(
            shown_pixels(&image(), &corners, &[[20.0, 20.0], [30.0, 30.0]]),
            None
        );
    }
}
