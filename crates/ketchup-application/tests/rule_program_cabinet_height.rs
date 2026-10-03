//! Cabinet clearances and opposing joinery are checked on the finished solids.
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::RuleProgramSource;
use std::collections::BTreeMap;

#[test]
fn plinth_bottom_and_brace_keep_blind_paired_bores_clear_after_rotation() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = crate::operations_support::worker();
    let text = r#"
offset=param("offset",11)
angle=param("angle",0)
bottom=board("bottom",(839,520,18),at=(0,0,80))
front=board("front-plinth",(839,18,80))
rear=board("rear-plinth",(839,18,80),at=(0,502,0))
brace=board("brace",(839,18,100),at=(0,502,98))
for rail in [front,rear]:
    dowels(rail,bottom,count=4,margin=50)
dowels(brace,bottom,count=4,margin=50,offset=offset)
for part in [bottom,front,rear,brace]:
    rotate(part,axis=(1,2,3),angle=angle,pivot=(0,0,0))
group("base",[bottom,front,rear,brace])
"#;
    let mut session = DocumentSession::default();
    for angle in [0.0, 31.0] {
        let source = RuleProgramSource {
            file_name: "plinth-brace.star".into(),
            source: text.into(),
            overrides: BTreeMap::from([("angle".into(), angle)]),
        };
        let safe = session.apply_rule_program(source.clone(), false).unwrap();
        assert!(safe.report.ok, "{:?}", safe.report.issues);
        assert_eq!(safe.model.joints.len(), 3);
        assert_eq!(safe.report.bom.hardware.len(), 1);
        assert_eq!(safe.report.bom.hardware[0].count, 12);
        let bottom = safe.model.part("bottom").unwrap();
        assert_eq!(bottom.holes().count(), 12);
        assert!(bottom.holes().all(|h| h.depth_mm == 12.0 && !h.through));
        for joint in &safe.model.joints {
            let rail = safe.model.part(&joint.parts[0]).unwrap();
            assert_eq!(rail.holes().count(), 4);
            assert!(rail.holes().all(|h| h.depth_mm == 26.0 && !h.through));
            for (link, centre) in joint.links.iter().zip(&joint.fasteners_mm) {
                assert_eq!(link.operations.len(), 2);
                for operation in &link.operations {
                    let part = safe.model.part(&operation.part).unwrap();
                    let hole = part.holes().find(|h| h.id == operation.id).unwrap();
                    assert!(
                        ketchup_geometry::linalg::distance(part.to_world(hole.entry_mm), *centre)
                            < 1e-6
                    );
                }
            }
        }
        // Independent volume oracle: every safe bore is disjoint and stays blind.
        for occurrence in safe.snapshot.occurrences() {
            let part = safe.model.part(occurrence.name()).unwrap();
            let definition = safe
                .snapshot
                .definition(occurrence.definition_id())
                .unwrap();
            let graph = ketchup_model::exact_brep_graph::ExactBRepGraph::from_snapshot(
                &safe.snapshot,
                occurrence.definition_id(),
                *definition.feature_ids().last().unwrap(),
            )
            .unwrap();
            let solid = worker.evaluate_exact_brep_graph(&graph).unwrap();
            let stock: f64 = part.size_mm.iter().product();
            let removed = if part.name == "bottom" {
                12.0 * 12.0
            } else {
                4.0 * 26.0
            };
            crate::operations_support::assert_volume(
                solid.volume_mm3,
                stock - std::f64::consts::PI * 16.0 * removed,
            );
        }
        let mut unsafe_source = source.clone();
        unsafe_source.overrides.insert("offset".into(), 0.0);
        let unsafe_model = session.apply_rule_program(unsafe_source, false).unwrap();
        let collisions: Vec<_> = unsafe_model
            .report
            .issues
            .iter()
            .filter(|i| i.kind == "opposing_holes_intersect")
            .collect();
        assert_eq!(collisions.len(), 4, "{:?}", unsafe_model.report.issues);
        assert!(collisions.iter().all(|i| i.parts == ["bottom"]));
        assert_eq!(
            unsafe_model.model.part("bottom").unwrap().holes().count(),
            12
        );
        assert_eq!(
            session.undo().unwrap().scene_query(),
            safe.snapshot.scene_query()
        );
        let repaired = session.apply_rule_program(source, false).unwrap();
        assert!(repaired.report.ok, "{:?}", repaired.report.issues);
        assert_eq!(repaired.model, safe.model);
    }
}

#[test]
fn equal_width_modules_keep_the_lower_right_650_mm_height_after_motion_and_reopen() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut source = RuleProgramSource {
        file_name: "cabinet-height.star".into(),
        source: r#"
t=18
w=875
h=1260
d=520
move=param("move",0)
def module(name,clearance,at):
    x,y,z=at
    left=board(name+"-left",(t,d,h),at=(x,y,z))
    right=board(name+"-right",(t,d,h),at=(x+w-t,y,z))
    bottom=board(name+"-bottom",(w-2*t,d,t),at=(x+t,y,z))
    top=board(name+"-top",(w-2*t,d,t),at=(x+t,y,z+h-t))
    shelf=board(name+"-shelf",(w-2*t,d,t),at=(x+t,y,z+t+clearance))
    for side in [left,right]:
        for horizontal in [bottom,top,shelf]:
            dowels(side,horizontal,dowel="8x35",count=2,margin=50)
    expect_gap(bottom,shelf,clearance)
    return component(name,[group(name+"-panels",[left,right,bottom,top,shelf])])
a=module("lower-0",400,(0,0,80))
v=module("lower-3",650,(3*w,0,80))
b=instance("lower-1",a,at=(w,0,0))
c=instance("lower-2",a,at=(2*w,0,0))
u0=instance("upper-0",a,at=(0,0,h))
u1=instance("upper-1",a,at=(w,move,h))
u2=instance("upper-2",a,at=(2*w,0,h))
u3=instance("upper-3",a,at=(3*w,0,h))
group("cabinet",[group("lower",[a,b,c,v]),group("upper",[u0,u1,u2,u3])])
"#
        .into(),
        overrides: BTreeMap::new(),
    };
    let initial = session.apply_rule_program(source.clone(), false).unwrap();
    assert_eq!(initial.model.parts.len(), 40);
    let lower = initial.model.part("lower-3-bottom").unwrap();
    let shelf = initial.model.part("lower-3-shelf").unwrap();
    assert_eq!(shelf.world_bounds().0[2] - lower.world_bounds().1[2], 650.0);
    assert_eq!(shelf.size_mm, [839.0, 520.0, 18.0]);
    assert_eq!(lower.at_mm, [2643.0, 0.0, 80.0]);
    for column in 0..4 {
        let prefix = if column == 0 || column == 3 {
            format!("lower-{column}")
        } else {
            format!("lower-{column}/lower-0")
        };
        let left = initial.model.part(&format!("{prefix}-left")).unwrap();
        let right = initial.model.part(&format!("{prefix}-right")).unwrap();
        assert_eq!(right.world_bounds().1[0] - left.world_bounds().0[0], 875.0);
        assert_eq!(left.at_mm[0], f64::from(column) * 875.0);
    }
    assert_eq!(initial.model.joints.len(), 48);
    let mut report = initial.report.clone();
    let exact = ketchup_application::verify_rule_program_all(
        &initial.snapshot,
        &initial.model,
        &mut report,
        &ketchup_model::persistence::ContainerData::default(),
        None,
        std::time::Duration::from_secs(60),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    )
    .unwrap();
    assert_eq!(exact["state"], "verified", "{exact}");
    assert_eq!(exact["distance_measurements_state"], "verified", "{exact}");
    assert!(report.ok, "{:?}", report.issues);
    assert!(exact["measurements"].as_array().unwrap().iter().any(|m| {
        m["distance_mm"] == 650.0
            && m["parts"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "lower-3-shelf")
    }));
    source.overrides.insert("move".into(), 150.0);
    let moved = session.apply_rule_program(source.clone(), false).unwrap();
    for old in &initial.model.parts {
        let new = moved.model.part(&old.name).unwrap();
        if old.name.starts_with("upper-1/") {
            assert_eq!(
                new.at_mm,
                [old.at_mm[0], old.at_mm[1] + 150.0, old.at_mm[2]]
            );
            assert_eq!(
                new.finished_holes().collect::<Vec<_>>(),
                old.finished_holes().collect::<Vec<_>>()
            );
        } else {
            assert_eq!(new, old, "untouched neighbour {}", old.name);
        }
    }
    assert_eq!(
        session.undo().unwrap().scene_query(),
        initial.snapshot.scene_query()
    );
    assert_eq!(
        session.redo().unwrap().scene_query(),
        moved.snapshot.scene_query()
    );
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cabinet.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(
        reopened.snapshot().scene_query(),
        moved.snapshot.scene_query()
    );
    let repeated = reopened.apply_rule_program(source, false).unwrap();
    assert_eq!(repeated.model, moved.model);
    assert_eq!(
        repeated.snapshot.scene_query(),
        moved.snapshot.scene_query()
    );
}
