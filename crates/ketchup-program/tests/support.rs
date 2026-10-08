use ketchup_program::{evaluate, run};
use std::collections::BTreeMap;

#[test]
fn explicit_floor_translation_preserves_support_and_names_only_the_detached_body() {
    let source = "z=param('z',0)\nfloor(z)\na=box('base',(20,20,10),at=(0,0,z))\nb=box('top',(20,20,10),at=(0,0,z+10))\nbox('detached',(5,5,5),at=(100,0,z+30))";
    for z in [0., 100., -35.] {
        let (evaluated, report) =
            run("floor.star", source, &BTreeMap::from([("z".into(), z)])).unwrap();
        assert_eq!(evaluated.model.floor_z_mm, Some(z));
        let floating = report
            .issues
            .iter()
            .filter(|i| i.kind == "floating_part")
            .collect::<Vec<_>>();
        assert_eq!(floating.len(), 1, "{:?}", report.issues);
        assert_eq!(floating[0].parts, ["detached"]);
        assert!(floating[0].message.contains(&format!("z = {z})")));
        assert_eq!(
            floating[0].where_mm,
            Some(([100., 0., z + 30.], [105., 5., z + 35.]))
        );
        assert_eq!(
            evaluated.model.part("top").unwrap().at_mm,
            [0., 0., z + 10.]
        );
    }
}

#[test]
fn explicit_floor_without_any_contact_still_reports_floating_bodies() {
    let (_, report) = run(
        "floor.star",
        "floor(100)\nbox('air',(10,10,10),at=(0,0,120))",
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.kind == "floating_part" && i.parts == ["air"])
    );
    let (_, report) = run(
        "floor.star",
        "box('air',(10,10,10),at=(0,0,120))",
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(!report.issues.iter().any(|i| i.kind == "floating_part"));
}

#[test]
fn grounding_one_leaf_supports_contacts_not_detached_siblings_in_nested_copies() {
    let source = "a=box('anchor',(20,20,10),at=(0,0,100),grounded=True)\nb=box('touching',(20,20,10),at=(0,0,110))\nc=box('air',(5,5,5),at=(100,0,100))\ng=group('g',[a,b,c])\nk=component('k',[g])\ni=instance('inner',k,at=(200,0,0))\no=component('outer',[k,i])\ninstance('copy',o,at=(0,300,0))";
    let (evaluated, report) = run("ground.star", source, &BTreeMap::new()).unwrap();
    assert_eq!(
        evaluated.model.grounded_parts(),
        ["anchor", "inner/anchor", "copy/anchor", "copy/inner/anchor"]
            .map(String::from)
            .into()
    );
    let floating = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "floating_part")
        .flat_map(|issue| issue.parts.iter().cloned())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        floating,
        ["air", "inner/air", "copy/air", "copy/inner/air"]
            .map(String::from)
            .into()
    );
    assert_eq!(
        evaluated.model.part("copy/anchor").unwrap().at_mm,
        [0., 300., 100.]
    );
}

#[test]
fn assembly_grounding_is_inherited_and_instance_override_only_changes_its_root() {
    let source = "floor(0)\na=box('a',(10,10,10),at=(0,0,100),grounded=True)\nb=box('b',(10,10,10),at=(100,0,100))\ng=group('g',[a,b])\nk=component('k',[g],grounded=True)\ninstance('inherit',k,at=(0,100,0))\ninstance('free',k,at=(0,200,0),grounded=False)";
    let (_, report) = run("ground.star", source, &BTreeMap::new()).unwrap();
    let floating = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "floating_part")
        .collect::<Vec<_>>();
    assert_eq!(floating.len(), 1, "{:?}", report.issues);
    assert_eq!(floating[0].parts, ["free/b"]);
    let (_, report) = run(
        "ground.star",
        &source.replace("group('g',[a,b])", "group('g',[a,b],grounded=True)"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert!(
        !report
            .issues
            .iter()
            .any(|issue| issue.kind == "floating_part")
    );
    assert!(
        evaluate(
            "ground.star",
            "box('a',(1,1,1),grounded='yes')",
            &BTreeMap::new()
        )
        .is_err()
    );
}

/// Support spreads up a tall stack declared from the top down, the order
/// in which one pass over the parts settles only one more level, and through
/// a joint across a gap. Only the box touching nothing floats.
#[test]
fn support_reaches_the_top_of_a_stack_declared_top_down_and_across_a_joint_gap() {
    const LEVELS: usize = 400;
    let mut source = String::new();
    for level in (0..LEVELS).rev() {
        source.push_str(&format!(
            "box('level {level}',(10,10,10),at=(0,0,{}))\n",
            level * 10
        ));
    }
    source.push_str(&format!(
        "box('tag',(10,10,10),at=(12,0,{}))\njoint('tag','level {}',kind='screw',max_gap=5)\n",
        (LEVELS - 1) * 10,
        LEVELS - 1
    ));
    source.push_str("box('detached',(10,10,10),at=(100,0,50))\n");
    let (_, report) = run("stack.star", &source, &BTreeMap::new()).unwrap();
    let floating: Vec<_> = report
        .issues
        .iter()
        .filter(|issue| issue.kind == "floating_part")
        .map(|issue| issue.parts.clone())
        .collect();
    assert_eq!(floating, [["detached"]]);
}

#[test]
fn floor_requires_one_finite_world_height() {
    for (source, reason) in [
        ("floor(0)\nfloor(100)", "declared twice"),
        ("floor(1e100)", "floor z"),
        ("floor('above')", "floor z"),
    ] {
        let error = evaluate("floor.star", source, &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains(reason), "{error}");
    }
}
