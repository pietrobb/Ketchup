//! Dimensions and labels drawn over the rendered samples, before they are averaged into pixels,
//! so their lines and letters are smoothed like the model's edges.

use super::{Camera, going_on, index, narrow, real};
use crate::presentation::{PresentationError, PresentationLabel};
use ab_glyph::{Font, FontRef, GlyphId, PxScale, ScaleFont, point};
use ketchup_geometry::linalg::Vec3;
use ketchup_model::annotation::{DecimalMark, dimension_text_with};
use std::sync::atomic::AtomicBool;

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

/// A length as a dimension shows it: whole millimetres, or tenths when the length has them,
/// with `mark` before them as the drawings write it (review 2026-11-12 P3).
pub(super) fn dimension_text(length_mm: f64, mark: DecimalMark) -> String {
    format!("{} mm", dimension_text_with(length_mm, mark))
}

/// Premultiplied RGBA samples the annotations are drawn into: the rows `own` of an image
/// `width` × `height` samples.
pub(super) struct Sheet<'a> {
    pub(super) colour: &'a mut [[f32; 4]],
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) own: std::ops::Range<usize>,
}

impl Sheet<'_> {
    /// Covers sample `(x, y)` by `cover` (0–1) of the opaque colour `rgb`.
    fn over(&mut self, x: usize, y: usize, rgb: [f64; 3], cover: f64) {
        let Some(row) = y.checked_sub(self.own.start) else {
            return;
        };
        let Some(sample) = self.colour.get_mut(row * self.width + x) else {
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

    /// Rows `from`–`to` (sample space) of the sheet's own.
    fn rows(&self, from: f64, to: f64) -> Option<std::ops::RangeInclusive<usize>> {
        let rows = Self::span(from, to, self.height)?;
        let (first, last) = (
            (*rows.start()).max(self.own.start),
            (*rows.end()).min(self.own.end.checked_sub(1)?),
        );
        (first <= last).then_some(first..=last)
    }

    /// A line from `a` to `b` (sample space) `half_width` samples either side, with round ends.
    fn line(&mut self, a: Vec3, b: Vec3, half_width: f64, rgb: [f64; 3]) {
        let reach = half_width + 1.0;
        let (Some(columns), Some(rows)) = (
            Self::span(a.x.min(b.x) - reach, a.x.max(b.x) + reach, self.width),
            self.rows(a.y.min(b.y) - reach, a.y.max(b.y) + reach),
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
            self.rows(low.y, high.y - 1.0),
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

/// The built-in typefaces, each letter drawn by the first that has it: text, then symbols,
/// then the emoji of two (review 2026-11-13 P3: emoji were the text typeface's empty box;
/// r95 P3: primes ′ ″, arrows and ∅ were refused in a label and left out of a part's name).
fn typefaces() -> Result<[FontRef<'static>; 4], PresentationError> {
    let typeface = |bytes: &'static [u8]| {
        FontRef::try_from_slice(bytes)
            .map_err(|_unreadable: ab_glyph::InvalidFont| PresentationError::Font)
    };
    Ok([
        typeface(epaint_default_fonts::UBUNTU_LIGHT)?,
        typeface(epaint_default_fonts::HACK_REGULAR)?,
        typeface(epaint_default_fonts::NOTO_EMOJI_REGULAR)?,
        typeface(epaint_default_fonts::EMOJI_ICON)?,
    ])
}

/// `text` as it is drawn: composed (NFC), so a name written decomposed, as macOS writes
/// it, draws as its letters do, and the diameter sign ⌀, which no built-in typeface has,
/// as Ø (review 2026-11-13 r95 P3: "⌀8 dowel" refused the image, and "e" with a combining
/// accent was refused for the accent).
pub(in crate::presentation) fn shown(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfc()
        .map(|letter| if letter == '\u{2300}' { 'Ø' } else { letter })
        .collect()
}

/// A letter that is not drawn: a zero-width space or joiner, a variation selector or a skin
/// tone, which only join or choose the look of the letters around them.
pub(in crate::presentation) fn unseen(letter: char) -> bool {
    matches!(
        letter,
        '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{1F3FB}'..='\u{1F3FF}'
    )
}

/// The typeface and glyph that draw `letter`, `None` when no built-in typeface has it.
fn glyph_of(typefaces: &[FontRef<'static>], letter: char) -> Option<(usize, GlyphId)> {
    typefaces.iter().enumerate().find_map(|(typeface, font)| {
        let glyph = font.glyph_id(letter);
        (glyph.0 != 0).then_some((typeface, glyph))
    })
}

/// Whether a built-in typeface draws `letter`, or it is [`unseen`]; `false` when the typefaces
/// cannot be read, which the image then refuses with [`PresentationError::Font`].
pub(in crate::presentation) fn drawable(letter: char) -> bool {
    unseen(letter) || typefaces().is_ok_and(|typefaces| glyph_of(&typefaces, letter).is_some())
}

/// The first letter of `text` no built-in typeface draws (review 2026-11-13 P3: CJK and
/// Arabic came out as empty boxes).
pub(in crate::presentation) fn undrawable(text: &str) -> Result<Option<char>, PresentationError> {
    let typefaces = typefaces()?;
    Ok(text.chars().find(|letter| {
        !letter.is_whitespace() && !unseen(*letter) && glyph_of(&typefaces, *letter).is_none()
    }))
}

/// Draws the dimensions and labels; `camera` maps world points to sample space.
pub(super) struct Annotator<'a> {
    camera: &'a Camera,
    typefaces: [FontRef<'static>; 4],
    /// Text size and line half-width in samples.
    text: f64,
    stroke: f64,
}

/// A label laid out once for the whole image: its point, the bend and end of its leader, its
/// text as drawn and the side of the point the text is on (1 right, -1 left).
pub(super) struct PlacedLabel {
    at: Vec3,
    bend: Vec3,
    end: Vec3,
    text: String,
    side: f64,
}

impl<'a> Annotator<'a> {
    /// The annotator of an image `height` samples high with `factor` samples per pixel side.
    pub(super) fn new(
        camera: &'a Camera,
        height: usize,
        factor: usize,
    ) -> Result<Self, PresentationError> {
        let typefaces = typefaces()?;
        let factor = real(factor);
        let output_height = real(height) / factor;
        Ok(Self {
            camera,
            typefaces,
            text: (output_height * TEXT_SHARE).max(TEXT_MIN_PX) * factor,
            stroke: (0.6 * factor).max(0.5),
        })
    }

    pub(super) fn dimension(
        &self,
        sheet: &mut Sheet<'_>,
        dimension: &Dimension,
        diagonal: f64,
        mark: DecimalMark,
    ) {
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
            &dimension_text(dimension.length_mm, mark),
            (a + b) * 0.5,
            0.5,
        );
    }

    /// Lays out `labels` once for an image `width` samples wide, seeing a cancel before each
    /// (review 2026-11-13 r95 P3: every band laid them all out again, 200 labels at 4K added
    /// 8 s, and nothing looked at the flag). A label whose point is behind the camera is left
    /// out.
    pub(super) fn place_labels(
        &self,
        labels: &[PresentationLabel],
        width: usize,
        cancelled: &AtomicBool,
    ) -> Result<Vec<PlacedLabel>, PresentationError> {
        let mut boxes = Vec::with_capacity(labels.len());
        let mut placed = Vec::with_capacity(labels.len());
        for label in labels {
            going_on(cancelled)?;
            placed.extend(self.place(label, width, &mut boxes));
        }
        Ok(placed)
    }

    /// Where `label` goes, with a leader from its point to its text. The text goes where it
    /// covers none of `boxes`, those of the texts placed before, and to the left of its point
    /// when it would run off the image on the right (review 2026-11-12 P3: labels drew over one
    /// another and long names ran off the image). Text longer than the room on the wider side
    /// is cut with an ellipsis (review 2026-11-13 P3).
    fn place(
        &self,
        label: &PresentationLabel,
        image_width: usize,
        boxes: &mut Vec<[f64; 4]>,
    ) -> Option<PlacedLabel> {
        let at = flat(self.camera.project(Vec3::from(label.at_mm))?);
        let (rise, reach) = (self.text * 1.1, self.text * 0.7);
        // The text starts `rise + reach` off its point, 0.2 text further, and its halo pads it.
        let room = at.x.max(real(image_width) - at.x) - rise - reach - self.text;
        let text = self.fitted(&shown(&label.text), room);
        let width = self.measure(&text).1 + self.text * 0.5;
        let side = if at.x + rise + reach + width > real(image_width) && at.x - rise - reach > width
        {
            -1.0
        } else {
            1.0
        };
        let start = at.x + side * (rise + reach);
        let (left, right) = if side > 0.0 {
            (start, start + width)
        } else {
            (start - width, start)
        };
        let half = self.text * 0.75;
        let box_at = |y: f64| [left, y - half, right, y + half];
        let overlaps = |[l, t, r, b]: [f64; 4]| {
            boxes
                .iter()
                .any(|[pl, pt, pr, pb]| l < *pr && *pl < r && t < *pb && *pt < b)
        };
        let step = half * 2.0;
        let rows = (0..24_u32).map(|row| at.y - rise - f64::from(row) * step);
        let height = at.y - rise;
        let y = rows
            .clone()
            .find(|y| *y - half >= 0.0 && !overlaps(box_at(*y)))
            .or_else(|| rows.clone().find(|y| !overlaps(box_at(*y))))
            .unwrap_or(height);
        boxes.push(box_at(y));
        let bend = Vec3::new(at.x + side * rise, y, 0.0);
        let end = bend + Vec3::new(side * self.text * 0.5, 0.0, 0.0);
        Some(PlacedLabel {
            at,
            bend,
            end,
            text,
            side,
        })
    }

    /// Draws a label laid out by [`Self::place_labels`] over the sheet's rows.
    pub(super) fn label(&self, sheet: &mut Sheet<'_>, label: &PlacedLabel) {
        let PlacedLabel {
            at,
            bend,
            end,
            text,
            side,
        } = label;
        sheet.line(*at, *at, self.text * 0.12, INK);
        sheet.line(*at, *bend, self.stroke, INK);
        sheet.line(*bend, *end, self.stroke, INK);
        self.text_at(
            sheet,
            text,
            *end + Vec3::new(side * self.text * 0.2, 0.0, 0.0),
            if *side > 0.0 { 0.0 } else { 1.0 },
        );
    }

    /// `text`, or as many of its whole graphemes as fit `room` samples with an ellipsis after
    /// them (review 2026-11-13 r95 P3: a cut between letters split a flag, a family emoji
    /// joined by ZWJ or a letter from its accent).
    fn fitted(&self, text: &str, room: f64) -> String {
        use unicode_segmentation::UnicodeSegmentation;
        if self.measure(text).1 <= room {
            return text.to_owned();
        }
        let graphemes = text.graphemes(true).collect::<Vec<_>>();
        (0..graphemes.len())
            .rev()
            .map(|kept| {
                let mut cut = graphemes[..kept].concat();
                cut.truncate(cut.trim_end().len());
                cut.push('…');
                cut
            })
            .find(|cut| self.measure(cut).1 <= room)
            .unwrap_or_else(|| "…".to_owned())
    }

    /// The glyphs of `text`, each with its typeface and where it starts along the line, and how
    /// long the line is. A letter no typeface has, refused before, takes no room.
    fn measure(&self, text: &str) -> (Vec<(usize, GlyphId, f64)>, f64) {
        let scale = PxScale::from(narrow(self.text));
        let mut advances = Vec::with_capacity(text.len());
        let mut caret = 0.0_f64;
        let mut previous: Option<(usize, GlyphId)> = None;
        for letter in text.chars().filter(|letter| !unseen(*letter)) {
            let letter = if letter.is_whitespace() { ' ' } else { letter };
            let Some((typeface, glyph)) = glyph_of(&self.typefaces, letter) else {
                continue;
            };
            let scaled = self.typefaces[typeface].as_scaled(scale);
            if let Some((before, previous)) = previous
                && before == typeface
            {
                caret += f64::from(scaled.kern(previous, glyph));
            }
            advances.push((typeface, glyph, caret));
            caret += f64::from(scaled.h_advance(glyph));
            previous = Some((typeface, glyph));
        }
        (advances, caret)
    }

    /// `text` on a halo, vertically centred on `anchor`; `align` 0 starts it at the anchor,
    /// 0.5 centres it there and 1 ends it there.
    fn text_at(&self, sheet: &mut Sheet<'_>, text: &str, anchor: Vec3, align: f64) {
        let scale = PxScale::from(narrow(self.text));
        let scaled = self.typefaces[0].as_scaled(scale);
        let (advances, caret) = self.measure(text);
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
        for (typeface, glyph, offset) in advances {
            let positioned = glyph
                .with_scale_and_position(scale, point(narrow(left + offset), narrow(baseline)));
            let Some(outline) = self.typefaces[typeface].outline_glyph(positioned) else {
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
        assert_eq!(dimension_text(1200.0, DecimalMark::Point), "1200 mm");
        assert_eq!(dimension_text(1199.996, DecimalMark::Point), "1200 mm");
        assert_eq!(dimension_text(12.5, DecimalMark::Point), "12.5 mm");
        assert_eq!(dimension_text(12.5, DecimalMark::Comma), "12,5 mm");
        assert_eq!(dimension_text(0.04, DecimalMark::Comma), "0 mm");
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

    fn camera() -> Camera {
        let corners = [Vec3::ZERO, Vec3::new(1_000.0, 1_000.0, 1_000.0)];
        Camera::fit(&corners, Vec3::new(-1.0, -1.6, 1.1), 320, 240).unwrap()
    }

    /// Review 2026-11-13 r95 P3: the ellipsis cut between letters, splitting a letter from
    /// the accent written after it.
    #[test]
    fn a_cut_label_keeps_whole_graphemes() {
        let camera = camera();
        let annotator = Annotator::new(&camera, 240, 1).unwrap();
        // No letter q with an acute is composed, so each stays two letters.
        let text = "q\u{301}".repeat(60);
        let cut = annotator.fitted(&text, 80.0);
        let kept = cut.strip_suffix('…').unwrap();
        assert!(!kept.is_empty() && kept.len() < text.len(), "{cut:?}");
        assert!(kept.ends_with('\u{301}'), "{cut:?}");
        assert_eq!(kept.matches('q').count(), kept.matches('\u{301}').count());
    }

    /// Review 2026-11-13 r95 P3: labels were laid out again in every band without a look at
    /// the cancel flag.
    #[test]
    fn labels_are_laid_out_once_and_see_a_cancel() {
        let camera = camera();
        let annotator = Annotator::new(&camera, 240, 1).unwrap();
        let labels = vec![
            PresentationLabel {
                text: "Post".into(),
                at_mm: [500.0, 500.0, 1_000.0],
            };
            3
        ];
        assert!(matches!(
            annotator.place_labels(&labels, 320, &AtomicBool::new(true)),
            Err(PresentationError::Cancelled)
        ));
        let placed = annotator
            .place_labels(&labels, 320, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(placed.len(), 3);
        // The same point three times: each text moves up clear of the ones before.
        assert!(placed[0].bend.y > placed[1].bend.y && placed[1].bend.y > placed[2].bend.y);
    }
}
