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
    // Review 2026-11-01 MF-102: one MPR program says it machines one of the
    // two made parts and points to the package, which programs both.
    assert_eq!(
        projection.woodwop_mpr_4_0_drill_export(&snapshot).err(),
        Some(
            ketchup_manufacturing::fabrication::GeneralFabricationError::OneProgramPerPart {
                parts: 2
            }
        )
    );
    let zip = projection
        .woodwop_mpr_4_0_production_zip(&snapshot, Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    for name in ["000000000001.mpr", "000000000002.mpr"] {
        assert!(
            zip.windows(name.len())
                .any(|window| window == name.as_bytes()),
            "{name}"
        );
    }
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(job["parts"].as_array().map(Vec::len), Some(2), "{job}");
}

/// Review 2026-11-02 MF-120: a HOMAG package of pieces without a usable
/// production code was a bare "export blocked"; it names the part and the code
/// it needs. The codes are assigned for every made piece, the program keeps
/// owning the document and still names them after it is applied again, and the
/// package is written.
#[test]
fn a_homag_package_names_the_piece_without_a_code_and_assigned_codes_pass() {
    use ketchup_assistant::sidecar::{
        AssistantCadEditOperation, AssistantCadEditProgram, AssistantCadEntitySelector,
    };
    use ketchup_manufacturing::fabrication::GeneralFabricationError;
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let program = |diameter: u32| RuleProgramSource {
        variants: Default::default(),
        file_name: "member.star".into(),
        source: format!(
            "rail = member('rail', (0, 0, 0), (1000, 0, 0), (60, 80))\n\
             hole(rail, 'y+', at = (30, 500), diameter = {diameter}, depth = 20)\n\
             shelf = board('shelf', (600, 300, 18), at = (0, 300, 0))\n\
             hole(shelf, 'z+', at = (100, 150), diameter = 8, depth = 10)\n"
        ),
        overrides: Default::default(),
    };
    let mut session = DocumentSession::default();
    session.apply_rule_program(program(8), false).unwrap();
    let set_code = |session: &mut DocumentSession, name: &str, code: Option<&str>| {
        let snapshot = session.snapshot();
        let occurrence = snapshot.occurrences().find(|o| o.name() == name).unwrap();
        session
            .apply_cad_program(
                &AssistantCadEditProgram {
                    operations: vec![AssistantCadEditOperation::SetProductionCode {
                        selector: AssistantCadEntitySelector::Occurrences {
                            occurrence_ids: vec![occurrence.id().0],
                        },
                        code: code.map(str::to_owned),
                    }],
                },
                &std::collections::BTreeSet::new(),
            )
            .unwrap();
    };
    set_code(&mut session, "shelf", Some("TOP-A"));
    assert!(
        session.rule_program().is_some(),
        "a code detached the program"
    );
    let snapshot = session.snapshot();
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    let refusal = projection
        .woodwop_mpr_4_0_production_zip(&snapshot, Default::default())
        .unwrap_err();
    assert_eq!(
        refusal,
        GeneralFabricationError::HomagProgramCodeUnusable {
            part: "rail".into(),
            code: None
        }
    );
    assert!(refusal.to_string().contains("exactly 12"), "{refusal}");
    set_code(&mut session, "rail", Some("000000000007"));
    let snapshot = session.snapshot();
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        projection
            .woodwop_mpr_4_0_production_zip(&snapshot, Default::default())
            .err(),
        Some(GeneralFabricationError::HomagProgramCodeUnusable {
            part: "shelf".into(),
            code: Some("TOP-A".into())
        })
    );
    // The rail keeps its code; the shelf's unusable one is replaced, counting on.
    let assignments = projection.production_code_assignments(&snapshot);
    assert_eq!(
        assignments
            .iter()
            .map(|(_, code)| code.as_str())
            .collect::<Vec<_>>(),
        ["000000000008"]
    );
    // Review 2026-11-03 MF-142: past the highest 12-digit code the count went
    // on to 13 digits, no program can be named by; it takes the lowest free one.
    set_code(&mut session, "rail", Some("999999999999"));
    let snapshot = session.snapshot();
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        projection
            .production_code_assignments(&snapshot)
            .iter()
            .map(|(_, code)| code.as_str())
            .collect::<Vec<_>>(),
        ["000000000001"]
    );
    set_code(&mut session, "rail", Some("000000000007"));
    set_code(&mut session, "shelf", Some(&assignments[0].1));
    session.apply_rule_program(program(10), false).unwrap();
    assert!(session.rule_program().is_some());
    let snapshot = session.snapshot();
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert!(projection.production_code_assignments(&snapshot).is_empty());
    let zip = projection
        .woodwop_mpr_4_0_production_zip(&snapshot, Default::default())
        .unwrap_or_else(|error| panic!("{error}"));
    for name in ["000000000007.mpr", "000000000008.svg"] {
        assert!(
            zip.windows(name.len())
                .any(|window| window == name.as_bytes()),
            "{name}"
        );
    }
}

/// Review 2026-11-01 MF-103: a table top with rounded corners, a profile of
/// lines and arcs, has a blank around its arcs and a production job; an
/// outline with a free-form edge is refused for its outline, not its cuts.
#[test]
fn a_rounded_table_top_is_described_and_a_free_form_outline_says_so() {
    use ketchup_manufacturing::fabrication::production::{HomagWoodwopAdapter, ProductionAdapter};
    use ketchup_manufacturing::fabrication::{GeneralFabricationError, ShopElement};
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let coded = |snapshot: Snapshot| {
        use ketchup_model::document::{CanonicalCommand, CommandBatch};
        let top = snapshot.scene_query()[0].instance_path.clone();
        snapshot
            .preview_batch(&CommandBatch::new(vec![
                CanonicalCommand::SetProductionCode {
                    instance_path: top,
                    code: Some("000000000001".into()),
                },
            ]))
            .unwrap()
    };
    let snapshot = coded(apply(
        "top = extrude('top', profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40), \
         distance = 18, material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n",
    ));
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        projection.manufacturing.undescribed_sources.is_empty(),
        "{:#?}",
        projection.manufacturing.undescribed_sources
    );
    let woodwop = HomagWoodwopAdapter {
        options: Default::default(),
    };
    let adapters: [&dyn ProductionAdapter; 1] = [&woodwop];
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    let part = &job["parts"][0];
    assert_eq!(part["stock_shape"], "profile_extrusion", "{job}");
    assert_eq!(
        part["dimensions_mm"],
        serde_json::json!([800.0, 500.0, 18.0]),
        "{job}"
    );
    // woodWOP lays a rectangular blank: the rounded top is refused by name.
    assert!(matches!(
        projection.production_job(&snapshot, &adapters),
        Err(GeneralFabricationError::CannotCarry {
            element: ShopElement::ShapedBlank,
            ..
        })
    ));

    let snapshot = coded(apply(
        "top = extrude('top', profile = ellipse(400, 250, center = (400, 250)), distance = 18, \
         material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n",
    ));
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    let Err(GeneralFabricationError::CannotCarry {
        part,
        element,
        reason,
        ..
    }) = projection.production_job(&snapshot, &[])
    else {
        panic!("a free-form outline must be refused");
    };
    assert_eq!(
        (part.as_str(), element),
        ("top", ShopElement::FreeFormBlank)
    );
    assert!(reason.contains("outline"), "{reason}");
}

/// Review 2026-11-02 MF-123, MF-124: a rounded board dropped a whole HOMAG
/// package, and a woodWOP program of it or of a board without machining, with a
/// bare "export blocked". woodWOP and BTLx refuse a blank that is not a
/// rectangle by its part, with the way out; a part without machining says so.
#[test]
fn woodwop_and_btlx_name_a_shaped_blank_and_a_part_without_machining() {
    use ketchup_manufacturing::fabrication::{GeneralFabricationError, ShopElement, ShopFormat};
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let refused = |error: GeneralFabricationError| match error {
        GeneralFabricationError::CannotCarry {
            format,
            part,
            element,
            reason,
        } => {
            assert!(reason.contains("production job"), "{reason}");
            (format, part, element)
        }
        other => panic!("{other}"),
    };
    const TOP: &str = "extrude('top', profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40), \
         distance = 18, material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n";
    let snapshot = apply(&format!(
        "{TOP}shelf = board('shelf', (600, 300, 18), at = (0, 600, 0))\n\
         hole(shelf, 'z+', at = (100, 150), diameter = 8, depth = 10)\n"
    ));
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    let woodwop = ShopFormat::Woodwop {
        vertical_pocket_tool: false,
    };
    let shaped = (woodwop, "top".to_owned(), ShopElement::ShapedBlank);
    assert_eq!(
        refused(
            projection
                .woodwop_mpr_4_0_production_zip(&snapshot, Default::default())
                .unwrap_err()
        ),
        shaped
    );
    let snapshot = apply(TOP);
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        refused(
            projection
                .woodwop_mpr_4_0_drill_export(&snapshot)
                .unwrap_err()
        ),
        shaped
    );
    let snapshot = apply("board('shelf', (600, 300, 18))\n");
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    let bare = projection
        .woodwop_mpr_4_0_drill_export(&snapshot)
        .unwrap_err();
    assert_eq!(
        bare,
        GeneralFabricationError::NothingToMachine {
            part: "shelf".into()
        }
    );
    assert!(bare.to_string().contains("no drilling or pocket"), "{bare}");
    // Review 2026-11-04 MF-123: a pocket tool that is no woodWOP tool number
    // is named as such, not as a bare "export blocked".
    for tool in [0, 1_000_000] {
        let refused = projection
            .woodwop_mpr_4_0_production_zip(
                &snapshot,
                ketchup_manufacturing::fabrication::WoodwopMprOptions {
                    vertical_pocket_tool_number: Some(tool),
                },
            )
            .unwrap_err();
        assert_eq!(
            refused,
            GeneralFabricationError::PocketToolNumberInvalid { tool }
        );
        assert!(refused.to_string().contains("999999"), "{refused}");
    }
    let snapshot = apply(
        "extrude('beam', profile = round_corners([[0, 0], [120, 0], [120, 200], [0, 200]], 10), \
         distance = 3000, material = 'C24', attributes = {FABRICATION_ROLE: TIMBER_MEMBER})\n",
    );
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        refused(projection.btlx_2_3_1_export(&snapshot).unwrap_err()),
        (
            ShopFormat::Btlx,
            "beam".to_owned(),
            ShopElement::ShapedBlank
        )
    );
}

/// Review 2026-11-02 MF-122: the shop drawing of a rounded table top drew the
/// three rectangles of its bounding box, without the radius or a word, and the
/// shop cut sharp corners. A blank that is not a rectangle is drawn as its
/// cross-section, arcs included; a rectangular one stays the views alone.
#[test]
fn the_shop_drawing_draws_a_blank_that_is_not_a_rectangle() {
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let drawing = |worker: &mut ExactWorkerSupervisor, source: &str| {
        let snapshot = apply(source);
        let projection = fabrication(worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
        String::from_utf8(projection.drawing_svg(&snapshot).unwrap()).unwrap()
    };
    let rounded = drawing(
        &mut worker,
        "extrude('top', profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 20), \
         distance = 18, material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n",
    );
    assert!(
        rounded.contains("blank outline (the views show its bounding box): width=800 mm, height=500 mm, length=18 mm"),
        "{rounded}"
    );
    assert_eq!(rounded.matches("A 20 20 0").count(), 4, "{rounded}");
    let shaped = drawing(
        &mut worker,
        "extrude('shelf', profile = [[0, 0], [800, 0], [800, 200], [300, 200], [300, 500], [0, 500]], \
         distance = 18, material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n",
    );
    assert!(
        shaped.contains(
            "blank outline (the views show its bounding box): width=800 mm, height=500 mm"
        ),
        "{shaped}"
    );
    // A shallow bow (R 1625) is sized by the arc that is cut, not its circle.
    let bowed = drawing(
        &mut worker,
        "extrude('top', profile = [['front', [0, 0], [800, 0], {'through': (400, -50)}], \
         ['right', [800, 0], [800, 500]], ['back', [800, 500], [0, 500]], ['left', [0, 500], [0, 0]]], \
         distance = 18, material = 'oak', attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})\n",
    );
    assert!(
        bowed.contains(
            "blank outline (the views show its bounding box): width=800 mm, height=550 mm"
        ),
        "{bowed}"
    );
    let plain = drawing(&mut worker, "board('shelf', (600, 300, 18))\n");
    assert!(!plain.contains("blank outline"), "{plain}");
}

/// Review 2026-11-02 MF-121: a round table top drawn as a circle (or two half
/// arcs a profile joins into one) is one circle, which the blank did not take:
/// the top was undescribed, with the reason that its cuts could not be
/// described, and blocked the job. A circle is two arcs.
#[test]
fn a_round_table_top_is_described_as_two_arcs() {
    use ketchup_assistant::sidecar::*;
    use ketchup_manufacturing::fabrication::{
        FABRICATION_ROLE_DIMENSION_V1, MANUFACTURED_ITEM_ROLE_V1,
    };
    use ketchup_model::document::{
        CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    };
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::CreatePart {
                    holes: Vec::new(),
                    pockets: Vec::new(),
                    name: "top".into(),
                    workplane: AssistantWorkplaneSpec::Principal {
                        plane: AssistantPrincipalPlane::Xy,
                    },
                    entities: vec![AssistantSketchEntity::Circle {
                        id: 1,
                        center_mm: [400.0, 400.0],
                        radius_mm: 400.0,
                    }],
                    constraints: vec![],
                    feature: AssistantCadPartFeature::Extrusion { distance_mm: 18.0 },
                    translation_mm: [0.0, 0.0, 0.0],
                    rotation: None,
                }],
            },
            &std::collections::BTreeSet::new(),
        )
        .unwrap();
    let snapshot = session.snapshot();
    let profiles = graph(&snapshot, "top").profiles;
    assert!(
        matches!(
            profiles[0].geometry,
            ketchup_model::exact_brep_graph::ExactBRepPlanarGeometry::Circle { .. }
        ),
        "{profiles:#?}"
    );
    let top = snapshot.scene_query()[0].instance_path.clone();
    let (dimension, category) = (
        ClassificationDimensionId(900),
        ClassificationCategoryId(901),
    );
    let snapshot = snapshot
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: dimension,
                name: FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(category, MANUFACTURED_ITEM_ROLE_V1.into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: top.root_occurrence(),
                dimension_id: dimension,
                category_id: Some(category),
            },
            CanonicalCommand::SetProductionCode {
                instance_path: top,
                code: Some("000000000001".into()),
            },
        ]))
        .unwrap();
    let projection = fabrication(&mut worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        projection.manufacturing.undescribed_sources.is_empty(),
        "{:#?}",
        projection.manufacturing.undescribed_sources
    );
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        job["parts"][0]["dimensions_mm"],
        serde_json::json!([800.0, 800.0, 18.0]),
        "{job}"
    );
    let drawing = String::from_utf8(projection.drawing_svg(&snapshot).unwrap()).unwrap();
    assert_eq!(drawing.matches("A 400 400 0").count(), 2, "{drawing}");
}

/// A worktop of 1200 x 600 x 38 mm drawn on XY with `openings`, made as a
/// manufactured item with a production code, and its fabrication; the
/// openings make `hole_count` holes of its region.
fn manufactured_worktop(
    worker: &mut ExactWorkerSupervisor,
    openings: Vec<ketchup_assistant::sidecar::AssistantSketchEntity>,
    hole_count: usize,
) -> (
    Snapshot,
    ketchup_manufacturing::fabrication::GeneralFabricationProjection,
) {
    use ketchup_assistant::sidecar::*;
    use ketchup_manufacturing::fabrication::{
        FABRICATION_ROLE_DIMENSION_V1, MANUFACTURED_ITEM_ROLE_V1,
    };
    use ketchup_model::document::{
        CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    };
    let mut session = DocumentSession::default();
    session
        .apply_cad_program(
            &AssistantCadEditProgram {
                operations: vec![AssistantCadEditOperation::CreatePart {
                    holes: Vec::new(),
                    pockets: Vec::new(),
                    name: "worktop".into(),
                    workplane: AssistantWorkplaneSpec::Principal {
                        plane: AssistantPrincipalPlane::Xy,
                    },
                    entities: worktop_rectangle(1, [0.0, 0.0], [1200.0, 600.0])
                        .chain(openings)
                        .collect(),
                    constraints: vec![],
                    feature: AssistantCadPartFeature::Extrusion { distance_mm: 38.0 },
                    translation_mm: [0.0, 0.0, 0.0],
                    rotation: None,
                }],
            },
            &std::collections::BTreeSet::new(),
        )
        .unwrap();
    let snapshot = session.snapshot();
    let profiles = graph(&snapshot, "worktop").profiles;
    assert!(
        matches!(
            &profiles[0].geometry,
            ketchup_model::exact_brep_graph::ExactBRepPlanarGeometry::Region { holes, .. }
                if holes.len() == hole_count
        ),
        "{profiles:#?}"
    );
    let worktop = snapshot.scene_query()[0].instance_path.clone();
    let (dimension, category) = (
        ClassificationDimensionId(900),
        ClassificationCategoryId(901),
    );
    let snapshot = snapshot
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: dimension,
                name: FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![(category, MANUFACTURED_ITEM_ROLE_V1.into())],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: worktop.root_occurrence(),
                dimension_id: dimension,
                category_id: Some(category),
            },
            CanonicalCommand::SetProductionCode {
                instance_path: worktop,
                code: Some("000000000001".into()),
            },
        ]))
        .unwrap();
    let projection = fabrication(worker, &snapshot).unwrap_or_else(|error| panic!("{error}"));
    assert!(
        projection.manufacturing.undescribed_sources.is_empty()
            && projection.manufacturing.free_form_sources.is_empty(),
        "{:#?}",
        projection.manufacturing
    );
    (snapshot, projection)
}

/// The four lines, ids from `first_id`, of the rectangle from `[x0, y0]` to
/// `[x1, y1]`.
fn worktop_rectangle(
    first_id: u64,
    [x0, y0]: [f64; 2],
    [x1, y1]: [f64; 2],
) -> impl Iterator<Item = ketchup_assistant::sidecar::AssistantSketchEntity> {
    let corners = [[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
    (0..4).map(
        move |index| ketchup_assistant::sidecar::AssistantSketchEntity::Line {
            id: first_id + index as u64,
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        },
    )
}

/// Review 2026-11-04 MF-121: a worktop drawn with the opening for its sink, an
/// outline with a hole, was a blank no format describes and refused by every
/// export. Its blank is the outline, and the opening is cut through it: the
/// production job carries it, and woodWOP refuses the cut by name. The cut
/// spans the opening, not the whole worktop (review 2026-11-04 r19 P3).
#[test]
fn a_worktop_drawn_with_its_sink_opening_is_its_outline_cut_through() {
    use ketchup_manufacturing::fabrication::production::{HomagWoodwopAdapter, ProductionAdapter};
    use ketchup_manufacturing::fabrication::{
        GeneralFabricationError, GeneralMachiningGeometry, GeneralManufacturingKind,
        PieceDimensions, ShopElement,
    };
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let (snapshot, projection) = manufactured_worktop(
        &mut worker,
        worktop_rectangle(5, [400.0, 150.0], [800.0, 450.0]).collect(),
        1,
    );
    let operations = &projection.manufacturing.operations;
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.kind)
            .collect::<Vec<_>>(),
        [
            GeneralManufacturingKind::Stock,
            GeneralManufacturingKind::ThroughCut
        ]
    );
    let GeneralMachiningGeometry::ProfileCut {
        segments,
        start_mm,
        end_mm,
        ..
    } = &operations[1].machining
    else {
        panic!("{:#?}", operations[1]);
    };
    assert_eq!(segments.len(), 4, "{segments:#?}");
    assert!(
        (end_mm - start_mm - 38.0).abs() < 1e-6,
        "{start_mm} {end_mm}"
    );
    assert_eq!(
        operations[1].bounds,
        PieceDimensions {
            length_mm: 400.0,
            width_mm: 300.0,
            height_mm: 38.0,
        }
    );
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!(
        job["parts"][0]["dimensions_mm"],
        serde_json::json!([1200.0, 600.0, 38.0]),
        "{job}"
    );
    let woodwop = HomagWoodwopAdapter {
        options: Default::default(),
    };
    let adapters: [&dyn ProductionAdapter; 1] = [&woodwop];
    assert!(matches!(
        projection.production_job(&snapshot, &adapters),
        Err(GeneralFabricationError::CannotCarry {
            element: ShopElement::Operation(GeneralManufacturingKind::ThroughCut),
            ..
        })
    ));
}

/// Review 2026-11-06 MF-181: every round opening of a worktop was a drilling
/// of its whole diameter, the Ø450 one of a sink as the Ø35 one of its tap,
/// which no drill makes. A round opening is its two half arcs cut to the
/// worktop's depth where it is (review 2026-11-04 r19 P3), which the
/// production job carries as a cut, not a drilling.
#[test]
fn a_round_opening_of_a_worktop_is_cut_not_drilled() {
    use ketchup_manufacturing::fabrication::production::{HomagWoodwopAdapter, ProductionAdapter};
    use ketchup_manufacturing::fabrication::{
        BtlxExportOptions, FABRICATION_ROLE_DIMENSION_V1, GeneralFabricationError,
        GeneralMachiningGeometry, GeneralManufacturingKind, MANUFACTURED_ITEM_ROLE_V1,
        PieceDimensions, ShopElement, TIMBER_MEMBER_ROLE_V1,
    };
    use ketchup_model::document::{
        CanonicalCommand, ClassificationCategoryId, ClassificationDimensionId, CommandBatch,
    };
    let _turn = crate::integration_support::file_turn();
    let mut worker = worker();
    let circle = |id: u64, center_mm: [f64; 2], radius_mm: f64| {
        ketchup_assistant::sidecar::AssistantSketchEntity::Circle {
            id,
            center_mm,
            radius_mm,
        }
    };
    let (snapshot, projection) = manufactured_worktop(
        &mut worker,
        vec![
            circle(5, [1000.0, 450.0], 17.5),
            circle(6, [450.0, 300.0], 225.0),
        ],
        2,
    );
    let operations = &projection.manufacturing.operations;
    assert_eq!(
        operations
            .iter()
            .map(|operation| operation.kind)
            .collect::<Vec<_>>(),
        [
            GeneralManufacturingKind::Stock,
            GeneralManufacturingKind::ThroughCut,
            GeneralManufacturingKind::ThroughCut
        ]
    );
    let mut diameters = Vec::new();
    for operation in &operations[1..] {
        let GeneralMachiningGeometry::ProfileCut {
            segments,
            start_mm,
            end_mm,
            ..
        } = &operation.machining
        else {
            panic!("{operation:#?}");
        };
        assert_eq!(segments.len(), 2, "{segments:#?}");
        // Cut to the worktop's thickness, a machine depth, not a cutter
        // running past the part as a through-all hole is.
        assert!(
            (end_mm - start_mm - 38.0).abs() < 1e-6,
            "{start_mm} {end_mm}"
        );
        let diameter = operation.bounds.length_mm;
        assert_eq!(
            operation.bounds,
            PieceDimensions {
                length_mm: diameter,
                width_mm: diameter,
                height_mm: 38.0,
            }
        );
        diameters.push(diameter);
    }
    diameters.sort_by(f64::total_cmp);
    assert_eq!(diameters, [35.0, 450.0]);
    let job = projection
        .production_job(&snapshot, &[])
        .unwrap_or_else(|error| panic!("{error}"));
    assert!(!job.to_string().contains("circular-drill"), "{job}");
    // BTLx writes the same board marked as timber. With the app's own
    // strategy, saw cuts then milling, an opening with no straight edge to
    // saw is milled along its two half arcs.
    let timber = snapshot
        .preview_batch(&CommandBatch::new(vec![
            CanonicalCommand::UpsertClassificationDimension {
                id: ClassificationDimensionId(900),
                name: FABRICATION_ROLE_DIMENSION_V1.into(),
                categories: vec![
                    (
                        ClassificationCategoryId(901),
                        MANUFACTURED_ITEM_ROLE_V1.into(),
                    ),
                    (ClassificationCategoryId(902), TIMBER_MEMBER_ROLE_V1.into()),
                ],
            },
            CanonicalCommand::SetOccurrenceClassification {
                occurrence_id: snapshot.scene_query()[0].instance_path.root_occurrence(),
                dimension_id: ClassificationDimensionId(900),
                category_id: Some(ClassificationCategoryId(902)),
            },
        ]))
        .unwrap();
    let timber_projection =
        fabrication(&mut worker, &timber).unwrap_or_else(|error| panic!("{error}"));
    let btlx = timber_projection
        .btlx_2_3_1_export_with_options(&timber, BtlxExportOptions::default())
        .unwrap_or_else(|error| panic!("{error}"));
    let btlx = String::from_utf8(btlx).unwrap();
    assert!(!btlx.contains("<Drilling"), "{btlx}");
    assert!(!btlx.contains("<SawContour"), "{btlx}");
    assert_eq!(btlx.matches("<MillContour ").count(), 2, "{btlx}");
    assert_eq!(btlx.matches("<Arc>").count(), 4, "{btlx}");
    // woodWOP refuses the openings as the through cuts they are.
    let woodwop = HomagWoodwopAdapter {
        options: Default::default(),
    };
    let adapters: [&dyn ProductionAdapter; 1] = [&woodwop];
    let refused = projection.production_job(&snapshot, &adapters).unwrap_err();
    assert!(
        matches!(
            refused,
            GeneralFabricationError::CannotCarry {
                element: ShopElement::Operation(GeneralManufacturingKind::ThroughCut),
                ..
            }
        ),
        "{refused}"
    );
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
    // Review 2026-10-24 MF-26: a pocket milled into the top beside a hole
    // drilled into the bottom is the same two-face part.
    let pocket_and_hole = woodwop_programs(
        &mut worker,
        "shelf = board('shelf', (600, 300, 18))\n\
         pocket(shelf, 'z+', rect = (40, 40, 140, 100), depth = 5)\n\
         hole(shelf, 'z-', at = (300, 150), diameter = 8, depth = 5)\n",
    )
    .expect_err("refused");
    assert!(
        pocket_and_hole.contains("\"shelf\" is drilled into both its top and its bottom face"),
        "{pocket_and_hole}"
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
