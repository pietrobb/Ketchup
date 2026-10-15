//! Standard nuts, washers, bolts and threaded rods from the library (topic
//! fasteners): catalogue dimensions, refused sizes and lengths and what the cut
//! list says about them.

use ketchup_program::cut_list::cut_list_table;
use ketchup_program::model::{Part, ProgramModel};
use ketchup_program::table::Cell;
use std::collections::BTreeMap;

fn model(source: &str) -> ProgramModel {
    ketchup_program::evaluate("fasteners.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model
}

fn refused(source: &str) -> String {
    match ketchup_program::evaluate("fasteners.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("accepted: {source}"),
        Err(error) => error.to_string(),
    }
}

/// (width across flats along local x, height along local z) of a part.
fn flats_and_height(part: &Part) -> (f64, f64) {
    let (min, max) = part.local_bounds();
    (max[0] - min[0], max[2] - min[2])
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() < 1e-6
}

#[test]
fn nuts_take_their_width_across_flats_and_height_from_the_norm() {
    let model = model(
        "nut(\"iso\", \"M8\")
nut(\"din\", \"M8\", norm = \"DIN 934\", at = (50, 0, 0))
nut(\"coupling\", \"M8\", norm = \"DIN 6334\", at = (100, 0, 0))
nut(\"nyloc\", \"M8\", norm = \"ISO 7040\", at = (150, 0, 0))
nut(\"din_m10\", \"M10\", norm = \"DIN 934\", at = (200, 0, 0))
nut(\"iso_m10\", \"M10\", at = (250, 0, 0))
",
    );
    let expected = [
        ("iso", 13.0, 6.8),
        ("din", 13.0, 6.5),
        ("coupling", 13.0, 24.0),
        ("nyloc", 13.0, 9.5),
        // DIN 934 keeps the larger spanner for M10; ISO 4032 takes 16.
        ("din_m10", 17.0, 8.0),
        ("iso_m10", 16.0, 8.4),
    ];
    for (name, s, m) in expected {
        let part = model.part(name).expect(name);
        let (flats, height) = flats_and_height(part);
        assert!(
            close(flats, s) && close(height, m),
            "{name}: {flats} x {height}"
        );
        assert_eq!(part.material.as_deref(), Some("steel"));
    }
    let iso = model.part("iso").unwrap();
    // Across corners 2 s / sqrt(3), seated on its origin, the bore the nominal 8 mm.
    let (min, max) = iso.local_bounds();
    assert!(
        close(max[1] - min[1], 26.0 / 3f64.sqrt()),
        "{min:?} {max:?}"
    );
    assert!(close(min[2], 0.0), "{min:?}");
    let bore = iso.cuts().next().expect("a bore");
    assert!(
        bore.segments
            .iter()
            .all(|segment| close(segment.start_mm[0].hypot(segment.start_mm[1]), 4.0)),
        "{:?}",
        bore.segments
    );
}

#[test]
fn an_iso_7089_washer_is_a_ring_of_the_catalogue_size() {
    let model = model("washer(\"w\", \"M8\")\n");
    let washer = model.part("w").unwrap();
    let (min, max) = washer.local_bounds();
    assert!(close(max[0] - min[0], 16.0), "{min:?} {max:?}");
    assert!(close(max[1] - min[1], 16.0), "{min:?} {max:?}");
    assert!(close(max[2] - min[2], 1.6), "{min:?} {max:?}");
    let bore = washer.cuts().next().expect("a hole");
    assert!(
        close(bore.segments[0].start_mm[0].abs(), 4.2),
        "{:?}",
        bore.segments
    );
    assert_eq!(washer.attributes["designation"], "ISO 7089 M8");
}

#[test]
fn a_size_the_norm_lacks_is_refused_with_the_valid_sizes() {
    let error = refused("nut(\"n\", \"M7\")\n");
    assert!(
        error.contains("ISO 4032 has no size \"M7\"")
            && error.contains(
                "valid sizes: M3, M4, M5, M6, M8, M10, M12, M14, M16, M18, M20, M22, M24"
            ),
        "{error}"
    );
    // ISO 7040 has no M18 or M22, DIN 6334 starts at M6.
    let error = refused("nut(\"n\", \"M18\", norm = \"ISO 7040\")\n");
    assert!(
        error.contains("valid sizes: M3, M4, M5, M6, M8, M10, M12, M14, M16, M20, M24"),
        "{error}"
    );
    let error = refused("nut(\"n\", \"M5\", norm = \"DIN 6334\")\n");
    assert!(error.contains("valid sizes: M6, M8,"), "{error}");
    let error = refused("washer(\"w\", \"M7\")\n");
    assert!(error.contains("ISO 7089 has no size \"M7\""), "{error}");
    let error = refused("nut(\"n\", \"M8\", norm = \"DIN 555\")\n");
    assert!(error.contains("unknown norm \"DIN 555\""), "{error}");
}

#[test]
fn a_nut_turned_onto_another_axis_keeps_its_seating_face_at_the_given_point() {
    // Under a plate whose underside is at z = -20: the nut hangs down from it.
    let hanging = model("nut(\"n\", \"M8\", at = (10, 20, -20), axis = (0, 0, -1))\n");
    let (min, max) = hanging.part("n").unwrap().world_bounds();
    assert!(
        close(max[2], -20.0) && close(min[2], -26.8),
        "{min:?} {max:?}"
    );
    assert!(close((min[0] + max[0]) / 2.0, 10.0), "{min:?} {max:?}");
    assert!(close((min[1] + max[1]) / 2.0, 20.0), "{min:?} {max:?}");
    // Sideways along +x.
    let sideways = model("nut(\"n\", \"M8\", axis = (1, 0, 0))\n");
    let (min, max) = sideways.part("n").unwrap().world_bounds();
    assert!(close(min[0], 0.0) && close(max[0], 6.8), "{min:?} {max:?}");
}

#[test]
fn bolts_take_their_head_from_the_norm_and_hang_their_shank_below_the_seat() {
    let model = model(
        "bolt(\"hex\", \"M8\", 40)
bolt(\"cap\", \"M8\", 40, norm = \"ISO 4762\", at = (100, 0, 0))
bolt(\"shank\", \"M10\", 45, norm = \"ISO 4014\", at = (200, 0, 0))
",
    );
    // (width along x, head height above the seat, shank length below it, shank diameter)
    let expected = [
        ("hex", 13.0, 5.3, 40.0, 8.0, "ISO 4017 M8x40"),
        ("cap", 13.0, 8.0, 40.0, 8.0, "ISO 4762 M8x40"),
        ("shank", 16.0, 6.4, 45.0, 10.0, "ISO 4014 M10x45"),
    ];
    for (name, width, head, length, diameter, designation) in expected {
        let part = model.part(name).expect(name);
        let (min, max) = part.world_bounds();
        assert!(
            close(max[0] - min[0], width) && close(max[2], head) && close(min[2], -length),
            "{name}: {min:?} {max:?}"
        );
        assert_eq!(part.attributes["designation"], designation);
        assert_eq!(part.material.as_deref(), Some("steel"));
        // One solid: the shank is joined to the head, not a separate part.
        let shank = part.booleans().next().expect("the shank");
        assert_eq!(shank.name, "shank");
        let (tool_min, tool_max) = shank.tool.world_bounds();
        assert!(
            close(tool_max[1] - tool_min[1], diameter),
            "{name}: shank {tool_min:?} {tool_max:?}"
        );
    }
    assert_eq!(model.parts.len(), 3, "the shanks are no parts of their own");
    // The socket of the cap screw: s 6 across flats, t 4 deep from the top.
    let socket = model.part("cap").unwrap().cuts().next().expect("a socket");
    assert_eq!(socket.name, "socket");
    assert_eq!(socket.depth_mm, Some(4.0));
    let xs: Vec<f64> = socket.segments.iter().map(|s| s.start_mm[0]).collect();
    let across =
        xs.iter().cloned().fold(f64::MIN, f64::max) - xs.iter().cloned().fold(f64::MAX, f64::min);
    assert!(close(across, 6.0), "{xs:?}");
    // Turned upside down under a plate: the head hangs below z = -20.
    let hanging = self::model("bolt(\"b\", \"M8\", 40, at = (0, 0, -20), axis = (0, 0, -1))\n");
    let (min, max) = hanging.part("b").unwrap().world_bounds();
    assert!(
        close(min[2], -25.3) && close(max[2], 20.0),
        "{min:?} {max:?}"
    );
}

#[test]
fn an_iso_4014_bolt_is_threaded_only_b_from_the_tip_and_an_iso_4017_bolt_to_the_head() {
    let model = model(
        "bolt(\"partial\", \"M8\", 80, norm = \"ISO 4014\")
bolt(\"full\", \"M8\", 40, at = (50, 0, 0))
bolt(\"mid\", \"M16\", 160, norm = \"ISO 4014\", at = (100, 0, 0))
bolt(\"long\", \"M24\", 240, norm = \"ISO 4014\", at = (150, 0, 0))
",
    );
    // (thread length from the tip, plain shank above it) by b = 2d + 6 / 12 / 25.
    for (name, length, thread, shank, diameter) in [
        ("partial", 80.0, 22.0, 58.0, 8.0),
        ("full", 40.0, 40.0, 0.0, 8.0),
        ("mid", 160.0, 44.0, 116.0, 16.0),
        ("long", 240.0, 73.0, 167.0, 24.0),
    ] {
        let part = model.part(name).unwrap();
        let bound = |key: &str| part.attributes[key].parse::<f64>().unwrap();
        let (from, to) = (bound("thread_from_mm"), bound("thread_to_mm"));
        assert!(
            close(from, -length) && close(to - from, thread) && close(-to, shank),
            "{name}: thread {from}..{to}"
        );
        // The plain shank keeps the nominal diameter d: one cylinder, tip to head.
        let (min, max) = part.booleans().next().unwrap().tool.world_bounds();
        assert!(
            close(max[1] - min[1], diameter) && close(max[2] - min[2], length),
            "{name}: shank {min:?} {max:?}"
        );
    }
    // M8 x 100 (past the catalogued 40..80, so by the rule alone): b = 22, shank 78.
    let m8x100 = self::model(
        "box(\"b\", (bolt_thread_length(\"M8\", 100, norm = \"ISO 4014\"), 1, 1))\n\
         box(\"f\", (bolt_thread_length(\"M8\", 40), 1, 1), at = (50, 0, 0))\n",
    );
    assert!(close(m8x100.part("b").unwrap().size_mm[0], 22.0));
    assert!(close(m8x100.part("f").unwrap().size_mm[0], 40.0));
    // b = 26 >= l = 20: the whole shank is threaded.
    let short =
        self::model("box(\"b\", (bolt_thread_length(\"M10\", 20, norm = \"ISO 4014\"), 1, 1))\n");
    assert!(close(short.part("b").unwrap().size_mm[0], 20.0));
}

#[test]
fn a_bolt_length_the_size_lacks_is_refused_with_the_valid_lengths() {
    let error = refused("bolt(\"b\", \"M8\", 42)\n");
    assert!(
        error.contains("ISO 4017 M8 has no length 42 here; valid lengths: 16, 20, 25, 30, 35, 40, 45, 50, 55, 60, 65, 70, 80"),
        "{error}"
    );
    let error = refused("bolt(\"b\", \"M3\", 35, norm = \"ISO 4014\")\n");
    assert!(error.contains("valid lengths: 20, 25, 30"), "{error}");
    // ISO 4017 (M14) has no catalogue lengths here, ISO 4762 has no M18.
    let error = refused("bolt(\"b\", \"M14\", 50)\n");
    assert!(
        error.contains("valid sizes: M3, M4, M5, M6, M8, M10, M12, M16, M20, M24"),
        "{error}"
    );
    let error = refused("bolt(\"b\", \"M18\", 50, norm = \"ISO 4762\")\n");
    assert!(error.contains("ISO 4762 has no size \"M18\""), "{error}");
    let error = refused("bolt(\"b\", \"M8\", 40, norm = \"DIN 931\")\n");
    assert!(error.contains("unknown norm \"DIN 931\""), "{error}");
    // The lengths are there to ask for.
    let model = model(
        "lengths = bolt_lengths(\"M12\", norm = \"ISO 4762\")
box(\"first\", (lengths[0], 1, 1))
box(\"last\", (lengths[-1], 1, 1), at = (0, 10, 0))
",
    );
    assert!(close(model.part("first").unwrap().size_mm[0], 20.0));
    assert!(close(model.part("last").unwrap().size_mm[0], 120.0));
}

#[test]
fn a_threaded_rod_takes_any_positive_length_along_its_axis() {
    let model = model(
        "threaded_rod(\"up\", \"M12\", 1000)
threaded_rod(\"along\", \"M12\", 333.5, at = (0, 50, 0), axis = (1, 0, 0))
",
    );
    let up = model.part("up").unwrap();
    let (min, max) = up.world_bounds();
    assert!(
        close(min[2], 0.0) && close(max[2], 1000.0) && close(max[0] - min[0], 12.0),
        "{min:?} {max:?}"
    );
    assert_eq!(up.attributes["designation"], "DIN 975 M12x1000");
    assert_eq!(up.attributes["standard"], "DIN 975");
    let (min, max) = model.part("along").unwrap().world_bounds();
    assert!(
        close(min[0], 0.0) && close(max[0], 333.5),
        "{min:?} {max:?}"
    );
    assert!(refused("threaded_rod(\"r\", \"M12\", 0)\n").contains("length must be positive"),);
    assert!(refused("threaded_rod(\"r\", \"M7\", 100)\n").contains("DIN 975 has no size \"M7\""));
}

#[test]
fn a_b500b_bar_is_a_cylinder_of_its_nominal_diameter_and_names_its_nominal_mass() {
    let model = model(
        "rebar(\"up\", 12, 1000)
rebar(\"along\", 40, 2500, at = (0, 100, 0), axis = (1, 0, 0))
",
    );
    let up = model.part("up").unwrap();
    let (min, max) = up.world_bounds();
    assert!(
        close(min[2], 0.0) && close(max[2], 1000.0) && close(max[0] - min[0], 12.0),
        "{min:?} {max:?}"
    );
    assert_eq!(up.material.as_deref(), Some("steel"));
    assert_eq!(up.attributes["grade"], "B500B");
    assert_eq!(up.cut_length_mm(), Some(1000.0));
    assert_eq!(up.attributes["standard"], "EN 10080");
    let mass: f64 = up.attributes["mass_kg"].parse().unwrap();
    assert!((mass - 0.888).abs() <= 0.00888, "{mass}");
    assert!(up.attributes["mass_basis"].starts_with("nominal EN 10080"));
    assert!(!up.attributes["source"].contains("only"));
    let along = model.part("along").unwrap();
    let (min, max) = along.world_bounds();
    assert!(
        close(min[0], 0.0) && close(max[0], 2500.0),
        "{min:?} {max:?}"
    );
    assert!(
        along.attributes["source"].contains("Ø40 kg/m in a table at reinforcing-bar.com only"),
        "{}",
        along.attributes["source"]
    );
    assert!(refused("rebar(\"r\", 11, 1000)\n").contains(
        "B500B has no diameter 11 here; valid diameters: 6, 8, 10, 12, 14, 16, 20, 25, 28, 32, 40"
    ));
    assert!(refused("rebar(\"r\", 12, 0)\n").contains("length must be positive"));
    assert!(refused("rebar(\"r\", 12, 1000, grade = \"B420\")\n").contains("unknown grade"));
}

#[test]
fn the_nominal_rebar_formula_matches_every_catalogue_mass_per_metre() {
    // Each diameter's catalogue kg/m against the formula; one beyond 1 % fails.
    let model = model(
        "for d, kg_m in REBAR_MASS_KG_M.items():
    formula = rebar_mass_kg(d, 1000)
    if abs(formula - kg_m) > 0.01 * kg_m:
        fail(\"d%s: catalogue %s kg/m, formula %s kg/m\" % (d, kg_m, formula))
    rebar(\"d%s\" % d, d, 1000, at = (d * 100, 0, 0))
",
    );
    assert_eq!(model.parts.len(), 11);
}

#[test]
fn a_value_with_one_source_is_named_at_its_size() {
    let model = model(
        "nut(\"nyloc3\", \"M3\", norm = \"ISO 7040\")
nut(\"nyloc8\", \"M8\", norm = \"ISO 7040\", at = (20, 0, 0))
bolt(\"cap\", \"M8\", 40, norm = \"ISO 4762\", at = (40, 0, 0))
bolt(\"long\", \"M12\", 80, norm = \"ISO 4014\", at = (60, 0, 0))
bolt(\"m10\", \"M10\", 45, norm = \"ISO 4014\", at = (90, 0, 0))
",
    );
    let source = |name: &str| model.part(name).unwrap().attributes["source"].clone();
    assert!(
        source("nyloc3").ends_with("; M3 h from IS 7002:2005 only (nfgab.com starts at M4)"),
        "{}",
        source("nyloc3")
    );
    // Two catalogues for M8 nylon insert nuts and for ISO 4014 M12 lengths.
    assert!(!source("nyloc8").contains("only"), "{}", source("nyloc8"));
    assert!(!source("long").contains("only"), "{}", source("long"));
    let cap = source("cap");
    assert!(
        cap.contains("M8 lengths from eitasarim.com only") && cap.contains("roymech.co.uk"),
        "{cap}"
    );
    assert!(
        source("m10")
            .contains("M10 shortest length 45 from engineersedge.com; intafast.com starts at 50")
    );
    // engineersedge leaves out 55 for ISO 4014 M6; the length series keeps it.
    let m6 = self::model("bolt(\"m6\", \"M6\", 55, norm = \"ISO 4014\")\n");
    let m6 = m6.part("m6").unwrap().attributes["source"].clone();
    assert!(
        m6.contains("M6 lengths from engineersedge.com only; it leaves out 55"),
        "{m6}"
    );
    for name in ["nyloc3", "cap", "long"] {
        assert!(source(name).contains("(catalogue tables, not the standard text)"));
    }
}

#[test]
fn the_hardware_list_counts_designated_parts_and_the_cut_list_leaves_them_out() {
    let model = model(
        "board(\"plate\", (300, 200, 18))
nut(\"n1\", \"M8\")
nut(\"n2\", \"M8\", at = (40, 0, 0))
nut(\"d1\", \"M8\", norm = \"DIN 934\", at = (80, 0, 0))
washer(\"w1\", \"M8\", at = (120, 0, 0))
bolt(\"b1\", \"M8\", 40, at = (160, 0, 0))
bolt(\"b2\", \"M8\", 40, at = (200, 0, 0))
bolt(\"b3\", \"M8\", 50, at = (240, 0, 0))
rebar(\"r1\", 12, 1000, at = (0, 100, 0), axis = (1, 0, 0))
rebar(\"r2\", 12, 1000, at = (0, 150, 0), axis = (1, 0, 0))
",
    );
    let bom = ketchup_program::bom(&model);
    let row = |wanted: &str| {
        bom.hardware
            .iter()
            .find(|row| row.item == wanted)
            .unwrap_or_else(|| panic!("{wanted} in {:?}", bom.hardware))
    };
    // Same designation, one row with the count; the two norms of M8 stay apart.
    assert_eq!(row("ISO 4032 M8").count, 2);
    assert_eq!(row("ISO 4032 M8").parts, ["n1", "n2"]);
    assert_eq!(row("ISO 4032 M8").attributes["standard"], "ISO 4032");
    assert_eq!(row("DIN 934 M8").count, 1);
    assert_eq!(row("ISO 7089 M8").attributes["standard"], "ISO 7089");
    // A bolt's length is part of its designation: 40 and 50 are two rows.
    assert_eq!(row("ISO 4017 M8x40").count, 2);
    assert_eq!(row("ISO 4017 M8x50").count, 1);
    assert_eq!(row("ISO 4017 M8x40").attributes["thread"], "M8");
    // The bars are not bought by weight: they are in the bar schedule.
    assert_eq!(bom.hardware.len(), 5, "{:?}", bom.hardware);
    for row in &bom.hardware {
        assert!(!row.attributes.contains_key("designation"), "{row:?}");
        assert!(
            row.attributes["mass_basis"].contains("simplified solid x 7850 kg/m3"),
            "{row:?}"
        );
        assert!(
            row.attributes["source"].contains("not the standard text"),
            "{row:?}"
        );
    }
    // Only the made plate is cut; none of the bought parts' attributes is a column.
    assert_eq!(bom.total_parts, 10);
    assert_eq!(bom.cut_list.len(), 1);
    assert_eq!(bom.cut_list[0].parts, ["plate"]);
    let table = cut_list_table(&model, |_| true);
    assert_eq!(table.rows.len(), 1, "{:?}", table.rows);
    assert!(matches!(&table.rows[0][1], Cell::Text(parts) if parts == "plate"));
    for column in [
        "designation",
        "standard",
        "thread",
        "thread_from_mm",
        "thread_to_mm",
    ] {
        assert!(
            !table.header.iter().any(|header| header == column),
            "{:?}",
            table.header
        );
    }
}

#[test]
fn bars_are_scheduled_as_pieces_of_a_length_not_as_hardware_or_cut_parts() {
    let model = model(
        "board(\"plate\", (300, 200, 18))
rebar(\"r1\", 12, 1000, at = (0, 100, 0), axis = (1, 0, 0))
rebar(\"r2\", 12, 1000, at = (0, 150, 0), axis = (1, 0, 0))
rebar(\"r3\", 12, 700, at = (0, 200, 0), axis = (1, 0, 0))
rebar(\"r4\", 8, 1000, at = (0, 250, 0), axis = (1, 0, 0))
box(\"own bar\", (10, 10, 2500), at = (0, 300, 0), attributes = {\"cut_length_mm\": \"2500\", \"grade\": \"S235\", \"shape\": \"straight\"})
",
    );
    let bom = ketchup_program::bom(&model);
    assert!(bom.hardware.is_empty(), "{:?}", bom.hardware);
    assert_eq!(bom.cut_list.len(), 1);
    assert_eq!(bom.cut_list[0].parts, ["plate"]);
    let rows: Vec<_> = bom
        .bar_schedule
        .iter()
        .map(|row| {
            (
                row.position,
                row.diameter_mm,
                row.grade.as_str(),
                row.shape.as_str(),
                row.cut_length_mm,
                row.count,
                row.total_length_m,
                row.parts.join(", "),
            )
        })
        .collect();
    // The rule is the cut length, not the name: a program's own bar is scheduled too.
    assert_eq!(
        rows,
        [
            (
                1,
                None,
                "S235",
                "straight",
                2500.0,
                1,
                2.5,
                "own bar".to_owned()
            ),
            (
                2,
                Some(8.0),
                "B500B",
                "straight",
                1000.0,
                1,
                1.0,
                "r4".to_owned()
            ),
            (
                3,
                Some(12.0),
                "B500B",
                "straight",
                700.0,
                1,
                0.7,
                "r3".to_owned()
            ),
            (
                4,
                Some(12.0),
                "B500B",
                "straight",
                1000.0,
                2,
                2.0,
                "r1, r2".to_owned()
            ),
        ]
    );
    // The mass is derived: 2 x 0.888 kg; a piece without mass_kg leaves it out.
    let mass = bom.bar_schedule[3].mass_kg.unwrap();
    assert!((mass - 1.776).abs() <= 0.01776, "{mass}");
    assert_eq!(bom.bar_schedule[0].mass_kg, None);
    let table = ketchup_program::bom::bar_schedule_table(&bom.bar_schedule);
    assert_eq!(
        table
            .csv(ketchup_program::table::DecimalSeparator::Comma)
            .lines()
            .take(2)
            .collect::<Vec<_>>(),
        [
            "\u{feff}position;diameter_mm;grade;shape;cut_length_mm;count;total_length_m;mass_kg;parts",
            "1;;S235;straight;2500;1;2,5;;own bar"
        ]
    );
}

#[test]
fn metric_threads_give_the_coarse_pitch_and_the_tap_drill_of_the_shop() {
    let threads = ketchup_program::library_metric_threads().unwrap();
    // A drill in 0.1 mm steps: M8 and M12 are not D - P (6.75, 10.25; TH-1).
    for (size, pitch_mm, tap_drill_mm) in [
        ("M3", 0.5, 2.5),
        ("M8", 1.25, 6.8),
        ("M12", 1.75, 10.2),
        ("M24", 3.0, 21.0),
    ] {
        let thread = threads[size];
        assert!(close(thread.pitch_mm, pitch_mm), "{size}: {thread:?}");
        assert!(
            close(thread.tap_drill_mm, tap_drill_mm),
            "{size}: {thread:?}"
        );
    }
    assert!(!threads.contains_key("M7"));
    // The library's metric sizes, as DIN 975 rods offer them.
    let sizes = [
        "M10", "M12", "M14", "M16", "M18", "M20", "M22", "M24", "M3", "M4", "M5", "M6", "M8",
    ];
    assert!(threads.keys().eq(sizes.iter()), "{threads:?}");
}

/// Export checks a tapped hole against the geometry crate's table, which the
/// manufacturing crate can reach; it must hold what the prelude holds.
#[test]
fn the_prelude_thread_tables_are_the_tapping_table_export_checks_against() {
    let threads = ketchup_program::library_metric_threads().unwrap();
    let table = ketchup_geometry::tapping::METRIC_COARSE_TAPPING;
    assert_eq!(threads.len(), table.len());
    for (size, tapping) in table {
        assert_eq!(threads.get(size), Some(&tapping), "{size}");
    }
}
