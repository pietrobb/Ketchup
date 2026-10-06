//! Timber members written as programs (stock box, then cuts) export to BTLx.
use crate::operations_support::worker;
use ketchup_application::DocumentSession;
use ketchup_model::document::{RuleProgramSource, Snapshot};
use ketchup_model::exact_brep_graph::ExactBRepGraph;
use ketchup_scheduler::ExactWorkerSupervisor;

const ROLE: &str =
    "attributes={'classification:ketchup.fabrication-role.v1':'fabrication.timber-member.v1'}";

fn apply(source: &str) -> Snapshot {
    let mut session = DocumentSession::default();
    session
        .apply_rule_program(
            RuleProgramSource {
                file_name: "member.star".into(),
                source: source.into(),
                overrides: Default::default(),
            },
            false,
        )
        .unwrap()
        .snapshot
}

fn graph(snapshot: &Snapshot, name: &str) -> ExactBRepGraph {
    let occurrence = snapshot.occurrences().find(|o| o.name() == name).unwrap();
    let definition = snapshot.definition(occurrence.definition_id()).unwrap();
    ExactBRepGraph::from_snapshot(
        snapshot,
        occurrence.definition_id(),
        *definition.feature_ids().last().unwrap(),
    )
    .unwrap()
}

fn projection(
    worker: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
    name: &str,
) -> ketchup_manufacturing::fabrication::GeneralFabricationProjection {
    use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
    use ketchup_model::exact_validation::{
        BuiltinGeneralBodyValidator, general_body_input_bytes, general_body_validation_policy,
    };
    use ketchup_model::tolerance::TolerancePolicy;
    use ketchup_model::validation::{
        HostNeutralValidator, ValidationExecution, ValidationInvocation, ValidationState,
    };
    let solid = worker
        .evaluate_exact_brep_graph(&graph(snapshot, name))
        .unwrap();
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

/// A stock box with a notch (box tool) and a slanted end cut (extruded profile
/// tool), each subtracted in turn, is one stock and two cuts in one BTLx part.
#[test]
fn a_stock_box_with_successive_prism_cuts_exports_to_btlx() {
    use ketchup_manufacturing::fabrication::GeneralManufacturingKind;
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let source = format!(
        "p=box('rafter',(60,140,2000),{ROLE})\n\
         a=box('rafter / a',(62,40,80),tool=True)\nplace(a,origin=(-1,-1,300),z=(0,0,1),x=(1,0,0))\nsubtract(p,a,name='a')\n\
         b=extrude('rafter / b',profile=[(0,0),(140,0),(0,300)],distance=62,tool=True)\nplace(b,origin=(61,0,1700),z=(-1,0,0),x=(0,1,0))\nsubtract(p,b,name='b')\n"
    );
    let snapshot = apply(&source);
    let projection = projection(&mut worker, &snapshot, "rafter");
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    let kinds = projection
        .manufacturing
        .operations
        .iter()
        .map(|operation| operation.kind)
        .collect::<Vec<_>>();
    assert_eq!(kinds.len(), 3, "{kinds:?}");
    assert_eq!(kinds[0], GeneralManufacturingKind::Stock);
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert_eq!(btlx.matches("<Part ").count(), 1);
    assert!(btlx.contains("Length=\"2000\""), "{btlx}");
    assert!(btlx.matches("ProcessID=").count() >= 2, "{btlx}");
}
