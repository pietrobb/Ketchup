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

fn compare(
    a: &ketchup_model::import::DxfUnderlay,
    b: &ketchup_model::import::DxfUnderlay,
) -> ketchup_model::import::RevisionComparison {
    compare_revisions(
        Revision {
            lines: &a.lines,
            offset_mm: a.offset_mm,
        },
        Revision {
            lines: &b.lines,
            offset_mm: b.offset_mm,
        },
    )
}

/// A drawing of `entities` with the text styles PLAIN and SAME in Arial and NOTES in
/// romans.shx.
fn font_drawing(entities: &[String]) -> Vec<u8> {
    let style = |name: &str, font: &str| {
        format!("0\nSTYLE\n2\n{name}\n70\n0\n40\n0\n41\n1\n50\n0\n71\n0\n42\n250\n3\n{font}\n")
    };
    let mut dxf = format!(
        "0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1015\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nSTYLE\n70\n3\n{}{}{}0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n",
        style("PLAIN", "arial.ttf"),
        style("SAME", "arial.ttf"),
        style("NOTES", "romans.shx"),
    );
    for entity in entities {
        dxf.push_str(entity);
    }
    dxf.push_str("0\nENDSEC\n0\nEOF\n");
    dxf.into_bytes()
}

/// An open LWPOLYLINE of `handle` along `y` with the groups `width` after its head
/// (43) or after each vertex (40, 41).
fn wide_polyline(handle: &str, y: f64, head: &str, vertex: &str) -> String {
    format!(
        "0\nLWPOLYLINE\n5\n{handle}\n100\nAcDbEntity\n8\nwalls\n100\nAcDbPolyline\n90\n2\n70\n0\n\
         {head}10\n0\n20\n{y}\n{vertex}10\n5000\n20\n{y}\n{vertex}"
    )
}

/// R1124og-DXF-902: a revision that sets a text in another font or draws a polyline
/// wider was reported unchanged; it is changed in style, also against the earlier
/// revision as the document saved it. A text whose style is renamed but keeps its font
/// is unchanged.
#[test]
fn a_revision_that_sets_a_text_in_another_font_or_draws_a_line_wider_is_restyled() {
    let entities = |revised: bool| {
        let pick = |plain: &'static str, revised_as: &'static str| {
            if revised { revised_as } else { plain }
        };
        vec![
            styled_text("A0", pick("7\nPLAIN\n", "7\nNOTES\n")),
            styled_text("A1", pick("7\nPLAIN\n", "7\nSAME\n")),
            wide_polyline("A2", 1000.0, pick("", "43\n50\n"), ""),
            wide_polyline(
                "A3",
                2000.0,
                "",
                pick("40\n20\n41\n20\n", "40\n20\n41\n80\n"),
            ),
            format!(
                "0\nMTEXT\n5\nA4\n100\nAcDbEntity\n8\nwalls\n100\nAcDbMText\n\
                 10\n0\n20\n7000\n30\n0\n40\n250\n71\n1\n1\n{}\n",
                pick("{\\fArial|b0|i0;Izba}", "{\\fTimes New Roman|b0|i0;Izba}")
            ),
            wide_polyline("A5", 3000.0, "43\n50\n", ""),
        ]
    };
    let a = read(&font_drawing(&entities(false)));
    let b = read(&font_drawing(&entities(true)));
    let saved =
        ketchup_model::underlay::UnderlayLines::decode(&a.lines.encode(), a.lines.layers().len())
            .expect("the saved lines read back");
    assert_eq!(saved, a.lines, "widths and fonts are saved");
    for earlier in [&a.lines, &saved] {
        let comparison = compare_revisions(
            Revision {
                lines: earlier,
                offset_mm: a.offset_mm,
            },
            Revision {
                lines: &b.lines,
                offset_mm: b.offset_mm,
            },
        );
        assert_eq!(
            handles(&comparison.changed),
            ["A0", "A2", "A3", "A4"],
            "{comparison:?}"
        );
        assert!(
            comparison
                .changed
                .iter()
                .all(|entity| entity.difference == Some(Difference::Style)),
            "{comparison:?}"
        );
        assert_eq!(comparison.unchanged, 2, "A1 and A5 draw alike");
    }
}

/// Review 2026-11-10 og:R1110-UL-401: ArchiCAD, Revit and Allplan give every entity a
/// new handle on each export, so one real change was reported as every entity removed
/// and added again. Those drawn alike in the same place pair up as renumbered.
#[test]
fn entities_renumbered_by_the_export_pair_up_by_what_they_draw() {
    let walls = |handles: [&str; 4], east_x: f64| {
        drawing(&[
            line(handles[0], "walls", [0.0, 0.0], [6000.0, 0.0]),
            line(handles[1], "walls", [0.0, 4000.0], [6000.0, 4000.0]),
            text(handles[2], [500.0, 500.0], "Obývačka"),
            // Apart from the others: lines meeting end to end are joined on reading.
            line(handles[3], "walls", [east_x, 500.0], [east_x, 3500.0]),
        ])
    };
    let a = read(&walls(["A0", "A1", "A2", "A3"], 6000.0));
    // Every handle new; the east wall moved 300 mm out.
    let b = read(&walls(["C0", "C1", "C2", "C3"], 6300.0));
    let comparison = compare(&a, &b);
    assert_eq!(comparison.renumbered, 3, "{comparison:?}");
    assert_eq!(handles(&comparison.added), ["C3"]);
    assert_eq!(handles(&comparison.removed), ["A3"]);
    assert!(comparison.changed.is_empty());
    assert_eq!(comparison.unchanged, 0);
    assert!(!comparison.lacks_handles());
}

/// UL-401: a drawing without handles against one with them was reported as all of one
/// removed and all of the other added; it is refused as nothing to compare by.
#[test]
fn a_revision_without_handles_against_one_with_them_is_refused() {
    let with = read(&drawing(&[line("A0", "walls", [0.0, 0.0], [1000.0, 0.0])]));
    let without = read(&drawing(&[
        "0\nLINE\n8\nwalls\n10\n0\n20\n0\n30\n0\n11\n1000\n21\n0\n31\n0\n".to_owned(),
    ]));
    for comparison in [compare(&with, &without), compare(&without, &with)] {
        assert!(comparison.lacks_handles(), "{comparison:?}");
    }
}

/// Review 2026-11-10 og:R1110-UL-402: 100 000 lines on one layer were compared in 0.13 s
/// but their overlay took 22.7 s, each entity walking every polyline of its layer.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_the_overlay_of_a_hundred_thousand_changed_lines_on_one_layer_takes_a_second() {
    let walls = |shift: f64| {
        let entities = (0..100_000_u32)
            .map(|at| {
                let x = 10.0 * f64::from(at);
                line(
                    &format!("{:X}", at + 0x100),
                    "walls",
                    [x, shift],
                    [x, 5.0 + shift],
                )
            })
            .collect::<Vec<_>>();
        read(&drawing(&entities))
    };
    let (a, b) = (walls(0.0), walls(1.0));
    let (earlier, later) = (
        Revision {
            lines: &a.lines,
            offset_mm: a.offset_mm,
        },
        Revision {
            lines: &b.lines,
            offset_mm: b.offset_mm,
        },
    );
    let started = std::time::Instant::now();
    let comparison = compare_revisions(earlier, later);
    let names = OverlayLayers {
        changed: "changed".to_owned(),
        added: "added".to_owned(),
        removed: "removed".to_owned(),
    };
    let (_, overlay) = revision_overlay(earlier, later, &comparison, &names);
    let took = started.elapsed();
    assert_eq!(comparison.changed.len(), 100_000);
    assert_eq!(overlay.layers()[0].polylines().count(), 100_000);
    assert!(
        took < ketchup_test_env::perf_budget(std::time::Duration::from_secs(1)),
        "{took:?}"
    );
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
    assert!(comparison.lacks_handles(), "{comparison:?}");
    assert_eq!(comparison.without_handle, 2);
}
