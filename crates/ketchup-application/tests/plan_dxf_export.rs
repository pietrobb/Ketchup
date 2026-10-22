//! A plan made by a program exports to DXF: profiles lifted off z = 0 in a
//! plane parallel to XY are drawn where they lie in plan, and the program's
//! annotations as TEXT, DIMENSION and POLYLINE on the layer of their tag.
use ketchup_application::DocumentSession;
use ketchup_manufacturing::dxf_export::{
    DxfProfileExportError, export_visible_plan_dxf, export_visible_profiles_dxf,
};
use ketchup_model::document::{CanonicalCommand, CommandBatch, ProfileSegment, RuleProgramSource};
use ketchup_model::import::{DxfImportOptions, inspect_dxf};

fn applied(source: &str) -> (DocumentSession, serde_json::Value) {
    let mut session = DocumentSession::default();
    let result = session
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
    let report = serde_json::to_value(&result.report).unwrap();
    (session, report)
}

#[test]
fn a_profile_extruded_above_the_floor_exports_where_it_lies_in_plan() {
    // As the reported anchor plan: a marker extruded from z = 5 next to a box on the floor.
    let (session, _) = applied(
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

/// The entities of one section of a DXF file, each as its type and its pairs.
fn section(dxf: &[u8], name: &str) -> Vec<(String, Vec<(i32, String)>)> {
    let text = std::str::from_utf8(dxf).unwrap();
    let lines = text.lines().collect::<Vec<_>>();
    let pairs = lines
        .chunks(2)
        .map(|pair| (pair[0].trim().parse::<i32>().unwrap(), pair[1].to_owned()))
        .collect::<Vec<_>>();
    let start = pairs
        .windows(2)
        .position(|window| window[0] == (0, "SECTION".into()) && window[1] == (2, name.into()))
        .unwrap_or_else(|| panic!("no {name} section"));
    let mut entities = Vec::new();
    for (code, value) in &pairs[start + 2..] {
        match (code, value.as_str()) {
            (0, "ENDSEC") => break,
            (0, kind) => entities.push((kind.to_owned(), Vec::new())),
            _ => entities.last_mut().unwrap().1.push((*code, value.clone())),
        }
    }
    entities
}

fn number(pairs: &[(i32, String)], code: i32) -> f64 {
    pairs
        .iter()
        .find(|(found, _)| *found == code)
        .unwrap_or_else(|| panic!("no group {code} in {pairs:?}"))
        .1
        .parse()
        .unwrap()
}

fn text(pairs: &[(i32, String)], code: i32) -> &str {
    &pairs.iter().find(|(found, _)| *found == code).unwrap().1
}

/// A 35 x 35 m outline with eight numbered anchor points 5 m apart along its
/// south side, as the anchor plan that had to be drawn elsewhere.
const ANCHOR_PLAN: &str = "side = 35000\n\
    polyline('outline', [(0, 0), (side, 0), (side, side), (0, side)], closed=True, tags='plan')\n\
    south = [(i * 5000, 0) for i in range(8)]\n\
    for i, p in enumerate(south):\n    \
    label = 'S-' + ('0' + str(i + 1))[-2:]\n    \
    text(label, label, at=(p[0] + 150, p[1] + 150), height_mm=3.5, tags='plan')\n\
    chain_dim('south', south, offset_mm=-1500, tags='plan')\n";

#[test]
fn an_anchor_plan_exports_one_text_per_point_and_a_measured_dimension_between_neighbours() {
    let (session, report) = applied(ANCHOR_PLAN);
    let snapshot = session.snapshot();
    assert_eq!(
        snapshot.occurrences().count(),
        0,
        "annotations are no parts"
    );
    assert_eq!(report["bom"]["cut_list"], serde_json::json!([]), "{report}");

    let exported = export_visible_plan_dxf(&snapshot, 50.0).unwrap();
    let entities = section(&exported.dxf, "ENTITIES");
    let of = |kind: &str| {
        entities
            .iter()
            .filter(|(found, _)| found == kind)
            .map(|(_, pairs)| pairs.as_slice())
            .collect::<Vec<_>>()
    };
    let (texts, dimensions) = (of("TEXT"), of("DIMENSION"));
    assert_eq!(
        (texts.len(), dimensions.len(), of("POLYLINE").len()),
        (8, 7, 1)
    );

    // Each label at its point, 3.5 mm tall on paper at 1:50, on the layer of its tag.
    for (index, pairs) in texts.iter().enumerate() {
        assert_eq!(text(pairs, 1), format!("S-0{}", index + 1));
        assert_eq!(text(pairs, 8), "plan");
        let x = 5000.0 * f64::from(u8::try_from(index).unwrap()) + 150.0;
        assert_eq!(
            [number(pairs, 10), number(pairs, 20), number(pairs, 40)],
            [x, 150.0, 175.0]
        );
    }

    // Each dimension measures its two neighbouring points; its block writes the value.
    let blocks = section(&exported.dxf, "BLOCKS");
    for pairs in dimensions {
        assert_eq!(text(pairs, 8), "plan");
        let (first, second) = (
            [number(pairs, 13), number(pairs, 23)],
            [number(pairs, 14), number(pairs, 24)],
        );
        let measured = (second[0] - first[0]).hypot(second[1] - first[1]);
        assert!((measured - 5000.0).abs() <= 0.5, "{measured}");
        assert_eq!([first[1], second[1]], [0.0, 0.0]);
        let block = text(pairs, 2);
        let start = blocks
            .iter()
            .position(|(kind, block_pairs)| kind == "BLOCK" && text(block_pairs, 2) == block)
            .unwrap();
        let value = blocks[start..]
            .iter()
            .find(|(kind, _)| kind == "TEXT")
            .map(|(_, block_pairs)| text(block_pairs, 1))
            .unwrap();
        assert_eq!(value, "5000");
        // The dimension line is 1.5 m south of the points.
        assert_eq!(number(pairs, 20), -1500.0);
    }
    assert!(
        exported.loss_report.contains("scale=1:50"),
        "{}",
        exported.loss_report
    );
    assert!(
        exported
            .loss_report
            .contains("annotation_dimension_count=7"),
        "{}",
        exported.loss_report
    );

    // Hiding their tag leaves nothing to draw.
    let plan = snapshot
        .tags()
        .find(|tag| tag.name() == "plan")
        .unwrap()
        .id();
    let hidden = snapshot
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: plan,
                visible: false,
            },
        ]))
        .unwrap();
    assert_eq!(
        export_visible_plan_dxf(&hidden, 50.0),
        Err(DxfProfileExportError::Empty)
    );
    assert_eq!(
        export_visible_plan_dxf(&snapshot, 0.5),
        Err(DxfProfileExportError::InvalidScale)
    );
}
