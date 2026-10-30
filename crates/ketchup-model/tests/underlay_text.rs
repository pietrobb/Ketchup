//! Texts of a DXF drawing read as an underlay: TEXT, MTEXT, the ATTRIBs of an INSERT and
//! the text of a DIMENSION, each on its layer where the drawing places it, kept with the
//! lines in their blob and never snapped to; what is not drawn as asked is reported.
use ketchup_model::import::{DxfImportOptions, DxfUnderlay, read_dxf_underlay};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::snap::UnderlaySnapIndex;
use ketchup_model::underlay::text::{TextFit, TextHorizontal, TextVertical, UnderlayText};

fn read(blocks: &str, entities: &str) -> DxfUnderlay {
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap()
}

/// The texts of `layer`, which must be one of the underlay's layers.
fn texts<'a>(underlay: &'a DxfUnderlay, layer: &str) -> &'a [UnderlayText] {
    let index = underlay
        .layers
        .iter()
        .position(|name| name == layer)
        .unwrap_or_else(|| panic!("no layer {layer} in {:?}", underlay.layers));
    underlay.lines.layers()[index].texts()
}

fn counted(underlay: &DxfUnderlay, code: &str, subject: Option<&str>) -> Option<u32> {
    underlay
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == code && diagnostic.subject() == subject)
        .map(|diagnostic| diagnostic.count())
}

fn close(left: [f64; 2], right: [f64; 2]) -> bool {
    (left[0] - right[0]).abs() < 1e-9 && (left[1] - right[1]).abs() < 1e-9
}

/// Review 2026-10-21 P3: `%%nnn` is the character nnn of the drawing's code page, not the
/// Unicode code point nnn; a drawing that names none is in ANSI_1252, as AutoCAD reads it.
#[test]
fn a_numbered_special_character_is_read_in_the_drawings_code_page() {
    let text = |header: &str, value: &str| {
        let source = format!(
            "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n{header}0\nENDSEC\n\
             0\nSECTION\n2\nENTITIES\n0\nTEXT\n8\nlabels\n10\n0\n20\n0\n40\n250\n1\n{value}\n\
             0\nENDSEC\n0\nEOF\n"
        );
        let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
        texts(&underlay, "labels")[0].text.clone()
    };
    let central_european = "9\n$DWGCODEPAGE\n3\nANSI_1250\n";
    assert_eq!(text(central_european, "%%142ilina 20%%176"), "Žilina 20°");
    assert_eq!(text(central_european, "%%%%%%200 %%065"), "%%200 A");
    assert_eq!(text("", "%%128 5"), "€ 5");
}

/// Review 2026-10-24 UL-23: a drawing before 2007 in ANSI_1252 writes Czech and Slovak
/// letters as `\U+011B`, in a TEXT and in a layer's name; only an MTEXT read them, and a
/// layer named so was refused for its backslash.
#[test]
fn an_escaped_character_is_read_in_a_text_an_attribute_and_a_layers_name() {
    let source = concat!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n",
        "0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n2\n",
        "0\nLAYER\n2\nskryt\\U+00E1\n70\n1\n62\n7\n6\nCONTINUOUS\n",
        "0\nENDTAB\n0\nENDSEC\n",
        "0\nSECTION\n2\nENTITIES\n",
        "0\nTEXT\n8\nst\\U+011Bny\n10\n0\n20\n0\n40\n250\n1\nSt\\U+011Bna \\M+5B0A1\n",
        "0\nLINE\n8\nskryt\\U+00E1\n10\n0\n20\n0\n11\n100\n21\n0\n",
        "0\nMTEXT\n8\nst\\U+011Bny\n10\n0\n20\n500\n40\n250\n1\n\\U+0161\\M+5B0A1\n",
        "0\nENDSEC\n0\nEOF\n",
    );
    let underlay = read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap();
    let read: Vec<_> = texts(&underlay, "stěny")
        .iter()
        .map(|text| text.text.as_str())
        .collect();
    assert_eq!(read, ["Stěna 啊", "š啊"]);
    // The line's layer is the frozen one in the table, matched by its name as escaped.
    assert_eq!(underlay.hidden_layers, ["skrytá"]);
    assert!(
        underlay.layers.iter().any(|layer| layer == "skrytá"),
        "{:?}",
        underlay.layers
    );
}

/// Review 2026-10-19 P3: an underlay read in the code page its pre-2007 drawing names
/// says which, as the profile import does; a UTF-8 drawing names none.
#[test]
fn an_underlay_read_in_a_code_page_names_it() {
    let source = |layer: &[u8]| {
        let mut source = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\nansi_1250\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n".to_vec();
        source.extend_from_slice(layer);
        source.extend_from_slice(b"\n10\n0\n20\n0\n11\n100\n21\n0\n0\nENDSEC\n0\nEOF\n");
        source
    };
    // "Stĺp" in Windows-1250: ĺ = 0xE5.
    let underlay = read_dxf_underlay(&source(b"St\xE5p"), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.layers, ["Stĺp"]);
    assert_eq!(
        counted(&underlay, "dxf.text-code-page", Some("ANSI_1250")),
        Some(1)
    );
    let underlay =
        read_dxf_underlay(&source("Stĺp".as_bytes()), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.layers, ["Stĺp"]);
    assert!(
        underlay
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code() != "dxf.text-code-page"),
        "{:?}",
        underlay.diagnostics
    );
}

/// Review 2026-10-24: DOS850, DOS852 and DOS866 refused the whole drawing, and a drawing
/// declared ANSI_1252 but written in CP1250 read „žlu\u{9d}oučký“ with no warning.
#[test]
fn dos_code_pages_are_read_and_a_doubtful_code_page_is_warned_of() {
    let source = |code_page: &str, layer: &[u8], text: &[u8]| {
        let mut source = format!("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\n{code_page}\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n").into_bytes();
        for (kind, rest) in [
            ("LINE", b"10\n0\n20\n0\n11\n100\n21\n0\n".as_slice()),
            ("TEXT", b"10\n0\n20\n0\n40\n2\n1\n"),
        ] {
            source.extend_from_slice(format!("0\n{kind}\n8\n").as_bytes());
            source.extend_from_slice(layer);
            source.extend_from_slice(b"\n");
            source.extend_from_slice(rest);
        }
        source.extend_from_slice(text);
        source.extend_from_slice(b"\n0\nENDSEC\n0\nEOF\n");
        source
    };
    let read = |code_page: &str, layer: &[u8], text: &[u8]| {
        read_dxf_underlay(&source(code_page, layer, text), DxfImportOptions::new(None)).unwrap()
    };
    // "Stena žlť" in CP852: ž = 0xA7, ť = 0x9C; "Façade" in CP850: ç = 0x87;
    // "Стена" in CP866.
    for (code_page, layer, expected) in [
        ("DOS852", b"Stena \xA7l\x9C".as_slice(), "Stena žlť"),
        ("DOS850", b"Fa\x87ade", "Façade"),
        ("DOS866", b"\x91\xE2\xA5\xAD\xA0", "Стена"),
    ] {
        let underlay = read(code_page, layer, b"A");
        assert_eq!(underlay.layers, [expected], "{code_page}");
        assert_eq!(
            counted(&underlay, "dxf.text-code-page", Some(code_page)),
            Some(1)
        );
        assert_eq!(
            counted(&underlay, "dxf.text-code-page-doubtful", Some(code_page)),
            None
        );
    }
    // "žluťoučký" in CP1250 (ť = 0x9D, undefined in CP1252) declared ANSI_1252: 1252 is
    // doubted, and the text read in 1250, which reads every byte of it (UL-43).
    let underlay = read("ANSI_1252", b"walls", b"\x9Elu\x9Dou\xE8k\xFD");
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
        Some(1),
        "{:?}",
        underlay.diagnostics
    );
    assert_eq!(underlay.lines.layers()[0].texts()[0].text, "žluťoučký");
    assert_eq!(
        counted(&underlay, "dxf.text-code-page", Some("ANSI_1250")),
        Some(1)
    );
    // Read rightly, the same text is not doubtful.
    let underlay = read("ANSI_1250", b"walls", b"\x9Elu\x9Dou\xE8k\xFD");
    assert_eq!(underlay.lines.layers()[0].texts()[0].text, "žluťoučký");
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1250")),
        None
    );

    // Review 2026-10-26 UL-43: Czech in CP1250 declared ANSI_1252 with no byte 1252 leaves
    // undefined read „Pøíèka Støecha“, „mìsto“ with no warning. It reads in 1250.
    let underlay = read("ANSI_1252", b"P\xF8\xED\xE8ka St\xF8echa", b"m\xECsto");
    assert_eq!(underlay.layers, ["Příčka Střecha"]);
    assert_eq!(underlay.lines.layers()[0].texts()[0].text, "město");
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
        Some(1)
    );
    // Western text in 1252 stays 1252 and undoubted: Danish ø beside æ, å and é, Italian
    // ì, ò, ù and à ending a word, French è and ê within one.
    for (layer, text, expected) in [
        (
            b"S\xF8jle G\xE5rd".as_slice(),
            b"Caf\xE9 \xF8l".as_slice(),
            "Søjle Gård",
        ),
        (b"Citt\xE0 pi\xF9", b"per\xF2 cos\xEC", "Città più"),
        (b"Pi\xE8ce tr\xE8s", b"fen\xEAtre o\xF9", "Pièce très"),
    ] {
        let underlay = read("ANSI_1252", layer, text);
        assert_eq!(underlay.layers, [expected]);
        assert_eq!(
            counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
            None,
            "{expected}: {:?}",
            underlay.diagnostics
        );
    }
}

const BLOCKS: &str = concat!(
    // A door: its leaf and its number, on layer 0, so on the layer of its insert.
    "0\nBLOCK\n8\n0\n2\ndoor\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n900\n21\n0\n",
    "0\nTEXT\n8\n0\n10\n1\n20\n2\n40\n2.5\n1\nD1\n",
    "0\nENDBLK\n",
    // What a linear dimension draws: its line and its text.
    "0\nBLOCK\n8\n0\n2\n*D1\n70\n1\n10\n0\n20\n0\n",
    "0\nLINE\n8\n0\n10\n0\n20\n100\n11\n1250\n21\n100\n",
    "0\nMTEXT\n8\n0\n10\n625\n20\n110\n40\n25\n71\n8\n1\n\\A1;{\\H1.2x;1250}\n",
    "0\nENDBLK\n",
);

#[test]
fn texts_mtext_attributes_and_dimension_text_are_read_on_their_layers() {
    let entities = concat!(
        "0\nLINE\n8\nwalls\n10\n0\n20\n0\n11\n5000\n21\n0\n",
        "0\nTEXT\n8\nlabels\n10\n100\n20\n200\n40\n250\n50\n90\n1\nRoom %%c100\n",
        "0\nMTEXT\n8\nnotes\n10\n3000\n20\n1000\n40\n100\n41\n2000\n71\n5\n",
        "3\n\\fArial|b0|i0;First \n1\nline\\PSecond {\\C1;line}\n",
        "0\nINSERT\n8\ndoors\n2\ndoor\n10\n1000\n20\n0\n41\n2\n42\n2\n50\n90\n66\n1\n",
        "0\nATTRIB\n8\n0\n2\nNO\n1\n12\n10\n1000\n20\n500\n40\n100\n0\nSEQEND\n",
        "0\nDIMENSION\n8\ndims\n2\n*D1\n70\n32\n10\n0\n20\n100\n",
    );
    let underlay = read(BLOCKS, entities);
    assert_eq!(underlay.lines.text_count(), 5);

    let [label] = texts(&underlay, "labels") else {
        panic!("one label")
    };
    assert_eq!(label.text, "Room \u{2300}100");
    assert!(close(label.anchor_mm, [100.0, 200.0]));
    assert_eq!(label.height_mm, 250.0);
    assert!((label.rotation_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-12);

    let [note] = texts(&underlay, "notes") else {
        panic!("one note")
    };
    assert_eq!(note.text, "First line\nSecond line");
    assert!(close(note.anchor_mm, [3000.0, 1000.0]));
    assert_eq!(note.height_mm, 100.0);
    assert_eq!(note.wrap_width_mm, 2000.0);
    assert_eq!(
        (note.horizontal, note.vertical),
        (TextHorizontal::Center, TextVertical::Middle)
    );

    // The block's text placed by the insert, turned a quarter and twice as large, and
    // the attribute where the drawing put it: both on the insert's layer.
    let doors = texts(&underlay, "doors");
    assert_eq!(doors.len(), 2);
    let number = doors.iter().find(|text| text.text == "D1").unwrap();
    assert!(
        close(number.anchor_mm, [996.0, 2.0]),
        "{:?}",
        number.anchor_mm
    );
    assert!((number.height_mm - 5.0).abs() < 1e-12);
    assert!((number.rotation_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    let attribute = doors.iter().find(|text| text.text == "12").unwrap();
    assert!(close(attribute.anchor_mm, [1000.0, 500.0]));
    assert_eq!(attribute.height_mm, 100.0);

    let [dimension] = texts(&underlay, "dims") else {
        panic!("one dimension text")
    };
    assert_eq!(dimension.text, "1250");
    assert!(close(dimension.anchor_mm, [625.0, 110.0]));
    assert_eq!(dimension.height_mm, 25.0);
    assert_eq!(dimension.horizontal, TextHorizontal::Center);
    assert_eq!(dimension.vertical, TextVertical::Bottom);

    // Every text is drawn in the program's font: the fonts it asked for are named.
    assert_eq!(
        counted(
            &underlay,
            "dxf.text-font-substituted",
            Some("style STANDARD")
        ),
        Some(5)
    );
    assert_eq!(
        counted(
            &underlay,
            "dxf.text-font-substituted",
            Some("MTEXT font Arial")
        ),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.insert-attributes-dropped", Some("door")),
        None
    );
    assert!(
        underlay
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code() != "dxf.entity-unreadable"),
        "{:?}",
        underlay.diagnostics
    );
}

#[test]
fn justified_text_sits_at_its_alignment_point_and_what_is_not_drawn_as_asked_is_counted() {
    let entities = concat!(
        // Centered on its middle at the second point.
        "0\nTEXT\n8\na\n10\n0\n20\n0\n11\n50\n21\n60\n40\n10\n72\n1\n73\n2\n1\nmiddle\n",
        // Fit between two points: along them, as wide as they are apart.
        "0\nTEXT\n8\na\n10\n0\n20\n0\n11\n0\n21\n300\n40\n10\n72\n5\n1\nfit\n",
        // An alignment DXF has no such value for: drawn left on the baseline, reported.
        "0\nTEXT\n8\na\n10\n7\n20\n8\n11\n70\n21\n80\n40\n10\n72\n7\n1\nodd\n",
        // A field nothing here evaluates: drawn as its code, reported.
        "0\nTEXT\n8\na\n10\n0\n20\n0\n40\n10\n1\n%<\\AcVar Date>%\n",
        // No height: left out and counted, not drawn at some guess.
        "0\nTEXT\n8\na\n10\n0\n20\n0\n1\nno height\n",
        // An invisible attribute is not drawn; the insert's line is.
        "0\nINSERT\n8\nb\n2\nmark\n10\n0\n20\n0\n66\n1\n",
        "0\nATTRIB\n8\nb\n2\nHIDDEN\n1\nx\n70\n1\n10\n0\n20\n0\n40\n5\n0\nSEQEND\n",
    );
    let blocks = concat!(
        "0\nBLOCK\n8\n0\n2\nmark\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n0\n11\n10\n21\n0\n",
        "0\nENDBLK\n",
    );
    let underlay = read(blocks, entities);
    let [middle, fit, odd, field] = texts(&underlay, "a") else {
        panic!("four texts: {:?}", texts(&underlay, "a"))
    };
    assert!(close(middle.anchor_mm, [50.0, 60.0]));
    assert_eq!(
        (middle.horizontal, middle.vertical),
        (TextHorizontal::Center, TextVertical::Middle)
    );
    assert!(close(fit.anchor_mm, [0.0, 0.0]));
    assert_eq!(fit.fit, TextFit::Width(300.0));
    assert!((fit.rotation_rad - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert!(close(odd.anchor_mm, [7.0, 8.0]));
    assert_eq!(
        (odd.horizontal, odd.vertical),
        (TextHorizontal::Left, TextVertical::Baseline)
    );
    assert_eq!(field.text, "%<\\AcVar Date>%");
    assert_eq!(texts(&underlay, "b"), []);
    assert_eq!(underlay.lines.layers()[1].segment_count(), 1);

    assert_eq!(
        counted(
            &underlay,
            "dxf.text-justification-unsupported",
            Some("TEXT 7/0")
        ),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.text-field-not-evaluated", Some("TEXT")),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.attribute-invisible", Some("HIDDEN")),
        Some(1)
    );
    let unreadable = underlay
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == "dxf.entity-unreadable")
        .unwrap();
    assert_eq!(unreadable.count(), 1);
    assert!(unreadable.subject().unwrap().starts_with("TEXT: "));
}

#[test]
fn texts_travel_with_the_lines_and_are_no_snap_points() {
    let entities = concat!(
        "0\nLINE\n8\nwalls\n10\n0\n20\n0\n11\n1000\n21\n0\n",
        "0\nTEXT\n8\nwalls\n10\n500\n20\n500\n40\n100\n1\nKitchen\n",
        "0\nTEXT\n8\nlabels\n10\n2000\n20\n2000\n40\n100\n1\nHall\n",
    );
    let underlay = read("", entities);
    let bytes = underlay.lines.encode();
    let decoded = UnderlayLines::decode(&bytes).unwrap();
    assert_eq!(decoded, underlay.lines);
    assert_eq!(decoded.text_count(), 2);
    assert!(bytes.starts_with(b"KUNDRLY6"));
    assert!(UnderlayLines::decode(&bytes[..bytes.len() - 1]).is_err());

    // Snapping meets the wall, never a text: nothing near either text's anchor or
    // letters, on a layer of lines or one of texts alone.
    let snaps = UnderlaySnapIndex::new(&decoded);
    assert!(snaps.snaps([500.0, 0.0], 10.0, |_| true).endpoint.is_none());
    assert!(snaps.snaps([500.0, 0.0], 10.0, |_| true).midpoint.is_some());
    for point in [[500.0, 500.0], [550.0, 550.0], [2000.0, 2000.0]] {
        assert_eq!(snaps.snaps(point, 100.0, |_| true).iter().count(), 0);
    }
}

#[test]
fn texts_in_national_grid_coordinates_move_with_the_lines() {
    let [x, y] = [-560_123_456.0, -1_210_987_654.0];
    let entities = format!(
        "0\nLINE\n8\nplan\n10\n{x}\n20\n{y}\n11\n{}\n21\n{y}\n0\nTEXT\n8\nplan\n10\n{}\n20\n{}\n40\n100\n1\nA\n0\nTEXT\n8\nplan\n10\n0\n20\n0\n40\n100\n1\nfar\n",
        x + 1000.0,
        x + 10.0,
        y + 20.0,
    );
    let underlay = read("", &entities);
    let [dx, dy] = underlay.offset_mm;
    let [text] = texts(&underlay, "plan") else {
        panic!("the far text is left out")
    };
    assert!(close(
        [text.anchor_mm[0] + dx, text.anchor_mm[1] + dy],
        [x + 10.0, y + 20.0]
    ));
    assert_eq!(
        counted(&underlay, "dxf.texts-outside-envelope", None),
        Some(1)
    );
}
