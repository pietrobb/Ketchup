use ketchup_program::{ExactPair, ExactShapes, run};
use std::collections::BTreeMap;

#[test]
fn anchored_group_reports_separate_contact_islands_not_unrelated_bodies() {
    let source = "d=param('d',200)\na=box('base',(100,100,10),at=(0,0,100))\nb=box('side',(10,100,100),at=(0,0,110))\nc=box('front',(10,100,100),at=(100+d,0,100))\ne=box('handle',(10,10,10),at=(110+d,0,100))\ng=group('assembly',[a,b,c,e],grounded=True)\nbox('unrelated',(1,1,1),at=(1000,0,0))";
    let (_, report) = run("contacts.star", source, &BTreeMap::new()).unwrap();
    let issue = report
        .issues
        .iter()
        .find(|i| i.kind == "disconnected_group")
        .unwrap();
    assert_eq!(issue.parts, ["base", "front", "handle", "side"]);
    assert!(issue.message.contains("[base, side]"), "{issue:?}");
    assert!(issue.message.contains("[front, handle]"), "{issue:?}");
    assert_eq!(issue.where_mm, Some(([0., 0., 100.], [320., 100., 210.])));
    assert!(!report.issues.iter().any(|i| i.kind == "floating_part"));
    let (_, fixed) = run("contacts.star", source, &BTreeMap::from([("d".into(), 0.)])).unwrap();
    assert!(
        !fixed.issues.iter().any(|i| i.kind == "disconnected_group"),
        "{fixed:?}"
    );
}

#[test]
fn contact_through_an_external_body_does_not_connect_group_members() {
    let source = "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(30,0,0))\nbox('bridge',(20,10,10),at=(10,0,0))\ngroup('g',[a,b])";
    let (_, report) = run("contacts.star", source, &BTreeMap::new()).unwrap();
    let issue = report
        .issues
        .iter()
        .find(|i| i.kind == "disconnected_group")
        .unwrap();
    assert_eq!(issue.parts, ["a", "b"]);
    assert!(!report.issues.iter().any(|i| i.kind == "floating_part"));
}

#[test]
fn contact_tolerance_does_not_hide_a_two_millimetre_gap() {
    let source = "gap=param('gap',0)\na=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(10+gap,0,0))\ngroup('g',[a,b])";
    for (gap, detached) in [(0., false), (0.005, false), (0.02, true), (2., true)] {
        let (_, report) = run(
            "contacts.star",
            source,
            &BTreeMap::from([("gap".into(), gap)]),
        )
        .unwrap();
        assert_eq!(
            report.issues.iter().any(|i| i.kind == "disconnected_group"),
            detached,
            "gap={gap}: {report:?}"
        );
    }
}

#[test]
fn nested_rotated_copies_keep_separate_group_contacts_and_member_names() {
    let source = "a=box('a',(10,10,10))\nb=box('b',(10,10,10),at=(210,0,0))\ng=group('g',[a,b])\nk=component('k',[g])\ninstance('copy',k,at=(0,500,0),x=(0,1,0))";
    let (_, report) = run("contacts.star", source, &BTreeMap::new()).unwrap();
    let issues = report
        .issues
        .iter()
        .filter(|i| i.kind == "disconnected_group")
        .collect::<Vec<_>>();
    assert!(
        issues
            .iter()
            .any(|i| i.message.starts_with("group g ") && i.parts == ["a", "b"])
    );
    assert!(
        issues
            .iter()
            .any(|i| i.message.starts_with("group copy/g ") && i.parts == ["copy/a", "copy/b"])
    );
    let (_, fixed) = run(
        "contacts.star",
        &source.replace("(210,0,0)", "(10,0,0)"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        !fixed.issues.iter().any(|i| i.kind == "disconnected_group"),
        "{fixed:?}"
    );
}

#[test]
fn non_box_contacts_remain_unverified_until_exact_solids_settle_them() {
    let source = "a=extrude('triangle',profile=[(0,0),(100,0),(0,100)],distance=20)\nb=box('corner',(20,20,20),at=(70,70,0))\ngroup('g',[a,b])";
    let (evaluated, mut report) = run("contacts.star", source, &BTreeMap::new()).unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.kind == "group_contact_unverified")
    );
    assert!(!report.issues.iter().any(|i| i.kind == "disconnected_group"));
    let mut exact = ExactShapes::default();
    exact.insert(
        "triangle",
        "corner",
        ExactPair {
            distance_mm: Some(28.284),
            ..Default::default()
        },
    );
    report.refine(&evaluated.model, &exact);
    assert!(report.issues.iter().any(|i| i.kind == "disconnected_group"));
    assert!(
        !report
            .issues
            .iter()
            .any(|i| i.kind == "group_contact_unverified")
    );
    exact.insert(
        "triangle",
        "corner",
        ExactPair {
            distance_mm: Some(0.),
            contact_area_mm2: 400.,
            ..Default::default()
        },
    );
    report.refine(&evaluated.model, &exact);
    assert!(
        !report
            .issues
            .iter()
            .any(|i| matches!(i.kind, "group_contact_unverified" | "disconnected_group"))
    );
}

#[test]
fn broken_joint_neither_connects_group_nor_provides_phantom_support() {
    let source = "a=box('anchor',(10,10,10),at=(0,0,100),grounded=True)\nb=box('loose',(10,10,10),at=(210,0,100))\njoint(a,b,kind='fixed',name='connection')\ngroup('g',[a,b])";
    let (_, report) = run("contacts.star", source, &BTreeMap::new()).unwrap();
    let joint = report
        .issues
        .iter()
        .find(|i| i.kind == "joint_without_contact")
        .unwrap();
    assert!(joint.message.contains("200 mm apart"), "{joint:?}");
    assert_eq!(joint.where_mm, Some(([0., 0., 100.], [220., 10., 110.])));
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.kind == "floating_part" && i.parts == ["loose"])
    );
    assert!(report.issues.iter().any(|i| i.kind == "disconnected_group"));
    let (_, fixed) = run(
        "contacts.star",
        &source.replace("(210,0,100)", "(10,0,100)"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        !fixed.issues.iter().any(|i| matches!(
            i.kind,
            "floating_part" | "disconnected_group" | "joint_without_contact"
        )),
        "{fixed:?}"
    );
}
