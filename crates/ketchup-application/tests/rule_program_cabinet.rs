//! A compact 4x2 cabinet uses the existing shared hierarchy, not name-only groups.
use ketchup_application::{
    DocumentSession, RuleProgramApplyResult, SaveOptions, SessionSettings, rule_program_part_name,
};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use ketchup_program::{ProgramModel, contact::contact};
use std::collections::BTreeMap;

fn program() -> RuleProgramSource {
    RuleProgramSource {
        file_name: "cabinet-4x2.star".into(),
        source: r#"
t = param("board",18)
width = param("width",3500)
height = param("height",2600)
depth = param("depth",520)
base = param("base",80)
opening = param("opening",650)
shift = param("upper_shift",0)
h = (height-base)/2
narrow_width = opening+2*t
w = (width-narrow_width)/3

def module(name, span, origin):
    x,y,z = origin
    left = board(name+"-left",(t,depth,h),at=(x,y,z))
    right = board(name+"-right",(t,depth,h),at=(x+span-t,y,z))
    bottom = board(name+"-bottom",(span-2*t,depth,t),at=(x+t,y,z))
    top = board(name+"-top",(span-2*t,depth,t),at=(x+t,y,z+h-t))
    for side in [left,right]:
        for rail in [bottom,top]:
            dowels(side,rail,dowel="8x35",count=2,margin=50)
    return component(name,[group(name+"-frame",[left,right,bottom,top])])

# Definition owners are real modules, not extra off-scene prototype geometry.
wide = module("bottom-0",w,(0,0,base))
narrow = module("upper-3",narrow_width,(3*w,0,base+h))
b1 = instance("bottom-1",wide,at=(w,0,0))
b2 = instance("bottom-2",wide,at=(2*w,0,0))
b3 = instance("bottom-3",wide,at=(3*w,0,0))
u0 = instance("upper-0",wide,at=(0,0,h))
u1 = instance("upper-1",wide,at=(w,shift,h))
u2 = instance("upper-2",wide,at=(2*w,0,h))
lo = group("lower-row",[wide,b1,b2,b3])
hi = group("upper-row",[u0,u1,u2,narrow])
group("cabinet",[lo,hi])
"#
        .into(),
        overrides: BTreeMap::new(),
    }
}

fn assert_same_document(actual: &Snapshot, expected: &Snapshot) {
    assert_eq!(actual.scene_query(), expected.scene_query());
    assert_eq!(
        actual.features().collect::<Vec<_>>(),
        expected.features().collect::<Vec<_>>()
    );
    assert_eq!(
        actual.definitions().collect::<Vec<_>>(),
        expected.definitions().collect::<Vec<_>>()
    );
    assert_eq!(
        actual.occurrences().collect::<Vec<_>>(),
        expected.occurrences().collect::<Vec<_>>()
    );
    assert_eq!(
        actual.groups().collect::<Vec<_>>(),
        expected.groups().collect::<Vec<_>>()
    );
    assert_eq!(
        actual.local_groups().collect::<Vec<_>>(),
        expected.local_groups().collect::<Vec<_>>()
    );
    assert_eq!(
        actual.local_occurrences().collect::<Vec<_>>(),
        expected.local_occurrences().collect::<Vec<_>>()
    );
}

fn assert_layout(applied: &RuleProgramApplyResult) {
    let model = &applied.model;
    assert_eq!(model.parts.len(), 32);
    assert_eq!(model.components.len(), 2);
    assert_eq!(model.instances.len(), 6);
    let mut min = [f64::INFINITY; 3];
    let mut max = [f64::NEG_INFINITY; 3];
    for part in &model.parts {
        let (lo, hi) = part.world_bounds();
        for axis in 0..3 {
            min[axis] = min[axis].min(lo[axis]);
            max[axis] = max[axis].max(hi[axis]);
        }
        assert_eq!(part.size_mm[1], 520.0);
        assert_eq!(part.size_mm[0].min(part.size_mm[2]), 18.0);
        assert_eq!(part.finished_holes().count(), 4);
    }
    // Width / height / depth = 3500 / 2600 / 520, including the 80 mm z offset.
    assert_eq!(min, [0.0, 0.0, 80.0]);
    assert_eq!(max, [3500.0, 520.0, 2600.0]);
    let left = model.part("bottom-3/upper-3-left").unwrap();
    let right = model.part("bottom-3/upper-3-right").unwrap();
    assert_eq!(right.world_bounds().0[0] - left.world_bounds().1[0], 650.0);
    assert_eq!(left.at_mm, [2814.0, 0.0, 80.0]);
    assert_eq!(model.joints.len(), 32);
    assert_eq!(applied.report.bom.total_parts, 32);
    assert_eq!(
        applied
            .report
            .bom
            .hardware
            .iter()
            .map(|row| row.count)
            .sum::<usize>(),
        64
    );

    let snapshot = &applied.snapshot;
    assert_eq!(snapshot.occurrences().count(), 8);
    assert_eq!(snapshot.local_occurrences().count(), 8);
    assert_eq!(snapshot.local_groups().count(), 2);
    assert_eq!(snapshot.groups().count(), 3);
    let cabinet = snapshot.groups().find(|g| g.name() == "cabinet").unwrap();
    for row in snapshot.groups().filter(|g| g.id() != cabinet.id()) {
        assert_eq!(row.parent(), Some(cabinet.id()));
        let roots = snapshot
            .occurrences()
            .filter(|r| r.parent() == Some(row.id()))
            .collect::<Vec<_>>();
        assert_eq!(roots.len(), 4);
        let prefix = if row.name() == "lower-row" {
            "bottom-"
        } else {
            "upper-"
        };
        assert!(roots.iter().all(|r| r.name().starts_with(prefix)));
    }
    for group in snapshot.local_groups() {
        assert_eq!(
            snapshot
                .local_occurrences()
                .filter(|m| m.key().definition_id == group.key().definition_id
                    && m.parent() == Some(group.key().local_id))
                .count(),
            4
        );
    }
    let wide = snapshot
        .occurrences()
        .find(|r| r.name() == "bottom-0")
        .unwrap()
        .definition_id();
    let narrow = snapshot
        .occurrences()
        .find(|r| r.name() == "upper-3")
        .unwrap()
        .definition_id();
    assert_ne!(wide, narrow);
    for root in snapshot.occurrences() {
        assert_eq!(
            root.definition_id(),
            if root.name().ends_with("-3") {
                narrow
            } else {
                wide
            }
        );
    }
    assert_scene_matches_model(applied);
}

fn assert_scene_matches_model(applied: &RuleProgramApplyResult) {
    let mut leaves = 0;
    for leaf in applied.snapshot.scene_query() {
        let Some(name) = rule_program_part_name(&applied.snapshot, &leaf.instance_path) else {
            continue;
        };
        if let Some(part) = applied.model.part(&name) {
            assert_eq!(leaf.transform.matrix(), &part.transform_matrix(), "{name}");
            leaves += 1;
        }
    }
    assert_eq!(leaves, applied.model.parts.len());
}

fn shifted(point: [f64; 3], delta: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|axis| point[axis] + delta[axis])
}

fn assert_module_moved(before: &RuleProgramApplyResult, after: &RuleProgramApplyResult) {
    let delta_for = |name: &str| {
        if name.starts_with("upper-1/") {
            [0.0, 150.0, 0.0]
        } else {
            [0.0; 3]
        }
    };
    let mut moved_parts = 0;
    let mut moved_holes = 0;
    for old in &before.model.parts {
        let new = after.model.part(&old.name).unwrap();
        let delta = delta_for(&old.name);
        if delta == [0.0; 3] {
            assert_eq!(new, old, "neighbour {} changed", old.name);
        } else {
            moved_parts += 1;
        }
        assert_eq!(new.at_mm, shifted(old.at_mm, delta));
        assert_eq!(new.size_mm, old.size_mm);
        assert_eq!(new.finished_holes().count(), old.finished_holes().count());
        for ((old_hole, (old_entry, old_axis)), (new_hole, (new_entry, new_axis))) in
            old.finished_holes().zip(new.finished_holes())
        {
            assert_eq!(new_hole, old_hole);
            assert_eq!(new_axis, old_axis);
            assert_eq!(
                new.to_world(new_entry),
                shifted(old.to_world(old_entry), delta)
            );
            moved_holes += usize::from(delta != [0.0; 3]);
        }
    }
    assert_eq!((moved_parts, moved_holes), (4, 16));
    let mut moved_joints = 0;
    for old in &before.model.joints {
        let new = after
            .model
            .joints
            .iter()
            .find(|j| j.name == old.name)
            .unwrap();
        let delta = delta_for(&old.parts[0]);
        assert_eq!(delta, delta_for(&old.parts[1]));
        assert_eq!(old.kind, "dowel");
        assert_eq!(old.fastener.as_deref(), Some("dowel 8x35"));
        assert_eq!(old.fasteners_mm.len(), 2);
        for endpoint in &old.parts {
            let part = before.model.part(endpoint).unwrap();
            for point in &old.fasteners_mm {
                assert!(
                    part.finished_holes()
                        .any(|(_, (entry, _))| part.to_world(entry) == *point),
                    "{endpoint}: joint fastener must match a drilled entry"
                );
            }
        }
        let mut expected = old.clone();
        expected.fasteners_mm = old
            .fasteners_mm
            .iter()
            .map(|p| shifted(*p, delta))
            .collect();
        assert_eq!(new, &expected);
        moved_joints += usize::from(delta != [0.0; 3]);
        let patch = |model: &ProgramModel| {
            contact(
                model.part(&old.parts[0]).unwrap(),
                model.part(&old.parts[1]).unwrap(),
            )
            .expect("internal boards remain in contact")
        };
        let old_patch = patch(&before.model);
        let new_patch = patch(&after.model);
        for (actual, expected) in new_patch.min_mm.into_iter().chain(new_patch.max_mm).zip(
            shifted(old_patch.min_mm, delta)
                .into_iter()
                .chain(shifted(old_patch.max_mm, delta)),
        ) {
            assert!(
                (actual - expected).abs() < 1e-8,
                "internal contact moved incorrectly"
            );
        }
        assert_eq!(new_patch.normal, old_patch.normal);
        assert_eq!(new_patch.face_a, old_patch.face_a);
        assert_eq!(new_patch.face_b, old_patch.face_b);
        assert!(new_patch.size_mm.iter().all(|size| *size > 0.0));
        let old_relation = before
            .report
            .relations
            .iter()
            .find(|r| r.joint.as_deref() == Some(old.name.as_str()))
            .expect("reported internal joint");
        let new_relation = after
            .report
            .relations
            .iter()
            .find(|r| r.joint.as_deref() == Some(old.name.as_str()))
            .expect("joint relation survives movement");
        assert_eq!(new_relation.parts, old_relation.parts);
        assert_eq!(new_relation.kind, old_relation.kind);
        assert_eq!(new_relation.faces, old_relation.faces);
        assert_eq!(new_relation.direction, old_relation.direction);
    }
    assert_eq!(moved_joints, 4);
    let snapshot = &after.snapshot;
    assert_eq!(
        snapshot.features().collect::<Vec<_>>(),
        before.snapshot.features().collect::<Vec<_>>()
    );
    assert_eq!(
        snapshot.local_occurrences().collect::<Vec<_>>(),
        before.snapshot.local_occurrences().collect::<Vec<_>>()
    );
    for old in before.snapshot.occurrences() {
        let new = snapshot.occurrence(old.id()).unwrap();
        assert_eq!(new.definition_id(), old.definition_id());
        assert_eq!(new.name(), old.name());
        assert_eq!(new.parent(), old.parent());
        if old.name() != "upper-1" {
            assert_eq!(new, old);
        }
    }
    let scene = snapshot.scene_query();
    for old in before.snapshot.scene_query() {
        let new = scene
            .iter()
            .find(|p| p.instance_path == old.instance_path)
            .expect("stable instance path");
        assert_eq!(new.definition_id, old.definition_id);
        let mut expected = *old.transform.matrix();
        if snapshot.occurrence(old.occurrence_id).unwrap().name() == "upper-1" {
            expected[7] += 150.0;
        }
        assert_eq!(new.transform.matrix(), &expected);
    }
    assert_scene_matches_model(after);
}

#[test]
fn cabinet_4x2_variant_move_reapply_history_persistence_and_invalid_membership() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::new(SessionSettings::default());
    let mut source = program();
    let initial = session.apply_rule_program(source.clone(), false).unwrap();
    let root = initial
        .snapshot
        .occurrences()
        .find(|r| r.name() == "bottom-3")
        .unwrap();
    let variant = initial
        .snapshot
        .occurrences()
        .find(|r| r.name() == "upper-3")
        .unwrap()
        .definition_id();
    assert_ne!(root.definition_id(), variant);
    // Reuse the predeclared variant; its owner is already the upper-right module.
    // Instance translations are relative to each definition's original board frame.
    source.source = source.source.replace(
        "instance(\"bottom-3\",wide,at=(3*w,0,0))",
        "instance(\"bottom-3\",narrow,at=(0,0,-h))",
    );
    let ready = session.apply_rule_program(source.clone(), false).unwrap();
    let repointed = ready.snapshot.occurrence(root.id()).unwrap();
    assert_eq!(repointed.definition_id(), variant);
    assert_eq!(repointed.name(), root.name());
    assert_eq!(repointed.parent(), root.parent());
    assert_eq!(
        ready.snapshot.definitions().collect::<Vec<_>>(),
        initial.snapshot.definitions().collect::<Vec<_>>()
    );
    assert_eq!(
        ready.snapshot.local_occurrences().collect::<Vec<_>>(),
        initial.snapshot.local_occurrences().collect::<Vec<_>>()
    );
    let scene = ready.snapshot.scene_query();
    for old in initial
        .snapshot
        .scene_query()
        .iter()
        .filter(|p| p.occurrence_id != root.id())
    {
        assert_eq!(
            scene.iter().find(|p| p.instance_path == old.instance_path),
            Some(&{
                let mut expected = old.clone();
                if initial
                    .snapshot
                    .occurrence(old.occurrence_id)
                    .unwrap()
                    .name()
                    == "upper-3"
                {
                    expected.shared_occurrence_count += 1;
                } else {
                    expected.shared_occurrence_count -= 1;
                }
                expected
            })
        );
    }
    assert_layout(&ready);
    let reapplied = session.apply_rule_program(source.clone(), false).unwrap();
    assert_same_document(&reapplied.snapshot, &ready.snapshot);
    assert_eq!(reapplied.model, ready.model);

    let stationary_source = source.clone();
    source.overrides.insert("upper_shift".into(), 150.0);
    let moved = session.apply_rule_program(source.clone(), false).unwrap();
    assert_module_moved(&ready, &moved);
    for members in [
        "[wide,wide]",
        "[\"missing\"]",
        "[\"lower-row\"]",
        "[wide,u0]",
    ] {
        let mut invalid = source.clone();
        invalid.source = invalid.source.replace("[wide,b1,b2,b3]", members);
        assert!(
            session.apply_rule_program(invalid, false).is_err(),
            "accepted invalid membership {members}"
        );
        assert_same_document(&session.snapshot(), &moved.snapshot);
        assert_eq!(session.rule_program(), Some(&source));
    }
    // Failed membership edits must not consume the single movement Undo/Redo.
    assert_same_document(&session.undo().unwrap(), &ready.snapshot);
    assert_eq!(session.rule_program(), Some(&stationary_source));
    assert_same_document(&session.redo().unwrap(), &moved.snapshot);
    assert_eq!(session.rule_program(), Some(&source));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cabinet-4x2.ketchup");
    session.save(&path, SaveOptions::default()).unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_same_document(&reopened.snapshot(), &moved.snapshot);
    assert_eq!(reopened.rule_program(), Some(&source));
    let reapplied = reopened.apply_rule_program(source, false).unwrap();
    assert_same_document(&reapplied.snapshot, &moved.snapshot);
    assert_eq!(reapplied.model, moved.model);
    assert_module_moved(&ready, &reapplied);
}
