use ketchup_program::{evaluate, run};
use std::collections::BTreeMap;

const ASSEMBLY: &str = r#"
w = param("width", 20)
a = box("a", (w,30,40), at=(10,20,0))
hole(a, "z+", at=(5,6), diameter=4, depth=10)
b = box("b", (w,30,10), at=(10,20,40))
g = group("inner", [a])
c = component("assembly", [g,b])
i = instance("second", c, at=(200,300,0), x=(0,1,0), z=(0,0,1))
group("all", [c,i])
"#;

#[test]
fn component_instances_rotate_all_members_and_machining_and_count_in_bom() {
    let (evaluated, report) = run("components.star", ASSEMBLY, &BTreeMap::new()).unwrap();
    let model = &evaluated.model;
    assert_eq!(model.components.len(), 1);
    assert_eq!(model.parts.len(), 4);
    assert_eq!(model.instances.len(), 1);
    let copy = model.part("second/a").unwrap();
    assert_eq!(
        copy.world_bounds(),
        ([150.0, 310.0, 0.0], [180.0, 330.0, 40.0])
    );
    let (_, (entry, _)) = copy.finished_holes().next().unwrap();
    assert_eq!(copy.to_world(entry), [174.0, 315.0, 40.0]);
    assert_eq!(
        model
            .groups
            .iter()
            .find(|group| group.name == "second")
            .unwrap()
            .members,
        ["second/inner", "second/b"]
    );
    assert_eq!(report.bom.total_parts, 4);
    assert_eq!(
        report
            .bom
            .cut_list
            .iter()
            .map(|row| row.count)
            .sum::<usize>(),
        4
    );
    let copied_lines = &evaluated.part_sources["second/a"];
    assert!(copied_lines.iter().any(|line| line.first == 3));
    assert!(copied_lines.iter().any(|line| line.first == 4));
    assert!(copied_lines.iter().any(|line| line.first == 8));
    let wider = evaluate(
        "components.star",
        ASSEMBLY,
        &BTreeMap::from([("width".into(), 35.0)]),
    )
    .unwrap();
    for name in ["a", "b", "second/a", "second/b"] {
        assert_eq!(wider.model.part(name).unwrap().size_mm[0], 35.0);
    }
}

#[test]
fn nested_components_expand_transforms_holes_sources_and_bom_without_losing_hierarchy() {
    let source = r#"
a=box("a",(10,20,30),at=(1,2,3))
hole(a,"z+",at=(4,5),diameter=2,depth=6)
c=component("child",[a])
i=instance("inner",c,at=(100,200,0),x=(0,1,0))
p=component("parent",[group("pair",[c,i])])
j=instance("outer",p,at=(400,500,0),x=(0,1,0))
g=component("grand",[p,j])
instance("last",g,at=(1000,0,0),x=(-1,0,0))
"#;
    let (evaluated, report) = run("nested.star", source, &BTreeMap::new()).unwrap();
    let model = &evaluated.model;
    assert_eq!(model.components.len(), 3);
    assert_eq!(model.parts.len(), 8);
    assert_eq!(report.bom.total_parts, 8);
    assert_eq!(
        model.part("outer/inner/a").unwrap().at_mm,
        [199.0, 598.0, 3.0]
    );
    let last = model.part("last/outer/inner/a").unwrap();
    assert_eq!(last.at_mm, [801.0, -598.0, 3.0]);
    assert_eq!(
        last.world_bounds(),
        ([801.0, -598.0, 3.0], [811.0, -578.0, 33.0])
    );
    let (_, (entry, _)) = last.finished_holes().next().unwrap();
    assert_eq!(last.to_world(entry), [805.0, -593.0, 33.0]);
    for line in [2, 3, 5, 7, 9] {
        assert!(
            evaluated.part_sources["last/outer/inner/a"]
                .iter()
                .any(|lines| lines.first == line)
        );
    }
    let group = model
        .groups
        .iter()
        .find(|group| group.name == "last/outer/pair")
        .unwrap();
    assert_eq!(group.members, ["last/outer/child", "last/outer/inner"]);
}

#[test]
fn shared_instances_keep_boolean_tools_in_the_same_relative_frame() {
    let source = r#"
a = box("a", (30,40,50), at=(10,20,0))
tool = box("tool", (10,10,50), at=(15,25,0), tool=True)
subtract(a, tool)
c = component("c", [a])
instance("i", c, at=(100,200,300), x=(0,1,0))
"#;
    let model = evaluate("boolean.star", source, &BTreeMap::new())
        .unwrap()
        .model;
    let copy = model.part("i/a").unwrap();
    assert_eq!(copy.at_mm, [80.0, 210.0, 300.0]);
    let tool = &copy.booleans().next().unwrap().tool;
    assert_eq!(tool.at_mm, [75.0, 215.0, 300.0]);
    assert_eq!(
        tool.world_bounds(),
        ([65.0, 215.0, 300.0], [75.0, 225.0, 350.0])
    );
}

#[test]
fn instances_copy_internal_joints_and_fastener_locations() {
    let source = r#"
a = box("a", (20,30,40))
b = box("b", (20,30,10), at=(0,0,40))
joint(a, b, name="j", kind="test", fasteners=[(5,6,40)], fastener="pin")
c = component("c", [a,b])
instance("i", c, at=(100,200,300), x=(0,1,0))
"#;
    let (evaluated, report) = run("joints.star", source, &BTreeMap::new()).unwrap();
    let copied = &evaluated.model.joints[1];
    assert_eq!(copied.name, "i/j");
    assert_eq!(copied.parts, ["i/a", "i/b"]);
    assert_eq!(copied.fasteners_mm, [[94.0, 205.0, 340.0]]);
    assert_eq!(report.bom.hardware[0].count, 2);
}

#[test]
fn invalid_component_edits_and_names_fail_instead_of_diverging_from_shared_geometry() {
    let base = "a=box(\"a\",(10,20,30))\nc=component(\"c\",[a])\n";
    for (suffix, message) in [
        ("instance(\"i\", \"missing\")", "declare component() first"),
        ("instance(\"a\", c)", "unique part or group name"),
        ("instance(\"i\", c, x=(0,0,1))", "not parallel"),
        ("instance(\"i\", c, at=(1e300,0,0))", "instance at"),
        (
            "place(a, origin=(1,2,3), x=(1,0,0), z=(0,0,1))",
            "finish machining",
        ),
        (
            "instance(\"i\", c)\nhole(\"i/a\", \"z+\", at=(3,4), diameter=2, depth=2)",
            "edited separately",
        ),
        ("component(\"nested\", [c,c])", "lists \"c\" twice"),
        (
            "box(\"i/a\",(1,1,1))\ninstance(\"i\",c)",
            "unique part or group name",
        ),
    ] {
        let error =
            evaluate("invalid.star", &format!("{base}{suffix}"), &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains(message), "{suffix}: {error}");
    }
}
