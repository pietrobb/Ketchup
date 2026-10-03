use ketchup_program::{evaluate, run};
use std::collections::BTreeMap;

const SOURCE: &str = r#"
p=param("position",40)
a=box("anchor",(10,10,10),at=(-100,0,0))
b=box("b",(10,20,30),at=(110,20,30))
hole(b,"z+",at=(4,5),diameter=2,depth=6)
c=component("child",[b])
i=instance("inner",c,at=(100,0,0),x=(0,1,0))
g=component("parent",[c,i])
j=instance("copy",g,at=(400,0,0))
joint(a,g,kind="motion",motion=slide((0,2,0),0,100),position=p)
"#;

#[test]
fn poses_nested_assembly_not_its_other_copy_and_uses_reference_holes() {
    for position in [40.0, 10.0, 0.0, 120.0] {
        let (evaluated, report) = run(
            "motion.star",
            SOURCE,
            &BTreeMap::from([("position".into(), position)]),
        )
        .unwrap();
        let model = &evaluated.model;
        assert_eq!(
            model.part("b").unwrap().at_mm,
            [110.0, 20.0 + position, 30.0]
        );
        assert_eq!(
            model.part("inner/b").unwrap().at_mm,
            [80.0, 110.0 + position, 30.0]
        );
        assert_eq!(model.part("copy/b").unwrap().at_mm, [510.0, 20.0, 30.0]);
        assert_eq!(
            evaluated.reference_model().part("b").unwrap().at_mm,
            [110.0, 20.0, 30.0]
        );
        let part = model.part("b").unwrap();
        let (_, (entry, _)) = part.finished_holes().next().unwrap();
        assert_eq!(part.to_world(entry), [114.0, 25.0 + position, 60.0]);
        assert_eq!(report.bom.total_parts, 5);
        assert_eq!(
            report
                .issues
                .iter()
                .filter(|i| i.kind == "joint_out_of_range")
                .count(),
            usize::from(position > 100.0)
        );
    }
}

#[test]
fn rotating_a_group_uses_world_pivot_and_moves_internal_fasteners() {
    let source = r#"
a=box("anchor",(5,5,5))
b=box("b",(10,10,10),at=(110,20,30))
c=box("c",(10,10,10),at=(110,20,40))
joint(b,c,kind="fixed",fasteners=[(115,25,40)])
g=group("moving",[b,c])
joint(a,g,kind="motion",motion=rotate_motion((0,0,1),0,90,pivot=(100,20,0)),position=90)
"#;
    let model = evaluate("rotate.star", source, &BTreeMap::new())
        .unwrap()
        .model;
    for (actual, expected) in model
        .part("b")
        .unwrap()
        .at_mm
        .into_iter()
        .zip([100.0, 30.0, 30.0])
    {
        assert!((actual - expected).abs() < 1e-9);
    }
    assert_eq!(model.joints[0].fasteners_mm, [[95.0, 35.0, 40.0]]);
}

#[test]
fn shared_internal_motion_is_instanced_in_rotated_frames_then_carried_by_parent() {
    let source = r#"
p=param("position",10)
a=box("a",(5,5,5))
b=box("b",(5,5,5),at=(20,0,0))
joint(a,b,kind="motion",motion=slide((1,0,0),0,100),position=p)
c=component("child",[a,b])
i=instance("inner",c,at=(100,0,0),x=(0,1,0))
g=component("parent",[c,i])
instance("copy",g,at=(0,200,0),x=(-1,0,0))
anchor=box("anchor",(5,5,5),at=(-100,0,0))
joint(anchor,g,kind="motion",motion=rotate_motion((0,0,1),0,90),position=90)
"#;
    let evaluated = evaluate("nested-motion.star", source, &BTreeMap::new()).unwrap();
    for (name, expected) in [
        ("b", [0., 30., 0.]),
        ("inner/b", [-30., 100., 0.]),
        ("copy/b", [-30., 200., 0.]),
        ("copy/inner/b", [-100., 170., 0.]),
    ] {
        for (actual, expected) in evaluated
            .model
            .part(name)
            .unwrap()
            .at_mm
            .into_iter()
            .zip(expected)
        {
            assert!(
                (actual - expected).abs() < 1e-8,
                "{name}: {actual} != {expected}"
            );
        }
    }
    assert_eq!(
        evaluated.reference_model().part("b").unwrap().at_mm,
        [20., 0., 0.]
    );
    assert_eq!(evaluated.model.motions.len(), 5);
}

#[test]
fn invalid_or_coupled_motion_cannot_silently_pass() {
    for (suffix, message) in [
        (
            "joint(a,c,kind='motion',motion=slide((0,1,0),0,10))",
            "requires both endpoints",
        ),
        (
            "joint(a,g,kind='motion',motion=slide((0,0,0),0,10))",
            "non-zero axis",
        ),
        (
            "joint(a,g,kind='motion',motion=slide((0,1,0),20,10))",
            "min <= max",
        ),
        (
            "joint(g,g,kind='motion',motion=slide((0,1,0),0,10))",
            "overlap",
        ),
        (
            "joint(a,g,kind='motion',motion=slide((0,1,0),0,10))\njoint(a,g,kind='motion',motion=slide((0,1,0),0,10))",
            "overlap",
        ),
        ("joint(a,g,kind='motion',position=5)", "requires motion"),
    ] {
        let source = format!("{}\n{suffix}", SOURCE.split("joint(a,g").next().unwrap());
        let error = evaluate("invalid.star", &source, &BTreeMap::new()).unwrap_err();
        assert!(error.message.contains(message), "{error}");
    }
}
