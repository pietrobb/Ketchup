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
                variants: Default::default(),
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
        &ketchup_manufacturing::fabrication::shown_parts(snapshot),
    )
    .unwrap()
}

/// The fabrication of every part of `snapshot`, on its exact bodies and
/// collision check.
fn fabrication(
    worker: &mut ExactWorkerSupervisor,
    snapshot: &Snapshot,
) -> Result<
    ketchup_manufacturing::fabrication::GeneralFabricationProjection,
    ketchup_manufacturing::fabrication::GeneralFabricationError,
> {
    use ketchup_application::validation::fabrication_collision_validation_with_worker;
    use ketchup_model::exact_product::{ExactBodyPackage, ExactResultRegistry};
    use ketchup_model::exact_validation::GeneralBodyParticipant;
    use ketchup_model::persistence::ContainerData;
    let occurrences = snapshot.scene_query();
    let packages = occurrences
        .iter()
        .map(|occurrence| {
            let definition = snapshot.definition(occurrence.definition_id).unwrap();
            let graph = ExactBRepGraph::from_snapshot(
                snapshot,
                occurrence.definition_id,
                *definition.feature_ids().last().unwrap(),
            )
            .unwrap();
            std::sync::Arc::new(ExactBodyPackage::from(
                worker.evaluate_exact_brep_graph(&graph).unwrap(),
            ))
        })
        .collect::<Vec<_>>();
    let registry = ExactResultRegistry::accept(snapshot, packages).unwrap();
    let tolerance = snapshot.tolerance();
    let participants = occurrences
        .iter()
        .map(|occurrence| {
            GeneralBodyParticipant::accept(
                snapshot,
                &registry,
                occurrence.instance_path.clone(),
                tolerance,
            )
            .unwrap()
        })
        .collect::<Vec<_>>();
    let validation = fabrication_collision_validation_with_worker(
        snapshot,
        &participants,
        &ContainerData::default(),
        None,
        std::time::Duration::from_secs(300),
    )
    .unwrap();
    ketchup_manufacturing::fabrication::project_general_fabrication(
        snapshot,
        &registry,
        &validation.cases,
        &validation.report,
        tolerance,
        &ketchup_manufacturing::fabrication::shown_parts(snapshot),
    )
}

/// Review 2026-10-19 MF-14: the library's parts carry their fabrication role,
/// so a program need not write one. A drilled member with a bolt, a nut and a
/// washer beside it exports to BTLx, woodWOP and a production job, the
/// fasteners as purchased rows of the BOM instead of parts that block the
/// export; a board is a manufactured row (its woodWOP and job export: idea #128).
#[test]
fn library_members_boards_and_fasteners_carry_their_fabrication_role() {
    use ketchup_manufacturing::fabrication::GeneralBomItemKind;
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let quantities =
        |projection: &ketchup_manufacturing::fabrication::GeneralFabricationProjection, kind| {
            projection
                .bom
                .rows
                .iter()
                .filter(|row| row.item_kind == kind)
                .map(|row| row.quantity)
                .sum::<usize>()
        };
    let snapshot = apply(
        "rail = member('rail', (0, 0, 0), (1000, 0, 0), (60, 80))\n\
         hole(rail, 'y+', at = (30, 500), diameter = 8, depth = 20)\n\
         bolt('bolt', 'M10', 100, at = (0, -300, 0))\n\
         nut('nut', 'M10', at = (100, -300, 0))\n\
         washer('washer', 'M10', at = (200, -300, 0))\n",
    );
    // A production job names each made piece by its code; the shop gives the
    // rail one, the purchased fasteners need none.
    let rail = snapshot
        .scene_query()
        .into_iter()
        .find(|occurrence| {
            snapshot
                .definition(occurrence.definition_id)
                .is_some_and(|definition| definition.name() == "rail")
        })
        .unwrap()
        .instance_path;
    let snapshot = {
        use ketchup_model::document::{CanonicalCommand, CommandBatch};
        snapshot
            .preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProductionCode {
                    instance_path: rail,
                    code: Some("000000000001".into()),
                },
            ]))
            .unwrap()
    };
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(quantities(&projection, GeneralBomItemKind::Timber), 1);
    assert_eq!(quantities(&projection, GeneralBomItemKind::Purchased), 3);
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert_eq!(btlx.matches("<Part ").count(), 1, "{btlx}");
    let programs = projection
        .woodwop_mpr_4_0_setup_a_programs(&snapshot, Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(programs.len(), 1);
    projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));

    let snapshot = apply("board('shelf', (600, 300, 18))\n");
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(quantities(&projection, GeneralBomItemKind::Manufactured), 1);
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

/// Review MF-N4: a cut open only to the side of the rafter (its tool from
/// z = 500 to 580 inside the 2000 mm length, reaching past the side face x = 60)
/// was written as a contour from a plane inside the rafter, so the cutter would
/// pass through wood that stays. A rectangular pocket is machined from the side
/// face it opens on, 10 mm deep.
#[test]
fn a_rectangular_pocket_open_only_to_the_side_is_machined_from_that_side() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let source = format!(
        "p=box('rafter',(60,140,2000),{ROLE})\n\
         a=box('rafter / slot',(20,40,80),tool=True)\nplace(a,origin=(50,50,500),z=(0,0,1),x=(1,0,0))\nsubtract(p,a,name='slot')\n"
    );
    let snapshot = apply(&source);
    let projection = projection(&mut worker, &snapshot, "rafter");
    assert!(projection.manufacturing.unresolved_sources.is_empty());
    let cut = &projection.manufacturing.operations[1].machining;
    let ketchup_manufacturing::fabrication::GeneralMachiningGeometry::ProfileCut {
        frame,
        start_mm,
        end_mm,
        ..
    } = cut
    else {
        panic!("{cut:?}")
    };
    assert_eq!(frame.normal, [-1.0, 0.0, 0.0], "{cut:?}");
    assert_eq!(
        frame.origin_mm[0] + frame.normal[0] * start_mm,
        60.0,
        "{cut:?}"
    );
    assert_eq!(end_mm - start_mm, 10.0, "{cut:?}");
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert_eq!(btlx.matches("ProcessID=").count(), 1, "{btlx}");
}

/// Review MF-N4: a triangular cut open only to the side (z = 500 to 520) has no
/// face to enter along any axis, so the BTLx export refuses it, naming the part.
#[test]
fn a_triangular_cut_open_only_to_the_side_is_refused_with_its_part() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let source = format!(
        "p=box('rafter',(60,140,2000),{ROLE})\n\
         a=extrude('rafter / v',profile=[(0,0),(40,0),(0,80)],distance=20,tool=True)\nplace(a,origin=(50,30,500),z=(0,0,1),x=(1,0,0))\nsubtract(p,a,name='v')\n"
    );
    let snapshot = apply(&source);
    let projection = projection(&mut worker, &snapshot, "rafter");
    let error = projection
        .btlx_2_3_1_export(&snapshot)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("\"rafter\"") && error.contains("starts inside the material"),
        "{error}"
    );
}
