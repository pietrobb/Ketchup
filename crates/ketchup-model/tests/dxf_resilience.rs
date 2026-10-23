//! One entity or block a drawing gets wrong is left out, not the drawing: a dimension
//! whose block is missing, a polygon with a repeated point and a hatch without area each
//! leave the rest imported, and the report counts what was left out by why, with the
//! first 20 handles or block names. Only a drawing with nothing readable left is refused.

use ketchup_model::document::{DocumentStore, ProposalContext};
use ketchup_model::import::{
    DxfImportError, DxfImportOptions, ImportDiagnostic, inspect_dxf, plan_dxf_import,
    read_dxf_underlay,
};

fn drawing(blocks: &str, entities: &str) -> Vec<u8> {
    format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    )
    .into_bytes()
}

fn line(handle: &str, [x0, y0, x1, y1]: [i32; 4]) -> String {
    format!("0\nLINE\n5\n{handle}\n8\nwalls\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n")
}

fn dimension(handle: &str, block: &str) -> String {
    format!("0\nDIMENSION\n5\n{handle}\n8\ndims\n2\n{block}\n10\n0\n20\n0\n70\n32\n")
}

/// A closed lightweight polyline through `points`.
fn polygon(handle: &str, points: &[[i32; 2]]) -> String {
    let vertices: String = points
        .iter()
        .map(|[x, y]| format!("10\n{x}\n20\n{y}\n"))
        .collect();
    format!(
        "0\nLWPOLYLINE\n5\n{handle}\n8\nrooms\n90\n{}\n70\n1\n{vertices}",
        points.len()
    )
}

/// A solid hatch whose one boundary is the closed polyline through `points`.
fn hatch(handle: &str, points: &[[i32; 2]]) -> String {
    let vertices: String = points
        .iter()
        .map(|[x, y]| format!("10\n{x}\n20\n{y}\n"))
        .collect();
    format!(
        "0\nHATCH\n5\n{handle}\n8\nfill\n10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n91\n1\n\
         92\n3\n72\n0\n73\n1\n93\n{}\n{vertices}97\n0\n75\n0\n76\n1\n98\n0\n",
        points.len()
    )
}

fn circle(handle: &str, radius: i32) -> String {
    format!("0\nCIRCLE\n5\n{handle}\n8\nbores\n10\n500\n20\n500\n40\n{radius}\n")
}

/// The count and subject of the report entry `code` whose subject starts with `what`.
fn entry<'a>(diagnostics: &'a [ImportDiagnostic], code: &str, what: &str) -> (u32, &'a str) {
    let found = diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.code() == code
                && diagnostic
                    .subject()
                    .is_some_and(|subject| subject.starts_with(what))
        })
        .unwrap_or_else(|| panic!("no {code} {what:?} in {diagnostics:#?}"));
    (found.count(), found.subject().unwrap())
}

/// Imports `source` as profiles into an empty document as one Undo step.
fn import_profiles(source: &[u8]) -> usize {
    let mut document = DocumentStore::new();
    let batch = plan_dxf_import(
        &document.current(),
        source,
        "drawing.dxf",
        DxfImportOptions::new(None),
    )
    .unwrap();
    let proposal = document
        .prepare_proposal_with_context(batch, ProposalContext::drawing_import())
        .unwrap();
    document.commit_verified_proposal(&proposal).unwrap();
    assert_eq!(document.visible_undo_steps(), 1);
    document.current().occurrences().count()
}

const VALID_DIMENSION_BLOCK: &str = "0\nBLOCK\n2\n*D1\n70\n1\n10\n0\n20\n0\n0\nLINE\n8\n0\n10\n0\n20\n-500\n11\n4000\n21\n-500\n0\nENDBLK\n";

#[test]
fn a_dimension_whose_block_is_missing_is_left_out_and_named() {
    // The real failure: a dimension inside a block whose own block is missing refused the
    // block, and with it the drawing.
    let frame = concat!(
        "0\nBLOCK\n2\nframe\n70\n0\n10\n0\n20\n0\n",
        "0\nLINE\n8\n0\n10\n0\n20\n9000\n11\n4000\n21\n9000\n",
        "0\nDIMENSION\n8\ndims\n2\n*D5\n10\n0\n20\n0\n70\n32\n",
        "0\nENDBLK\n",
    );
    let source = drawing(
        &format!("{VALID_DIMENSION_BLOCK}{frame}"),
        &[
            line("1A", [0, 0, 4000, 0]),
            dimension("2A", "*D1"),
            dimension("2B", "*D3"),
            dimension("2C", "*D4"),
            "0\nINSERT\n5\n3A\n8\n0\n2\nframe\n10\n0\n20\n0\n".to_owned(),
        ]
        .concat(),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    // The line, the dimension of *D1 and the frame's line.
    assert_eq!(parsed.profiles().len(), 3);
    assert_eq!(
        entry(
            parsed.diagnostics(),
            "dxf.dimension-block-missing",
            "DIMENSION"
        ),
        (
            3,
            "DIMENSION whose drawn block is undefined, unreadable or empty; blocks: *D5, *D3, *D4"
        )
    );
    assert_eq!(import_profiles(&source), 3);

    let underlay = read_dxf_underlay(&source, DxfImportOptions::new(None)).unwrap();
    assert_eq!(underlay.lines.segment_count(), 3);
    assert_eq!(
        entry(
            &underlay.diagnostics,
            "dxf.dimension-block-missing",
            "DIMENSION"
        )
        .0,
        3
    );
}

#[test]
fn a_repeated_point_is_cleaned_out_of_a_polygon_which_is_kept() {
    let source = drawing(
        "",
        &[
            // The last point repeats the first, as many drawings close a polygon.
            polygon("4A", &[[0, 0], [4000, 0], [4000, 3000], [0, 3000], [0, 0]]),
            // A point drawn twice in the middle.
            polygon(
                "4B",
                &[[5000, 0], [9000, 0], [9000, 0], [9000, 3000], [5000, 3000]],
            ),
        ]
        .concat(),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    assert_eq!(parsed.profiles().len(), 2);
    for profile in parsed.profiles() {
        assert!(profile.closed());
        assert_eq!(profile.segments().len(), 4, "{:?}", profile.segments());
    }
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.repeated-vertex-removed", "rooms"),
        (2, "rooms")
    );
    assert!(
        !parsed
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.code() == "dxf.entity-unreadable")
    );
    assert_eq!(import_profiles(&source), 2);
}

#[test]
fn a_hatch_without_area_is_left_out_and_the_drawing_imported() {
    let source = drawing(
        "",
        &[
            line("1A", [0, 0, 4000, 0]),
            // Three points on one line: closed, it encloses nothing.
            hatch("5A", &[[0, 1000], [2000, 1000], [4000, 1000]]),
            // Three times one point.
            hatch("5B", &[[0, 2000], [0, 2000], [0, 2000]]),
        ]
        .concat(),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    assert_eq!(parsed.profiles().len(), 1);
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.profile-invalid", "DXF"),
        (
            1,
            "DXF contains a closed profile that encloses no area or crosses itself; handles: 5A"
        )
    );
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.entity-unreadable", "HATCH"),
        (
            1,
            "HATCH: DXF contains degenerate zero-length geometry; handles: 5B"
        )
    );
    assert_eq!(import_profiles(&source), 1);
}

#[test]
fn a_drawing_of_valid_and_faulty_entities_keeps_the_valid_and_names_twenty_faulty() {
    let mut entities = vec![
        line("1A", [0, 0, 4000, 0]),
        polygon("4A", &[[0, 0], [4000, 0], [4000, 3000], [0, 3000], [0, 0]]),
        dimension("2A", "*D1"),
        dimension("2B", "*D9"),
        hatch("5A", &[[0, 1000], [2000, 1000], [4000, 1000]]),
        "0\nINSERT\n5\n3A\n8\n0\n2\nnowhere\n10\n0\n20\n0\n".to_owned(),
    ];
    // Twenty-two circles without a radius: the report names the first twenty.
    entities.extend((1..=22).map(|index| circle(&format!("C{index}"), 0)));
    let source = drawing(VALID_DIMENSION_BLOCK, &entities.concat());

    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    // The line, the polygon and the dimension of *D1.
    assert_eq!(parsed.profiles().len(), 3);
    let handles = (1..=20)
        .map(|index| format!("C{index}"))
        .collect::<Vec<_>>()
        .join(", ");
    let circles = format!(
        "CIRCLE: DXF contains degenerate zero-length geometry; first 20 handles: {handles}"
    );
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.entity-unreadable", "CIRCLE"),
        (22, circles.as_str())
    );
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.entity-unreadable", "INSERT"),
        (
            1,
            "INSERT: DXF contains an invalid, duplicate, cyclic, or undefined block; handles: 3A"
        )
    );
    assert_eq!(
        entry(
            parsed.diagnostics(),
            "dxf.dimension-block-missing",
            "DIMENSION"
        ),
        (
            1,
            "DIMENSION whose drawn block is undefined, unreadable or empty; blocks: *D9"
        )
    );
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.profile-invalid", "DXF").0,
        1
    );
    assert_eq!(
        entry(parsed.diagnostics(), "dxf.repeated-vertex-removed", "rooms").0,
        1
    );
    assert_eq!(import_profiles(&source), 3);

    // An underlay keeps the hatch's lines too and reports the rest alike.
    let underlay = read_dxf_underlay(&source, DxfImportOptions::new(None)).unwrap();
    assert_eq!(
        entry(&underlay.diagnostics, "dxf.entity-unreadable", "CIRCLE"),
        (22, circles.as_str())
    );
    assert_eq!(
        entry(
            &underlay.diagnostics,
            "dxf.dimension-block-missing",
            "DIMENSION"
        )
        .0,
        1
    );
    assert!(underlay.lines.segment_count() > 5);
}

fn line_on(layer: &str, handle: &str, [x0, y0, x1, y1]: [i32; 4]) -> String {
    format!("0\nLINE\n5\n{handle}\n8\n{layer}\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n")
}

#[test]
fn lines_that_do_not_chain_unambiguously_are_left_out_alone_and_named() {
    // Review run 253: one repeated line, one branch or one shared segment on any layer
    // refused every profile of the drawing.
    let tee = [
        "0\nBLOCK\n2\ntee\n70\n0\n10\n0\n20\n0\n".to_owned(),
        line_on("block", "T1", [0, 0, 1000, 0]),
        line_on("block", "T2", [1000, 0, 2000, 0]),
        line_on("block", "T3", [1000, 0, 1000, 1000]),
        line_on("block", "T4", [5000, 0, 6000, 0]),
        "0\nENDBLK\n".to_owned(),
    ]
    .concat();
    let source = drawing(
        &tee,
        &[
            polygon("4A", &[[0, 0], [10, 0], [10, 10], [0, 10]]),
            // The same line twice: the second is left out, the first kept.
            line_on("doubled", "B1", [100, 0, 110, 0]),
            line_on("doubled", "B2", [110, 0, 100, 0]),
            // Three lines meeting at (200, 0).
            line_on("branch", "C1", [190, 0, 200, 0]),
            line_on("branch", "C2", [200, 0, 210, 0]),
            line_on("branch", "C3", [200, 0, 200, 10]),
            // The second polygon of its layer repeats the first one's segments.
            polygon("P1", &[[300, 0], [310, 0], [310, 10], [300, 10]]),
            polygon("P2", &[[300, 0], [310, 0], [310, 10], [300, 10]]),
            "0\nINSERT\n5\n3A\n8\n0\n2\ntee\n10\n0\n20\n50000\n".to_owned(),
        ]
        .concat(),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    // The square, B1, P1 and the block's lone line T4.
    assert_eq!(parsed.profiles().len(), 4, "{:#?}", parsed.profiles());
    let ambiguous = "DXF line/arc connectivity is branched, duplicated, or otherwise ambiguous";
    let mut left_out = parsed
        .diagnostics()
        .iter()
        .filter(|diagnostic| diagnostic.code() == "dxf.profile-ambiguous")
        .map(|diagnostic| (diagnostic.count(), diagnostic.subject().unwrap()))
        .collect::<Vec<_>>();
    left_out.sort_unstable();
    // One entry per group: its kind, layer, lines and the rectangle around them, in the
    // block's coordinates for a group inside a block.
    assert_eq!(
        left_out,
        [
            (
                1,
                format!(
                    "{ambiguous}: a repeated line, layer doubled, 1 line within x 100..110, \
                     y 0..0 mm; handles: B2"
                )
                .as_str()
            ),
            (
                1,
                format!(
                    "{ambiguous}: a segment shared with another profile of its layer, layer \
                     rooms, 4 lines within x 300..310, y 0..10 mm; handles: P2"
                )
                .as_str()
            ),
            (
                3,
                format!(
                    "{ambiguous}: a T-junction (lines that branch), layer block, 3 lines \
                     within x 0..2000, y 0..1000 mm of block tee; handles: T1, T2, T3"
                )
                .as_str()
            ),
            (
                3,
                format!(
                    "{ambiguous}: a T-junction (lines that branch), layer branch, 3 lines \
                     within x 190..210, y 0..10 mm; handles: C1, C2, C3"
                )
                .as_str()
            ),
        ]
    );
    assert_eq!(import_profiles(&source), 4);

    // With nothing else, the same lines are still refused.
    let only_branch = drawing(
        "",
        &[
            line_on("branch", "C1", [190, 0, 200, 0]),
            line_on("branch", "C2", [200, 0, 210, 0]),
            line_on("branch", "C3", [200, 0, 200, 10]),
        ]
        .concat(),
    );
    assert_eq!(
        inspect_dxf(&only_branch, DxfImportOptions::new(None)),
        Err(DxfImportError::AmbiguousGeometry)
    );
}

#[test]
fn a_closed_outline_imports_beside_a_t_junction_on_its_own_and_on_another_layer() {
    let source = drawing(
        "",
        &[
            // A closed square of four lines on `walls`.
            line_on("walls", "S1", [0, 0, 1000, 0]),
            line_on("walls", "S2", [1000, 0, 1000, 1000]),
            line_on("walls", "S3", [1000, 1000, 0, 1000]),
            line_on("walls", "S4", [0, 1000, 0, 0]),
            // A T-junction apart from it on the same layer.
            line_on("walls", "W1", [2000, 0, 2100, 0]),
            line_on("walls", "W2", [2100, 0, 2200, 0]),
            line_on("walls", "W3", [2100, 0, 2100, 100]),
            // And one on another layer.
            line_on("other", "O1", [5000, 5000, 5100, 5000]),
            line_on("other", "O2", [5100, 5000, 5200, 5000]),
            line_on("other", "O3", [5100, 5000, 5100, 4900]),
        ]
        .concat(),
    );
    let parsed = inspect_dxf(&source, DxfImportOptions::new(None)).unwrap();
    assert_eq!(parsed.profiles().len(), 1, "{:#?}", parsed.profiles());
    let square = &parsed.profiles()[0];
    assert!(square.closed());
    assert_eq!(square.layer(), "walls");
    assert_eq!(square.segments().len(), 4);

    let ambiguous = "DXF line/arc connectivity is branched, duplicated, or otherwise ambiguous";
    let tee = "a T-junction (lines that branch)";
    assert_eq!(
        entry(
            parsed.diagnostics(),
            "dxf.profile-ambiguous",
            &format!("{ambiguous}: {tee}, layer walls")
        ),
        (
            3,
            format!(
                "{ambiguous}: {tee}, layer walls, 3 lines within x 2000..2200, y 0..100 mm; \
                 handles: W1, W2, W3"
            )
            .as_str()
        )
    );
    assert_eq!(
        entry(
            parsed.diagnostics(),
            "dxf.profile-ambiguous",
            &format!("{ambiguous}: {tee}, layer other")
        ),
        (
            3,
            format!(
                "{ambiguous}: {tee}, layer other, 3 lines within x 5000..5200, y 4900..5000 mm; \
                 handles: O1, O2, O3"
            )
            .as_str()
        )
    );
    assert_eq!(
        parsed
            .diagnostics()
            .iter()
            .filter(|diagnostic| diagnostic.code() == "dxf.profile-ambiguous")
            .count(),
        2
    );
    assert_eq!(import_profiles(&source), 1);
}

#[test]
fn a_drawing_with_nothing_readable_left_is_refused_with_why_the_first_was_left_out() {
    let options = DxfImportOptions::new(None);
    let faulty = drawing("", &[circle("C1", 0), dimension("2B", "*D9")].concat());
    assert_eq!(
        inspect_dxf(&faulty, options),
        Err(DxfImportError::DegenerateGeometry)
    );
    assert_eq!(
        read_dxf_underlay(&faulty, options).map(|_| ()),
        Err(DxfImportError::DegenerateGeometry)
    );
    let flat = drawing("", &hatch("5A", &[[0, 1000], [2000, 1000], [4000, 1000]]));
    assert_eq!(
        inspect_dxf(&flat, options),
        Err(DxfImportError::InvalidProfile)
    );
}
