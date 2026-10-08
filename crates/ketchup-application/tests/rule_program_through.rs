use crate::operations_support::{assert_volume, worker};
use ketchup_application::{DocumentSession, SaveOptions, SessionSettings};
use ketchup_model::document::{RuleProgramSource, Snapshot};
use ketchup_model::exact_brep_graph::{ExactBRepGraph, ExactBRepOperation};
use ketchup_scheduler::ExactWorkerSupervisor;
use std::f64::consts::PI;

fn source(face: &str, through: bool, depth: f64, thickness: f64) -> RuleProgramSource {
    RuleProgramSource {
        file_name: "through.star".into(),
        source: format!(
            "p=box('stock',(100,100,{thickness}))\nhole(p,'{face}',at=(50,50),diameter=8,depth={depth},id='bore',through={})\n",
            if through { "True" } else { "False" }
        ),
        overrides: Default::default(),
    }
}

fn graph(snapshot: &Snapshot) -> ExactBRepGraph {
    let occurrence = snapshot
        .occurrences()
        .find(|o| o.name() == "stock")
        .unwrap();
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    ExactBRepGraph::from_snapshot(
        snapshot,
        occurrence.definition_id(),
        *definition.feature_ids().last().unwrap(),
    )
    .unwrap()
}

fn assert_cut(
    worker: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
    through: bool,
    depth: f64,
    thickness: f64,
) {
    let graph = graph(snapshot);
    if through && depth < thickness {
        assert!(
            matches!(
                graph.nodes.last().unwrap().operation,
                ExactBRepOperation::Boolean { .. }
            ),
            "invalid through intent must not masquerade as blind machining"
        );
    } else {
        let ExactBRepOperation::ProfileCut { depth_bits, .. } =
            &graph.nodes.last().unwrap().operation
        else {
            panic!("expected canonical profile cut")
        };
        assert_eq!(depth_bits.map(f64::from_bits), (!through).then_some(depth));
    }
    let solid = worker.evaluate_exact_brep_graph(&graph).unwrap();
    assert_eq!(solid.topology_counts[4], 1);
    assert_volume(
        solid.volume_mm3,
        100.0 * 100.0 * thickness - PI * 16.0 * depth.min(thickness),
    );
}

#[test]
fn through_entry_directions_and_overdepth_remove_only_stock() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    // A cube makes the same closed-form volume apply to all six entry faces.
    for face in ["x-", "x+", "y-", "y+", "z-", "z+"] {
        let mut session = DocumentSession::default();
        let applied = session
            .apply_rule_program(source(face, true, 105.0, 100.0), false)
            .unwrap();
        assert!(
            !applied
                .report
                .issues
                .iter()
                .any(|i| i.kind == "through_hole_too_shallow")
        );
        assert_cut(&mut worker, &applied.snapshot, true, 105.0, 100.0);
    }
}

#[test]
fn intent_only_edits_thickness_undo_redo_and_reopen_keep_cut_and_source_together() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let mut session = DocumentSession::default();
    let through = source("z+", true, 18.0, 18.0);
    let blind = source("z+", false, 18.0, 18.0);
    let initial = session.apply_rule_program(through.clone(), false).unwrap();
    let occurrence = initial.snapshot.occurrences().next().unwrap().id();
    assert_cut(&mut worker, &initial.snapshot, true, 18.0, 18.0);
    let changed = session.apply_rule_program(blind.clone(), false).unwrap();
    assert!(!changed.replaced_document);
    assert_eq!(
        changed.snapshot.occurrences().next().unwrap().id(),
        occurrence
    );
    assert!(
        changed
            .report
            .issues
            .iter()
            .any(|i| i.kind == "hole_breaks_through")
    );
    assert_cut(&mut worker, &changed.snapshot, false, 18.0, 18.0);
    session.undo().unwrap();
    assert_eq!(session.rule_program().unwrap(), &through);
    assert_cut(&mut worker, &session.snapshot(), true, 18.0, 18.0);
    session.redo().unwrap();
    assert_eq!(session.rule_program().unwrap(), &blind);
    assert_cut(&mut worker, &session.snapshot(), false, 18.0, 18.0);
    session.apply_rule_program(through.clone(), false).unwrap();
    assert_cut(&mut worker, &session.snapshot(), true, 18.0, 18.0);

    // Increasing stock thickness must not silently deepen the requested hole.
    let shallow = source("z+", true, 18.0, 24.0);
    let changed = session.apply_rule_program(shallow.clone(), false).unwrap();
    assert!(
        changed
            .report
            .issues
            .iter()
            .any(|i| i.kind == "through_hole_too_shallow")
    );
    assert_cut(&mut worker, &changed.snapshot, true, 18.0, 24.0);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("through.ketchup");
    session
        .save(&path, SaveOptions { overwrite: false })
        .unwrap();
    drop(session);
    let mut reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.rule_program().unwrap(), &shallow);
    assert_cut(&mut worker, &reopened.snapshot(), true, 18.0, 24.0);
    reopened.apply_rule_program(through.clone(), false).unwrap();
    assert_cut(&mut worker, &reopened.snapshot(), true, 18.0, 18.0);
    reopened
        .save(&path, SaveOptions { overwrite: true })
        .unwrap();
    drop(reopened);
    let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
    assert_eq!(reopened.rule_program().unwrap(), &through);
    assert_cut(&mut worker, &reopened.snapshot(), true, 18.0, 18.0);
}

#[test]
fn shallow_through_and_blind_overdepth_keep_requested_numeric_geometry() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (through, depth, issue) in [
        (true, 12.0, "through_hole_too_shallow"),
        (false, 22.0, "hole_breaks_through"),
    ] {
        let mut session = DocumentSession::default();
        let applied = session
            .apply_rule_program(source("z-", through, depth, 18.0), false)
            .unwrap();
        assert!(applied.report.issues.iter().any(|i| i.kind == issue));
        assert_cut(&mut worker, &applied.snapshot, through, depth, 18.0);
    }
}

fn detached_btlx_projection(
    worker: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
) -> ketchup_manufacturing::fabrication::GeneralFabricationProjection {
    use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
    use ketchup_model::exact_validation::{
        BuiltinGeneralBodyValidator, general_body_input_bytes, general_body_validation_policy,
    };
    use ketchup_model::tolerance::TolerancePolicy;
    use ketchup_model::validation::{
        HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationState,
    };
    let solid = worker.evaluate_exact_brep_graph(&graph(snapshot)).unwrap();
    let registry = ExactResultRegistry::accept(
        snapshot,
        [std::sync::Arc::new(ExactBodyPackage::from(solid))],
    )
    .unwrap();
    let tolerance = TolerancePolicy::default();
    let validator = BuiltinGeneralBodyValidator::new(tolerance);
    let policy = general_body_validation_policy();
    let invocation = ValidationInvocation::bind(
        snapshot,
        validator.descriptor(),
        &policy,
        vec![],
        &general_body_input_bytes(&[]),
    );
    let report = validator.invoke(ValidationExecution {
        snapshot,
        invocation,
        policy: &policy,
        input: &[],
    });
    assert_eq!(report.state, ValidationState::Passed);
    ketchup_manufacturing::fabrication::project_general_fabrication(
        snapshot,
        &registry,
        &[],
        &report,
        tolerance,
    )
    .unwrap()
}

#[test]
fn blind_depth_btlx_export_stays_fail_closed_after_detach_and_reopen() {
    use ketchup_manufacturing::fabrication::{
        FABRICATION_ROLE_DIMENSION_V1, GeneralFabricationError, GeneralMachiningGeometry,
        TIMBER_MEMBER_ROLE_V1,
    };
    use ketchup_model::document::{
        CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
        Transform,
    };
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    // Equal depth and overdepth must both fail; opposite entry and world rotation
    // must not bypass the local stock check. A valid blind hole still exports.
    for (face, depth) in [("z+", 12.0), ("z+", 18.0), ("z+", 22.0), ("z-", 22.0)] {
        let mut session = DocumentSession::default();
        let applied = session
            .apply_rule_program(source(face, false, depth, 18.0), false)
            .unwrap();
        assert_eq!(
            applied
                .report
                .issues
                .iter()
                .any(|issue| issue.kind == "hole_breaks_through"),
            depth >= 18.0,
        );
        let id = applied.snapshot.occurrences().next().unwrap().id();
        let dimension = ClassificationDimensionId(900);
        let category = ClassificationCategoryId(901);
        let proposal = session
            .plan_commands(CommandBatch::new(vec![
                CanonicalCommand::SetOccurrenceTransform {
                    id,
                    transform: Transform::from_matrix([
                        0.0, 0.0, 1.0, 200.0, 0.0, 1.0, 0.0, 300.0, -1.0, 0.0, 0.0, 400.0, 0.0,
                        0.0, 0.0, 1.0,
                    ])
                    .unwrap(),
                },
                CanonicalCommand::UpsertClassificationDimension {
                    id: dimension,
                    name: FABRICATION_ROLE_DIMENSION_V1.into(),
                    categories: vec![(category, TIMBER_MEMBER_ROLE_V1.into())],
                },
                CanonicalCommand::SetOccurrenceClassification {
                    occurrence_id: id,
                    dimension_id: dimension,
                    category_id: Some(category),
                },
            ]))
            .unwrap();
        session.apply_proposal(&proposal).unwrap();
        assert!(session.rule_program().is_none());
        let detached = session.snapshot();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("blind.ketchup");
        session
            .save(&path, SaveOptions { overwrite: false })
            .unwrap();
        drop(session);
        let reopened = DocumentSession::open(&path, SessionSettings::default()).unwrap();
        assert!(reopened.rule_program().is_none());
        for snapshot in [detached, reopened.snapshot()] {
            assert_cut(&mut worker, &snapshot, false, depth, 18.0);
            let projection = detached_btlx_projection(&mut worker, &snapshot);
            let GeneralMachiningGeometry::CircularDrill {
                through,
                start_mm,
                end_mm,
                ..
            } = &projection.manufacturing.operations[1].machining
            else {
                panic!("expected preserved blind drilling")
            };
            assert!(!through);
            assert_eq!(end_mm - start_mm, depth);
            let output = projection.btlx_2_3_1_export(&snapshot);
            if depth < 18.0 {
                let xml = String::from_utf8(output.unwrap()).unwrap();
                assert!(xml.contains("<Drilling "), "{xml}");
                assert!(xml.contains("<Depth>12</Depth>"), "{xml}");
            } else {
                assert_eq!(
                    output,
                    Err(GeneralFabricationError::PartExportBlocked {
                        part: "stock".to_owned()
                    })
                );
            }
        }
    }
}

#[test]
fn drilling_after_another_operation_remains_a_boolean_not_machine_drilling() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let mut program = source("z+", true, 18.0, 18.0);
    program.source = program.source.replace(
        "hole(p,",
        "push_pull(p,face='x+',distance=10,name='wider')\nhole(p,",
    );
    let mut session = DocumentSession::default();
    let applied = session.apply_rule_program(program, false).unwrap();
    let later_graph = graph(&applied.snapshot);
    assert!(matches!(
        later_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::Boolean { .. }
    ));
    let solid = worker.evaluate_exact_brep_graph(&later_graph).unwrap();
    assert_volume(solid.volume_mm3, 110.0 * 100.0 * 18.0 - PI * 16.0 * 18.0);
    assert!(
        session
            .rule_program()
            .unwrap()
            .source
            .contains("through=True")
    );

    let mut nonpanel = source("end", true, 18.0, 18.0);
    nonpanel.source = "p=extrude('stock',profile=[['a',(0,0),(100,0)],['b',(100,0),(0,100)],['c',(0,100),(0,0)]],distance=18)\nhole(p,'end',at=(20,20),diameter=8,depth=18,id='bore',through=True)\n".into();
    let mut session = DocumentSession::default();
    let applied = session.apply_rule_program(nonpanel, false).unwrap();
    let nonpanel_graph = graph(&applied.snapshot);
    assert!(matches!(
        nonpanel_graph.nodes.last().unwrap().operation,
        ExactBRepOperation::Boolean { .. }
    ));
    let solid = worker.evaluate_exact_brep_graph(&nonpanel_graph).unwrap();
    assert_volume(
        solid.volume_mm3,
        0.5 * 100.0 * 100.0 * 18.0 - PI * 16.0 * 18.0,
    );
}
