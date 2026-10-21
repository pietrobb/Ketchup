//! Texts of an underlay: drawn where the drawing places them, on the layer they belong
//! to, and never snapped to. A text keeps what its drawing says of it (anchor, height,
//! rotation, alignment); how wide its letters are is up to the font that draws it.

use super::UnderlayLinesError;

/// Where a text's lines sit against its anchor along the baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextHorizontal {
    Left,
    Center,
    Right,
}

/// Where a text's lines sit against its anchor across the baseline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TextVertical {
    /// The anchor is on the baseline of the first line, or of the last of several.
    Baseline,
    /// The anchor is at the bottom of the descenders of the last line.
    Bottom,
    Middle,
    /// The anchor is at the top of the capitals of the first line.
    Top,
}

/// A text stretched along its baseline from its anchor to a given width (DXF Fit and
/// Aligned).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum TextFit {
    None,
    /// Its letters widen or narrow to this many millimetres; its height stays.
    Width(f64),
    /// It is scaled as a whole to this many millimetres wide.
    Scaled(f64),
}

/// One text of an underlay, in millimetres in the XY plane.
#[derive(Clone, Debug, PartialEq)]
pub struct UnderlayText {
    /// Its lines, separated by `'\n'`, without formatting codes.
    pub text: String,
    pub anchor_mm: [f64; 2],
    /// The height of a capital letter.
    pub height_mm: f64,
    /// Of the baseline, counterclockwise from +X.
    pub rotation_rad: f64,
    /// How much wider than the font draws them its letters are.
    pub width_factor: f64,
    pub horizontal: TextHorizontal,
    pub vertical: TextVertical,
    pub fit: TextFit,
    /// A line wider than this wraps at a space; `0` for none.
    pub wrap_width_mm: f64,
    /// From one baseline to the next.
    pub line_pitch_mm: f64,
    /// The index of the style whose color it is drawn in ([`super::UnderlayLines::style`]).
    pub style: u32,
}

impl UnderlayText {
    /// A one-line text on its baseline from its anchor, `height_mm` high.
    #[must_use]
    pub fn new(text: impl Into<String>, anchor_mm: [f64; 2], height_mm: f64) -> Self {
        Self {
            text: text.into(),
            anchor_mm,
            height_mm,
            rotation_rad: 0.0,
            width_factor: 1.0,
            horizontal: TextHorizontal::Left,
            vertical: TextVertical::Baseline,
            fit: TextFit::None,
            wrap_width_mm: 0.0,
            line_pitch_mm: height_mm * LINE_PITCH_PER_HEIGHT,
            style: 0,
        }
    }

    pub(super) fn encode(&self, bytes: &mut Vec<u8>) {
        let (fit_kind, fit_mm) = match self.fit {
            TextFit::None => (0, 0.0),
            TextFit::Width(width) => (1, width),
            TextFit::Scaled(width) => (2, width),
        };
        for value in [
            self.anchor_mm[0],
            self.anchor_mm[1],
            self.height_mm,
            self.rotation_rad,
            self.width_factor,
            fit_mm,
            self.wrap_width_mm,
            self.line_pitch_mm,
        ] {
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&[
            match self.horizontal {
                TextHorizontal::Left => 0,
                TextHorizontal::Center => 1,
                TextHorizontal::Right => 2,
            },
            match self.vertical {
                TextVertical::Baseline => 0,
                TextVertical::Bottom => 1,
                TextVertical::Middle => 2,
                TextVertical::Top => 3,
            },
            fit_kind,
        ]);
        super::push_count(bytes, self.text.len());
        bytes.extend_from_slice(self.text.as_bytes());
        bytes.extend_from_slice(&self.style.to_le_bytes());
    }

    /// Reads what [`Self::encode`] wrote, its length checked against the bytes first; a
    /// text saved before styles is not `styled`.
    pub(super) fn decode(
        reader: &mut super::Reader<'_>,
        styled: bool,
    ) -> Result<Self, UnderlayLinesError> {
        let numbers = reader.take(8, 8)?;
        let [
            x,
            y,
            height_mm,
            rotation_rad,
            width_factor,
            fit_mm,
            wrap_width_mm,
            line_pitch_mm,
        ] = std::array::from_fn(|index| super::f64_at(&numbers[index * 8..index * 8 + 8]));
        let kinds = reader.take(3, 1)?;
        let length = reader.count()?;
        let text = std::str::from_utf8(reader.take(length, 1)?)
            .map_err(UnderlayLinesError::TextNotUtf8)?
            .to_owned();
        let style = if styled {
            let bytes = reader.take(4, 1)?;
            u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])
        } else {
            0
        };
        let numbers = [
            x,
            y,
            height_mm,
            rotation_rad,
            width_factor,
            fit_mm,
            wrap_width_mm,
            line_pitch_mm,
        ];
        if numbers.iter().any(|value| !value.is_finite()) {
            return Err(UnderlayLinesError::NotFinite);
        }
        let horizontal = match kinds[0] {
            0 => TextHorizontal::Left,
            1 => TextHorizontal::Center,
            2 => TextHorizontal::Right,
            _ => return Err(UnderlayLinesError::InvalidText),
        };
        let vertical = match kinds[1] {
            0 => TextVertical::Baseline,
            1 => TextVertical::Bottom,
            2 => TextVertical::Middle,
            3 => TextVertical::Top,
            _ => return Err(UnderlayLinesError::InvalidText),
        };
        let fit = match kinds[2] {
            0 => TextFit::None,
            1 => TextFit::Width(fit_mm),
            2 => TextFit::Scaled(fit_mm),
            _ => return Err(UnderlayLinesError::InvalidText),
        };
        if height_mm <= 0.0
            || width_factor <= 0.0
            || wrap_width_mm < 0.0
            || line_pitch_mm < 0.0
            || (fit != TextFit::None && fit_mm <= 0.0)
        {
            return Err(UnderlayLinesError::InvalidText);
        }
        Ok(Self {
            text,
            anchor_mm: [x, y],
            height_mm,
            rotation_rad,
            width_factor,
            horizontal,
            vertical,
            fit,
            wrap_width_mm,
            line_pitch_mm,
            style,
        })
    }
}

/// From one baseline to the next of a text at single line spacing, per its height.
pub const LINE_PITCH_PER_HEIGHT: f64 = 5.0 / 3.0;
