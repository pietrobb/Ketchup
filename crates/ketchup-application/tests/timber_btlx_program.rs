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
/// export. A drilled board beside them is a manufactured row: woodWOP and the
/// job machine it, BTLx carries the member alone.
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
         washer('washer', 'M10', at = (200, -300, 0))\n\
         shelf = board('shelf', (600, 300, 18), at = (0, 300, 0))\n\
         hole(shelf, 'z+', at = (100, 150), diameter = 8, depth = 10)\n",
    );
    // A production job names each made piece by its code; the shop gives the
    // rail and the shelf one, the purchased fasteners need none.
    let path = |name: &str| {
        snapshot
            .scene_query()
            .into_iter()
            .find(|occurrence| {
                snapshot
                    .definition(occurrence.definition_id)
                    .is_some_and(|definition| definition.name() == name)
            })
            .unwrap()
            .instance_path
    };
    let snapshot = {
        use ketchup_model::document::{CanonicalCommand, CommandBatch};
        snapshot
            .preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProductionCode {
                    instance_path: path("rail"),
                    code: Some("000000000001".into()),
                },
                CanonicalCommand::SetProductionCode {
                    instance_path: path("shelf"),
                    code: Some("000000000002".into()),
                },
            ]))
            .unwrap()
    };
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(quantities(&projection, GeneralBomItemKind::Timber), 1);
    assert_eq!(quantities(&projection, GeneralBomItemKind::Manufactured), 1);
    assert_eq!(quantities(&projection, GeneralBomItemKind::Purchased), 3);
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert_eq!(btlx.matches("<Part ").count(), 1, "{btlx}");
    let programs = projection
        .woodwop_mpr_4_0_setup_a_programs(&snapshot, Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(programs.len(), 2);
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(job["parts"].as_array().map(Vec::len), Some(2), "{job}");
}

/// Idea #128: a drilled library board on its own is one woodWOP program and
/// one part of a production job, a manufactured row of the BOM.
#[test]
fn a_drilled_board_exports_to_woodwop_and_a_production_job() {
    use ketchup_manufacturing::fabrication::GeneralBomItemKind;
    use ketchup_manufacturing::fabrication::production::{HomagWoodwopAdapter, ProductionAdapter};
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let snapshot = apply(
        "shelf = board('shelf', (600, 300, 18))\n\
         hole(shelf, 'z+', at = (100, 150), diameter = 8, depth = 10)\n",
    );
    let shelf = snapshot.scene_query()[0].instance_path.clone();
    let snapshot = {
        use ketchup_model::document::{CanonicalCommand, CommandBatch};
        snapshot
            .preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProductionCode {
                    instance_path: shelf,
                    code: Some("000000000001".into()),
                },
            ]))
            .unwrap()
    };
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(projection.bom.rows.len(), 1);
    assert_eq!(
        projection.bom.rows[0].item_kind,
        GeneralBomItemKind::Manufactured
    );
    let programs = projection
        .woodwop_mpr_4_0_setup_a_programs(&snapshot, Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(programs.len(), 1);
    let woodwop = HomagWoodwopAdapter {
        options: Default::default(),
    };
    let adapters: [&dyn ProductionAdapter; 1] = [&woodwop];
    let job = projection
        .production_job(&snapshot, &adapters)
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(job["parts"].as_array().map(Vec::len), Some(1), "{job}");
    assert_eq!(
        job["outputs"]["homag-woodwop4"].as_array().map(Vec::len),
        Some(1),
        "{job}"
    );
}

/// `source` with a production code on every part, and its woodWOP programs.
fn woodwop_programs(
    worker: &mut ExactWorkerSupervisor,
    source: &str,
) -> Result<Vec<(ketchup_model::document::InstancePath, Vec<u8>)>, String> {
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    let snapshot = apply(source);
    let codes = snapshot
        .scene_query()
        .into_iter()
        .enumerate()
        .map(|(index, occurrence)| CanonicalCommand::SetProductionCode {
            instance_path: occurrence.instance_path,
            code: Some(format!("{:012}", index + 1)),
        })
        .collect();
    let snapshot = snapshot.preview_batch(&CommandBatch::new(codes)).unwrap();
    let projection = fabrication(worker, &snapshot).map_err(|error| error.to_string())?;
    // The cabinet's back groove is milled as a vertical pocket.
    let options = ketchup_manufacturing::fabrication::WoodwopMprOptions {
        vertical_pocket_tool_number: Some(101),
    };
    projection
        .woodwop_mpr_4_0_setup_a_programs(&snapshot, options)
        .map_err(|error| error.to_string())
}

/// Review 2026-10-21 MF-19: a board drilled from below only is laid the other
/// face up, so its holes are drilled from above; one drilled into both faces
/// is refused by name, not as a stock that is not a box.
#[test]
fn a_board_drilled_from_below_is_turned_over_and_one_drilled_from_both_faces_is_named() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let below = woodwop_programs(
        &mut worker,
        "shelf = board('shelf', (600, 300, 18))\n\
         hole(shelf, 'z-', at = (100, 150), diameter = 8, depth = 10)\n",
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let [(_, mpr)] = below.as_slice() else {
        panic!("one program")
    };
    let mpr = String::from_utf8_lossy(mpr);
    assert_eq!(mpr.matches("\\BohrVert\\").count(), 1, "{mpr}");
    // Turned over about X: x stays 100, y is 300 - 150 from the near edge.
    assert!(
        mpr.contains("XA=\"100\"") && mpr.contains("YA=\"150\""),
        "{mpr}"
    );
    let both = woodwop_programs(
        &mut worker,
        "shelf = board('shelf', (600, 300, 18))\n\
         hole(shelf, 'z-', at = (100, 150), diameter = 8, depth = 5)\n\
         hole(shelf, 'z+', at = (300, 150), diameter = 8, depth = 5)\n",
    )
    .expect_err("refused");
    assert!(
        both.contains("\"shelf\" is drilled into both its top and its bottom face"),
        "{both}"
    );
}

/// Review 2026-10-24 MF-20: a production job and its own woodWOP program agree
/// where a hole is in a board turned over. The job's stock frame took the
/// hole the program drills at (180, 280) from above back to (180, 280, 19),
/// not the (180, 120, 0) drawn, and the program did not say which face goes up.
#[test]
fn a_production_job_turns_its_stock_frame_over_as_its_woodwop_program() {
    use ketchup_manufacturing::fabrication::production::HomagWoodwopAdapter;
    use ketchup_model::document::{CanonicalCommand, CommandBatch};
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for (face, entry, turned) in [
        ("z-", [180.0, 120.0, 0.0], true),
        ("z+", [180.0, 120.0, 19.0], false),
    ] {
        let snapshot = apply(&format!(
            "panel = board('panel', (600, 400, 19))\n\
             hole(panel, '{face}', at = (180, 120), diameter = 8, depth = 10)\n"
        ));
        let panel = snapshot.scene_query()[0].instance_path.clone();
        let snapshot = snapshot
            .preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProductionCode {
                    instance_path: panel,
                    code: Some("000000000001".into()),
                },
            ]))
            .unwrap();
        let projection =
            fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
        let woodwop = HomagWoodwopAdapter {
            options: Default::default(),
        };
        let job = projection
            .production_job(&snapshot, &[&woodwop])
            .unwrap_or_else(|error| panic!("{error}"));
        let part = &job["parts"][0];
        let mpr = job["outputs"]["homag-woodwop4"][0]["content"]
            .as_str()
            .unwrap();
        let value = |key: &str| -> f64 {
            let start = mpr.find(&format!("\n{key}=\"")).unwrap() + key.len() + 3;
            mpr[start..start + mpr[start..].find('"').unwrap()]
                .parse()
                .unwrap()
        };
        // A vertical drilling enters the top face of the blank as the program lays it.
        let machine = [value("XA"), value("YA"), value("DI")];
        let vector = |value: &serde_json::Value| -> [f64; 3] {
            std::array::from_fn(|axis| value[axis].as_f64().unwrap())
        };
        let origin = vector(&part["stock_frame"]["origin_mm"]);
        let axes: [[f64; 3]; 3] =
            std::array::from_fn(|axis| vector(&part["stock_frame"]["axes"][axis]));
        let drawn: [f64; 3] = std::array::from_fn(|index| {
            origin[index] + (0..3).map(|k| axes[k][index] * machine[k]).sum::<f64>()
        });
        assert!(
            drawn
                .iter()
                .zip(entry)
                .all(|(got, want)| (got - want).abs() < 1e-9),
            "{face}: the job takes the program's hole to {drawn:?}, drawn at {entry:?}\n{mpr}"
        );
        assert_eq!(part["turned_over"], turned, "{face}: {part}");
        assert_eq!(mpr.contains("\n\\turned over: "), turned, "{face}\n{mpr}");
    }
}

/// Review 2026-10-24 MF-21: the BOM wrote a board's rows in the BTLx order of
/// a timber member, so a cabinet side 18 × 560 × 720 was L 720, W 18, H 560
/// and its thickness stood in the width column. A board is listed longest
/// first and thinnest last, as a cut list states it; a member keeps the BTLx
/// order.
#[test]
fn the_bom_lists_a_board_by_length_width_and_thickness() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let snapshot = apply(&format!(
        "side = board('side', (18, 560, 720))\n\
         beam = box('beam', (60, 140, 3000), {ROLE})\n\
         place(beam, origin = (2000, 0, 0), z = (0, 0, 1), x = (1, 0, 0))\n"
    ));
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    let bom = String::from_utf8(projection.bom_export(&snapshot).unwrap()).unwrap();
    assert!(
        bom.contains("length_mm=720;width_mm=560;height_mm=18;material=board"),
        "{bom}"
    );
    assert!(
        bom.contains("length_mm=3000;width_mm=60;height_mm=140;"),
        "{bom}"
    );
}

/// MF-19: the cabinet and the table, both doweled from below or from inside,
/// are each part one woodWOP program.
#[test]
fn the_cabinet_and_the_table_export_to_woodwop() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    for source in [
        include_str!("../../../examples/programs/cabinet.star"),
        include_str!("../../../examples/programs/table.star"),
    ] {
        let programs =
            woodwop_programs(&mut worker, source).unwrap_or_else(|error| panic!("{error}"));
        assert!(!programs.is_empty());
    }
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

/// Review 2026-10-21 MF-22: a member drawn lying along x was written with its
/// depth as the BTLx length (Length 200, Width 4000) and its machining in that
/// frame. Its length is the 4000 mm, and a hole 1000 mm along it is 1000 mm
/// along the part.
#[test]
fn a_member_drawn_along_x_is_written_with_its_length_along_the_part() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let source = format!(
        "p=box('beam',(4000,120,200),{ROLE})\n\
         hole(p, 'z+', at = (1000, 40), diameter = 10, depth = 50)\n"
    );
    let snapshot = apply(&source);
    let projection = projection(&mut worker, &snapshot, "beam");
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert!(
        btlx.contains("Length=\"4000\" Width=\"120\" Height=\"200\""),
        "{btlx}"
    );
    // Entering the top face (Z 200) 1000 mm along the part, 40 mm across it.
    assert!(
        btlx.contains("<ReferencePoint X=\"1000\" Y=\"40\" Z=\"200\"/>"),
        "{btlx}"
    );
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
