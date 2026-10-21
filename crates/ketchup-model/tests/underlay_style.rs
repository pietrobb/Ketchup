//! Line styles of a DXF drawing read as an underlay: each line keeps the color, dash
//! pattern and lineweight the drawing gives it, by its own groups, its layer or the insert
//! that places it; what cannot be drawn as asked is reported.
use ketchup_model::import::{DxfImportOptions, DxfUnderlay, read_dxf_underlay};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::style::LineStyle;

const TABLES: &str = concat!(
    "0\nSECTION\n2\nTABLES\n",
    "0\nTABLE\n2\nLTYPE\n70\n2\n",
    "0\nLTYPE\n2\nDASHED\n70\n0\n3\n__ __\n72\n65\n73\n2\n40\n15\n49\n10\n74\n0\n49\n-5\n74\n0\n",
    "0\nLTYPE\n2\nFENCE\n70\n0\n3\n-o-\n72\n65\n73\n2\n40\n2\n49\n1\n74\n0\n49\n-1\n74\n4\n75\n0\n",
    "0\nENDTAB\n",
    "0\nTABLE\n2\nLAYER\n70\n2\n",
    "0\nLAYER\n2\nWALLS\n70\n0\n62\n1\n6\nDASHED\n370\n50\n",
    "0\nLAYER\n2\naxes\n70\n0\n62\n-5\n6\nCONTINUOUS\n370\n-3\n",
    "0\nENDTAB\n",
    "0\nENDSEC\n",
);

const BLOCKS: &str = concat!(
    "0\nSECTION\n2\nBLOCKS\n",
    // A frame drawn ByBlock: it takes the color and linetype of its insert.
    "0\nBLOCK\n8\n0\n2\nframe\n70\n0\n10\n0\n20\n0\n",
    "0\nLINE\n8\n0\n62\n0\n6\nBYBLOCK\n10\n0\n20\n0\n11\n100\n21\n0\n",
    "0\nENDBLK\n",
    "0\nENDSEC\n",
);

fn read(entities: &str) -> DxfUnderlay {
    read_with("2", TABLES, entities)
}

fn read_with(ltscale: &str, tables: &str, entities: &str) -> DxfUnderlay {
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n9\n$LTSCALE\n40\n{ltscale}\n9\n$LWDEFAULT\n370\n18\n0\nENDSEC\n{tables}{BLOCKS}0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap()
}

/// The style of each polyline of `layer` from left to right, by its first point.
fn styles<'a>(underlay: &'a DxfUnderlay, layer: &str) -> Vec<&'a LineStyle> {
    let index = underlay
        .layers
        .iter()
        .position(|name| name == layer)
        .unwrap_or_else(|| panic!("no layer {layer} in {:?}", underlay.layers));
    let mut polylines = underlay.lines.layers()[index]
        .styled_polylines()
        .collect::<Vec<_>>();
    polylines.sort_by(|left, right| left.1[0][0].total_cmp(&right.1[0][0]));
    polylines
        .into_iter()
        .map(|(style, _)| underlay.lines.style(style))
        .collect()
}

fn counted(underlay: &DxfUnderlay, code: &str, subject: &str) -> Option<u32> {
    underlay
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code() == code && diagnostic.subject() == Some(subject))
        .map(|diagnostic| diagnostic.count())
}

fn style(color: Option<[u8; 3]>, weight_hmm: u16, pattern_mm: &[f64]) -> LineStyle {
    LineStyle {
        color,
        weight_hmm,
        pattern_mm: pattern_mm.to_vec(),
    }
}

#[test]
fn each_line_keeps_the_color_pattern_and_weight_its_drawing_gives_it() {
    let underlay = read(concat!(
        // By its layer, which the table names in capitals: red, DASHED at $LTSCALE 2, 0.50 mm.
        "0\nLINE\n8\nWalls\n10\n0\n20\n0\n11\n100\n21\n0\n",
        // Its own: green ACI 3, continuous at 1.00 mm.
        "0\nLINE\n8\nWalls\n62\n3\n6\nCONTINUOUS\n370\n100\n10\n200\n20\n0\n11\n300\n21\n0\n",
        // A true color over its ACI, DASHED at its own scale 0.5, the default weight.
        "0\nLINE\n8\nWalls\n62\n1\n420\n1193046\n6\nDASHED\n48\n0.5\n370\n-3\n10\n400\n20\n0\n11\n500\n21\n0\n",
        // ACI 7, the foreground: drawn in the underlay's own color.
        "0\nLINE\n8\nWalls\n62\n7\n10\n600\n20\n0\n11\n700\n21\n0\n",
        // On a layer turned off (color -5): blue, the default weight.
        "0\nLINE\n8\naxes\n10\n0\n20\n100\n11\n0\n21\n200\n",
        // A block drawn ByBlock, inserted magenta, DASHED, twice as large.
        "0\nINSERT\n8\naxes\n2\nframe\n62\n6\n6\nDASHED\n41\n2\n42\n2\n10\n1000\n20\n0\n",
    ));
    assert_eq!(
        styles(&underlay, "Walls"),
        [
            &style(Some([255, 0, 0]), 50, &[20.0, -10.0]),
            &style(Some([0, 255, 0]), 100, &[]),
            &style(Some([0x12, 0x34, 0x56]), 18, &[10.0, -5.0]),
            &style(None, 50, &[20.0, -10.0]),
        ]
    );
    assert_eq!(
        styles(&underlay, "axes"),
        [
            &style(Some([0, 0, 255]), 18, &[]),
            &style(Some([255, 0, 255]), 18, &[40.0, -20.0]),
        ]
    );
    // Read, the style groups and variables are not reported ignored.
    assert!(
        underlay.diagnostics.iter().all(|diagnostic| {
            diagnostic.code() != "dxf.entity-group-ignored"
                && diagnostic.code() != "dxf.section-ignored"
                && diagnostic.subject() != Some("$LTSCALE")
        }),
        "{:?}",
        underlay.diagnostics
    );
    // The styles travel with the lines in their blob.
    let decoded = UnderlayLines::decode(&underlay.lines.encode()).unwrap();
    assert_eq!(decoded, underlay.lines);
}

#[test]
fn dashed_lines_start_their_pattern_anew_and_continuous_ones_join() {
    let underlay = read(concat!(
        "0\nLINE\n8\nWalls\n10\n0\n20\n0\n11\n100\n21\n0\n",
        "0\nLINE\n8\nWalls\n10\n100\n20\n0\n11\n100\n21\n100\n",
        "0\nLINE\n8\naxes\n10\n0\n20\n0\n11\n0\n21\n100\n",
        "0\nLINE\n8\naxes\n10\n0\n20\n100\n11\n0\n21\n200\n",
    ));
    let polylines = |layer: usize| underlay.lines.layers()[layer].polylines().count();
    let [walls, axes] = ["Walls", "axes"].map(|name| {
        underlay
            .layers
            .iter()
            .position(|layer| layer == name)
            .unwrap()
    });
    assert_eq!((polylines(walls), polylines(axes)), (2, 1));
}

#[test]
fn what_a_style_cannot_draw_as_asked_is_reported() {
    let underlay = read(concat!(
        // A linetype the drawing does not define: drawn continuous.
        "0\nLINE\n8\nWalls\n6\nPHANTOM\n10\n0\n20\n0\n11\n100\n21\n0\n",
        // A complex linetype: its dashes are drawn, its shape is not.
        "0\nLINE\n8\nWalls\n6\nFENCE\n10\n200\n20\n0\n11\n300\n21\n0\n",
        // A color, lineweight and scale that are none: drawn as its layer, at scale 1.
        "0\nLINE\n8\nWalls\n62\n300\n370\n999\n48\n-1\n10\n400\n20\n0\n11\n500\n21\n0\n",
    ));
    assert_eq!(
        styles(&underlay, "Walls"),
        [
            &style(Some([255, 0, 0]), 50, &[]),
            &style(Some([255, 0, 0]), 50, &[2.0, -2.0]),
            &style(Some([255, 0, 0]), 50, &[20.0, -10.0]),
        ]
    );
    assert_eq!(counted(&underlay, "dxf.color-unknown", "300"), Some(1));
    assert_eq!(counted(&underlay, "dxf.lineweight-unknown", "999"), Some(1));
    assert_eq!(
        counted(&underlay, "dxf.linetype-scale-invalid", "-1"),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.linetype-undefined", "PHANTOM"),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.linetype-shapes-not-drawn", "FENCE"),
        Some(1)
    );

    // A drawing scale that is none, and table records without a name.
    let underlay = read_with(
        "-2",
        concat!(
            "0\nSECTION\n2\nTABLES\n",
            "0\nTABLE\n2\nLTYPE\n70\n1\n0\nLTYPE\n70\n0\n49\n1\n0\nENDTAB\n",
            "0\nTABLE\n2\nLAYER\n70\n1\n0\nLAYER\n70\n0\n62\n1\n0\nENDTAB\n",
            "0\nENDSEC\n",
        ),
        "0\nLINE\n8\nWalls\n10\n0\n20\n0\n11\n100\n21\n0\n",
    );
    assert_eq!(
        counted(&underlay, "dxf.linetype-scale-invalid", "-2"),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.table-record-unnamed", "LAYER"),
        Some(1)
    );
    assert_eq!(
        counted(&underlay, "dxf.table-record-unnamed", "LTYPE"),
        Some(1)
    );
}
