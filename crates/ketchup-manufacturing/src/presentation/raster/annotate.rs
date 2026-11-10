//! Dimensions and labels drawn over the rendered samples, before they are averaged into pixels,
//! so their lines and letters are smoothed like the model's edges.

use super::{Camera, index, narrow, real};
use crate::presentation::{PresentationError, PresentationLabel};
use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};
use ketchup_geometry::linalg::Vec3;

const INK: [f64; 3] = [0.13, 0.13, 0.15];
/// Light box behind text, so it reads over any part or background.
const HALO: [f64; 3] = [1.0, 1.0, 1.0];
const HALO_ALPHA: f64 = 0.85;
/// Text height in output pixels as a share of the image height, and its smallest size.
const TEXT_SHARE: f64 = 0.026;
const TEXT_MIN_PX: f64 = 11.0;
/// Distance of a dimension line from the model, as a share of its diagonal.
const DIMENSION_GAP: f64 = 0.10;
/// Extension lines start this share of the diagonal off the model and run past the dimension line.
const EXTENSION_CLEAR: f64 = 0.012;
const EXTENSION_PAST: f64 = 0.025;

/// One overall extent of the model: its two corners, the matching ends of the dimension line
/// and the direction the line is moved off the model.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Dimension {
    pub(super) anchors: [Vec3; 2],
    pub(super) ends: [Vec3; 2],
    pub(super) outward: Vec3,
    pub(super) length_mm: f64,
}

/// Width (x), depth (y) and height (z) of the box `low`–`high`, each along the bottom edges or
/// vertical edge facing the viewer at `view_from`. Extents without length are left out.
pub(super) fn dimensions(low: Vec3, high: Vec3, view_from: Vec3) -> Vec<Dimension> {
    let gap = low.distance(high) * DIMENSION_GAP;
    let (near_x, out_x) = if view_from.x >= 0.0 {
        (high.x, 1.0)
    } else {
        (low.x, -1.0)
    };
    let (near_y, out_y) = if view_from.y >= 0.0 {
        (high.y, 1.0)
    } else {
        (low.y, -1.0)
    };
    let far_x = if out_x > 0.0 { low.x } else { high.x };
    let extent = |anchors: [Vec3; 2], outward: Vec3| Dimension {
        anchors,
        ends: anchors.map(|anchor| anchor + outward * gap),
        outward,
        length_mm: anchors[0].distance(anchors[1]),
    };
    [
        extent(
            [
                Vec3::new(low.x, near_y, low.z),
                Vec3::new(high.x, near_y, low.z),
            ],
            Vec3::new(0.0, out_y, 0.0),
        ),
        extent(
            [
                Vec3::new(near_x, low.y, low.z),
                Vec3::new(near_x, high.y, low.z),
            ],
            Vec3::new(out_x, 0.0, 0.0),
        ),
        extent(
            [
                Vec3::new(far_x, near_y, low.z),
                Vec3::new(far_x, near_y, high.z),
            ],
            Vec3::new(-out_x, 0.0, 0.0),
        ),
    ]
    .into_iter()
    .filter(|dimension| dimension.length_mm > gap * ketchup_tolerance::APPROXIMATION)
    .collect()
}

impl Dimension {
    /// Points the camera must frame for this dimension to show whole.
    pub(super) fn reach(&self, diagonal: f64) -> [Vec3; 2] {
        self.ends
            .map(|end| end + self.outward * (diagonal * EXTENSION_PAST))
    }
}

/// A length as a dimension shows it: whole millimetres, or tenths when the length has them.
pub(super) fn dimension_text(length_mm: f64) -> String {
    let tenths = (length_mm * 10.0).round();
    if tenths % 10.0 == 0.0 {
        format!("{:.0} mm", tenths / 10.0)
    } else {
        format!("{:.1} mm", tenths / 10.0)
    }
}

/// Premultiplied RGBA samples the annotations are drawn into.
pub(super) struct Sheet<'a> {
    pub(super) colour: &'a mut [[f32; 4]],
    pub(super) width: usize,
    pub(super) height: usize,
    /// Samples per output pixel along each axis.
    pub(super) factor: usize,
}

impl Sheet<'_> {
    /// Covers sample `(x, y)` by `cover` (0–1) of the opaque colour `rgb`.
    fn over(&mut self, x: usize, y: usize, rgb: [f64; 3], cover: f64) {
        let Some(sample) = self.colour.get_mut(y * self.width + x) else {
            return;
        };
        let keep = 1.0 - cover;
        let below = *sample;
        *sample = [
            narrow(rgb[0] * cover + f64::from(below[0]) * keep),
            narrow(rgb[1] * cover + f64::from(below[1]) * keep),
            narrow(rgb[2] * cover + f64::from(below[2]) * keep),
            narrow(cover + f64::from(below[3]) * keep),
        ];
    }

    /// Sample columns or rows `from`–`to` (sample space) clamped to `count`.
    fn span(from: f64, to: f64, count: usize) -> Option<std::ops::RangeInclusive<usize>> {
        let last = real(count) - 1.0;
        let (from, to) = (from.floor().max(0.0), to.ceil().min(last));
        (from <= to).then(|| index(from)..=index(to))
    }

    /// A line from `a` to `b` (sample space) `half_width` samples either side, with round ends.
    fn line(&mut self, a: Vec3, b: Vec3, half_width: f64, rgb: [f64; 3]) {
        let reach = half_width + 1.0;
        let (Some(columns), Some(rows)) = (
            Self::span(a.x.min(b.x) - reach, a.x.max(b.x) + reach, self.width),
            Self::span(a.y.min(b.y) - reach, a.y.max(b.y) + reach, self.height),
        ) else {
            return;
        };
        let along = b - a;
        let length_squared = along.dot(along);
        for y in rows {
            for x in columns.clone() {
                let centre = Vec3::new(real(x) + 0.5, real(y) + 0.5, 0.0);
                let share = if length_squared > 0.0 {
                    ((centre - a).dot(along) / length_squared).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let distance = centre.distance(a + along * share);
                let cover = (half_width + 0.5 - distance).clamp(0.0, 1.0);
                if cover > 0.0 {
                    self.over(x, y, rgb, cover);
                }
            }
        }
    }

    fn rectangle(&mut self, low: Vec3, high: Vec3, rgb: [f64; 3], alpha: f64) {
        let (Some(columns), Some(rows)) = (
            Self::span(low.x, high.x - 1.0, self.width),
            Self::span(low.y, high.y - 1.0, self.height),
        ) else {
            return;
        };
        for y in rows {
            for x in columns.clone() {
                self.over(x, y, rgb, alpha);
            }
        }
    }
}

/// Draws the dimensions and labels; `camera` maps world points to sample space.
pub(super) struct Annotator<'a> {
    camera: &'a Camera,
    font: FontRef<'static>,
    /// Text size and line half-width in samples.
    text: f64,
    stroke: f64,
}

impl<'a> Annotator<'a> {
    pub(super) fn new(camera: &'a Camera, sheet: &Sheet<'_>) -> Result<Self, PresentationError> {
        let font = FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT)
            .map_err(|_unreadable: ab_glyph::InvalidFont| PresentationError::Font)?;
        let factor = real(sheet.factor);
        let output_height = real(sheet.height) / factor;
        Ok(Self {
            camera,
            font,
            text: (output_height * TEXT_SHARE).max(TEXT_MIN_PX) * factor,
            stroke: (0.6 * factor).max(0.5),
        })
    }

    pub(super) fn dimension(&self, sheet: &mut Sheet<'_>, dimension: &Dimension, diagonal: f64) {
        let project = |point: Vec3| self.camera.project(point);
        let clear = dimension.outward * (diagonal * EXTENSION_CLEAR);
        let past = dimension.outward * (diagonal * EXTENSION_PAST);
        for (anchor, end) in dimension.anchors.into_iter().zip(dimension.ends) {
            if let (Some(from), Some(to)) = (project(anchor + clear), project(end + past)) {
                sheet.line(flat(from), flat(to), self.stroke * 0.7, INK);
            }
        }
        let (Some(a), Some(b)) = (project(dimension.ends[0]), project(dimension.ends[1])) else {
            return;
        };
        let (a, b) = (flat(a), flat(b));
        sheet.line(a, b, self.stroke, INK);
        // Oblique ticks at both ends, as architectural drawings mark them.
        if let Some(direction) = (b - a).normalized() {
            let across = Vec3::new(-direction.y, direction.x, 0.0);
            if let Some(tick) = (direction + across).normalized() {
                let tick = tick * (self.text * 0.35);
                for end in [a, b] {
                    sheet.line(end - tick, end + tick, self.stroke * 1.4, INK);
                }
            }
        }
        self.text_at(
            sheet,
            &dimension_text(dimension.length_mm),
            (a + b) * 0.5,
            0.5,
        );
    }

    pub(super) fn label(&self, sheet: &mut Sheet<'_>, label: &PresentationLabel) {
        let Some(at) = self.camera.project(Vec3::from(label.at_mm)) else {
            return;
        };
        let at = flat(at);
        let bend = at + Vec3::new(self.text * 1.1, -self.text * 1.1, 0.0);
        let end = bend + Vec3::new(self.text * 0.5, 0.0, 0.0);
        sheet.line(at, at, self.text * 0.12, INK);
        sheet.line(at, bend, self.stroke, INK);
        sheet.line(bend, end, self.stroke, INK);
        self.text_at(
            sheet,
            &label.text,
            end + Vec3::new(self.text * 0.2, 0.0, 0.0),
            0.0,
        );
    }

    /// `text` on a halo, vertically centred on `anchor`; `align` 0 starts it at the anchor,
    /// 0.5 centres it there.
    fn text_at(&self, sheet: &mut Sheet<'_>, text: &str, anchor: Vec3, align: f64) {
        let scale = PxScale::from(narrow(self.text));
        let scaled = self.font.as_scaled(scale);
        let mut advances = Vec::with_capacity(text.len());
        let mut caret = 0.0_f64;
        let mut previous = None;
        for letter in text.chars() {
            let glyph = self.font.glyph_id(letter);
            if let Some(previous) = previous {
                caret += f64::from(scaled.kern(previous, glyph));
            }
            advances.push((glyph, caret));
            caret += f64::from(scaled.h_advance(glyph));
            previous = Some(glyph);
        }
        let ascent = f64::from(scaled.ascent());
        let descent = f64::from(scaled.descent());
        let left = anchor.x - caret * align;
        let baseline = anchor.y + (ascent + descent) * 0.5;
        let pad = self.text * 0.25;
        sheet.rectangle(
            Vec3::new(left - pad, baseline - ascent - pad * 0.5, 0.0),
            Vec3::new(left + caret + pad, baseline - descent + pad * 0.5, 0.0),
            HALO,
            HALO_ALPHA,
        );
        for (glyph, offset) in advances {
            let positioned = glyph
                .with_scale_and_position(scale, point(narrow(left + offset), narrow(baseline)));
            let Some(outline) = self.font.outline_glyph(positioned) else {
                continue;
            };
            let bounds = outline.px_bounds();
            let (origin_x, origin_y) = (f64::from(bounds.min.x), f64::from(bounds.min.y));
            outline.draw(|x, y, cover| {
                let (x, y) = (origin_x + f64::from(x), origin_y + f64::from(y));
                if x >= 0.0 && y >= 0.0 && x < real(sheet.width) && y < real(sheet.height) {
                    sheet.over(index(x), index(y), INK, f64::from(cover).clamp(0.0, 1.0));
                }
            });
        }
    }
}

/// A projected point without its depth.
fn flat(projected: Vec3) -> Vec3 {
    Vec3::new(projected.x, projected.y, 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dimension_reads_in_whole_millimetres_or_tenths() {
        assert_eq!(dimension_text(1200.0), "1200 mm");
        assert_eq!(dimension_text(1199.996), "1200 mm");
        assert_eq!(dimension_text(12.5), "12.5 mm");
        assert_eq!(dimension_text(0.04), "0 mm");
    }

    #[test]
    fn the_three_extents_face_the_viewer_and_skip_a_flat_axis() {
        let (low, high) = (
            Vec3::new(-200.0, -150.0, 0.0),
            Vec3::new(200.0, 150.0, 140.0),
        );
        let found = dimensions(low, high, Vec3::new(-1.0, -1.6, 1.1));
        let lengths = found.iter().map(|d| d.length_mm).collect::<Vec<_>>();
        assert_eq!(lengths, vec![400.0, 300.0, 140.0]);
        // Seen from -x, -y the width runs along the front (y = -150) and moves further to -y.
        assert_eq!(found[0].anchors[0].y, -150.0);
        assert!(found[0].ends[0].y < -150.0);
        // The depth along the left side (x = -200), the height up the front right corner.
        assert_eq!(found[1].anchors[0].x, -200.0);
        assert_eq!(found[2].anchors[0].x, 200.0);
        assert!(found[2].ends[0].x > 200.0);
        let flat = dimensions(
            low,
            Vec3::new(200.0, 150.0, 0.0),
            Vec3::new(-1.0, -1.6, 1.1),
        );
        assert_eq!(flat.len(), 2);
    }
}
