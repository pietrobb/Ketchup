//! How an underlay draws a line: in the color, weight and dash pattern its drawing gives it.

use super::{Reader, UnderlayLinesError, f64_at, push_count};

/// The style of a polyline or a text of an underlay; a text is drawn in its color only.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LineStyle {
    /// sRGB; `None` draws in the underlay's own color, as the drawing's foreground color
    /// (ACI 7, black or white with the background) is drawn.
    pub color: Option<[u8; 3]>,
    /// The weight the drawing plots the line with, in hundredths of a millimetre on paper.
    pub weight_hmm: u16,
    /// Repeated from the start of each polyline: a dash (> 0), a gap (< 0) or a dot (0),
    /// in millimetres. Empty for a continuous line.
    pub pattern_mm: Vec<f64>,
    /// How wide the drawing draws the polyline, in millimetres in the drawing: its
    /// constant width, or else the widest of its vertices; `0` for a plain line. Not
    /// drawn: kept so that a revision drawing a line wider is told apart (review
    /// 2026-11-24 R1124og-DXF-902).
    pub width_mm: f64,
    /// The font the drawing gives a text: its style's font file and family, or the
    /// style's name where the drawing does not define it, then each font the text
    /// switches to; empty for a line. Not drawn: kept as [`Self::width_mm`] is.
    pub font: String,
}

/// The style of what has none of its own, and of an underlay saved before styles.
pub(super) static CONTINUOUS: LineStyle = LineStyle {
    color: None,
    weight_hmm: 0,
    pattern_mm: Vec::new(),
    width_mm: 0.0,
    font: String::new(),
};

impl LineStyle {
    #[must_use]
    pub fn is_dashed(&self) -> bool {
        !self.pattern_mm.is_empty()
    }

    /// Whether it says what only version 7 of the blob holds: a width or a font.
    pub(super) fn needs_width_and_font(&self) -> bool {
        self.width_mm != 0.0 || !self.font.is_empty()
    }

    /// Writes it; `with_width_and_font` its width and font after its pattern.
    pub(super) fn encode(&self, bytes: &mut Vec<u8>, with_width_and_font: bool) {
        let [red, green, blue] = self.color.unwrap_or_default();
        bytes.extend_from_slice(&[u8::from(self.color.is_some()), red, green, blue]);
        bytes.extend_from_slice(&self.weight_hmm.to_le_bytes());
        push_count(bytes, self.pattern_mm.len());
        for length in &self.pattern_mm {
            bytes.extend_from_slice(&length.to_le_bytes());
        }
        if with_width_and_font {
            bytes.extend_from_slice(&self.width_mm.to_le_bytes());
            push_count(bytes, self.font.len());
            bytes.extend_from_slice(self.font.as_bytes());
        }
    }

    /// Reads what [`Self::encode`] wrote: a pattern of finite lengths, not all of them
    /// dots, and a finite width of at least `0`.
    pub(super) fn decode(
        reader: &mut Reader<'_>,
        with_width_and_font: bool,
    ) -> Result<Self, UnderlayLinesError> {
        let head = reader.take(6, 1)?;
        let color = match head[0] {
            0 => None,
            1 => Some([head[1], head[2], head[3]]),
            _ => return Err(UnderlayLinesError::InvalidStyle),
        };
        let weight_hmm = u16::from_le_bytes([head[4], head[5]]);
        let count = reader.count()?;
        let pattern_mm = reader
            .take(count, 8)?
            .chunks_exact(8)
            .map(f64_at)
            .collect::<Vec<_>>();
        if pattern_mm.iter().any(|length| !length.is_finite())
            || (!pattern_mm.is_empty() && pattern_mm.iter().all(|length| *length == 0.0))
        {
            return Err(UnderlayLinesError::InvalidStyle);
        }
        let (width_mm, font) = if with_width_and_font {
            let width_mm = f64_at(reader.take(1, 8)?);
            let length = reader.count()?;
            let font = std::str::from_utf8(reader.take(length, 1)?)
                .map_err(UnderlayLinesError::TextNotUtf8)?
                .to_owned();
            if !width_mm.is_finite() || width_mm < 0.0 {
                return Err(UnderlayLinesError::InvalidStyle);
            }
            (width_mm, font)
        } else {
            (0.0, String::new())
        };
        Ok(Self {
            color,
            weight_hmm,
            pattern_mm,
            width_mm,
            font,
        })
    }
}
