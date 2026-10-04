//! A vector drawing sheet and its PDF: filled and stroked paths and text in
//! page millimetres (origin top left, y down), written with an embedded
//! TrueType font so every letter, Slovak diacritics included, prints as drawn.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::sync::OnceLock;

const POINTS_PER_MM: f64 = 72.0 / 25.4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Anchor {
    Start,
    Middle,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub width_mm: f64,
    pub color: [u8; 3],
    /// Dash and gap lengths in mm; empty for a solid line.
    pub dash_mm: Vec<f64>,
}

impl Stroke {
    #[must_use]
    pub fn solid(width_mm: f64) -> Self {
        Self {
            width_mm,
            color: [0, 0, 0],
            dash_mm: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    /// Polygons filled even-odd (a loop inside another is a hole), optionally outlined.
    Fill {
        loops: Vec<Vec<[f64; 2]>>,
        color: [u8; 3],
        outline: Option<Stroke>,
    },
    Line {
        points: Vec<[f64; 2]>,
        closed: bool,
        stroke: Stroke,
    },
    /// Text whose capital letters are `height_mm` tall, turned `angle_deg`
    /// counter-clockwise about its anchor point.
    Text {
        at: [f64; 2],
        text: String,
        height_mm: f64,
        anchor: Anchor,
        angle_deg: f64,
        bold: bool,
    },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Page {
    pub size_mm: [f64; 2],
    pub marks: Vec<Mark>,
}

impl Page {
    pub fn line(&mut self, points: Vec<[f64; 2]>, stroke: Stroke) {
        self.marks.push(Mark::Line {
            points,
            closed: false,
            stroke,
        });
    }

    pub fn rect(&mut self, min: [f64; 2], max: [f64; 2], stroke: Stroke) {
        self.marks.push(Mark::Line {
            points: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
            closed: true,
            stroke,
        });
    }

    pub fn text(&mut self, at: [f64; 2], text: &str, height_mm: f64, anchor: Anchor) {
        self.marks.push(Mark::Text {
            at,
            text: text.to_owned(),
            height_mm,
            anchor,
            angle_deg: 0.0,
            bold: false,
        });
    }

    /// Every text on the page, in drawing order.
    pub fn texts(&self) -> impl Iterator<Item = &str> {
        self.marks.iter().filter_map(|mark| match mark {
            Mark::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }
}

/// The sheet typeface: Ubuntu Light, bundled with the application.
pub struct SheetFont {
    face: ttf_parser::Face<'static>,
    data: &'static [u8],
    units: f64,
}

impl SheetFont {
    /// The bundled typeface, parsed once.
    #[must_use]
    pub fn standard() -> &'static Self {
        static FONT: OnceLock<SheetFont> = OnceLock::new();
        FONT.get_or_init(|| {
            let data = epaint_default_fonts::UBUNTU_LIGHT;
            let face = ttf_parser::Face::parse(data, 0).expect("the bundled typeface parses");
            Self {
                units: f64::from(face.units_per_em()),
                face,
                data,
            }
        })
    }

    fn glyph(&self, character: char) -> u16 {
        self.face
            .glyph_index(character)
            .or_else(|| self.face.glyph_index('?'))
            .map_or(0, |glyph| glyph.0)
    }

    /// Advance of a glyph in em.
    fn advance(&self, glyph: u16) -> f64 {
        self.face
            .glyph_hor_advance(ttf_parser::GlyphId(glyph))
            .map_or(0.0, |advance| f64::from(advance) / self.units)
    }

    /// Height of a capital letter in em.
    fn cap_height(&self) -> f64 {
        self.face
            .capital_height()
            .filter(|height| *height > 0)
            .map_or(0.7, |height| f64::from(height) / self.units)
    }

    /// The font size whose capitals are `height_mm` tall.
    fn size_for(&self, height_mm: f64) -> f64 {
        height_mm / self.cap_height()
    }

    /// Width of `text` with capitals `height_mm` tall.
    #[must_use]
    pub fn width_mm(&self, text: &str, height_mm: f64) -> f64 {
        let em: f64 = text
            .chars()
            .map(|character| self.advance(self.glyph(character)))
            .sum();
        em * self.size_for(height_mm)
    }

    fn postscript_name(&self) -> String {
        self.face
            .names()
            .into_iter()
            .filter(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
            .find_map(|name| name.to_string())
            .filter(|name| name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
            .unwrap_or_else(|| "SheetSans".to_owned())
    }

    fn per_mille(&self, value: i16) -> i64 {
        #[allow(clippy::cast_possible_truncation)]
        {
            (f64::from(value) * 1000.0 / self.units).round() as i64
        }
    }
}

/// Document information shown by PDF viewers.
#[derive(Clone, Debug, Default)]
pub struct PdfInfo {
    pub title: String,
    pub author: String,
    pub subject: String,
}

fn number(value: f64) -> String {
    let mut text = format!("{value:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" { "0".to_owned() } else { text }
}

fn rgb(color: [u8; 3]) -> String {
    color
        .map(|channel| number(f64::from(channel) / 255.0))
        .join(" ")
}

/// Graphics state already set in the content stream, so runs of marks of one
/// style write it once.
#[derive(Default)]
struct State {
    fill: Option<[u8; 3]>,
    stroke: Option<[u8; 3]>,
    width: Option<f64>,
    dash: Option<Vec<f64>>,
}

impl State {
    fn fill(&mut self, out: &mut String, color: [u8; 3]) {
        if self.fill != Some(color) {
            let _ = writeln!(out, "{} rg", rgb(color));
            self.fill = Some(color);
        }
    }

    fn stroke(&mut self, out: &mut String, stroke: &Stroke) {
        if self.stroke != Some(stroke.color) {
            let _ = writeln!(out, "{} RG", rgb(stroke.color));
            self.stroke = Some(stroke.color);
        }
        if self.width != Some(stroke.width_mm) {
            let _ = writeln!(out, "{} w", number(stroke.width_mm));
            self.width = Some(stroke.width_mm);
        }
        if self.dash.as_ref() != Some(&stroke.dash_mm) {
            let dashes = stroke
                .dash_mm
                .iter()
                .map(|value| number(*value))
                .collect::<Vec<_>>()
                .join(" ");
            let _ = writeln!(out, "[{dashes}] 0 d");
            self.dash = Some(stroke.dash_mm.clone());
        }
    }
}

struct ContentWriter<'a> {
    font: &'a SheetFont,
    height: f64,
    out: String,
    state: State,
    /// Glyphs used, with the character each stands for.
    glyphs: BTreeMap<u16, char>,
}

impl ContentWriter<'_> {
    fn point(&self, [x, y]: [f64; 2]) -> String {
        format!("{} {}", number(x), number(self.height - y))
    }

    fn path(&mut self, points: &[[f64; 2]], closed: bool) {
        for (index, point) in points.iter().enumerate() {
            let point = self.point(*point);
            let _ = writeln!(self.out, "{point} {}", if index == 0 { "m" } else { "l" });
        }
        if closed {
            self.out.push_str("h\n");
        }
    }

    fn mark(&mut self, mark: &Mark) {
        match mark {
            Mark::Fill {
                loops,
                color,
                outline,
            } => {
                self.state.fill(&mut self.out, *color);
                if let Some(stroke) = outline {
                    self.state.stroke(&mut self.out, stroke);
                }
                for points in loops.iter().filter(|points| points.len() >= 3) {
                    self.path(points, true);
                }
                self.out
                    .push_str(if outline.is_some() { "B*\n" } else { "f*\n" });
            }
            Mark::Line {
                points,
                closed,
                stroke,
            } => {
                if points.len() < 2 {
                    return;
                }
                self.state.stroke(&mut self.out, stroke);
                self.path(points, *closed);
                self.out.push_str("S\n");
            }
            Mark::Text {
                at,
                text,
                height_mm,
                anchor,
                angle_deg,
                bold,
            } => self.text(*at, text, *height_mm, *anchor, *angle_deg, *bold),
        }
    }

    fn text(
        &mut self,
        at: [f64; 2],
        text: &str,
        height_mm: f64,
        anchor: Anchor,
        angle_deg: f64,
        bold: bool,
    ) {
        if text.is_empty() {
            return;
        }
        let (sin, cos) = angle_deg.to_radians().sin_cos();
        let shift = match anchor {
            Anchor::Start => 0.0,
            Anchor::Middle => self.font.width_mm(text, height_mm) / 2.0,
            Anchor::End => self.font.width_mm(text, height_mm),
        };
        let x = at[0] - shift * cos;
        let y = self.height - at[1] - shift * sin;
        let mut hex = String::with_capacity(text.len() * 4);
        for character in text.chars() {
            let glyph = self.font.glyph(character);
            self.glyphs.entry(glyph).or_insert(character);
            let _ = write!(hex, "{glyph:04X}");
        }
        self.state.fill(&mut self.out, [0, 0, 0]);
        let size = self.font.size_for(height_mm);
        let mode = if bold {
            self.state
                .stroke(&mut self.out, &Stroke::solid(size * 0.03));
            "2 Tr"
        } else {
            "0 Tr"
        };
        let _ = writeln!(
            self.out,
            "BT /F1 {} Tf {mode} {} {} {} {} {} {} Tm <{hex}> Tj ET",
            number(size),
            number(cos),
            number(sin),
            number(-sin),
            number(cos),
            number(x),
            number(y),
        );
    }
}

fn pdf_text(value: &str) -> String {
    let mut hex = String::from("<FEFF");
    for unit in value.encode_utf16() {
        let _ = write!(hex, "{unit:04X}");
    }
    hex.push('>');
    hex
}

fn stream(dictionary: &str, data: &[u8]) -> Vec<u8> {
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(data, 6);
    let mut object = format!(
        "<< {dictionary} /Filter /FlateDecode /Length {} >>\nstream\n",
        compressed.len()
    )
    .into_bytes();
    object.extend_from_slice(&compressed);
    object.extend_from_slice(b"\nendstream");
    object
}

fn to_unicode(glyphs: &BTreeMap<u16, char>) -> String {
    let mut cmap = String::from(
        "/CIDInit /ProcSet findresource begin\n12 dict begin\nbegincmap\n/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) /Supplement 0 >> def\n/CMapName /Adobe-Identity-UCS def\n/CMapType 2 def\n1 begincodespacerange\n<0000> <FFFF>\nendcodespacerange\n",
    );
    let entries = glyphs.iter().collect::<Vec<_>>();
    for chunk in entries.chunks(100) {
        let _ = writeln!(cmap, "{} beginbfchar", chunk.len());
        for (glyph, character) in chunk {
            let mut units = [0_u16; 2];
            let target = character
                .encode_utf16(&mut units)
                .iter()
                .map(|unit| format!("{unit:04X}"))
                .collect::<String>();
            let _ = writeln!(cmap, "<{glyph:04X}> <{target}>");
        }
        cmap.push_str("endbfchar\n");
    }
    cmap.push_str("endcmap\nCMapName currentdict /CMap defineresource pop\nend\nend\n");
    cmap
}

/// The page as a one-page PDF with the sheet typeface embedded.
#[must_use]
pub fn page_pdf(page: &Page, info: &PdfInfo) -> Vec<u8> {
    let font = SheetFont::standard();
    let [width, height] = page.size_mm;
    let mut content = ContentWriter {
        font,
        height,
        out: format!("{0} 0 0 {0} 0 0 cm\n1 J 1 j\n", number(POINTS_PER_MM)),
        state: State::default(),
        glyphs: BTreeMap::new(),
    };
    for mark in &page.marks {
        content.mark(mark);
    }
    let name = font.postscript_name();
    let widths = content
        .glyphs
        .keys()
        .map(|glyph| {
            #[allow(clippy::cast_possible_truncation)]
            let width = (font.advance(*glyph) * 1000.0).round() as i64;
            format!("{glyph} [{width}]")
        })
        .collect::<Vec<_>>()
        .join(" ");
    let bbox = font.face.global_bounding_box();
    let descriptor = format!(
        "<< /Type /FontDescriptor /FontName /{name} /Flags 32 /FontBBox [{} {} {} {}] /ItalicAngle 0 /Ascent {} /Descent {} /CapHeight {} /StemV 80 /FontFile2 8 0 R >>",
        font.per_mille(bbox.x_min),
        font.per_mille(bbox.y_min),
        font.per_mille(bbox.x_max),
        font.per_mille(bbox.y_max),
        font.per_mille(font.face.ascender()),
        font.per_mille(font.face.descender()),
        font.per_mille(font.face.capital_height().unwrap_or(700)),
    );
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Resources << /Font << /F1 5 0 R >> >> /Contents 4 0 R >>",
            number(width * POINTS_PER_MM),
            number(height * POINTS_PER_MM)
        )
        .into_bytes(),
        stream("", content.out.as_bytes()),
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /{name} /Encoding /Identity-H /DescendantFonts [6 0 R] /ToUnicode 9 0 R >>"
        )
        .into_bytes(),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /{name} /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> /FontDescriptor 7 0 R /DW 500 /W [{widths}] /CIDToGIDMap /Identity >>"
        )
        .into_bytes(),
        descriptor.into_bytes(),
        stream(&format!("/Length1 {}", font.data.len()), font.data),
        stream("", to_unicode(&content.glyphs).as_bytes()),
        format!(
            "<< /Title {} /Author {} /Subject {} /Creator (Ketchup) /Producer (Ketchup) >>",
            pdf_text(&info.title),
            pdf_text(&info.author),
            pdf_text(&info.subject)
        )
        .into_bytes(),
    ];
    let mut pdf = b"%PDF-1.7\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::with_capacity(objects.len());
    for (index, object) in objects.iter().enumerate() {
        offsets.push(pdf.len());
        pdf.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        pdf.extend_from_slice(object);
        pdf.extend_from_slice(b"\nendobj\n");
    }
    let xref = pdf.len();
    let mut table = format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1);
    for offset in offsets {
        let _ = writeln!(table, "{offset:010} 00000 n ");
    }
    let _ = write!(
        table,
        "trailer\n<< /Size {} /Root 1 0 R /Info 10 0 R >>\nstartxref\n{xref}\n%%EOF\n",
        objects.len() + 1
    );
    pdf.extend_from_slice(table.as_bytes());
    pdf
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_font_measures_wider_text_wider() {
        let font = SheetFont::standard();
        assert!(font.width_mm("Pôdorys", 2.5) > font.width_mm("Rez", 2.5));
        assert!((font.width_mm("MMMM", 5.0) - 2.0 * font.width_mm("MMMM", 2.5)).abs() < 1e-9);
        for character in "áäčďéíĺľňóôŕšťúýžÁÄČĎÉÍĹĽŇÓÔŔŠŤÚÝŽ–×±°".chars()
        {
            assert_ne!(
                font.face.glyph_index(character),
                None,
                "{character} is in the font"
            );
        }
    }

    #[test]
    fn a_page_becomes_a_pdf_with_the_font_and_the_text_embedded() {
        let mut page = Page {
            size_mm: [420.0, 297.0],
            marks: Vec::new(),
        };
        page.rect([10.0, 10.0], [410.0, 287.0], Stroke::solid(0.7));
        page.text([20.0, 20.0], "Pôdorys – ľavá časť", 3.5, Anchor::Start);
        page.marks.push(Mark::Fill {
            loops: vec![vec![[50.0, 50.0], [80.0, 50.0], [80.0, 70.0]]],
            color: [200, 100, 50],
            outline: Some(Stroke::solid(0.5)),
        });
        let pdf = page_pdf(&page, &PdfInfo::default());
        let find = |needle: &[u8]| {
            pdf.windows(needle.len())
                .rposition(|window| window == needle)
        };
        assert!(pdf.starts_with(b"%PDF-1.7"));
        assert!(find(b"/FontFile2 8 0 R").is_some());
        assert!(find(b"/Subtype /CIDFontType2").is_some());
        assert!(find(b"/MediaBox [0 0 1190.551 841.89]").is_some());
        // The cross-reference table points at each object.
        let xref = find(b"\nxref\n").expect("the PDF has a cross-reference table") + 1;
        let tail = String::from_utf8_lossy(&pdf[xref..]).into_owned();
        let start = tail[tail.find("startxref\n").expect("startxref") + 10..]
            .lines()
            .next()
            .and_then(|line| line.parse::<usize>().ok());
        assert_eq!(start, Some(xref));
        for (index, line) in tail.lines().skip(3).take(10).enumerate() {
            let offset = line[..10].parse::<usize>().expect("an offset");
            let header = format!("{} 0 obj", index + 1);
            assert!(pdf[offset..].starts_with(header.as_bytes()), "{line}");
        }
    }
}
