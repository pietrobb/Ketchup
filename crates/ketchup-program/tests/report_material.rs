//! Independent material-area oracle for the reviewed example report changes.
use ketchup_program::{RelationKind, run};

#[test]
fn cabinet_contacts_exclude_bores_and_the_full_groove_strip() {
    let (_, report) = run(
        "cabinet.star",
        include_str!("../../../examples/programs/cabinet.star"),
        &Default::default(),
    )
    .expect("cabinet evaluates");
    assert!(report.ok, "{:?}", report.issues);
    let contacts: Vec<_> = report
        .relations
        .iter()
        .filter(|relation| relation.kind == RelationKind::Contact)
        .collect();
    assert_eq!(contacts.len(), 4);
    // The side's groove removes the entire 18 mm strip at each panel end.
    let material_area = 350.0 * 18.0 - 4.0 * 18.0 - 2.0 * std::f64::consts::PI * 4.0_f64.powi(2);
    for relation in contacts {
        assert!(
            (relation.area_mm2.expect("contact area") - material_area).abs() < 1.0,
            "{relation:?}"
        );
        assert!(relation.joint.as_deref().unwrap().starts_with("dowel:"));
    }
}
