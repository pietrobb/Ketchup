//! CNC DXF of the made flat parts (R9-C step 19): every file is read back
//! with Kečup's own DXF reader, and its outline, holes and pockets are checked
//! against the part as the program wrote it.

use ketchup_model::document::ProfileSegment;
use ketchup_model::import::{DxfImportOptions, inspect_dxf, read_dxf_underlay};
use ketchup_program::cnc_dxf::{CncExport, CncPart, Entity, cnc_parts};
use std::collections::BTreeMap;

const PROGRAM: &str = r#"
side = board("Bočnica ľavá", [18, 400, 720], material = "DTD")
hole(side, "x+", world = (18, 37, 32), diameter = 5, depth = 13, id = "shelf pin")
hole(side, "x-", world = (0, 200, 360), diameter = 8, depth = 18, through = True, id = "bolt")
hole(side, "y-", world = (9, 0, 300), diameter = 8, depth = 30, id = "dowel")
pocket(side, "x+", rect = (0, 0, 400, 8), depth = 6, id = "back groove")
top = extrude("top", profile = round_corners([(0, 0), (600, 0), (600, 300), (0, 300)], 20),
              distance = 18, at = (0, 0, 720), attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})
cut(top, profile = [["s1", [200, 150], [100, 150], {"center": (150, 150)}],
                    ["s2", [100, 150], [200, 150], {"center": (150, 150)}]], through = True, name = "grommet")
cut(top, profile = [(400, 100), (500, 100), (500, 200), (400, 200)], depth = 5, name = "inlay")
member("rail", (0, 1000, 0), (2000, 1000, 0), (60, 120))
box("plain", [100, 100, 18], at = (0, 2000, 0))
bent = board("bent", [400, 300, 18], at = (0, 3000, 0))
mirror(bent, axis = "x")
"#;

fn export() -> CncExport {
    let model = ketchup_program::evaluate("cnc.star", PROGRAM, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    cnc_parts(&model)
}

fn part<'a>(export: &'a CncExport, name: &str) -> &'a CncPart {
    export
        .parts
        .iter()
        .find(|part| part.part == name)
        .unwrap_or_else(|| panic!("no drawing of {name}: {export:#?}"))
}

/// The profiles the DXF reader finds, by layer, each as its segments' start
/// points rounded to a hundredth.
fn profiles(dxf: &[u8]) -> BTreeMap<String, Vec<Vec<ProfileSegment>>> {
    let read = inspect_dxf(dxf, DxfImportOptions::new(None))
        .unwrap_or_else(|error| panic!("{error}\n{}", String::from_utf8_lossy(dxf)));
    let mut by_layer = BTreeMap::<String, Vec<Vec<ProfileSegment>>>::new();
    for profile in read.profiles() {
        by_layer
            .entry(profile.layer().to_owned())
            .or_default()
            .push(profile.segments().to_vec());
    }
    by_layer
}

fn start(segment: &ProfileSegment) -> [f64; 2] {
    let point = match segment {
        ProfileSegment::Line { start_mm, .. } | ProfileSegment::CircularArc { start_mm, .. } => {
            *start_mm
        }
        other => panic!("unexpected {other:?}"),
    };
    point.map(|value| (value * 100.0).round() / 100.0)
}

#[test]
fn a_part_on_its_side_is_drawn_on_its_face_with_its_holes_and_pockets_on_their_layers() {
    let export = export();
    let side = part(&export, "Bočnica ľavá");
    // 18 x 400 x 720 seen along x as woodWOP lays it: the 720 (z) to the
    // right, the 400 (y) up from the far edge.
    assert_eq!(side.thickness_mm, 18.0);
    assert_eq!(side.size_mm, [720.0, 400.0]);
    assert_eq!(side.file, "Bocnica_lava.dxf");
    let dxf = side.dxf();
    assert!(dxf.is_ascii(), "the file is ASCII");
    let text = String::from_utf8(dxf.clone()).expect("ASCII");
    assert!(text.contains("$ACADVER\n1\nAC1009\n"), "R12");
    assert!(!text.contains("BLOCK"), "no blocks");
    let read = profiles(&dxf);
    let contour = &read["CONTOUR"];
    assert_eq!(contour.len(), 1);
    assert_eq!(
        contour[0].iter().map(start).collect::<Vec<_>>(),
        [[0.0, 0.0], [720.0, 0.0], [720.0, 400.0], [0.0, 400.0]]
    );
    // The shelf pin from the face seen; the bolt goes through, so it is as
    // deep as the part whichever face it is drilled from.
    let circle_center = |profile: &Vec<ProfileSegment>| match profile[0] {
        ProfileSegment::CircularArc { center_mm, .. } => center_mm,
        ref other => panic!("not a circle: {other:?}"),
    };
    assert_eq!(read["DRILL_D5_Z13"].len(), 1);
    assert_eq!(circle_center(&read["DRILL_D5_Z13"][0]), [32.0, 363.0]);
    assert_eq!(circle_center(&read["DRILL_D8_Z18"][0]), [360.0, 200.0]);
    // The groove runs along the left edge of the face seen, 8 wide.
    let groove = &read["POCKET_Z6"][0];
    let mut corners = groove.iter().map(start).collect::<Vec<_>>();
    corners.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    assert_eq!(
        corners,
        [[0.0, 0.0], [0.0, 400.0], [8.0, 0.0], [8.0, 400.0]]
    );
    // The dowel into the top edge (local y = 0): a line from y = 400 down
    // 30 mm, at mid-thickness.
    let Some(Entity::Line {
        layer,
        start_mm,
        end_mm,
    }) = side
        .entities
        .iter()
        .find(|entity| matches!(entity, Entity::Line { .. }))
    else {
        panic!("no edge hole: {side:#?}");
    };
    assert_eq!(layer, "HDRILL_D8_Z30_H9");
    assert_eq!((*start_mm, *end_mm), ([300.0, 400.0], [300.0, 370.0]));
    assert_eq!(
        read["HDRILL_D8_Z30_H9"].len(),
        1,
        "the reader finds the line"
    );
    // The name comes back with its accents.
    let underlay = read_dxf_underlay(&dxf, DxfImportOptions::new(None))
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(underlay.layers.iter().any(|layer| layer == "PART_NAME"));
    assert_eq!(underlay.lines.text_count(), 1);
    assert!(text.contains("Bo\\U+010Dnica \\U+013Eav\\U+00E1"), "{text}");
}

/// MF-63: the drawing lies as the woodWOP program lays the part: x along its
/// longest side, y along the next, the frame right-handed, and a part drilled
/// from below only turned over, its underside up.
#[test]
fn a_board_is_drawn_in_the_frame_its_woodwop_program_lays_it_in() {
    let source = "panel = board('panel', [600, 2400, 18], material = 'DTD')\n\
         hole(panel, 'z+', world = (100, 300, 18), diameter = 5, depth = 10, id = 'from above')\n\
         pocket(panel, 'z+', rect = (0, 0, 600, 8), depth = 6, id = 'groove')\n\
         under = board('under', [600, 2400, 18], at = (0, 0, 100), material = 'DTD')\n\
         hole(under, 'z-', world = (100, 300, 100), diameter = 5, depth = 10, id = 'from below')\n\
         hole(under, 'y-', world = (100, 0, 106), diameter = 8, depth = 30, id = 'dowel')\n";
    let model = ketchup_program::evaluate("frame.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    let export = cnc_parts(&model);
    let circle = |part: &CncPart| {
        part.entities
            .iter()
            .find_map(|entity| match entity {
                Entity::Circle {
                    layer, center_mm, ..
                } => Some((layer.clone(), *center_mm)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no hole: {part:#?}"))
    };

    // 600 x 2400: woodWOP X is the 2 400 (local y), Y the 600 (local x) from
    // the far edge, so the frame stays right-handed.
    let panel = part(&export, "panel");
    assert_eq!(panel.size_mm, [2400.0, 600.0]);
    assert_eq!(circle(panel), ("DRILL_D5_Z10".to_owned(), [300.0, 500.0]));
    let read = profiles(&panel.dxf());
    let mut groove = read["POCKET_Z6"][0].iter().map(start).collect::<Vec<_>>();
    groove.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    assert_eq!(groove, [[0.0, 0.0], [0.0, 600.0], [8.0, 0.0], [8.0, 600.0]]);

    // Drilled from below only, it lies underside up: the hole is drilled
    // from the face seen, the edge hole's height counts from the new
    // underside, and Y is no longer reversed.
    let under = part(&export, "under");
    assert_eq!(under.size_mm, [2400.0, 600.0]);
    assert_eq!(circle(under), ("DRILL_D5_Z10".to_owned(), [300.0, 100.0]));
    let edge = under
        .entities
        .iter()
        .find_map(|entity| match entity {
            Entity::Line {
                layer,
                start_mm,
                end_mm,
            } => Some((layer.as_str(), *start_mm, *end_mm)),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no edge hole: {under:#?}"));
    assert_eq!(edge, ("HDRILL_D8_Z30_H12", [0.0, 100.0], [30.0, 100.0]));
}

#[test]
fn a_profile_keeps_its_arcs_and_its_cuts_are_a_cutout_or_a_pocket() {
    let export = export();
    let top = part(&export, "top");
    assert_eq!(top.thickness_mm, 18.0);
    assert_eq!(top.size_mm, [600.0, 300.0]);
    let read = profiles(&top.dxf());
    let contour = &read["CONTOUR"][0];
    let arcs = contour
        .iter()
        .filter(|segment| matches!(segment, ProfileSegment::CircularArc { .. }))
        .count();
    assert_eq!(
        (contour.len(), arcs),
        (8, 4),
        "four sides, four rounded corners"
    );
    let cutout = &read["CUTOUT"][0];
    assert!(
        cutout.iter().all(|segment| matches!(
            segment,
            ProfileSegment::CircularArc { center_mm, .. } if *center_mm == [150.0, 150.0]
        )),
        "the grommet is a circle: {cutout:?}"
    );
    assert_eq!(read["POCKET_Z5"].len(), 1);
}

/// A framed wall sheathed with OSB boards: every board is drawn, 15 thick,
/// and the outlines read back cover the wall less its window.
#[test]
fn every_sheathing_piece_of_a_buildup_is_drawn_and_together_they_cover_the_wall() {
    let source = "buildup('wall', (0, 0, 0), (1, 0, 0), (0, 0, 1), 3000, height = 2500,\n\
         layers = [{'name': 'frame', 'thickness': 140, 'material': 'C24', 'spacing': 625},\n\
                   {'name': 'osb', 'thickness': 15, 'material': 'OSB', 'sheet': (1250, 2500), 'grain': 'v'}],\n\
         openings = [(1000, 800, 900, 1200)])\n";
    let model = ketchup_program::evaluate("wall.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    let export = cnc_parts(&model);
    assert_eq!(export.not_drawn, [], "the framing is bars, the OSB drawn");
    let osb = model
        .parts
        .iter()
        .filter(|part| part.material.as_deref() == Some("OSB"))
        .count();
    assert!(osb > 2);
    assert_eq!(export.parts.len(), osb, "{export:#?}");
    let mut area = 0.0;
    for part in &export.parts {
        assert_eq!(part.thickness_mm, 15.0, "{}", part.part);
        let read = profiles(&part.dxf());
        let [outline] = read["CONTOUR"].as_slice() else {
            panic!("{}: {read:?}", part.part);
        };
        let points = outline.iter().map(start).collect::<Vec<_>>();
        area += (0..points.len())
            .map(|i| {
                let (a, b) = (points[i], points[(i + 1) % points.len()]);
                a[0] * b[1] - b[0] * a[1]
            })
            .sum::<f64>()
            .abs()
            / 2.0;
    }
    assert!(
        (area - (3000.0 * 2500.0 - 900.0 * 1200.0)).abs() < 1e-3,
        "{area}"
    );
}

#[test]
fn only_made_parts_are_drawn_and_a_shape_no_face_drawing_holds_is_named() {
    let export = export();
    let drawn = export
        .parts
        .iter()
        .map(|part| part.part.as_str())
        .collect::<Vec<_>>();
    assert_eq!(drawn, ["Bočnica ľavá", "top"], "the bar is no CNC part");
    let not_drawn = export
        .not_drawn
        .iter()
        .map(|part| (part.part.as_str(), part.reason.to_string()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(not_drawn["plain"], "it has no fabrication role");
    assert!(not_drawn["bent"].contains("mirror"), "{not_drawn:?}");
    // The archive holds every drawing and the table naming what is missing.
    let zip = export.zip();
    let csv = String::from_utf8_lossy(&zip);
    assert!(csv.contains("Bocnica_lava.dxf") && csv.contains("top.dxf"));
    assert!(csv.contains("parts.csv") && csv.contains("bent,,,,,,"));
}
