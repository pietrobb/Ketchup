//! Standard nuts and washers from the library (topic fasteners): catalogue
//! dimensions, refused sizes and what the cut list says about them.

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
fn the_cut_list_names_the_designation_standard_and_mass_basis() {
    let model = model(
        "nut(\"n1\", \"M8\")
nut(\"n2\", \"M8\", at = (40, 0, 0))
nut(\"d1\", \"M8\", norm = \"DIN 934\", at = (80, 0, 0))
washer(\"w1\", \"M8\", at = (120, 0, 0))
",
    );
    let table = cut_list_table(&model, |_| true);
    let column = |name: &str| {
        table
            .header
            .iter()
            .position(|header| header == name)
            .unwrap_or_else(|| panic!("{name} in {:?}", table.header))
    };
    let (designation, standard, basis, count, source) = (
        column("designation"),
        column("standard"),
        column("mass_basis"),
        column("count"),
        column("source"),
    );
    let text = |cell: &Cell| match cell {
        Cell::Text(text) => text.clone(),
        Cell::Count(count) => count.to_string(),
        Cell::Number(value) => value.to_string(),
    };
    let row = |wanted: &str| {
        table
            .rows
            .iter()
            .find(|row| text(&row[designation]) == wanted)
            .unwrap_or_else(|| panic!("{wanted} in {:?}", table.rows))
    };
    // Same designation, one row with the count; the two norms of M8 stay apart.
    assert_eq!(text(&row("ISO 4032 M8")[count]), "2");
    assert_eq!(text(&row("ISO 4032 M8")[standard]), "ISO 4032");
    assert_eq!(text(&row("DIN 934 M8")[count]), "1");
    assert_eq!(text(&row("ISO 7089 M8")[standard]), "ISO 7089");
    assert_eq!(table.rows.len(), 3, "{:?}", table.rows);
    for row in &table.rows {
        assert!(
            text(&row[basis]).contains("simplified solid x 7850 kg/m3"),
            "{row:?}"
        );
        assert!(
            text(&row[source]).contains("not the standard text"),
            "{row:?}"
        );
    }
}
