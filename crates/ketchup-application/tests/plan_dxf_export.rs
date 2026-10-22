//! A plan made by a program exports to DXF: profiles lifted off z = 0 in a
//! plane parallel to XY are drawn where they lie in plan.
use ketchup_application::DocumentSession;
use ketchup_manufacturing::dxf_export::export_visible_profiles_dxf;
use ketchup_model::document::{ProfileSegment, RuleProgramSource};
use ketchup_model::import::{DxfImportOptions, inspect_dxf};

fn applied(source: &str) -> DocumentSession {
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(
            RuleProgramSource {
                variants: Default::default(),
                file_name: "plan.star".into(),
                source: source.into(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap();
    session
}

#[test]
fn a_profile_extruded_above_the_floor_exports_where_it_lies_in_plan() {
    // As the reported anchor plan: a marker extruded from z = 5 next to a box on the floor.
    let session = applied(
        "extrude('marker', profile=[(-50, -10), (50, -10), (50, 10), (-50, 10)], distance=20, at=(1000, 2000, 5))\n\
         box('outline', (3000, 40, 5), at=(0, 0, 0))\n",
    );
    let exported = export_visible_profiles_dxf(&session.snapshot()).unwrap();
    let inspected = inspect_dxf(&exported.dxf, DxfImportOptions::new(None)).unwrap();
    let mut corners = inspected
        .profiles()
        .iter()
        .map(|profile| {
            let mut points = profile
                .segments()
                .iter()
                .map(|segment| match segment {
                    ProfileSegment::Line { start_mm, .. } => start_mm.map(f64::round),
                    other => panic!("both outlines are straight: {other:?}"),
                })
                .collect::<Vec<_>>();
            points.sort_by(|a, b| a.partial_cmp(b).unwrap());
            points
        })
        .collect::<Vec<_>>();
    corners.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_eq!(
        corners,
        [
            vec![[0.0, 0.0], [0.0, 40.0], [3000.0, 0.0], [3000.0, 40.0]],
            vec![
                [950.0, 1990.0],
                [950.0, 2010.0],
                [1050.0, 1990.0],
                [1050.0, 2010.0]
            ],
        ]
    );
    assert!(
        exported.loss_report.contains("elevated_profile_count=1"),
        "{}",
        exported.loss_report
    );
}
