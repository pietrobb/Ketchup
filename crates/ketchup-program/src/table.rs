//! A plain table of text and numbers written as CSV or as an XLSX workbook,
//! for spreadsheets and cutting optimisers.

use std::fmt::Write as _;

#[derive(Clone, Debug, PartialEq)]
pub enum Cell {
    Text(String),
    Number(f64),
    Count(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<Vec<Cell>>,
}

/// How numbers are written in CSV: with a decimal comma the columns are split
/// on `;` (what a spreadsheet in Slovak expects), with a decimal point on `,`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecimalSeparator {
    Comma,
    Point,
}

impl DecimalSeparator {
    const fn column(self) -> char {
        match self {
            Self::Comma => ';',
            Self::Point => ',',
        }
    }
}

/// A text cell. A leading `=`, `+`, `-`, `@` or control character would make
/// a spreadsheet run the text as a formula, so such text is written after an
/// apostrophe, which the spreadsheet shows as plain text.
fn csv_text(text: &str, column: char) -> String {
    let text = if text.starts_with(['=', '+', '-', '@', '\t', '\r']) {
        format!("'{text}")
    } else {
        text.to_owned()
    };
    if text.contains([column, '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

fn number_text(value: f64) -> String {
    // Millimetres to a thousandth: no binary noise such as 0.30000000000000004.
    let rounded = (value * 1000.0).round() / 1000.0;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    rounded.to_string()
}

impl Table {
    /// UTF-8 with a byte order mark (so diacritics survive), one header line,
    /// one line per row.
    #[must_use]
    pub fn csv(&self, decimal: DecimalSeparator) -> String {
        let column = decimal.column();
        let mut csv = String::from('\u{feff}');
        let header: Vec<String> = self
            .header
            .iter()
            .map(|name| csv_text(name, column))
            .collect();
        csv.push_str(&header.join(&column.to_string()));
        csv.push('\n');
        for row in &self.rows {
            let cells: Vec<String> = row
                .iter()
                .map(|cell| match cell {
                    Cell::Text(text) => csv_text(text, column),
                    Cell::Number(value) => match decimal {
                        DecimalSeparator::Comma => number_text(*value).replace('.', ","),
                        DecimalSeparator::Point => number_text(*value),
                    },
                    Cell::Count(count) => count.to_string(),
                })
                .collect();
            csv.push_str(&cells.join(&column.to_string()));
            csv.push('\n');
        }
        csv
    }

    /// An XLSX workbook with one sheet: the header row, then the rows, numbers
    /// as numbers and text as text (never as formulas).
    #[must_use]
    pub fn xlsx(&self) -> Vec<u8> {
        let mut sheet = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><sheetData>",
        );
        let header = self
            .header
            .iter()
            .map(|name| Cell::Text(name.clone()))
            .collect::<Vec<_>>();
        for (index, row) in std::iter::once(&header).chain(&self.rows).enumerate() {
            let number = index + 1;
            let _ = write!(sheet, "<row r=\"{number}\">");
            for (column, cell) in row.iter().enumerate() {
                let reference = format!("{}{number}", column_name(column));
                match cell {
                    Cell::Text(text) => {
                        let _ = write!(
                            sheet,
                            "<c r=\"{reference}\" t=\"inlineStr\"><is><t xml:space=\"preserve\">{}</t></is></c>",
                            xml_text(text)
                        );
                    }
                    Cell::Number(value) => {
                        let _ = write!(
                            sheet,
                            "<c r=\"{reference}\"><v>{}</v></c>",
                            number_text(*value)
                        );
                    }
                    Cell::Count(count) => {
                        let _ = write!(sheet, "<c r=\"{reference}\"><v>{count}</v></c>");
                    }
                }
            }
            sheet.push_str("</row>");
        }
        sheet.push_str("</sheetData></worksheet>\n");
        let files: [(&str, &[u8]); 5] = [
            ("[Content_Types].xml", CONTENT_TYPES.as_bytes()),
            ("_rels/.rels", ROOT_RELATIONS.as_bytes()),
            ("xl/workbook.xml", WORKBOOK.as_bytes()),
            ("xl/_rels/workbook.xml.rels", WORKBOOK_RELATIONS.as_bytes()),
            ("xl/worksheets/sheet1.xml", sheet.as_bytes()),
        ];
        stored_zip(&files)
    }
}

const CONTENT_TYPES: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\"><Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/><Default Extension=\"xml\" ContentType=\"application/xml\"/><Override PartName=\"/xl/workbook.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml\"/><Override PartName=\"/xl/worksheets/sheet1.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml\"/></Types>\n";
const ROOT_RELATIONS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" Target=\"xl/workbook.xml\"/></Relationships>\n";
const WORKBOOK: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><sheets><sheet name=\"Sheet1\" sheetId=\"1\" r:id=\"rId1\"/></sheets></workbook>\n";
const WORKBOOK_RELATIONS: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"><Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/></Relationships>\n";

/// Spreadsheet column letters: 0 → A, 25 → Z, 26 → AA.
fn column_name(mut index: usize) -> String {
    let mut name = Vec::new();
    loop {
        name.push(b'A' + u8::try_from(index % 26).expect("a letter offset"));
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).expect("ASCII letters")
}

/// XML text: markup characters escaped, and characters XML 1.0 forbids dropped.
fn xml_text(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\t' | '\n' | '\r' => escaped.push(character),
            control if u32::from(control) < 0x20 => {}
            other => escaped.push(other),
        }
    }
    escaped
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFF_u32;
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

/// A ZIP archive with every file stored uncompressed (method 0), which every
/// reader accepts; the workbook is small text.
fn stored_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    fn u16_le(output: &mut Vec<u8>, value: usize) {
        output.extend_from_slice(
            &u16::try_from(value)
                .expect("ZIP field fits 16 bits")
                .to_le_bytes(),
        );
    }
    fn u32_le(output: &mut Vec<u8>, value: usize) {
        output.extend_from_slice(
            &u32::try_from(value)
                .expect("ZIP field fits 32 bits")
                .to_le_bytes(),
        );
    }
    // 1980-01-01 00:00, the earliest DOS date: the bytes do not depend on the clock.
    const DOS_DATE: usize = 0x21;
    let mut archive = Vec::new();
    let mut directory = Vec::new();
    for (name, data) in files {
        let offset = archive.len();
        let crc = crc32(data);
        for (output, signature) in [(&mut archive, 0x0403_4b50), (&mut directory, 0x0201_4b50)] {
            u32_le(output, signature);
            if signature == 0x0201_4b50 {
                u16_le(output, 20); // made by
            }
            u16_le(output, 20); // needed to extract
            u16_le(output, 0); // flags
            u16_le(output, 0); // stored
            u16_le(output, 0); // time
            u16_le(output, DOS_DATE);
            output.extend_from_slice(&crc.to_le_bytes());
            u32_le(output, data.len());
            u32_le(output, data.len());
            u16_le(output, name.len());
            u16_le(output, 0); // extra field
            if signature == 0x0201_4b50 {
                u16_le(output, 0); // comment
                u16_le(output, 0); // disk
                u16_le(output, 0); // internal attributes
                u32_le(output, 0); // external attributes
                u32_le(output, offset);
            }
            output.extend_from_slice(name.as_bytes());
        }
        archive.extend_from_slice(data);
    }
    let directory_offset = archive.len();
    archive.extend_from_slice(&directory);
    u32_le(&mut archive, 0x0605_4b50);
    u16_le(&mut archive, 0);
    u16_le(&mut archive, 0);
    u16_le(&mut archive, files.len());
    u16_le(&mut archive, files.len());
    u32_le(&mut archive, directory.len());
    u32_le(&mut archive, directory_offset);
    u16_le(&mut archive, 0);
    archive
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> Table {
        Table {
            header: vec!["name".into(), "length_mm".into()],
            rows: vec![
                vec![Cell::Text("=SUM(1)".into()), Cell::Number(0.1 + 0.2)],
                vec![Cell::Text("a;b \"c\" & <d>".into()), Cell::Number(1250.0)],
            ],
        }
    }

    #[test]
    fn csv_follows_the_decimal_separator_and_never_runs_formulas() {
        let comma = table().csv(DecimalSeparator::Comma);
        assert_eq!(
            comma,
            "\u{feff}name;length_mm\n'=SUM(1);0,3\n\"a;b \"\"c\"\" & <d>\";1250\n"
        );
        let point = table().csv(DecimalSeparator::Point);
        assert_eq!(
            point,
            "\u{feff}name,length_mm\n'=SUM(1),0.3\n\"a;b \"\"c\"\" & <d>\",1250\n"
        );
    }

    /// Reads the stored archive back: every local header names a file whose
    /// bytes match the recorded CRC, and the directory lists the same files.
    fn stored_entries(archive: &[u8]) -> Vec<(String, Vec<u8>)> {
        let read16 = |at: usize| usize::from(u16::from_le_bytes([archive[at], archive[at + 1]]));
        let read32 = |at: usize| u32::from_le_bytes(archive[at..at + 4].try_into().unwrap());
        let end = archive.len() - 22;
        assert_eq!(read32(end), 0x0605_4b50);
        let count = read16(end + 10);
        let mut directory = read32(end + 16) as usize;
        let mut entries = Vec::new();
        for _ in 0..count {
            assert_eq!(read32(directory), 0x0201_4b50);
            let local = read32(directory + 42) as usize;
            assert_eq!(read32(local), 0x0403_4b50);
            assert_eq!(read16(local + 8), 0, "stored");
            let crc = read32(local + 14);
            let size = read32(local + 22) as usize;
            let name_length = read16(local + 26);
            let name =
                String::from_utf8(archive[local + 30..local + 30 + name_length].to_vec()).unwrap();
            let data = archive[local + 30 + name_length..local + 30 + name_length + size].to_vec();
            assert_eq!(crc32(&data), crc, "{name}");
            entries.push((name, data));
            directory += 46 + read16(directory + 28);
        }
        entries
    }

    #[test]
    fn xlsx_is_a_valid_package_with_numbers_and_escaped_text() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let entries = stored_entries(&table().xlsx());
        let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "[Content_Types].xml",
                "_rels/.rels",
                "xl/workbook.xml",
                "xl/_rels/workbook.xml.rels",
                "xl/worksheets/sheet1.xml"
            ]
        );
        let sheet = String::from_utf8(entries[4].1.clone()).unwrap();
        assert!(sheet.contains("<c r=\"B2\"><v>0.3</v></c>"), "{sheet}");
        assert!(
            sheet.contains("<t xml:space=\"preserve\">=SUM(1)</t>"),
            "{sheet}"
        );
        assert!(
            sheet.contains("a;b &quot;c&quot; &amp; &lt;d&gt;"),
            "{sheet}"
        );
        assert!(!sheet.contains("<f>"), "no formulas: {sheet}");
        assert_eq!(column_name(0), "A");
        assert_eq!(column_name(25), "Z");
        assert_eq!(column_name(26), "AA");
        assert_eq!(column_name(701), "ZZ");
        assert_eq!(column_name(702), "AAA");
    }
}
