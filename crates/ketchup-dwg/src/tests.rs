use std::io::Cursor;

use acadrust::types::Vector3;
use acadrust::{Arc, CadDocument, Circle, DxfReader, DxfWriter, EntityType, Line};

use super::*;

/// A drawing of two lines, a circle and an arc, as acadrust's DXF text.
fn drawing_dxf() -> Vec<u8> {
    let mut document = CadDocument::new();
    for entity in [
        EntityType::Line(Line::from_coords(0.0, 0.0, 0.0, 600.0, 0.0, 0.0)),
        EntityType::Line(Line::from_coords(600.0, 0.0, 0.0, 600.0, 400.0, 0.0)),
        EntityType::Circle(Circle::from_coords(300.0, 200.0, 0.0, 17.5)),
        EntityType::Arc(Arc::from_coords(0.0, 400.0, 0.0, 42.25, 0.0, 90.0)),
    ] {
        document.add_entity(entity).expect("the entity is added");
    }
    DxfWriter::new(&document)
        .write_to_vec()
        .expect("the drawing is written")
}

/// Kind and radius (0 for a line) of every entity of a DXF text, in drawing order.
fn entities(dxf: &[u8]) -> Vec<(&'static str, f64)> {
    let document = DxfReader::from_reader(Cursor::new(dxf.to_vec()))
        .and_then(DxfReader::read)
        .expect("the DXF reads");
    document
        .entities()
        .map(|entity| match entity {
            EntityType::Line(_) => ("LINE", 0.0),
            EntityType::Circle(circle) => ("CIRCLE", circle.radius),
            EntityType::Arc(arc) => ("ARC", arc.radius),
            _ => ("OTHER", 0.0),
        })
        .collect()
}

#[test]
fn a_dxf_drawing_written_as_dwg_reads_back_with_the_same_lines_circle_and_arc() {
    let dxf = drawing_dxf();
    let dwg = dxf_to_dwg(&dxf).expect("the drawing is written as DWG");
    assert!(dwg.starts_with(WRITTEN_VERSION.as_bytes()));

    let read = dwg_to_dxf(&dwg, 64 * 1024 * 1024).expect("the DWG reads");
    assert_eq!(read.version, WRITTEN_VERSION);
    assert!(read.skipped.is_empty(), "{:?}", read.skipped);
    assert_eq!(
        entities(&read.dxf),
        [
            ("LINE", 0.0),
            ("LINE", 0.0),
            ("CIRCLE", 17.5),
            ("ARC", 42.25)
        ]
    );
}

/// The shape of a drawing sheet's DXF (review 2026-10-17 F-3): ASCII R12, the
/// KETCHUP id at the end of each entity and a character outside ASCII as its
/// `\U+XXXX` escape.
const R12_SHEET: &str = "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009\n9\n$DWGCODEPAGE\n3\nANSI_1252\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nAPPID\n70\n1\n0\nAPPID\n2\nKETCHUP\n70\n0\n0\nENDTAB\n0\nENDSEC\n\
0\nSECTION\n2\nENTITIES\n\
0\nLINE\n8\nVISIBLE\n6\nCONTINUOUS\n10\n20\n20\n10\n30\n0\n11\n410\n21\n10\n31\n0\n1001\nKETCHUP\n1000\nsheet-border-0\n\
0\nTEXT\n8\nANNOTATION\n10\n30\n20\n60\n30\n0\n40\n3.5\n1\nKe\\U+010Dup \\U+230010 \\U+00B10.2 mm\n1001\nKETCHUP\n1000\ntitle-block/author\n\
0\nENDSEC\n0\nEOF\n";

#[test]
fn an_r12_sheet_written_as_dwg_keeps_its_line_and_its_text_with_the_escaped_characters() {
    let dwg = dxf_to_dwg(R12_SHEET.as_bytes()).expect("the R12 sheet is written as DWG");
    let read = dwg_to_dxf(&dwg, 64 * 1024 * 1024).expect("the DWG reads");
    let document = DxfReader::from_reader(Cursor::new(read.dxf))
        .and_then(DxfReader::read)
        .expect("the DXF of the DWG reads");
    let entities = document.entities().collect::<Vec<_>>();
    let [EntityType::Line(line), EntityType::Text(text)] = entities.as_slice() else {
        panic!("a line and a text: {entities:?}");
    };
    assert_eq!(line.start, Vector3::new(20.0, 10.0, 0.0));
    assert_eq!(line.end, Vector3::new(410.0, 10.0, 0.0));
    assert_eq!(text.value, "Kečup ⌀10 ±0.2 mm");
    assert_eq!(text.insertion_point, Vector3::new(30.0, 60.0, 0.0));
}

#[test]
fn a_dwg_whose_dxf_text_outgrows_the_limit_stops_at_the_limit() {
    let dwg = dxf_to_dwg(&drawing_dxf()).expect("the drawing is written as DWG");
    let error = dwg_to_dxf(&dwg, 1000).expect_err("the DXF text is over the limit");
    assert!(matches!(error, DwgError::DxfTooLarge { limit: 1000 }));
}

/// Names of the sections of a DXF text with the number of pairs inside each.
fn sections(dxf: &str) -> Vec<(String, usize)> {
    let lines: Vec<&str> = dxf.lines().map(str::trim).collect();
    let mut sections = Vec::new();
    let mut index = 0;
    while index + 3 < lines.len() {
        if lines[index] == "0" && lines[index + 1] == "SECTION" {
            let name = lines[index + 3].to_owned();
            let start = index + 4;
            let mut end = start;
            while !(lines[end] == "0" && lines[end + 1] == "ENDSEC") {
                end += 2;
            }
            sections.push((name, (end - start) / 2));
            index = end + 2;
        } else {
            index += 2;
        }
    }
    sections
}

#[test]
fn the_dxf_of_a_dwg_keeps_what_the_import_reads_and_writes_the_rest_empty() {
    let dwg = dxf_to_dwg(&drawing_dxf()).expect("the drawing is written as DWG");
    let mut whole = SectionWriter::new(usize::MAX);
    let document =
        DwgReader::from_stream_with_options(Cursor::new(&dwg), DwgReadOptions::failsafe())
            .read_with_stats()
            .expect("the DWG reads")
            .document;
    let whole_dxf = DxfWriter::new(&document).write_to_vec().expect("written");
    // Written a byte at a time, the pairs are still found across the writes.
    for byte in &whole_dxf {
        whole.write_all(std::slice::from_ref(byte)).expect("kept");
    }
    let kept = whole.finish().expect("under the limit");
    let read = dwg_to_dxf(&dwg, usize::MAX).expect("the DWG reads");
    assert_eq!(read.dxf, kept);

    let all = sections(&String::from_utf8(whole_dxf.clone()).unwrap());
    let written = sections(&String::from_utf8(read.dxf.clone()).unwrap());
    assert_eq!(
        written.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        all.iter().map(|(name, _)| name).collect::<Vec<_>>(),
        "every section is still there"
    );
    for ((name, pairs), (_, all_pairs)) in written.iter().zip(&all) {
        if name == "TABLES" {
            assert!(
                0 < *pairs && pairs < all_pairs,
                "{name} keeps layers and linetypes"
            );
        } else if DXF_SECTIONS_KEPT.contains(&name.as_str()) {
            assert_eq!(pairs, all_pairs, "{name} is kept whole");
        } else {
            assert_eq!(*pairs, 0, "{name} is written empty");
        }
    }
    assert!(
        all.iter()
            .any(|(name, pairs)| name == "OBJECTS" && *pairs > 0)
    );
    assert!(
        read.dxf.len() * 2 < whole_dxf.len(),
        "{} of {}",
        read.dxf.len(),
        whole_dxf.len()
    );
    assert_eq!(entities(&read.dxf).len(), 4);
}

/// Review 2026-10-29 CD-60: the DXF of a drawing written back is the whole drawing, every
/// section and table, and still stops at the limit.
#[test]
fn the_whole_dxf_of_a_dwg_is_every_section_and_still_stops_at_the_limit() {
    let dwg = testing::styled_drawing_dwg();
    let document =
        DwgReader::from_stream_with_options(Cursor::new(&dwg), DwgReadOptions::failsafe())
            .read()
            .expect("the DWG reads");
    let whole = DxfWriter::new(&document).write_to_vec().expect("written");
    let read = dwg_to_whole_dxf(&dwg, usize::MAX).expect("the DWG reads");
    assert_eq!(read.dxf, whole);
    assert!(read.skipped.is_empty(), "{:?}", read.skipped);
    assert!(dwg_to_dxf(&dwg, usize::MAX).unwrap().dxf.len() < whole.len());

    let error = dwg_to_whole_dxf(&dwg, whole.len() - 1).expect_err("one byte over the limit");
    assert!(matches!(error, DwgError::DxfTooLarge { .. }), "{error:?}");
}

/// Review DR-4: tables and objects that the import does not read no longer count
/// against the DXF limit, so a drawing of one line in a file with many text styles reads.
#[test]
fn tables_and_objects_the_import_does_not_read_do_not_count_against_the_limit() {
    let mut document = CadDocument::new();
    for index in 0..500 {
        document
            .text_styles
            .add(acadrust::tables::TextStyle::new(format!("style-{index}")))
            .expect("the text style is added");
    }
    document
        .add_entity(EntityType::Line(Line::from_coords(
            0.0, 0.0, 0.0, 10.0, 0.0, 0.0,
        )))
        .expect("the line is added");
    let dxf = DxfWriter::new(&document).write_to_vec().expect("written");
    let dwg = dxf_to_dwg(&dxf).expect("the drawing is written as DWG");
    let whole = DwgReader::from_stream_with_options(Cursor::new(&dwg), DwgReadOptions::failsafe())
        .read()
        .map(|document| DxfWriter::new(&document).write_to_vec().expect("written"))
        .expect("the DWG reads");
    let limit = whole.len() / 2;

    let read = dwg_to_dxf(&dwg, limit).expect("the kept sections fit the limit");
    assert!(
        read.dxf.len() < limit,
        "{} of {}",
        read.dxf.len(),
        whole.len()
    );
    assert_eq!(entities(&read.dxf), [("LINE", 0.0)]);
}

#[test]
fn bytes_that_are_not_a_dwg_file_are_a_named_error() {
    let error = dwg_to_dxf(b"AC1032 is only the magic of a DWG file", usize::MAX)
        .expect_err("the bytes are not a DWG file");
    assert!(
        matches!(
            error,
            DwgError::NothingRead { .. }
                | DwgError::Library {
                    stage: Stage::ReadDwg,
                    ..
                }
        ),
        "{error:?}"
    );
}

#[test]
fn a_panic_inside_acadrust_becomes_a_named_error_with_its_message() {
    let error = guarded(Stage::WriteDwg, || -> Result<(), DxfError> {
        panic!("record {} has no owner", 7)
    })
    .expect_err("the panic is caught");
    assert!(matches!(
        &error,
        DwgError::Panicked { stage: Stage::WriteDwg, message } if message == "record 7 has no owner"
    ));
    assert_eq!(
        error.to_string(),
        "acadrust panicked writing the DWG file: record 7 has no owner"
    );
}
