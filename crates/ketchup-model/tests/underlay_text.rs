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

/// Review 2026-11-02 UL-121: a pre-2007 drawing in Windows-1252 whose bytes happen to be
/// valid UTF-8 was read as UTF-8: „Rohr Ø½“ (0xD8 0xBD) read „Rohr ؽ“. UTF-8 of letters
/// its code page or the Latin script has is still read as UTF-8.
#[test]
fn a_code_page_drawing_that_is_valid_utf8_by_chance_is_read_in_its_code_page() {
    let source = |layer: &[u8]| {
        let mut source = b"0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\nANSI_1252\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n0\nLINE\n8\n".to_vec();
        source.extend_from_slice(layer);
        source.extend_from_slice(b"\n10\n0\n20\n0\n11\n100\n21\n0\n0\nENDSEC\n0\nEOF\n");
        source
    };
    let underlay =
        read_dxf_underlay(&source(b"Rohr \xD8\xBD"), DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.layers, ["Rohr Ø½"]);
    assert_eq!(
        counted(&underlay, "dxf.text-code-page", Some("ANSI_1252")),
        Some(1)
    );
    for utf8 in ["Rohr Ø½", "Příčka", "Größe €"] {
        let underlay =
            read_dxf_underlay(&source(utf8.as_bytes()), DxfImportOptions::new(None)).unwrap();
        assert_eq!(underlay.layers, [utf8]);
    }
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
    // doubted, and the text read in 1250, which reads every byte of it (UL-43), as the
    // word is on its layer too: one Central European word alone is doubted, not obeyed
    // (review 2026-11-04 UL-140).
    let underlay = read(
        "ANSI_1252",
        b"\x9Elu\x9Dou\xE8k\xFD",
        b"\x9Elu\x9Dou\xE8k\xFD",
    );
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
    // Review 2026-11-01 UL-100: Slovak in CP1250 declared ANSI_1252 read „Obývaèka“ and
    // „Kúpe¾“ with no warning, as ý beside è is no Western language's word and ¾ follows
    // no letter in one. Each word is warned of; one alone, though written on two
    // entities, does not turn the drawing, as a word votes once (review 2026-11-04 r19
    // P3); two do.
    for (layer, expected) in [
        (b"Ob\xFDva\xE8ka".as_slice(), "Obývaèka"),
        (b"K\xFApe\xBE", "Kúpe¾"),
        // „Dlažba“ reads alike in both.
        (b"Dla\x9Eba \xE8ist\xE1", "Dlažba èistá"),
        // Review 2026-11-02 UL-100: each of these alone was not warned of. No Italian word
        // has k or y, nor ends in an accented vowel after e; no Danish one has c or z; no
        // Western word begins with è, as the abbreviation „č.“ would.
        (b"Predsie\xF2", "Predsieò"),
        (b"Kuchyn\xEC", "Kuchynì"),
        (b"\xD8EZ A-A", "ØEZ A-A"),
        (b"St\xF8echa", "Støecha"),
        (b"Izba \xE8. 1", "Izba è. 1"),
        (b"Kuchyn\xEC P\xF8eds\xED\xF2", "Kuchyně Předsíň"),
    ] {
        let underlay = read("ANSI_1252", layer, b"A");
        assert_eq!(underlay.layers, [expected]);
        assert_eq!(
            counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
            Some(1),
            "{expected}: {:?}",
            underlay.diagnostics
        );
    }
    // Western text in 1252 stays 1252 and undoubted: Danish ø beside æ, å and é, Italian
    // ì, ò, ù and à ending a word, French è and ê within one, Spanish, German and
    // Icelandic, and fractions and powers after a number or a unit (UL-100).
    for (layer, text, expected) in [
        (
            b"Ba\xF1o se\xF1or".as_slice(),
            b"\xBFQu\xE9?".as_slice(),
            "Baño señor",
        ),
        (
            b"Gr\xF6\xDFe T\xFCr",
            b"m\xB3 m\xB2 3\xBE\" 1\xBCin",
            "Größe Tür",
        ),
        (b"\xDEj\xF3\xF0vegur", b"N\xFDr", "Þjóðvegur"),
        (
            b"S\xF8jle G\xE5rd".as_slice(),
            b"Caf\xE9 \xF8l".as_slice(),
            "Søjle Gård",
        ),
        (b"Citt\xE0 pi\xF9", b"per\xF2 cos\xEC", "Città più"),
        (b"Pi\xE8ce tr\xE8s", b"fen\xEAtre o\xF9", "Pièce très"),
        // Review 2026-11-02 UL-120: one „Nº“, „1ª“, „45º“, „G¾“ or „m³“ turned a whole
        // Western drawing to 1250 („Bańo seńor“, „Cittŕ“, „Coraçăo“, „Gľ“, „mł“).
        (b"Ba\xF1o N\xBA 3", b"1\xAA se\xF1or", "Baño Nº 3"),
        (b"Citt\xE0 45\xBA", b"Cora\xE7\xE3o", "Città 45º"),
        (b"Anschluss G\xBE", b"R\xBE m\xB3", "Anschluss G¾"),
        // A word that reads as Central European is outvoted by the Western ones.
        (b"Cora\xE7\xE3o \xBFQu\xE9", b"Dvo\xF8\xE1k", "Coração ¿Qué"),
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

/// A pre-2007 drawing naming `code_page`, with one TEXT of each of `texts` on layer
/// `labels`, `bom` first if given.
fn texts_drawing(bom: &[u8], code_page: &str, texts: &[&[u8]]) -> Vec<u8> {
    let mut source = bom.to_vec();
    source.extend_from_slice(format!("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$DWGCODEPAGE\n3\n{code_page}\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n").as_bytes());
    for text in texts {
        source.extend_from_slice(b"0\nTEXT\n8\nlabels\n10\n0\n20\n0\n40\n2\n1\n");
        source.extend_from_slice(text);
        source.push(b'\n');
    }
    source.extend_from_slice(b"0\nENDSEC\n0\nEOF\n");
    source
}

fn read_texts(underlay: &DxfUnderlay) -> Vec<&str> {
    texts(underlay, "labels")
        .iter()
        .map(|text| text.text.as_str())
        .collect()
}

/// Review 2026-11-04 UL-140: one superscript, or one byte Windows-1252 leaves undefined,
/// turned a whole Western drawing to Windows-1250: „m²“ read „m˛“, „Rohr Ø 100“ „Rohr Ř
/// 100“, „Pièce à vivre“ „Pičce ŕ vivre“. Signs vote for the West, and the drawing turns
/// only when Central European words outvote the Western ones well.
#[test]
fn one_sign_or_undefined_byte_does_not_turn_a_western_drawing_central_european() {
    let read = |texts: &[&[u8]]| {
        read_dxf_underlay(
            &texts_drawing(b"", "ANSI_1252", texts),
            DxfImportOptions::new(None),
        )
        .unwrap()
    };
    let doubted = |underlay: &DxfUnderlay| {
        counted(underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252"))
    };
    let western: [&[u8]; 4] = [
        b"Fenster\xB9 Achse A\xB3",
        b"20 m\xB2",
        b"Rohr \xD8 100",
        b"Pi\xE8ce \xE0 vivre",
    ];
    let underlay = read(&western);
    assert_eq!(
        read_texts(&underlay),
        ["Fenster¹ Achse A³", "20 m²", "Rohr Ø 100", "Pièce à vivre"]
    );
    assert_eq!(doubted(&underlay), None, "{:?}", underlay.diagnostics);
    // One Slovak word among signs is outvoted; alone it is doubted, not obeyed.
    let underlay = read(&[b"K\xFApe\xBE", b"20 m\xB2 \xBD"]);
    assert_eq!(read_texts(&underlay), ["Kúpe¾", "20 m² ½"]);
    assert_eq!(doubted(&underlay), None, "{:?}", underlay.diagnostics);
    let underlay = read(&[b"K\xFApe\xBE", b"Pi\xE8ce"]);
    assert_eq!(read_texts(&underlay), ["Kúpe¾", "Pièce"]);
    assert_eq!(doubted(&underlay), Some(1), "{:?}", underlay.diagnostics);
    // A byte 1252 leaves undefined is doubted, and alone does not turn the drawing.
    let underlay = read(&[b"Pi\xE8ce \x8D"]);
    assert_eq!(read_texts(&underlay), ["Pièce \u{8D}"]);
    assert_eq!(doubted(&underlay), Some(1), "{:?}", underlay.diagnostics);
    // Two Slovak words, and none Western, do.
    let underlay = read(&[b"K\xFApe\xBE", b"Ob\xFDva\xE8ka"]);
    assert_eq!(read_texts(&underlay), ["Kúpeľ", "Obývačka"]);
    assert_eq!(
        counted(&underlay, "dxf.text-code-page", Some("ANSI_1250")),
        Some(1)
    );
    // Review 2026-11-04 r19 P3: a word votes once however often it is written, so one
    // Slovak word repeated, as a layer name is on each entity, neither turns the drawing
    // nor outvotes the Western words.
    let slovak: &[u8] = b"Ob\xFDva\xE8ka";
    let underlay = read(&[slovak; 3]);
    assert_eq!(read_texts(&underlay), ["Obývaèka"; 3]);
    assert_eq!(doubted(&underlay), Some(1), "{:?}", underlay.diagnostics);
    let underlay = read(&[slovak, slovak, slovak, b"Pi\xE8ce"]);
    assert_eq!(
        read_texts(&underlay),
        ["Obývaèka", "Obývaèka", "Obývaèka", "Pièce"]
    );
}

/// Review 2026-11-04 r19 P3: the character set a text's font names (`\f…|c238;`, the
/// East European one, Windows-1250) was passed over. It votes as a Central European
/// word does, so with one word it turns the drawing.
#[test]
fn a_font_of_the_east_european_character_set_votes_for_windows_1250() {
    let read = |texts: &[&[u8]]| {
        let source = texts_drawing(b"", "ANSI_1252", texts);
        let text = b"0\nTEXT\n";
        let mut mtexts = Vec::new();
        let mut rest = source.as_slice();
        while let Some(at) = rest.windows(text.len()).position(|window| window == text) {
            mtexts.extend_from_slice(&rest[..at]);
            mtexts.extend_from_slice(b"0\nMTEXT\n");
            rest = &rest[at + text.len()..];
        }
        mtexts.extend_from_slice(rest);
        read_dxf_underlay(&mtexts, DxfImportOptions::new(None)).unwrap()
    };
    let doubted = |underlay: &DxfUnderlay| {
        counted(underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252"))
    };
    let underlay = read(&[b"{\\fArial|b0|i0|c238|p34;Kuchyn\xEC}"]);
    assert_eq!(read_texts(&underlay), ["Kuchyně"]);
    assert_eq!(doubted(&underlay), Some(1), "{:?}", underlay.diagnostics);
    // Another character set does not.
    let underlay = read(&[b"{\\fArial|b0|i0|c0|p34;Kuchyn\xEC}"]);
    assert_eq!(read_texts(&underlay), ["Kuchynì"]);
    // Nor does it turn a Western drawing on its own.
    let underlay = read(&[b"{\\fArial|b0|i0|c238|p34;Pi\xE8ce tr\xE8s}"]);
    assert_eq!(read_texts(&underlay), ["Pièce très"]);
    // A text style's font names its character set in the second byte of its flags.
    let styled = |charset: u32| {
        let source = texts_drawing(b"", "ANSI_1252", &[b"Kuchyn\xEC"]);
        let entities = b"0\nENDSEC\n0\nSECTION\n2\nENTITIES\n";
        let at = source
            .windows(entities.len())
            .position(|window| window == entities)
            .unwrap();
        let mut styled = source[..at].to_vec();
        styled.extend_from_slice(
            format!(
                "0\nENDSEC\n0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n70\n1\n0\nSTYLE\n2\n\
                 Standard\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n2.5\n3\narial.ttf\n1001\n\
                 ACAD\n1000\nArial\n1071\n{}\n0\nENDTAB\n",
                (charset << 8) | 34
            )
            .as_bytes(),
        );
        styled.extend_from_slice(&source[at..]);
        read_dxf_underlay(&styled, DxfImportOptions::new(None)).unwrap()
    };
    assert_eq!(read_texts(&styled(238)), ["Kuchyně"]);
    assert_eq!(read_texts(&styled(0)), ["Kuchynì"]);
}

/// Review 2026-11-04 UL-141: one character its code page cannot write sent a whole UTF-8
/// drawing through the code page, unwarned: „Größe ⌀ 100“ read „GrÃ¶ÃŸe âŒ€ 100“. A
/// character of three or four bytes is UTF-8; of two, the most decide, and a drawing both
/// readings fit is warned of.
#[test]
fn a_utf8_drawing_naming_a_code_page_is_read_as_utf8_by_most_of_its_characters() {
    let read = |bom: &[u8], texts: &[&str]| {
        let texts = texts.iter().map(|text| text.as_bytes()).collect::<Vec<_>>();
        read_dxf_underlay(
            &texts_drawing(bom, "ANSI_1252", &texts),
            DxfImportOptions::new(None),
        )
        .unwrap()
    };
    let underlay = read(b"", &["Größe ⌀ 100", "Küche"]);
    assert_eq!(read_texts(&underlay), ["Größe ⌀ 100", "Küche"]);
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
        None
    );
    // Greek 1252 has no letter for, among German it does: UTF-8, warned of.
    let underlay = read(b"", &["Größe Küche", "α"]);
    assert_eq!(read_texts(&underlay), ["Größe Küche", "α"]);
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
        Some(1),
        "{:?}",
        underlay.diagnostics
    );
    assert_eq!(
        counted(&underlay, "dxf.text-code-page", Some("UTF-8")),
        Some(1)
    );

    // Review 2026-11-04 UL-142: the byte order mark of a UTF-8 drawing that names a code
    // page did not fit it, and „Küche“ read „KÃ¼che“.
    let underlay = read(b"\xEF\xBB\xBF", &["Küche"]);
    assert_eq!(read_texts(&underlay), ["Küche"]);
}

/// Review 2026-11-04 r19 UL-160: a character of two bytes its code page has no letter
/// for counted against UTF-8, so a UTF-8 drawing of a few Greek letters was read in the
/// code page, unwarned: „Δh“ read „Î”h“, „α = 30°“ „Î± = 30Â°“. Read in a code page, a
/// character of two bytes is a capital letter followed by a sign („Î”“, „Â°“), which no
/// text writes but „Ø½“; it is UTF-8, warned of where the code page could not write it.
#[test]
fn utf8_letters_a_code_page_reads_as_a_letter_and_a_sign_are_utf8() {
    let read = |code_page: &str, texts: &[&[u8]]| {
        read_dxf_underlay(
            &texts_drawing(b"", code_page, texts),
            DxfImportOptions::new(None),
        )
        .unwrap()
    };
    for probe in ["Δh", "α = 30°"] {
        let underlay = read("ANSI_1252", &[probe.as_bytes()]);
        assert_eq!(read_texts(&underlay), [probe]);
        assert_eq!(
            counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
            Some(1),
            "{probe}: {:?}",
            underlay.diagnostics
        );
    }
    // UTF-8 of Czech letters under a Central European header: „ř“ is „Ĺ™“ in 1250.
    let underlay = read("ANSI_1250", &["Kuchyně", "Předsíň"].map(str::as_bytes));
    assert_eq!(read_texts(&underlay), ["Kuchyně", "Předsíň"]);
    // A pipe size in Windows-1252 that is valid UTF-8 stays in its code page.
    let underlay = read("ANSI_1252", &[b"Rohr \xD8\xBD"]);
    assert_eq!(read_texts(&underlay), ["Rohr Ø½"]);
    // Capital letters of Windows-1250 that are valid UTF-8 stay in it: „NÍŽE“.
    let underlay = read("ANSI_1250", &[b"N\xCD\x8EE"]);
    assert_eq!(read_texts(&underlay), ["NÍŽE"]);
    // Review 2026-11-04 r19 capability finding: UTF-8 letters the code page reads as two
    // letters, „ε“ as „Îµ“ and „Ке“ as „ÐšÐµ“, fit neither way; both readings are
    // possible, so the code page is doubted.
    for (probe, as_1252) in [("ε = 0,5", "Îµ = 0,5"), ("Ке", "ÐšÐµ")] {
        let underlay = read("ANSI_1252", &[probe.as_bytes()]);
        assert!(
            [probe, as_1252].contains(&read_texts(&underlay)[0]),
            "{probe}"
        );
        assert_eq!(
            counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1252")),
            Some(1),
            "{probe}: {:?}",
            underlay.diagnostics
        );
    }
    assert_eq!(
        counted(&underlay, "dxf.text-code-page-doubtful", Some("ANSI_1250")),
        Some(1)
    );
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
    let decoded = UnderlayLines::decode(&bytes, usize::MAX).unwrap();
    assert_eq!(decoded, underlay.lines);
    assert_eq!(decoded.text_count(), 2);
    assert!(bytes.starts_with(b"KUNDRLY6"));
    assert!(UnderlayLines::decode(&bytes[..bytes.len() - 1], usize::MAX).is_err());

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
