//! The document's switch between cosmetic and modelled threads. Cosmetic is
//! the default and saves as before; switched on, the tapped hole and the
//! bolt screwed into it get their helical grooves, in phase, so they no
//! longer collide. The switch keeps the program owning the document and is
//! one undo step; a part's own `modeled_thread=` wins over the document.
use ketchup_application::threads::modeled_threads_batch;
use ketchup_application::validation::assistant_validation_context_with_worker;
use ketchup_application::{AssistantValidationSelection, DocumentSession};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_model::exact_product::ExactResultRegistry;
use ketchup_model::persistence::{self, ContainerData};
use std::time::Duration;

/// An M5 bolt 7.3 mm into a hole tapped 10 mm deep, turned 37° about its
/// axis; the head stands clear of the plate.
fn source(bolt_option: &str) -> String {
    format!(
        "p = box('plate', (20, 20, 12))\n\
         hole(p, 'z+', at = (10, 10), depth = 12, id = 'a', thread = ('M5', 10))\n\
         s = bolt('s', 'M5', 10, at = (10, 10, 14.7){bolt_option})\n\
         place(s, origin = (10, 10, 14.7), z = (0, 0, 1), x = (0.7986355100472928, 0.6018150231520483, 0))\n"
    )
}

fn program(source: &str) -> RuleProgramSource {
    RuleProgramSource {
        variants: Default::default(),
        file_name: "threads.star".into(),
        source: source.into(),
        overrides: Default::default(),
    }
}

fn graph(snapshot: &Snapshot, part: &str) -> ExactBRepGraph {
    let occurrence = snapshot
        .occurrences()
        .find(|occurrence| occurrence.name() == part)
        .unwrap_or_else(|| panic!("no part {part:?}"));
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    ExactBRepGraph::from_snapshot(
        snapshot,
        occurrence.definition_id(),
        *definition.feature_ids().last().unwrap(),
    )
    .unwrap()
}

/// How many helical grooves the plate's and the bolt's bodies are cut with.
fn grooves(snapshot: &Snapshot) -> [usize; 2] {
    ["plate", "s"].map(|part| {
        graph(snapshot, part)
            .nodes
            .iter()
            .filter(|node| matches!(node.operation, ExactBRepOperation::ThreadGroove(_)))
            .count()
    })
}

fn volumes(snapshot: &Snapshot) -> [f64; 2] {
    let mut worker = crate::operations_support::worker();
    ["plate", "s"].map(|part| {
        worker
            .evaluate_exact_brep_graph(&graph(snapshot, part))
            .unwrap()
            .volume_mm3
    })
}

fn collision(snapshot: &Snapshot) -> serde_json::Value {
    assistant_validation_context_with_worker(
        snapshot,
        &ExactResultRegistry::default(),
        &AssistantValidationSelection::only(&["collision"]),
        &ContainerData::default(),
        None,
        Duration::from_secs(120),
    )
}

fn switch(session: &mut DocumentSession, modeled: bool) -> Snapshot {
    let source = session.rule_program().unwrap().clone();
    let evaluated =
        ketchup_program::evaluate(&source.file_name, &source.source, &source.overrides).unwrap();
    let batch = modeled_threads_batch(&session.snapshot(), Some(&evaluated.model), modeled);
    let proposal = session.plan_commands(batch).unwrap();
    session.apply_proposal(&proposal).unwrap()
}

#[test]
fn modelled_threads_cut_a_bolt_and_its_hole_in_phase_and_undo_restores_cosmetic() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let cosmetic = session
        .apply_rule_program(program(&source("")), false)
        .unwrap()
        .snapshot;
    assert!(!cosmetic.modeled_threads());
    assert_eq!(grooves(&cosmetic), [0, 0]);
    assert_eq!(cosmetic.shaft_threads().count(), 0);
    // A cosmetic document encodes neither the mode nor a bolt's thread.
    let saved = persistence::save(&cosmetic);
    let text = String::from_utf8_lossy(&saved);
    assert!(text.contains("hole_threads"), "the encoding is searchable");
    assert!(!text.contains("modeled_threads") && !text.contains("shaft_threads"));
    // The cosmetic M5 shank is wider than the 4.2 mm tap drill: a collision.
    let report = collision(&cosmetic);
    assert_eq!(report["state"], "failed", "{report}");
    let cosmetic_volumes = volumes(&cosmetic);

    let modeled = switch(&mut session, true);
    assert!(
        session.rule_program().is_some(),
        "the program keeps the document"
    );
    assert!(modeled.modeled_threads());
    assert_eq!(grooves(&modeled), [1, 1]);
    // Left without its grooves (as drawings show it), each modelled body is
    // the cosmetic one, node for node.
    for part in ["plate", "s"] {
        let plain = graph(&modeled, part).without_thread_grooves().unwrap();
        assert_eq!(
            plain.map(|plain| plain.graph_digest),
            Some(graph(&cosmetic, part).graph_digest),
            "{part}"
        );
        assert!(
            graph(&cosmetic, part)
                .without_thread_grooves()
                .unwrap()
                .is_none()
        );
    }
    let modeled_volumes = volumes(&modeled);
    for (cut, plain) in modeled_volumes.iter().zip(cosmetic_volumes) {
        assert!(
            *cut < plain - 1.0,
            "{modeled_volumes:?} vs {cosmetic_volumes:?}"
        );
    }
    // In phase at 7.3 mm (not a whole number of pitches) and turned 37°.
    let report = collision(&modeled);
    assert_eq!(report["state"], "passed", "{report}");
    assert_eq!(report["issue_count"], 0, "{report}");
    // The mode saves and loads.
    let reopened = persistence::load(&persistence::save(&modeled)).unwrap();
    assert!(reopened.snapshot().modeled_threads());
    assert_eq!(reopened.snapshot().shaft_threads().count(), 1);

    let undone = session.undo().unwrap();
    assert_eq!(undone.canonical_digest(), cosmetic.canonical_digest());
    assert_eq!(grooves(&undone), [0, 0]);
    assert!(session.rule_program().is_some());
    session.redo().unwrap();
    // Switching back is the cosmetic document again, digest for digest.
    let back = switch(&mut session, false);
    assert_eq!(back.canonical_digest(), cosmetic.canonical_digest());
    assert_eq!(grooves(&back), [0, 0]);
    assert_eq!(back.shaft_threads().count(), 0);
}

#[test]
fn a_parts_own_choice_wins_over_the_document_and_reapplying_keeps_the_mode() {
    let _turn = crate::integration_support::file_turn();
    let mut session = DocumentSession::default();
    let own = session
        .apply_rule_program(program(&source(", modeled_thread = True")), false)
        .unwrap()
        .snapshot;
    assert!(!own.modeled_threads());
    assert_eq!(grooves(&own), [0, 1], "only the bolt is modelled");

    session
        .apply_rule_program(program(&source("")), false)
        .unwrap();
    let modeled = switch(&mut session, true);
    assert_eq!(grooves(&modeled), [1, 1]);
    // Re-applying the program keeps the document's mode and its bolt.
    let reapplied = session
        .apply_rule_program(program(&source(" ")), false)
        .unwrap()
        .snapshot;
    assert!(reapplied.modeled_threads());
    assert_eq!(grooves(&reapplied), [1, 1]);
    let cosmetic_bolt = session
        .apply_rule_program(program(&source(", modeled_thread = False")), false)
        .unwrap()
        .snapshot;
    assert_eq!(grooves(&cosmetic_bolt), [1, 0], "the bolt stays cosmetic");
    assert_eq!(cosmetic_bolt.shaft_threads().count(), 0);
}
