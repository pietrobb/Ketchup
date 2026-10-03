use ketchup_program::{evaluate, run};
use std::collections::BTreeMap;

const BASE: &str = "a=box('a',(100,100,20))\nb=box('b',(100,100,20),at=(0,0,20))\nha=hole(a,'z+',at=(40,50),diameter=8,depth=8,id='mount')\nhb=hole(b,'z-',at=(40,50),diameter=8,depth=8,id='mate')\npin=box('pin',(2,2,8),at=(39,49,16))\n";

#[test]
fn moving_both_endpoints_carries_fasteners_once_in_either_order() {
    let declaration =
        "joint(a,b,kind='mount',fasteners=[(40,50,20)],links=[joint_link([ha,hb])])\n";
    for order in ["[a,b]", "[b,a]"] {
        let text = format!(
            "{BASE}{declaration}for p in {order}:\n    rotate(p,axis=(1,2,3),angle=31,pivot=(0,0,0))\n    move(p,by=(70,-40,90))\n"
        );
        let evaluated = evaluate("moved.star", &text, &BTreeMap::new()).unwrap();
        let centre = evaluated.model.joints[0].fasteners_mm[0];
        for name in ["a", "b"] {
            let part = evaluated.model.part(name).unwrap();
            let entry = part.to_world(part.holes().next().unwrap().entry_mm);
            assert!(ketchup_geometry::linalg::distance(entry, centre) < 1e-6);
        }
    }
    let (apart, report) = run(
        "apart.star",
        &format!("{BASE}{declaration}move(b,by=(0,0,30))"),
        &BTreeMap::new(),
    )
    .unwrap();
    assert_eq!(apart.model.joints[0].fasteners_mm[0], [40., 50., 20.]);
    assert!(
        report
            .issues
            .iter()
            .any(|i| i.kind == "joint_without_contact")
    );
}

#[test]
fn generic_links_remap_operations_and_physical_hardware_in_rotated_nested_instances() {
    let source = format!(
        "{BASE}joint(a,b,kind='mount',links=[joint_link([ha,hb],hardware=[pin])])\nc=component('pair',[a,b,pin])\ninstance('turned',c,at=(200,0,0),x=(0,0,-1),z=(1,0,0))\nn=component('nested',[c])\ninstance('outer',n,at=(0,300,0))\n"
    );
    let evaluated = evaluate("links.star", &source, &BTreeMap::new()).unwrap();
    for prefix in ["", "turned/", "outer/"] {
        let joint = evaluated
            .model
            .joints
            .iter()
            .find(|joint| joint.parts[0] == format!("{prefix}a"))
            .unwrap();
        let link = &joint.links[0];
        assert_eq!(link.hardware_parts, [format!("{prefix}pin")]);
        assert_eq!(link.operations[0].part, format!("{prefix}a"));
        assert_eq!(link.operations[0].id, "mount");
        assert_eq!(link.operations[1].part, format!("{prefix}b"));
        for operation in &link.operations {
            assert!(
                evaluated
                    .model
                    .part(&operation.part)
                    .unwrap()
                    .holes()
                    .any(|hole| hole.id == operation.id)
            );
        }
    }
    let turned = evaluated.model.part("turned/a").unwrap();
    assert_eq!(
        turned.to_world(turned.holes().next().unwrap().entry_mm),
        [220., 50., -40.]
    );
}

#[test]
fn generated_hole_ids_and_named_pockets_are_reusable_generic_links() {
    let source = "a=box('a',(100,100,20))\nb=box('b',(100,100,20),at=(0,0,20))\nh=hole(a,'z+',at=(40,50),diameter=8,depth=8,through=False)\npocket(b,'z-',rect=(35,45,45,55),depth=8,id='socket')\njoint(a,b,kind='mount',links=[joint_link([h,struct(part=b,id='socket')])])\n";
    let evaluated = evaluate("links.star", source, &BTreeMap::new()).unwrap();
    let link = &evaluated.model.joints[0].links[0];
    assert_eq!(link.operations[0].id, "h1");
    assert_eq!(link.operations[1].id, "socket");
    assert!(link.hardware_parts.is_empty());
}

#[test]
fn bad_references_and_duplicate_owners_are_integrity_errors() {
    for suffix in [
        "joint(a,b,kind='test',links=[joint_link([struct(part=a,id='missing')])])",
        "joint(a,b,kind='test',links=[joint_link([struct(part='missing',id='mount')])])",
        "joint(a,b,kind='test',links=[joint_link([ha,ha])])",
        "joint(a,b,kind='one',links=[joint_link([ha])])\njoint(a,b,kind='two',links=[joint_link([ha])])",
        "joint(a,b,kind='test',links=[joint_link([ha],hardware=['missing'])])",
        "joint(a,b,kind='test',links=[joint_link([ha],hardware=[pin])])\ncomponent('pair',[a,b])",
    ] {
        assert!(
            evaluate("links.star", &format!("{BASE}{suffix}"), &BTreeMap::new()).is_err(),
            "{suffix}"
        );
    }
}

#[test]
fn coincident_metadata_and_standalone_machining_do_not_acquire_owners() {
    let source = format!("{BASE}joint(a,b,kind='mount',fasteners=[(40,50,20)],fastener='pin')");
    let evaluated = evaluate("links.star", &source, &BTreeMap::new()).unwrap();
    assert!(evaluated.model.joints[0].links.is_empty());
    assert_eq!(evaluated.model.part("a").unwrap().holes().count(), 1);
}

#[test]
fn adjacent_source_helpers_replace_and_delete_in_either_order_without_residual_holes() {
    use ketchup_program::document::{ProgramDocument, ProgramSource};
    let parts = "p=box('shared',(18,120,120))\na=box('left',(80,120,18),at=(-80,0,40))\nb=box('right',(80,120,18),at=(18,0,40))\nhole(p,'z+',at=(9,60),diameter=4,depth=5,id='service')\n";
    let left = "dowels(p,a,count=2,margin=20)\n";
    let right = "dowels(p,b,count=2,margin=35)\n";
    for reverse in [false, true] {
        let (first, second, changed_name, preserved_name) = if reverse {
            (right, left, "right", "left")
        } else {
            (left, right, "left", "right")
        };
        let source = |text: String| ProgramSource {
            file_name: "adjacent.star".into(),
            source: text,
            overrides: BTreeMap::new(),
        };
        let mut document = ProgramDocument::new(source(format!("{parts}{first}{second}"))).unwrap();
        let before = document.evaluated().model.clone();
        let changed = first.replace("count=2", "count=1");
        document
            .replace(source(format!("{parts}{changed}{second}")))
            .unwrap();
        assert_eq!(
            document
                .evaluated()
                .model
                .part(changed_name)
                .unwrap()
                .holes()
                .count(),
            1
        );
        assert_eq!(
            document.evaluated().model.part(preserved_name),
            before.part(preserved_name)
        );
        let after = document.evaluated().model.clone();
        document.undo().unwrap();
        assert_eq!(document.evaluated().model, before);
        document.redo().unwrap();
        assert_eq!(document.evaluated().model, after);
        document
            .replace(source(format!("{parts}{second}")))
            .unwrap();
        let model = &document.evaluated().model;
        assert_eq!(model.part(changed_name).unwrap().holes().count(), 0);
        assert_eq!(model.part("shared").unwrap().holes().count(), 3);
        assert_eq!(model.part(preserved_name), before.part(preserved_name));
        assert_eq!(model.joints.len(), 1);
        assert_eq!(model.joints[0].links.len(), 2);
        let reopened = run("adjacent.star", &document.source().source, &BTreeMap::new())
            .unwrap()
            .0;
        assert_eq!(&reopened.model, model);
    }
}
