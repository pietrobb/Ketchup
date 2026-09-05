use ketchup_application::evaluation::{EvidenceStatus, exact_source, exact_worker_candidates};
use ketchup_application::{DocumentSession, SessionError, SessionSettings};
use ketchup_core::assistant_sidecar::{
    AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadPartFeature,
    AssistantPrincipalPlane, AssistantSketchConstraint, AssistantSketchEntity,
    AssistantWorkplaneSpec,
};
use ketchup_core::document::OccurrenceId;
use std::{collections::BTreeSet, time::Duration};

fn program() -> AssistantCadEditProgram {
    AssistantCadEditProgram {
        operations: vec![AssistantCadEditOperation::CreatePart {
            name: "Deadline test part".into(),
            workplane: AssistantWorkplaneSpec::Principal {
                plane: AssistantPrincipalPlane::Xy,
            },
            entities: vec![AssistantSketchEntity::Circle {
                id: 1,
                center_mm: [0.0, 0.0],
                radius_mm: 12.0,
            }],
            constraints: vec![AssistantSketchConstraint::Radius {
                id: 1,
                entity_id: 1,
                value_mm: 12.0,
            }],
            feature: AssistantCadPartFeature::Extrusion { distance_mm: 30.0 },
            translation_mm: [0.0, 0.0, 0.0],
            rotation: None,
        }],
    }
}

fn worker_session() -> DocumentSession {
    let path = exact_worker_candidates()
        .into_iter()
        .find(|path| path.is_file())
        .expect("build ketchup-exact-worker before this test");
    DocumentSession::new(SessionSettings {
        exact_worker_path: Some(path),
        ..SessionSettings::default()
    })
}

fn add_part(session: &mut DocumentSession) {
    session
        .apply_cad_program(&program(), &BTreeSet::new())
        .unwrap();
}

fn canonical_state(session: &DocumentSession) -> (u64, String, usize, usize, bool) {
    (
        session.snapshot().revision_id(),
        session.snapshot().canonical_digest(),
        session.visible_undo_steps(),
        session.visible_redo_steps(),
        session.is_modified(),
    )
}

fn assert_timeout_unchanged(session: &mut DocumentSession, timeout: Duration) {
    let before = canonical_state(session);
    // Include registry stamps and packages, not just counts: even replacing cached
    // evidence with an equivalent publication must not escape the deadline guard.
    let render = format!("{:?}", session.exact_results());
    let topology = format!("{:?}", session.topology_results());
    assert!(matches!(
        session.evaluate_with_timeout(timeout),
        Err(SessionError::Evaluation(reason)) if reason == "exact evaluation timed out"
    ));
    assert_eq!(canonical_state(session), before);
    assert_eq!(format!("{:?}", session.exact_results()), render);
    assert_eq!(format!("{:?}", session.topology_results()), topology);
}

#[test]
fn default_evaluation_works_and_zero_refuses_even_current_results() {
    assert_eq!(
        SessionSettings::default().evaluation_timeout,
        Duration::from_secs(30)
    );
    let mut session = worker_session();
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    add_part(&mut session);
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    let before = canonical_state(&session);
    let report = session.evaluate().unwrap();
    assert!(report.complete, "{report:?}");
    assert!(report.topology_complete, "{report:?}");
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert_eq!(canonical_state(&session), before);
    assert!(!session.exact_results().is_empty());
    assert!(!session.topology_results().is_empty());
    assert_timeout_unchanged(&mut session, Duration::ZERO);

    let cached = session.evaluate().unwrap();
    assert!(cached.complete && cached.topology_complete, "{cached:?}");
    assert!(cached.producers.iter().all(|entry| {
        entry.render == EvidenceStatus::Current && entry.topology == EvidenceStatus::Current
    }));
    assert_eq!(canonical_state(&session), before);
}

#[test]
fn short_deadline_cannot_publish_late_or_poison_real_worker_retry() {
    let mut session = worker_session();
    add_part(&mut session);
    let populated = canonical_state(&session);
    // Preparation alone exceeds this positive budget. Queued results must not
    // turn recv_timeout(0) into success after the overall budget is exhausted.
    assert_timeout_unchanged(&mut session, Duration::from_nanos(1));
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    session.undo().unwrap();
    let empty = canonical_state(&session);
    assert_eq!(session.visible_redo_steps(), 1);
    assert_timeout_unchanged(&mut session, Duration::ZERO);
    let report = session.evaluate().unwrap();
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert!(!report.complete);
    assert!(report.producers.is_empty());
    assert_eq!(canonical_state(&session), empty);
    assert!(session.exact_results().is_empty());
    assert!(session.topology_results().is_empty());

    session.redo().unwrap();
    assert_eq!(canonical_state(&session), populated);
    let report = session.evaluate().unwrap();
    assert!(report.complete && report.topology_complete, "{report:?}");
    assert_eq!(report.source, exact_source(&session.snapshot()));
    assert_eq!(canonical_state(&session), populated);
    for registry in [session.exact_results(), session.topology_results()] {
        assert!(!registry.is_empty());
        assert!(registry.is_bound_to(&session.snapshot()));
        assert!(
            registry
                .values()
                .all(|package| package.is_current(&session.snapshot()))
        );
    }
    assert_timeout_unchanged(&mut session, Duration::from_nanos(1));
    assert!(session.evaluate().unwrap().complete);
    assert_eq!(canonical_state(&session), populated);
}

#[test]
fn per_call_override_does_not_change_the_settings_default() {
    let mut session = DocumentSession::new(SessionSettings {
        evaluation_timeout: Duration::ZERO,
        ..SessionSettings::default()
    });
    let before = canonical_state(&session);
    assert!(matches!(
        session.evaluate(),
        Err(SessionError::Evaluation(_))
    ));
    let report = session
        .evaluate_with_timeout(Duration::from_secs(30))
        .unwrap();
    assert!(!report.complete);
    assert!(report.producers.is_empty());
    assert!(matches!(
        session.evaluate(),
        Err(SessionError::Evaluation(_))
    ));
    assert_eq!(canonical_state(&session), before);
}

#[test]
fn redo_count_tracks_authoritative_history_including_branching() {
    let mut session = DocumentSession::default();
    assert_eq!(session.visible_redo_steps(), 0);
    add_part(&mut session);
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert_eq!(session.visible_undo_steps(), 2);
    assert_eq!(session.visible_redo_steps(), 0);
    session.undo().unwrap();
    assert_eq!(session.visible_redo_steps(), 1);
    session.undo().unwrap();
    assert_eq!(session.visible_redo_steps(), 2);
    session.redo().unwrap();
    assert_eq!(session.visible_redo_steps(), 1);
    session.set_grounded(OccurrenceId(1), true).unwrap();
    assert_eq!(session.visible_redo_steps(), 0);
    assert!(matches!(session.redo(), Err(SessionError::NoRedo)));
    assert_eq!(session.visible_undo_steps(), 2);
}
