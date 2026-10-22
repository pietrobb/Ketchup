//! Drawing annotations a program declares: texts at points, measured dimensions
//! between them, a leader and an outline. They reach the document and its file,
//! follow their tag, and are no parts.
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::annotation::{AnnotationMark, dimension_text};
use ketchup_model::document::{CanonicalCommand, CommandBatch, RuleProgramSource};

fn apply(session: &mut DocumentSession, source: &str) -> serde_json::Value {
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
    serde_json::to_value(&result.report).unwrap()
}

/// Two crossing markers, so the report has a collision to compare.
const PARTS: &str = "box('marker 1', (300, 300, 20), at=(-150, -150, 5))\n\
                     box('marker 2', (300, 300, 20), at=(0, 0, 5))\n";

/// A 10 x 4 m outline with five numbered points along its south side.
const PLAN: &str = "points = [(i * 2500, 0, 5) for i in range(5)]\n\
                    for i, p in enumerate(points):\n    \
                    text('S-' + ('0' + str(i + 1))[-2:], 'S-' + ('0' + str(i + 1))[-2:], at=(p[0] + 100, p[1] + 100, 5), height_mm=3.5, tags='plan')\n\
                    chain_dim('south', points, offset_mm=-1000, tags='plan')\n\
                    leader('anchor', 'M24', point=points[0], at=(-1500, -1500), tags='plan')\n\
                    polyline('outline', [(0, 0), (10000, 0), (10000, 4000), (0, 4000)], closed=True, tags='plan')\n";

#[test]
fn annotations_reach_the_document_follow_their_tag_and_are_no_parts() {
    let _turn = crate::integration_support::file_turn();
    let mut plain = DocumentSession::default();
    let without = apply(&mut plain, PARTS);
    let mut session = DocumentSession::default();
    let with = apply(&mut session, &format!("{PARTS}{PLAN}"));

    let snapshot = session.snapshot();
    let annotations = snapshot.annotations();
    let kinds = |kind: &str| annotations.iter().filter(|a| a.kind() == kind).count();
    assert_eq!(
        (
            kinds("text"),
            kinds("dimension"),
            kinds("leader"),
            kinds("line")
        ),
        (5, 4, 1, 1)
    );
    let measured = annotations
        .iter()
        .filter_map(|annotation| annotation.dimension().map(|layout| layout.value_mm))
        .collect::<Vec<_>>();
    assert_eq!(measured, [2500.0; 4]);
    assert_eq!(annotations[5].name, "south/1");
    assert_eq!(annotations[5].text().as_deref(), Some("2500"));
    assert_eq!(dimension_text(measured[0]), "2500");
    let AnnotationMark::Text { at_mm, .. } = &annotations[1].mark else {
        panic!("S-02 is a text")
    };
    assert_eq!(*at_mm, [2600.0, 100.0, 5.0]);

    // No part, no cut-list row, no collision more than the parts alone have.
    assert_eq!(snapshot.occurrences().count(), 2);
    assert_eq!(with["bom"], without["bom"]);
    assert_eq!(with["issues"], without["issues"]);
    assert_eq!(with["relations"], without["relations"]);

    // The tag shows and hides them; the file keeps them.
    let plan = snapshot
        .tags()
        .find(|tag| tag.name() == "plan")
        .unwrap()
        .id();
    assert!(annotations.iter().all(|a| a.tags.contains(&plan)));
    let proposal = session
        .plan_commands(CommandBatch::new(vec![
            CanonicalCommand::SetTagVisibility {
                id: plan,
                visible: false,
            },
        ]))
        .unwrap();
    session.apply_proposal(&proposal).unwrap();
    assert!(
        session.rule_program().is_some(),
        "hiding a tag keeps the program"
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("plan.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.snapshot().annotations(), annotations);

    // Without them in the program they leave the document, and so does their tag.
    apply(&mut session, PARTS);
    let snapshot = session.snapshot();
    assert!(snapshot.annotations().is_empty());
    assert!(snapshot.tags().all(|tag| tag.name() != "plan"));
    session.undo().unwrap();
    assert_eq!(session.snapshot().annotations().len(), 11);
}

#[test]
fn an_annotation_that_cannot_be_drawn_names_its_line() {
    let mut session = DocumentSession::default();
    let error = session
        .apply_rule_program(
            RuleProgramSource {
                variants: Default::default(),
                file_name: "plan.star".into(),
                source: "box('a', (10, 10, 10))\ndim('d', (5, 5), (5, 5, 0))\n".into(),
                overrides: Default::default(),
            },
            false,
        )
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("dim(\"d\")"), "{error}");
    assert!(error.contains("less than 0.001 mm apart"), "{error}");
    assert!(error.contains(":2:"), "{error}");
}
