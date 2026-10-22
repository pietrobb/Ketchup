//! A DXF drawing read as an underlay: every line kept by layer, no profile limits, one
//! Undo step to add or remove it, and lines that survive their container blob.
use ketchup_model::document::{DocumentStore, TagId, Transform};
use ketchup_model::graph::sha256_bytes;
use ketchup_model::import::{
    DxfImportError, DxfImportOptions, read_dxf_underlay, read_dxf_underlay_from,
};
use ketchup_model::persistence::{self, LoadOutcome};
use ketchup_model::underlay::placement::{
    UnderlayFitError, plan_underlay_edit, plan_underlay_fit, planar_turn,
};
use ketchup_model::underlay::{
    UnderlayLines, UnderlayLinesError, plan_underlay, plan_underlay_removal,
};

fn dxf(entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn line(layer: &str, from: [f64; 2], to: [f64; 2]) -> String {
    format!(
        "0\nLINE\n8\n{layer}\n10\n{}\n20\n{}\n11\n{}\n21\n{}\n",
        from[0], from[1], to[0], to[1]
    )
}

fn read(entities: &str) -> ketchup_model::import::DxfUnderlay {
    read_dxf_underlay(&dxf(entities), DxfImportOptions::new(None)).unwrap()
}

#[test]
fn a_drawing_past_every_profile_limit_reads_whole_and_by_layer() {
    // 12,000 lines on two layers: past the 10,000 entities, ~170 profiles and 1,024
    // segments a profile import keeps to.
    let mut entities = String::new();
    for index in 0..6_000 {
        let x = f64::from(index);
        entities.push_str(&line("walls", [x, 0.0], [x + 1.0, 0.0]));
        entities.push_str(&line("doors", [x, 10.0 + x], [x, 11.0 + x]));
    }
    let underlay = read(&entities);
    assert_eq!(underlay.layers, ["walls", "doors"]);
    assert_eq!(underlay.lines.segment_count(), 12_000);
    let [walls, doors] = underlay.lines.layers() else {
        panic!("two layers")
    };
    // The walls meet end to end: one polyline of 6,000 segments. The doors touch nothing.
    assert_eq!(walls.polylines().count(), 1);
    assert_eq!(walls.segment_count(), 6_000);
    assert_eq!(doors.polylines().count(), 6_000);
}

#[test]
fn lines_join_only_where_two_of_them_meet_alone() {
    // A ring of three, and a T: the stem meets two others at one point.
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        line("0", [10.0, 0.0], [10.0, 10.0]),
        line("0", [0.0, 0.0], [10.0, 10.0]),
        line("0", [20.0, 0.0], [30.0, 0.0]),
        line("0", [30.0, 0.0], [40.0, 0.0]),
        line("0", [30.0, 0.0], [30.0, 10.0]),
    ]
    .concat();
    let underlay = read(&entities);
    let polylines = underlay.lines.layers()[0]
        .polylines()
        .map(<[_]>::to_vec)
        .collect::<Vec<_>>();
    assert_eq!(underlay.lines.segment_count(), 6);
    assert_eq!(polylines.len(), 4);
    let ring = polylines.iter().find(|points| points.len() == 4).unwrap();
    assert_eq!(ring.first(), ring.last());
}

#[test]
fn a_drawing_read_line_by_line_keeps_entities_that_wait_for_their_seqend_whole() {
    let blocks = concat!(
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n8\n0\n2\npart\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n2\n21\n0\n",
        "0\nENDBLK\n0\nENDSEC\n",
    );
    let entities = concat!(
        "999\na comment\n",
        // Left out with its vertices: a POLYLINE in paper space.
        "0\nPOLYLINE\n8\nframe\n67\n1\n66\n1\n70\n0\n",
        "0\nVERTEX\n8\nframe\n10\n0\n20\n0\n0\nVERTEX\n8\nframe\n10\n9\n20\n0\n0\nSEQEND\n",
        "0\nPOLYLINE\n8\nwalls\n66\n1\n70\n0\n",
        "0\nVERTEX\n8\nwalls\n10\n0\n20\n0\n0\nVERTEX\n8\nwalls\n10\n5\n20\n0\n",
        "0\nVERTEX\n8\nwalls\n10\n5\n20\n5\n0\nSEQEND\n",
        "0\nINSERT\n8\ndoors\n2\npart\n10\n0\n20\n0\n66\n1\n",
        "0\nATTRIB\n2\nNAME\n1\nfirst\n10\n0\n20\n0\n40\n2.5\n0\nSEQEND\n",
        "0\nLINE\n8\naxes\n10\n0\n20\n0\n11\n0\n21\n7\n",
    );
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n{blocks}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .replace('\n', "\r\n");
    let options = DxfImportOptions::new(None);
    let whole = read_dxf_underlay(source.as_bytes(), options).unwrap();
    // A buffer of four bytes splits every line across reads.
    let by_lines = read_dxf_underlay_from(
        std::io::BufReader::with_capacity(4, std::io::Cursor::new(source.as_bytes())),
        options,
    )
    .unwrap();
    assert_eq!(by_lines, whole);
    assert_eq!(whole.layers, ["walls", "doors", "axes"]);
    assert_eq!(whole.lines.segment_count(), 4);
    let counted = |code: &str| {
        whole
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code() == code)
            .map(|diagnostic| diagnostic.count())
    };
    assert_eq!(counted("dxf.entity-paper-space"), Some(1));
    assert_eq!(counted("dxf.entity-unsupported"), None);
    assert_eq!(counted("dxf.entity-unreadable"), None);
}

#[test]
fn a_line_too_long_or_a_missing_end_of_file_is_refused_while_reading() {
    let long = dxf(&line(&"x".repeat(600), [0.0, 0.0], [1.0, 0.0]));
    assert_eq!(
        read_dxf_underlay(&long, DxfImportOptions::new(None)),
        Err(DxfImportError::LineTooLong)
    );
    let source = dxf(&line("0", [0.0, 0.0], [1.0, 0.0]));
    let cut = &source[..source.len() - "0\nEOF\n".len()];
    assert_eq!(
        read_dxf_underlay(cut, DxfImportOptions::new(None)),
        Err(DxfImportError::MalformedSections)
    );
}

#[test]
fn an_entity_it_cannot_read_is_left_out_and_counted() {
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        "0\nLINE\n8\n0\n10\nnot-a-number\n20\n0\n11\n1\n21\n0\n".to_owned(),
    ]
    .concat();
    let underlay = read(&entities);
    assert_eq!(underlay.lines.segment_count(), 1);
    let unreadable = underlay
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == "dxf.entity-unreadable")
        .unwrap();
    assert_eq!(unreadable.count(), 1);
    assert!(unreadable.subject().unwrap().starts_with("LINE: "));
}

/// UL-1: a surveyed drawing carries a height on nearly every line. An underlay is a
/// plan, so heights, elevations and thicknesses are read as 0 and counted once; only
/// an extrusion that is not ±Z is still left out.
#[test]
fn heights_are_flattened_into_the_plan_and_reported_once() {
    let entities = concat!(
        "0\nLINE\n8\nsurvey\n10\n0\n20\n0\n30\n251.3\n11\n10\n21\n0\n31\n250.9\n",
        "0\nLWPOLYLINE\n8\nsurvey\n90\n2\n70\n0\n38\n250\n39\n2\n10\n0\n20\n5\n10\n10\n20\n5\n",
        "0\nCIRCLE\n8\nsurvey\n10\n5\n20\n5\n30\n249\n40\n1\n",
    );
    let tilted = "0\nLINE\n8\nsurvey\n10\n0\n20\n0\n11\n1\n21\n0\n210\n1\n220\n0\n230\n0\n";
    let underlay = read(&format!("{entities}{tilted}"));
    assert_eq!(underlay.layers, ["survey"]);
    let polylines = underlay.lines.layers()[0].polylines().count();
    // The line, the polyline and the circle; the tilted line is left out.
    assert_eq!(polylines, 3);
    let diagnostic = |code: &str| {
        underlay
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code() == code)
    };
    assert_eq!(diagnostic("dxf.height-flattened").unwrap().count(), 3);
    assert_eq!(diagnostic("dxf.entity-unreadable").unwrap().count(), 1);
}

/// UL-2: a block it could not read refused the whole underlay. A block with a raised
/// base is read flat; one with a second name, a second block of a name and a block
/// that inserts itself are left out and counted, and the rest of the drawing is kept.
#[test]
fn a_block_it_cannot_read_is_left_out_and_counted() {
    let block = |name: &str, header: &str, entities: &str| {
        format!("0\nBLOCK\n8\n0\n2\n{name}\n70\n0\n10\n0\n20\n0\n{header}{entities}0\nENDBLK\n")
    };
    let short = line("0", [0.0, 0.0], [2.0, 0.0]);
    let insert = |name: &str| format!("0\nINSERT\n8\n0\n2\n{name}\n10\n5\n20\n5\n");
    let blocks = [
        block("door", "", &short),
        block("raised", "30\n100\n", &short),
        block("named", "3\nanother\n", &short),
        block("door", "", &short),
        block("loop", "", &insert("loop")),
    ]
    .concat();
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        insert("door"),
        insert("raised"),
        insert("named"),
        insert("loop"),
    ]
    .concat();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    // The line, the door and the raised block.
    assert_eq!(underlay.lines.segment_count(), 3);
    let unreadable = (underlay.diagnostics.iter())
        .filter(|diagnostic| diagnostic.code() == "dxf.block-unreadable")
        .map(|diagnostic| diagnostic.subject().unwrap().split(':').next().unwrap())
        .collect::<Vec<_>>();
    // The block with a second name and the second "door".
    assert_eq!(unreadable.len(), 2, "{unreadable:?}");
    assert!(unreadable.contains(&"named"), "{unreadable:?}");
}

/// UL-3: past ~21 m of radius an arc was drawn with 1024 chords to the circle, 0.47 mm
/// off at 100 m and 2.35 mm at 500 m, not the 0.1 mm snapping and MCP promise.
#[test]
fn the_chords_of_a_large_arc_stay_within_a_tenth_of_a_millimetre() {
    let entities = concat!(
        "0\nCIRCLE\n8\nr100\n10\n0\n20\n0\n40\n100000\n",
        "0\nARC\n8\nr500\n10\n0\n20\n0\n40\n500000\n50\n0\n51\n90\n",
    );
    let underlay = read(entities);
    assert_eq!(underlay.offset_mm, [0.0, 0.0]);
    for (layer, radius) in underlay.lines.layers().iter().zip([100_000.0, 500_000.0]) {
        let mut farthest = 0.0_f64;
        for points in layer.polylines() {
            for pair in points.windows(2) {
                let middle = [
                    (pair[0][0] + pair[1][0]) / 2.0,
                    (pair[0][1] + pair[1][1]) / 2.0,
                ];
                farthest = farthest.max(radius - middle[0].hypot(middle[1]));
            }
        }
        assert!(farthest <= 0.1 + 1.0e-6, "{radius} mm: {farthest} mm");
    }
}

#[test]
fn an_xref_and_its_blocks_are_not_loaded_but_named_with_their_inserts() {
    // SITE is a resolved xref (4 | 32, and 64, ignored on input); SITE|door a block of it (16).
    let blocks = concat!(
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n8\n0\n2\nSITE\n70\n100\n10\n0\n20\n0\n1\nC:\\site.dwg\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\nSITE|door\n70\n48\n10\n0\n20\n0\n",
        "0\nINSERT\n8\n0\n2\nSITE\n10\n0\n20\n0\n0\nENDBLK\n0\nENDSEC\n",
    );
    let insert = |name: &str| format!("0\nINSERT\n8\n0\n2\n{name}\n10\n5\n20\n5\n");
    let entities = [
        line("0", [0.0, 0.0], [10.0, 0.0]),
        insert("SITE"),
        insert("SITE"),
        insert("SITE|door"),
    ]
    .concat();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n{blocks}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.lines.segment_count(), 1);
    let not_loaded = underlay
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code() == "dxf.xref-not-loaded")
        .map(|diagnostic| (diagnostic.subject().unwrap(), diagnostic.count()))
        .collect::<Vec<_>>();
    assert_eq!(not_loaded, [("SITE", 2), ("SITE|door", 1)]);
}

/// A drawing whose reading fails after `good` bytes, as a file on a share that goes away.
struct FailingAfter {
    source: std::io::Cursor<Vec<u8>>,
    good: u64,
}

impl std::io::Read for FailingAfter {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let left = self.good.saturating_sub(self.source.position());
        if left == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "the share went away",
            ));
        }
        let length = buffer
            .len()
            .min(usize::try_from(left).unwrap_or(usize::MAX));
        std::io::Read::read(&mut self.source, &mut buffer[..length])
    }
}

impl std::io::Seek for FailingAfter {
    fn seek(&mut self, position: std::io::SeekFrom) -> std::io::Result<u64> {
        std::io::Seek::seek(&mut self.source, position)
    }
}

/// Review 2026-10-21 P3: a failed read was reported by its kind only ("permission
/// denied"), its message lost. The message is in the report and the error is its source.
#[test]
fn a_read_that_fails_partway_names_the_systems_message() {
    let entities = line("0", [0.0, 0.0], [10.0, 0.0]).repeat(100);
    let source = dxf(&entities);
    let reader = std::io::BufReader::with_capacity(
        64,
        FailingAfter {
            good: u64::try_from(source.len() / 2).unwrap(),
            source: std::io::Cursor::new(source),
        },
    );
    let error = read_dxf_underlay_from(reader, DxfImportOptions::new(None)).unwrap_err();
    assert!(matches!(error, DxfImportError::Read(_)), "{error:?}");
    assert!(error.to_string().contains("the share went away"), "{error}");
    let source = std::error::Error::source(&error).expect("the read error as its source");
    assert!(source.to_string().contains("the share went away"));
}

/// Review 2026-10-21 P3: an xref inserted in a block was counted once, when the block was
/// read, however often the block was placed, and even when it never was. It is counted
/// per place it would be drawn, and a block holding only an xref is placed as one.
#[test]
fn an_xref_in_a_block_is_counted_once_per_insert_of_the_block() {
    let blocks = concat!(
        "0\nSECTION\n2\nBLOCKS\n",
        "0\nBLOCK\n8\n0\n2\nSITE\n70\n4\n10\n0\n20\n0\n1\nC:\\site.dwg\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\nplot\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n",
        "0\nINSERT\n8\n0\n2\nSITE\n10\n0\n20\n0\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\nonly_site\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n8\n0\n2\nSITE\n10\n0\n20\n0\n0\nENDBLK\n",
        "0\nBLOCK\n8\n0\n2\nunused\n70\n0\n10\n0\n20\n0\n",
        "0\nINSERT\n8\n0\n2\nSITE\n10\n0\n20\n0\n0\nENDBLK\n0\nENDSEC\n",
    );
    let insert = |name: &str| format!("0\nINSERT\n8\n0\n2\n{name}\n10\n5\n20\n5\n");
    let entities = [
        insert("plot"),
        insert("plot"),
        insert("plot"),
        insert("only_site"),
    ]
    .concat();
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n{blocks}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.lines.segment_count(), 3);
    let counted = |code: &str| {
        underlay
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code() == code)
            .map(|diagnostic| (diagnostic.subject().unwrap_or(""), diagnostic.count()))
            .collect::<Vec<_>>()
    };
    assert_eq!(counted("dxf.xref-not-loaded"), [("SITE", 4)]);
    assert!(counted("dxf.entity-unreadable").is_empty());
}

#[test]
fn a_drawing_in_national_grid_coordinates_is_moved_near_the_origin_by_its_median() {
    // Five lines in S-JTSK, hundreds of kilometres from the origin, a stray line at the
    // origin and one 5 km off.
    let [x, y] = [-560_123_456.7, -1_210_987_654.3];
    let mut entities = (0..5)
        .map(|index| {
            let at = x + f64::from(index) * 1000.0;
            line("plan", [at, y], [at, y + 500.0])
        })
        .collect::<String>();
    entities.push_str(&line("stray", [0.0, 0.0], [1000.0, 0.0]));
    entities.push_str(&line("plan", [x + 5.0e6, y], [x + 5.0e6 + 100.0, y]));
    entities.push_str(&line(
        "plan",
        [x - 2.0e7, y + 500.0],
        [x + 2.0e7, y + 500.0],
    ));
    let underlay = read(&entities);
    // The median point, in whole metres: the stray lines do not move it.
    assert_eq!(underlay.offset_mm, [-560_120_000.0, -1_210_987_000.0]);
    assert_eq!(underlay.layers, ["plan", "stray"]);
    // UL-4: the line 5 km off is kept, as the edge of a map of a whole village would be.
    assert_eq!(underlay.lines.segment_count(), 6);
    let first = underlay.lines.layers()[0].polylines().next().unwrap();
    assert!((first[0][0] - -3456.7).abs() < 1e-6 && (first[0][1] - -654.3).abs() < 1e-6);
    let diagnostic = |code: &str| {
        underlay
            .diagnostics
            .iter()
            .find(|diagnostic| diagnostic.code() == code)
    };
    assert_eq!(
        diagnostic("dxf.wcs-origin-moved").unwrap().subject(),
        Some("x -560120 m, y -1210987 m")
    );
    assert!(diagnostic("dxf.wcs-origin-preserved").is_none());
    // Even moved, the stray line and the one from 20 km off to 20 km off are more than
    // 10 km away.
    assert_eq!(diagnostic("dxf.lines-outside-envelope").unwrap().count(), 2);

    // A drawing near the origin stays where it is, and a profile import on the same
    // thread still keeps to the envelope.
    let near = read(&line("0", [0.0, 0.0], [10.0, 0.0]));
    assert_eq!(near.offset_mm, [0.0, 0.0]);
    assert!(
        near.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code() == "dxf.wcs-origin-preserved")
    );
    assert_eq!(
        ketchup_model::import::inspect_dxf(
            &dxf(&line("0", [x, y], [x + 10.0, y])),
            DxfImportOptions::new(None)
        )
        .map(|_| ()),
        Err(DxfImportError::CoordinateOutOfRange)
    );
}

#[test]
fn every_entity_kind_in_s_jtsk_metres_reaches_the_move_instead_of_being_refused() {
    // A drawing in metres around X = -300,000 m, Y = -1,200,000 m: a line, a polyline
    // with an arc segment, an arc and a circle, each on its own layer. The reader must
    // keep all of them for move_near_origin, not refuse them as out of range first.
    let [x, y] = [-300_123.456, -1_200_456.789];
    let entities = [
        line("line", [x, y], [x + 10.0, y]),
        format!(
            "0\nLWPOLYLINE\n8\npoly\n90\n3\n70\n0\n10\n{}\n20\n{}\n42\n0.5\n10\n{}\n20\n{}\n10\n{}\n20\n{}\n",
            x,
            y + 20.0,
            x + 10.0,
            y + 20.0,
            x + 10.0,
            y + 30.0
        ),
        format!(
            "0\nARC\n8\narc\n10\n{}\n20\n{}\n40\n5\n50\n0\n51\n90\n",
            x + 30.0,
            y
        ),
        format!("0\nCIRCLE\n8\ncircle\n10\n{}\n20\n{}\n40\n3\n", x + 50.0, y),
    ]
    .concat();
    let source = dxf(&entities)
        .into_iter()
        .map(char::from)
        .collect::<String>()
        .replace("$INSUNITS\n70\n4\n", "$INSUNITS\n70\n6\n");
    let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.layers, ["line", "poly", "arc", "circle"]);
    assert!(
        underlay
            .lines
            .layers()
            .iter()
            .all(|layer| layer.segment_count() > 0)
    );
    let codes = underlay
        .diagnostics
        .iter()
        .map(|diagnostic| diagnostic.code())
        .collect::<Vec<_>>();
    assert!(codes.contains(&"dxf.wcs-origin-moved"), "{codes:?}");
    assert!(!codes.contains(&"dxf.entity-unreadable"), "{codes:?}");
    assert!(!codes.contains(&"dxf.lines-outside-envelope"), "{codes:?}");
    // The offset is whole metres near the drawing, and added back it gives the line's
    // own start point in millimetres.
    let [dx, dy] = underlay.offset_mm;
    assert!(dx % 1000.0 == 0.0 && dy % 1000.0 == 0.0);
    assert!((dx - x * 1000.0).abs() < 100_000.0 && (dy - y * 1000.0).abs() < 100_000.0);
    let start = underlay.lines.layers()[0].polylines().next().unwrap()[0];
    assert!((start[0] + dx - x * 1000.0).abs() < 1e-3);
    assert!((start[1] + dy - y * 1000.0).abs() < 1e-3);
}

#[test]
fn lines_decode_as_they_were_encoded_and_damaged_bytes_are_refused() {
    let underlay = read(&line("0", [0.0, 0.0], [10.0, 0.0]));
    let bytes = underlay.lines.encode();
    assert_eq!(UnderlayLines::decode(&bytes).unwrap(), underlay.lines);
    assert_eq!(
        UnderlayLines::decode(&bytes[..bytes.len() - 1]),
        Err(UnderlayLinesError::Truncated)
    );
    assert_eq!(
        UnderlayLines::decode(b"not lines"),
        Err(UnderlayLinesError::NotUnderlayLines)
    );
}

#[test]
fn an_underlay_is_added_and_removed_with_its_tags_in_one_undo_step_each() {
    let underlay = read(
        &[
            line("walls", [0.0, 0.0], [1.0, 0.0]),
            line("doors", [5.0, 0.0], [5.0, 1.0]),
        ]
        .concat(),
    );
    let lines = sha256_bytes(&underlay.lines.encode());
    let mut document = DocumentStore::new();
    let offset = [-560_120_000.0, -1_210_987_000.0];
    let (batch, id) = plan_underlay(
        &document.current(),
        "Plan",
        &underlay.layers,
        &underlay.hidden_layers,
        lines,
        offset,
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let added = document.current();
    let stored = added.underlay(id).unwrap();
    assert_eq!(stored.layers.len(), 2);
    assert_eq!(stored.offset_mm(), offset);
    let tags = stored.tags().collect::<Vec<TagId>>();
    assert_eq!(tags.len(), 3);
    assert!(tags.iter().all(|tag| added.tag(*tag).is_some()));
    // No part: nothing that counts or makes parts meets it.
    assert_eq!(added.occurrences().count(), 0);

    document
        .apply_batch(&plan_underlay_removal(&added, id).unwrap())
        .unwrap();
    let removed = document.current();
    assert!(removed.underlay(id).is_none());
    assert!(tags.iter().all(|tag| removed.tag(*tag).is_none()));

    document.undo().unwrap();
    assert!(document.current().underlay(id).is_some());
    document.undo().unwrap();
    assert!(document.current().underlay(id).is_none());
    assert_eq!(document.current().tags().count(), 0);
}

/// Two points picked on an underlay land on two points of the model: turned, and scaled
/// only when asked. Each fit is one Undo step, the placement is saved with the file, and
/// two picks at one point are refused.
#[test]
fn a_two_point_fit_places_the_underlay_on_the_model_in_one_undo_step() {
    let underlay = read(&line("walls", [0.0, 0.0], [1000.0, 0.0]));
    let bytes = underlay.lines.encode();
    let mut document = DocumentStore::new();
    let (batch, id) = plan_underlay(
        &document.current(),
        "Plan",
        &underlay.layers,
        &underlay.hidden_layers,
        sha256_bytes(&bytes),
        [0.0; 2],
    )
    .unwrap();
    document.apply_batch(&batch).unwrap();
    let on = |document: &DocumentStore, local: [f64; 2], target: [f64; 2]| {
        let at = document.current().underlay(id).unwrap().to_document(local);
        assert!(
            (at[0] - target[0]).hypot(at[1] - target[1]) < 1e-9,
            "{local:?} lies at {at:?}, not {target:?}"
        );
    };

    // The line's ends onto a model edge 2 m away, turned a quarter.
    let ends = [[0.0, 0.0], [1000.0, 0.0]];
    let edge = [[2000.0, 500.0], [2000.0, 1500.0]];
    let (batch, fit) = plan_underlay_fit(&document.current(), id, ends, edge, false).unwrap();
    document.apply_batch(&batch).unwrap();
    on(&document, ends[0], edge[0]);
    on(&document, ends[1], edge[1]);
    assert!((fit.turn_deg - 90.0).abs() < 1e-9);
    assert_eq!((fit.scale, fit.length_mismatch_mm), (1.0, 0.0));

    // Picked where it is drawn now, onto an edge ten times as long: without scaling the
    // first lands and the second misses by what the lengths differ; scaled, both land.
    let long = [[2000.0, 500.0], [2000.0, 10_500.0]];
    let (_, rigid) = plan_underlay_fit(&document.current(), id, edge, long, false).unwrap();
    assert!((rigid.length_mismatch_mm - 9000.0).abs() < 1e-6);
    let (batch, fit) = plan_underlay_fit(&document.current(), id, edge, long, true).unwrap();
    document.apply_batch(&batch).unwrap();
    on(&document, ends[0], long[0]);
    on(&document, ends[1], long[1]);
    assert!((fit.scale - 10.0).abs() < 1e-9 && fit.length_mismatch_mm.abs() < 1e-6);
    assert!((document.current().underlay(id).unwrap().scale() - 10.0).abs() < 1e-9);

    // Saved and opened again, the placement stays.
    let placement = document.current().underlay(id).unwrap().placement;
    let mut container = persistence::ContainerData::default();
    container.insert_blob(bytes).unwrap();
    let saved = persistence::save_container(&document.current(), &container).unwrap();
    let LoadOutcome::Editable {
        document: opened, ..
    } = persistence::load(&saved).unwrap()
    else {
        panic!("the saved document is editable")
    };
    assert_eq!(opened.current().underlay(id).unwrap().placement, placement);

    // One Undo step per fit.
    document.undo().unwrap();
    on(&document, ends[1], edge[1]);
    document.undo().unwrap();
    on(&document, ends[1], ends[1]);

    let snapshot = document.current();
    let point = [[3.0, 4.0], [3.0, 4.0]];
    assert_eq!(
        plan_underlay_fit(&snapshot, id, point, edge, false).unwrap_err(),
        UnderlayFitError::UnderlayPointsCoincide
    );
    assert_eq!(
        plan_underlay_fit(&snapshot, id, ends, point, true).unwrap_err(),
        UnderlayFitError::ModelPointsCoincide
    );

    // A move or turn of the document goes into the placement as one Undo step; one that
    // would lift or tilt the underlay is refused.
    let turn = planar_turn([1000.0, 0.0], 90.0).unwrap();
    let shift = Transform::from_translation(0.0, 0.0, 5.0).unwrap();
    document
        .apply_batch(&plan_underlay_edit(&snapshot, id, turn).unwrap())
        .unwrap();
    on(&document, ends[0], [1000.0, -1000.0]);
    on(&document, ends[1], ends[1]);
    let matrix = *document.current().underlay(id).unwrap().placement.matrix();
    assert_eq!(
        [matrix[2], matrix[6], matrix[8], matrix[9], matrix[10]],
        [0.0, 0.0, 0.0, 0.0, 1.0]
    );
    document.undo().unwrap();
    on(&document, ends[0], ends[0]);
    assert_eq!(
        plan_underlay_edit(&document.current(), id, shift).unwrap_err(),
        UnderlayFitError::OffPlane
    );

    // Review 2026-10-21 P3: a fit or move that scales the underlay a billion times or
    // places it 1e12 mm away, a unit mix-up, is refused with the bounds instead of saved.
    let snapshot = document.current();
    let far = [[1.0e12, 0.0], [1.0e12 + 1000.0, 0.0]];
    let huge = [[0.0, 0.0], [1.0e12, 0.0]];
    for (to, scaled) in [(far, false), (huge, true)] {
        let refusal = plan_underlay_fit(&snapshot, id, ends, to, scaled)
            .unwrap_err()
            .to_string();
        assert!(refusal.contains("1000000 mm"), "{refusal}");
    }
    let away = Transform::from_translation(1.0e12, 0.0, 0.0).unwrap();
    let refusal = plan_underlay_edit(&snapshot, id, away)
        .unwrap_err()
        .to_string();
    assert!(refusal.contains("1000000 mm"), "{refusal}");
}
