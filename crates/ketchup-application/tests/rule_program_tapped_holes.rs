//! `hole(..., thread=)` in a program records the thread on the hole's cut in
//! the document, like the `drill_holes` operation, wherever the hole is cut.
use ketchup_application::DocumentSession;
use ketchup_model::document::{RuleProgramSource, Snapshot};

fn program(source: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "tapped.star".into(),
        source: source.into(),
        overrides: Default::default(),
    }
}

/// (cut feature name, thread label) of every tapped hole, by feature name.
fn threads(snapshot: &Snapshot) -> Vec<(String, String)> {
    let mut threads = snapshot
        .hole_threads()
        .map(|(id, thread)| {
            (
                snapshot.feature(id).unwrap().name().to_owned(),
                thread.label(),
            )
        })
        .collect::<Vec<_>>();
    threads.sort();
    threads
}

const PLATE: &str = "p=box('plate',(60,40,20))\n";

#[test]
fn a_panel_hole_and_a_later_hole_both_carry_their_thread() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let applied = session
        .apply_rule_program(
            program(&format!(
                "{PLATE}hole(p,'z+',at=(10,10),depth=15,id='a',thread=('M5',10))\n\
                 push_pull(p,face='x+',distance=10,name='wider')\n\
                 hole(p,'z+',at=(50,30),diameter=6.8,depth=19,id='b',thread=('M8',12))\n\
                 hole(p,'z+',at=(30,20),diameter=5,depth=15,id='c')\n"
            )),
            false,
        )
        .unwrap();
    let recorded = threads(&applied.snapshot);
    assert_eq!(recorded.len(), 2, "{recorded:?}");
    assert!(
        recorded
            .iter()
            .any(|(feature, label)| feature.contains(" a ") && label == "M5×0.8 depth 10"),
        "{recorded:?}"
    );
    assert!(
        recorded
            .iter()
            .any(|(_, label)| label == "M8×1.25 depth 12"),
        "{recorded:?}"
    );
}

#[test]
fn threads_run_from_each_hole_entry_and_along_each_bolt_shank() {
    let _turn = crate::integration_support::file_turn();
    let source = format!(
        "{PLATE}hole(p,'z+',at=(10,10),diameter=6.8,depth=17,id='a',thread=('M8',10))\n\
         hole(p,'x+',at=(10,10),diameter=5,depth=15,id='b')\n\
         push_pull(p,face='x+',distance=10,name='wider')\n\
         hole(p,'y+',at=(30,10),diameter=4.2,depth=13,id='c',thread=('M5',8))\n\
         bolt('s','M8',40,at=(100,10,5),axis=(1,0,0))\n\
         nut('n','M8',at=(0,100,0))\n"
    );
    let mut session = DocumentSession::default();
    let applied = session.apply_rule_program(program(&source), false).unwrap();
    let evaluated = ketchup_program::evaluate("tapped.star", &source, &Default::default()).unwrap();
    let snapshot = &applied.snapshot;
    let mut threads = ketchup_application::threads::thread_cylinders(snapshot, Some(&evaluated));
    threads.sort_by(|a, b| a.size.cmp(&b.size).then(a.internal.cmp(&b.internal)));
    let rounded = |values: [f64; 3]| values.map(|value| (value * 1e6).round() / 1e6 + 0.0);
    let summary = threads
        .iter()
        .map(|thread| {
            (
                snapshot
                    .definition(thread.definition_id)
                    .unwrap()
                    .name()
                    .to_owned(),
                thread.size.clone(),
                thread.internal,
                rounded(thread.start_mm),
                rounded(thread.direction),
                thread.length_mm,
                thread.surface_diameter_mm,
            )
        })
        .collect::<Vec<_>>();
    // The untapped hole and the nut have no thread; the later hole is cut
    // through a tool and still starts at its entry face.
    assert_eq!(
        summary,
        [
            (
                "plate".to_owned(),
                "M5".to_owned(),
                true,
                [30.0, 40.0, 10.0],
                [0.0, -1.0, 0.0],
                8.0,
                4.2
            ),
            (
                "s".to_owned(),
                "M8".to_owned(),
                false,
                [0.0, 0.0, -40.0],
                [0.0, 0.0, 1.0],
                40.0,
                8.0
            ),
            (
                "plate".to_owned(),
                "M8".to_owned(),
                true,
                [10.0, 10.0, 20.0],
                [0.0, 0.0, -1.0],
                10.0,
                6.8
            ),
        ],
        "{summary:#?}"
    );
    assert!(threads.iter().all(|thread| thread.nominal_diameter_mm
        == if thread.size == "M8" { 8.0 } else { 5.0 }));
    // Without the program's evaluation only the tapped holes are known.
    assert_eq!(
        ketchup_application::threads::thread_cylinders(snapshot, None).len(),
        2
    );
}

/// The view, the drawing and the modelled groove all take a bolt's thread
/// from here: an ISO 4014 bolt only b from its tip, an ISO 4017 one to the head.
#[test]
fn an_iso_4014_bolt_is_threaded_b_from_its_tip_and_an_iso_4017_bolt_to_its_head() {
    let _turn = crate::integration_support::file_turn();
    let source = "bolt('partial','M8',80,norm='ISO 4014')\nbolt('full','M8',40,at=(50,0,0))\n";
    let mut session = DocumentSession::default();
    let applied = session.apply_rule_program(program(source), false).unwrap();
    let evaluated = ketchup_program::evaluate("tapped.star", source, &Default::default()).unwrap();
    let snapshot = &applied.snapshot;
    let mut threads = ketchup_application::threads::thread_cylinders(snapshot, Some(&evaluated))
        .into_iter()
        .map(|thread| {
            let name = snapshot.definition(thread.definition_id).unwrap().name();
            (name.to_owned(), thread.start_mm[2], thread.length_mm)
        })
        .collect::<Vec<_>>();
    threads.sort_by(|a, b| a.0.cmp(&b.0));
    assert_eq!(
        threads,
        [
            ("full".to_owned(), -40.0, 40.0),
            ("partial".to_owned(), -80.0, 22.0)
        ]
    );
}

/// TH-3: a tap cannot cut thread to the bottom of a blind hole; DIN 76-1 e1
/// (M8: 6.2 mm) is drilled below the thread. A through hole needs none.
#[test]
fn a_blind_tapped_hole_is_drilled_the_din_76_runout_below_its_thread() {
    let _turn = crate::integration_support::file_turn();
    let hole = |depth: &str, through: &str| {
        program(&format!(
            "{PLATE}hole(p,'z+',at=(10,10),depth={depth}{through},id='a',thread=('M8',12))\n"
        ))
    };
    let shallow = DocumentSession::default()
        .apply_rule_program(hole("18", ""), false)
        .err()
        .expect("an 18 mm blind hole is too shallow for an M8 thread 12 deep")
        .to_string();
    assert!(
        shallow.contains("needs a blind hole at least 18.2 mm deep")
            && shallow.contains("DIN 76-1"),
        "{shallow}"
    );
    for (depth, through) in [("18.2", ""), ("20", ",through=True")] {
        let applied = DocumentSession::default()
            .apply_rule_program(hole(depth, through), false)
            .unwrap();
        assert_eq!(
            threads(&applied.snapshot)
                .into_iter()
                .map(|(_, label)| label)
                .collect::<Vec<_>>(),
            ["M8×1.25 depth 12"],
            "depth={depth}{through}"
        );
    }
}

#[test]
fn changing_or_removing_a_thread_rerecords_it_and_undo_restores_it() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let hole = |thread: &str| {
        program(&format!(
            "{PLATE}hole(p,'z+',at=(10,10),diameter=4.2,depth=17,id='a'{thread})\n"
        ))
    };
    let labels = |snapshot: &Snapshot| {
        threads(snapshot)
            .into_iter()
            .map(|(_, label)| label)
            .collect::<Vec<_>>()
    };
    let first = session
        .apply_rule_program(hole(",thread=('M5',12)"), false)
        .unwrap();
    assert_eq!(labels(&first.snapshot), ["M5×0.8 depth 12"]);
    let deeper = session
        .apply_rule_program(hole(",thread=('M5',8)"), false)
        .unwrap();
    assert_eq!(labels(&deeper.snapshot), ["M5×0.8 depth 8"]);
    let plain = session.apply_rule_program(hole(""), false).unwrap();
    assert!(labels(&plain.snapshot).is_empty());
    session.undo().unwrap();
    assert_eq!(labels(&session.snapshot()), ["M5×0.8 depth 8"]);
}
