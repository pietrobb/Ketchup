//! Cosmetic threads in the viewport: a helix on every tapped hole and every
//! threaded fastener, none on a plain hole or a nut's bore.
use crate::*;

const SOURCE: &str = "p = box('plate', (60, 40, 20))\n\
    hole(p, 'z+', at = (10, 10), diameter = 6.8, depth = 17, id = 'a', thread = ('M8', 10))\n\
    hole(p, 'x+', at = (10, 10), diameter = 5, depth = 15, id = 'b')\n\
    bolt('s', 'M8', 40, at = (100, 10, 5), axis = (1, 0, 0))\n\
    nut('n', 'M8', at = (0, 100, 0))\n";

fn program() -> ketchup_model::document::RuleProgramSource {
    ketchup_model::document::RuleProgramSource {
        variants: Default::default(),
        file_name: "bolted.star".into(),
        source: SOURCE.into(),
        overrides: BTreeMap::new(),
    }
}

/// The helix pieces a definition draws about the axis through `center` along
/// `axis` (both in its frame): pieces that keep `radius` from the axis while
/// they climb it and turn about it. Returns (how far along the axis they
/// reach, how many turns they make).
fn helix_about(
    plan: &InstancedRenderPlan,
    snapshot: &Snapshot,
    definition: &str,
    center: [f64; 3],
    axis: [f64; 3],
    radius: f64,
) -> Option<((f64, f64), f64)> {
    let mut reach = (f64::INFINITY, f64::NEG_INFINITY);
    let mut turned = 0.0;
    let across = if axis[2].abs() < 0.9 {
        [0.0, 0.0, 1.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let side = ketchup_geometry::linalg::cross(axis, across);
    let local = |point: [f32; 3]| {
        let offset = ketchup_geometry::linalg::sub(point.map(f64::from), center);
        let along = ketchup_geometry::linalg::dot(offset, axis);
        let (u, v) = (
            ketchup_geometry::linalg::dot(offset, across),
            ketchup_geometry::linalg::dot(offset, side),
        );
        (along, u.hypot(v), v.atan2(u))
    };
    for batch in plan.batches() {
        if snapshot.definition(batch.definition_id)?.name() != definition {
            continue;
        }
        for [start, end] in batch.geometry.edge_segments() {
            let ((a0, r0, t0), (a1, r1, t1)) = (local(start), local(end));
            let mut turn = t1 - t0;
            if turn > std::f64::consts::PI {
                turn -= std::f64::consts::TAU;
            } else if turn < -std::f64::consts::PI {
                turn += std::f64::consts::TAU;
            }
            if (r0 - radius).abs() > 1e-3 || (r1 - radius).abs() > 1e-3 {
                continue;
            }
            if (a1 - a0).abs() < 1e-6 || turn.abs() < 1e-6 {
                continue;
            }
            reach = (reach.0.min(a0.min(a1)), reach.1.max(a0.max(a1)));
            turned += turn.abs() / std::f64::consts::TAU;
        }
    }
    (turned > 0.0).then_some((reach, turned))
}

#[test]
fn tapped_holes_and_bolts_draw_their_thread_and_plain_bores_do_not() {
    let mut app = KetchupApp::new();
    app.apply_program_source(program(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let snapshot = app.document.current();
    let plan = app.rebuild_render_plan(&snapshot);
    let in_hole = 3.4 * (ketchup_scheduler::exact_worker::STEP_MESH_ANGULAR_DEFLECTION / 2.0).cos();
    let close = |(low, high): (f64, f64), expected: (f64, f64)| {
        (low - expected.0).abs() < 1e-3 && (high - expected.1).abs() < 1e-3
    };

    // The tapped hole: 10 mm of M8 (8 turns of 1.25) down from its entry.
    let (reach, turns) = helix_about(
        &plan,
        &snapshot,
        "plate",
        [10.0, 10.0, 20.0],
        [0.0, 0.0, -1.0],
        in_hole,
    )
    .expect("the tapped hole draws its thread");
    assert!(close(reach, (0.0, 10.0)), "{reach:?}");
    assert!((turns - 8.0).abs() < 1e-3, "{turns}");
    // The plain hole beside it draws none.
    for radius in [2.5, 2.5 * in_hole / 3.4] {
        assert!(
            helix_about(
                &plan,
                &snapshot,
                "plate",
                [60.0, 10.0, 10.0],
                [-1.0, 0.0, 0.0],
                radius
            )
            .is_none()
        );
    }
    // The bolt: its whole 40 mm shank below the head, on the 8 mm diameter.
    let (reach, turns) = helix_about(&plan, &snapshot, "s", [0.0; 3], [0.0, 0.0, 1.0], 4.0)
        .expect("the bolt draws its thread");
    assert!(close(reach, (-40.0, 0.0)), "{reach:?}");
    assert!((turns - 32.0).abs() < 1e-3, "{turns}");
    // A nut names no thread of its own; its bore stays plain.
    assert!(helix_about(&plan, &snapshot, "n", [0.0; 3], [0.0, 0.0, 1.0], 4.0).is_none());
}

/// The display mesh vertices of `definition` in `plan`.
fn vertices(plan: &InstancedRenderPlan, snapshot: &Snapshot, definition: &str) -> usize {
    plan.batches()
        .iter()
        .filter(|batch| {
            snapshot
                .definition(batch.definition_id)
                .is_some_and(|found| found.name() == definition)
        })
        .map(|batch| batch.geometry.vertex_count())
        .sum()
}

#[test]
fn the_window_switch_models_threads_keeps_the_program_and_undo_restores_cosmetic() {
    let mut app = KetchupApp::new();
    app.apply_program_source(program(), true).unwrap();
    app.program_evaluations.get_blocking(&program()).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let cosmetic = app.document.current();
    let cosmetic_plan = app.rebuild_render_plan(&cosmetic);
    let hole = |plan: &InstancedRenderPlan, snapshot: &Snapshot| {
        let in_hole =
            3.4 * (ketchup_scheduler::exact_worker::STEP_MESH_ANGULAR_DEFLECTION / 2.0).cos();
        helix_about(
            plan,
            snapshot,
            "plate",
            [10.0, 10.0, 20.0],
            [0.0, 0.0, -1.0],
            in_hole,
        )
    };
    let bolt = |plan: &InstancedRenderPlan, snapshot: &Snapshot| {
        helix_about(plan, snapshot, "s", [0.0; 3], [0.0, 0.0, 1.0], 4.0)
    };
    assert!(hole(&cosmetic_plan, &cosmetic).is_some());
    assert!(bolt(&cosmetic_plan, &cosmetic).is_some());

    assert!(app.set_modeled_threads(true));
    assert!(
        app.document.current_rule_program().is_some(),
        "the program keeps the document"
    );
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let modeled = app.document.current();
    assert!(modeled.modeled_threads());
    let modeled_plan = app.rebuild_render_plan(&modeled);
    // The grooves are in the geometry: no drawn helix, many more vertices.
    assert!(hole(&modeled_plan, &modeled).is_none());
    assert!(bolt(&modeled_plan, &modeled).is_none());
    for part in ["plate", "s"] {
        assert!(
            vertices(&modeled_plan, &modeled, part) > 2 * vertices(&cosmetic_plan, &cosmetic, part),
            "{part}"
        );
    }
    assert!(!app.set_modeled_threads(true), "already modelled");
    // Scaling the bolt would stretch its modelled M8 thread out of size.
    let bolt_id = modeled
        .occurrences()
        .find(|occurrence| {
            modeled
                .definition(occurrence.definition_id())
                .is_some_and(|definition| definition.name() == "s")
        })
        .unwrap()
        .id();
    assert!(app.headless_select_occurrence(bolt_id));
    assert!(!app.scale_selected(1.5));
    assert_eq!(
        app.action_digest(),
        app.catalog.text("digest-scale-refused-thread")
    );
    assert_eq!(
        app.document.current().canonical_digest(),
        modeled.canonical_digest()
    );
    // The document refuses such a Scale whoever asks for it.
    let (threaded, _) = modeled.shaft_threads().next().unwrap();
    let definition_id = modeled.feature(threaded).unwrap().definition_id();
    let id = FeatureId(
        modeled
            .features()
            .map(|feature| feature.id().0)
            .max()
            .unwrap()
            + 1,
    );
    let error = app
        .document
        .apply_batch(&CommandBatch::new(vec![CanonicalCommand::CreateFeature {
            id,
            definition_id,
            name: "Scale".to_owned(),
            kind: FeatureKind::Scale {
                target: threaded,
                origin_mm: [0.0; 3],
                factors: [1.5; 3],
            },
        }]))
        .err();
    assert!(
        matches!(error, Some(CanonicalError::ScaledHoleThread(_))),
        "{error:?}"
    );

    app.dispatch_command(AppCommand::Undo);
    let undone = app.document.current();
    assert_eq!(undone.canonical_digest(), cosmetic.canonical_digest());
    assert!(app.document.current_rule_program().is_some());
    let undone_plan = app.rebuild_render_plan(&undone);
    assert!(hole(&undone_plan, &undone).is_some());
    assert!(bolt(&undone_plan, &undone).is_some());
}

#[test]
fn a_plan_built_before_the_program_is_evaluated_gains_the_bolt_thread_after() {
    let mut app = KetchupApp::new();
    app.apply_program_source(program(), true).unwrap();
    crate::drawn_shape::tests::evaluate_exact(&mut app);
    let snapshot = app.document.current();
    app.program_evaluations.forget();
    let early = app.rebuild_render_plan(&snapshot);
    assert!(helix_about(&early, &snapshot, "s", [0.0; 3], [0.0, 0.0, 1.0], 4.0).is_none());
    // The tapped hole is in the document, so it is drawn at once.
    assert!(
        helix_about(
            &early,
            &snapshot,
            "plate",
            [10.0, 10.0, 20.0],
            [0.0, 0.0, -1.0],
            3.4 * (ketchup_scheduler::exact_worker::STEP_MESH_ANGULAR_DEFLECTION / 2.0).cos()
        )
        .is_some()
    );

    app.program_evaluations.get_blocking(&program()).unwrap();
    assert!(app.render_plan_misses_program());
    let later = app.rebuild_render_plan(&snapshot);
    assert!(!app.render_plan_misses_program());
    assert!(helix_about(&later, &snapshot, "s", [0.0; 3], [0.0, 0.0, 1.0], 4.0).is_some());
}
