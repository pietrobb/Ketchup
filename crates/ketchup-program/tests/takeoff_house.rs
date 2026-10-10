//! The material takeoff of the timber-frame house example: every part carries a
//! material from the program, and leaving out the concept layer counts only the
//! construction.
use ketchup_program::takeoff::{material_takeoff, material_takeoff_of_all};
use std::collections::BTreeMap;

const HOUSE: &str = include_str!("../../../examples/programs/tiny-house.star");

#[test]
fn house_takeoff_groups_the_construction_by_material_and_section() {
    let model = ketchup_program::evaluate("tiny-house.star", HOUSE, &BTreeMap::new())
        .expect("the house evaluates")
        .model;
    // production(tags = ["konštrukcia"]) makes the timber frame only: the
    // takeoff of everything counts exactly those parts.
    let made = model
        .parts
        .iter()
        .filter(|part| part.tags.contains("konštrukcia"))
        .count();
    assert!(made > 0 && made < model.parts.len());
    let all = material_takeoff_of_all(&model);
    assert_eq!(all.counted_parts, made);
    assert_eq!(all.counted_parts + all.excluded_parts, model.parts.len());
    assert!(
        model
            .parts
            .iter()
            .all(|part| part.material.as_deref().is_some_and(|m| m != "unspecified")),
        "every house part names its material"
    );

    // Showing everything does not reach the concept, the slab or the furniture.
    let shown = model
        .parts
        .iter()
        .map(|part| (part.name.clone(), None))
        .collect::<BTreeMap<_, _>>();
    let takeoff = material_takeoff(&model, &shown);
    assert_eq!(takeoff.counted_parts, made);
    for outside in ["drevostavba", "betón", "matrac", "liatina"] {
        assert!(
            takeoff
                .materials
                .iter()
                .all(|total| total.material != outside),
            "{outside} is not made"
        );
    }
    for material in [
        "OSB 3",
        "sadrokartón",
        "drevovláknitá doska",
        "plechová krytina",
    ] {
        let total = takeoff
            .materials
            .iter()
            .find(|total| total.material == material)
            .unwrap_or_else(|| panic!("{material} is in the takeoff"));
        assert!(total.count > 0 && total.area_m2 > 0.0 && total.volume_m3 > 0.0);
    }
    // Studs and rafters are timber of one cross-section each, counted in running metres.
    let framing = takeoff
        .rows
        .iter()
        .filter(|row| row.count > 4 && row.length_m > 10.0)
        .count();
    assert!(framing > 0, "{:#?}", takeoff.rows);
    let row_parts: usize = takeoff.rows.iter().map(|row| row.count).sum();
    assert_eq!(row_parts, takeoff.counted_parts);
}

/// The imported house names its parts `<layer name> NNN` without a `/`: the
/// takeoff groups them by that name, not one row per member.
#[test]
fn numbered_part_names_share_one_category() {
    let source = include_str!("../../../examples/programs/house-project.star");
    let model = ketchup_program::evaluate("house-project.star", source, &BTreeMap::new())
        .expect("the house evaluates")
        .model;
    let takeoff = material_takeoff_of_all(&model);
    assert!(takeoff.counted_parts > 1800);
    assert!(
        takeoff.rows.len() < 300,
        "{} rows for {} parts",
        takeoff.rows.len(),
        takeoff.counted_parts
    );
    let footings = takeoff
        .rows
        .iter()
        .filter(|row| row.category == "Základy · pätky")
        .map(|row| row.count)
        .sum::<usize>();
    assert!(footings > 10, "{footings}");
}
