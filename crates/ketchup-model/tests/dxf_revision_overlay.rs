//! Review 2026-11-08 §5.3: an architect's revision B of a plan compared with revision A
//! by the handles of its entities — changed, added and removed — and drawn as an overlay
//! of three layers over B, even where the two lie far from the origin and were moved
//! near it by different offsets.
use ketchup_model::import::{
    Difference, DxfImportOptions, OverlayLayers, Revision, compare_revisions, read_dxf_underlay,
    revision_overlay,
};

/// A drawing in millimetres of `entities`, `x0` millimetres east of the origin.
fn drawing(entities: &[String]) -> Vec<u8> {
    let mut dxf = String::from(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n",
    );
    for entity in entities {
        dxf.push_str(entity);
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    dxf.into_bytes()
}

/// A national-grid easting, far from the origin as a surveyed plan lies.
const EAST_MM: f64 = 500_000_000.0;

fn line(handle: &str, layer: &str, from: [f64; 2], to: [f64; 2]) -> String {
    format!(
        "0\nLINE\n5\n{handle}\n100\nAcDbEntity\n8\n{layer}\n100\nAcDbLine\n\
         10\n{}\n20\n{}\n30\n0\n11\n{}\n21\n{}\n31\n0\n",
        from[0] + EAST_MM,
        from[1],
        to[0] + EAST_MM,
        to[1]
    )
}

fn text(handle: &str, at: [f64; 2], value: &str) -> String {
    format!(
        "0\nTEXT\n5\n{handle}\n100\nAcDbEntity\n8\nnotes\n100\nAcDbText\n\
         10\n{}\n20\n{}\n30\n0\n40\n250\n1\n{value}\n100\nAcDbText\n",
        at[0] + EAST_MM,
        at[1]
    )
}

fn handles(entities: &[ketchup_model::import::RevisedEntity]) -> Vec<&str> {
    entities
        .iter()
        .map(|entity| entity.handle.as_str())
        .collect()
}

fn read(dxf: &[u8]) -> ketchup_model::import::DxfUnderlay {
    read_dxf_underlay(dxf, DxfImportOptions::new(None)).expect("the drawing reads")
}

#[test]
fn revision_b_is_compared_with_a_by_handle_and_drawn_over_it() {
    let a = read(&drawing(&[
        line("A0", "walls", [0.0, 0.0], [6000.0, 0.0]),
        line("A1", "walls", [6000.0, 0.0], [6000.0, 4000.0]),
        text("A3", [500.0, 500.0], "Obývačka"),
        line("A4", "walls", [0.0, 4000.0], [6000.0, 4000.0]),
    ]));
    let b = read(&drawing(&[
        line("A0", "walls", [0.0, 0.0], [6000.0, 0.0]),
        // The east wall moved 300 mm out.
        line("A1", "walls", [6300.0, 0.0], [6300.0, 4000.0]),
        line("A4", "walls", [0.0, 4000.0], [6300.0, 4000.0]),
        // A partition added; the room's name taken out.
        line("B0", "walls", [3000.0, 0.0], [3000.0, 4000.0]),
    ]));
    assert_ne!(
        a.offset_mm, b.offset_mm,
        "each drawing was moved by its own offset"
    );
    let earlier = Revision {
        lines: &a.lines,
        offset_mm: a.offset_mm,
    };
    let later = Revision {
        lines: &b.lines,
        offset_mm: b.offset_mm,
    };
    let comparison = compare_revisions(earlier, later);
    assert_eq!(handles(&comparison.changed), ["A1", "A4"]);
    assert_eq!(handles(&comparison.added), ["B0"]);
    assert_eq!(handles(&comparison.removed), ["A3"]);
    assert_eq!(
        comparison.unchanged, 1,
        "A0 lies alike though the offsets differ"
    );
    assert_eq!(comparison.without_handle, 0);
    assert_eq!(comparison.changed[0].difference, Some(Difference::Lines));
    assert_eq!(comparison.removed[0].kind, "TEXT");
    let east = format!("{EAST_MM:.0}").parse::<i64>().unwrap();
    assert_eq!(comparison.added[0].at_mm, Some([east + 3000, 0]));
    assert_eq!(comparison.removed[0].at_mm, Some([east + 500, 500]));

    let names = OverlayLayers {
        changed: "Zmenené".to_owned(),
        added: "Pridané".to_owned(),
        removed: "Zmazané".to_owned(),
    };
    let (layers, overlay) = revision_overlay(earlier, later, &comparison, &names);
    assert_eq!(layers, ["Zmenené", "Pridané", "Zmazané"]);
    let layer =
        |name: &str| &overlay.layers()[layers.iter().position(|layer| layer == name).unwrap()];
    // In the later drawing's coordinates: added back its offset, where the file has it.
    let in_file = |point: [f64; 2]| {
        [
            point[0] + b.offset_mm[0] - EAST_MM,
            point[1] + b.offset_mm[1],
        ]
    };
    let added = layer("Pridané").polylines().collect::<Vec<_>>();
    assert_eq!(added.len(), 1);
    let ends = added[0].iter().copied().map(in_file).collect::<Vec<_>>();
    assert!(
        ends.iter().all(|point| (point[0] - 3000.0).abs() < 0.01),
        "the partition at x 3000: {ends:?}"
    );
    assert_eq!(layer("Zmenené").polylines().count(), 2);
    let removed = layer("Zmazané");
    assert_eq!(removed.texts().len(), 1);
    let [x, y] = in_file(removed.texts()[0].anchor_mm);
    assert!(
        (x - 500.0).abs() < 0.01 && (y - 500.0).abs() < 0.01,
        "{x} {y}"
    );
    assert_eq!(removed.texts()[0].text, "Obývačka");
    let objects = &overlay.objects().objects;
    assert_eq!(objects.len(), 4, "an object per entity, by its handle");
    assert!(objects.iter().any(|object| object.handle == 0xB0));
}

/// A drawing of `entities` whose layer `doors` has color `doors_color` and whose
/// linetype table has DASHED.
fn styled_drawing(doors_color: u8, entities: &[String]) -> Vec<u8> {
    let mut dxf = format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLTYPE\n70\n1\n\
         0\nLTYPE\n2\nDASHED\n70\n0\n3\n__ __\n72\n65\n73\n2\n40\n15\n49\n10\n74\n0\n49\n-5\n74\n0\n\
         0\nENDTAB\n0\nTABLE\n2\nLAYER\n70\n2\n\
         0\nLAYER\n2\nwalls\n70\n0\n62\n7\n6\nCONTINUOUS\n\
         0\nLAYER\n2\ndoors\n70\n0\n62\n{doors_color}\n6\nCONTINUOUS\n\
         0\nENDTAB\n0\nENDSEC\n0\nSECTION\n2\nENTITIES\n"
    );
    for entity in entities {
        dxf.push_str(entity);
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    dxf.into_bytes()
}

/// A line of `handle` on `layer` with the entity groups `style` (color, linetype,
/// lineweight), at the height `y`.
fn styled_line(handle: &str, layer: &str, style: &str, y: f64) -> String {
    format!(
        "0\nLINE\n5\n{handle}\n100\nAcDbEntity\n8\n{layer}\n{style}100\nAcDbLine\n\
         10\n0\n20\n{y}\n30\n0\n11\n5000\n21\n{y}\n31\n0\n"
    )
}

fn styled_text(handle: &str, style: &str) -> String {
    format!(
        "0\nTEXT\n5\n{handle}\n100\nAcDbEntity\n8\nwalls\n{style}100\nAcDbText\n\
         10\n0\n20\n9000\n30\n0\n40\n250\n1\nKuchyňa\n100\nAcDbText\n"
    )
}

/// Review 2026-11-10 og:R1110-UL-400: a revision that only redraws an entity otherwise —
/// another color, linetype or lineweight, its layer's color, a text's color — was reported
/// unchanged, so a wall the architect dashed as to be pulled down was not seen.
#[test]
fn a_revision_that_only_restyles_an_entity_reports_it_changed_in_style() {
    let entities = |restyled: bool| {
        let pick = |plain: &'static str, styled: &'static str| {
            if restyled { styled } else { plain }
        };
        vec![
            styled_line("A0", "walls", pick("", "62\n1\n"), 0.0),
            styled_line("A1", "walls", pick("", "6\nDASHED\n"), 1000.0),
            styled_line("A2", "walls", pick("", "370\n50\n"), 2000.0),
            styled_text("A3", pick("", "62\n3\n")),
            // ByLayer: only the layer's color changes.
            styled_line("A4", "doors", "", 4000.0),
            styled_line("A5", "walls", "", 5000.0),
        ]
    };
    let a = read(&styled_drawing(7, &entities(false)));
    let b = read(&styled_drawing(5, &entities(true)));
    let comparison = compare_revisions(
        Revision {
            lines: &a.lines,
            offset_mm: a.offset_mm,
        },
        Revision {
            lines: &b.lines,
            offset_mm: b.offset_mm,
        },
    );
    assert_eq!(
        handles(&comparison.changed),
        ["A0", "A1", "A2", "A3", "A4"],
        "{comparison:?}"
    );
    assert!(
        comparison
            .changed
            .iter()
            .all(|entity| entity.difference == Some(Difference::Style)),
        "{comparison:?}"
    );
    assert_eq!(comparison.unchanged, 1, "A5 draws alike");
}

#[test]
fn drawings_without_handles_compare_nothing_rather_than_guess() {
    let unhandled = |x: f64| {
        drawing(&[format!(
            "0\nLINE\n8\nwalls\n10\n{x}\n20\n0\n30\n0\n11\n{}\n21\n0\n31\n0\n",
            x + 1000.0
        )])
    };
    let (a, b) = (read(&unhandled(0.0)), read(&unhandled(500.0)));
    let comparison = compare_revisions(
        Revision {
            lines: &a.lines,
            offset_mm: a.offset_mm,
        },
        Revision {
            lines: &b.lines,
            offset_mm: b.offset_mm,
        },
    );
    assert!(comparison.compared_nothing(), "{comparison:?}");
    assert_eq!(comparison.without_handle, 2);
}
