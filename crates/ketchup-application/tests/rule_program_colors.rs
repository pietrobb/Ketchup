use ketchup_application::{DocumentSession, SaveOptions, SessionSettings, rule_program_part_name};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use std::collections::BTreeMap;

const SOURCE: &str = "red=param('red',0)\ncolor=None if red==0 else (red,128,220)\na=box('a',(20,20,20),color=color)\nb=box('b',(20,20,20),at=(30,0,0),color=(200,90,50))\nc=component('c',[a,b])\ni=instance('inner',c,at=(100,0,0))\nd=component('outer',[c,i])\ninstance('copy',d,at=(0,100,0))\nbox('root',(10,10,10),at=(300,0,0),color=color)";
fn colors(snapshot: &Snapshot) -> BTreeMap<String, Option<[u8; 3]>> {
    snapshot
        .scene_query()
        .into_iter()
        .filter(|leaf| {
            !snapshot
                .definition(leaf.definition_id)
                .unwrap()
                .feature_ids()
                .is_empty()
        })
        .map(|leaf| {
            (
                rule_program_part_name(snapshot, &leaf.instance_path).unwrap(),
                leaf.color(),
            )
        })
        .collect()
}
fn apply(session: &mut DocumentSession, red: f64) {
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "colors.star".into(),
                source: SOURCE.into(),
                overrides: BTreeMap::from([("red".into(), red)]),
            },
            false,
        )
        .unwrap();
}
#[test]
fn root_and_nested_shared_colors_change_without_rebuilding_and_survive_history_and_reopen() {
    let mut session = DocumentSession::default();
    apply(&mut session, 0.);
    let before = session.snapshot();
    let identities = |snapshot: &Snapshot| {
        snapshot
            .scene_query()
            .into_iter()
            .map(|leaf| (leaf.instance_path, leaf.definition_id, leaf.transform))
            .collect::<Vec<_>>()
    };
    let original = identities(&before);
    let features = before.features().cloned().collect::<Vec<_>>();
    let initial = colors(&before);
    assert_eq!(initial.len(), 9);
    for red in [40., 90., 0.] {
        apply(&mut session, red);
        let snapshot = session.snapshot();
        assert_eq!(identities(&snapshot), original);
        assert_eq!(snapshot.features().cloned().collect::<Vec<_>>(), features);
        let current = colors(&snapshot);
        for name in ["a", "inner/a", "copy/a", "copy/inner/a", "root"] {
            assert_eq!(
                current[name],
                if red == 0. {
                    None
                } else {
                    Some([red as u8, 128, 220])
                },
                "{name}"
            );
        }
        for name in ["b", "inner/b", "copy/b", "copy/inner/b"] {
            assert_eq!(current[name], Some([200, 90, 50]), "{name}");
        }
        session.undo().unwrap();
        assert_ne!(colors(&session.snapshot()), current);
        session.redo().unwrap();
        assert_eq!(colors(&session.snapshot()), current);
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("colors.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(colors(&reopened.snapshot()), initial);
    assert_eq!(identities(&reopened.snapshot()), original);
}

#[test]
fn initially_colored_shared_parts_keep_colors_when_rebuilt_or_new_members_are_added() {
    let mut session = DocumentSession::default();
    let original = "a=box('a',(20,20,20),color=(40,128,220))\nc=component('c',[a])\ninstance('copy',c,at=(100,0,0))";
    for source in [original.to_owned(), original.replace("(20,20,20)", "(30,20,20)"), "a=box('a',(30,20,20),color=(90,128,220))\nfillet(a,edges=[['x+','y+']],radius=2)\nb=box('b',(10,10,10),at=(0,40,0),color=(200,90,50))\nc=component('c',[a,b])\ninstance('copy',c,at=(100,0,0))".into()] {
        session.apply_rule_program(RuleProgramSource { file_name: "colors.star".into(), source, overrides: BTreeMap::new() }, false).unwrap();
        let current = colors(&session.snapshot());
        let red = if current.len()==4 { 90 } else { 40 };
        assert_eq!(current["a"], Some([red,128,220]));
        assert_eq!(current["copy/a"], current["a"]);
        if current.len()==4 {
            assert_eq!(current["b"], Some([200,90,50]));
            assert_eq!(current["copy/b"], current["b"]);
        }
    }
}
