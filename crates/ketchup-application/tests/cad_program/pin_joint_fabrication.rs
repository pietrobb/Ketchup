//! A pin joint's dowel holes reach every shop format exactly once: drilled
//! into the parts they are their pockets' drillings, logical they are the
//! joint's alone, and a format that cannot carry them refuses them by name.
use super::*;
use ketchup_application::validation::fabrication_collision_validation_with_worker;
use ketchup_manufacturing::fabrication::{
    Carriage, FABRICATION_ROLE_DIMENSION_V1, GeneralFabricationError, GeneralFabricationProjection,
    ShopElement, ShopFormat, TIMBER_MEMBER_ROLE_V1, carriage, project_general_fabrication,
    shown_parts,
};
use ketchup_model::document::{ClassificationCategoryId, ClassificationDimensionId};
use ketchup_model::exact_validation::GeneralBodyParticipant;
use std::time::Duration;

const PINS: usize = 3;

/// Two 100×50×18 timbers butted end to end at x = 100 and joined by a row of
/// three Ø8 dowels drilled horizontally into both end faces.
fn doweled_timbers(holes: AssistantPinHoles) -> Snapshot {
    let mut document = DocumentStore::new();
    let timber = |name: &str, x| {
        cuboid_part(
            name.into(),
            [100.0, 50.0, 18.0],
            Vec::new(),
            Vec::new(),
            [x, 0.0, 0.0],
            None,
        )
    };
    let timbers = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![
            timber("Left timber", 0.0),
            timber("Right timber", 100.0),
        ]),
    )
    .unwrap();
    document.apply_batch(&timbers).unwrap();
    let face = |occurrence_id, face_x, inward_x| AssistantPinJointFace {
        instance_path: AssistantInstancePath {
            root_occurrence_id: occurrence_id,
            steps: Vec::new(),
        }
        .into(),
        face_origin_local_mm: [face_x, 0.0, 0.0],
        inward_unit_local: [inward_x, 0.0, 0.0],
        bounds_min_local_mm: [0.0, 0.0, 0.0],
        bounds_max_local_mm: [100.0, 50.0, 18.0],
    };
    let joint = plan(
        &document,
        &BTreeSet::new(),
        &ExactResultRegistry::default(),
        &program(vec![AssistantCadEditOperation::CreatePinJoint {
            joint_id: None,
            name: "End dowels".into(),
            first: face(1, 100.0, -1.0),
            second: face(2, 0.0, 1.0),
            first_center_local_mm: [100.0, 10.0, 9.0],
            row_unit_first_local: [0.0, 1.0, 0.0],
            count: PINS as u32,
            spacing_mm: 15.0,
            pin: AssistantPin {
                diameter_mm: 8.0,
                length_mm: 30.0,
                hole_clearance_mm: 1.0,
            },
            holes,
        }]),
    )
    .unwrap();
    document.apply_batch(&joint).unwrap();
    let dimension = ClassificationDimensionId(900);
    let category = ClassificationCategoryId(901);
    let mut commands = vec![CanonicalCommand::UpsertClassificationDimension {
        id: dimension,
        name: FABRICATION_ROLE_DIMENSION_V1.into(),
        categories: vec![(category, TIMBER_MEMBER_ROLE_V1.into())],
    }];
    for (occurrence, code) in [(1, "000000000061"), (2, "000000000062")] {
        commands.push(CanonicalCommand::SetOccurrenceClassification {
            occurrence_id: OccurrenceId(occurrence),
            dimension_id: dimension,
            category_id: Some(category),
        });
        commands.push(CanonicalCommand::SetProductionCode {
            instance_path: InstancePath::root(OccurrenceId(occurrence)),
            code: Some(code.into()),
        });
    }
    document.apply_batch(&CommandBatch::new(commands)).unwrap();
    document.current()
}

/// The fabrication of `snapshot` on its exact bodies and collision check.
fn fabrication(snapshot: &Snapshot) -> GeneralFabricationProjection {
    let mut worker = crate::operations_support::worker();
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
            Arc::new(ExactBodyPackage::from(
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
        Duration::from_secs(300),
    )
    .unwrap();
    project_general_fabrication(
        snapshot,
        &registry,
        &validation.cases,
        &validation.report,
        tolerance,
        &shown_parts(snapshot),
    )
    .unwrap()
}

/// The dowel holes each timber's woodWOP program and production job part drill.
fn woodwop_and_job_holes(
    snapshot: &Snapshot,
    projection: &GeneralFabricationProjection,
) -> (Vec<usize>, Vec<usize>) {
    let woodwop = projection
        .woodwop_mpr_4_0_setup_a_programs(snapshot, Default::default())
        .unwrap()
        .into_iter()
        .map(|(_, mpr)| {
            String::from_utf8(mpr)
                .unwrap()
                .matches("\\BohrHoriz\\")
                .count()
        })
        .collect();
    let job = projection.production_job(snapshot, &[]).unwrap();
    let job = job["parts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|part| {
            part["operations"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|operation| operation["kind"] == "circular-drill")
                .count()
                + part["pin_holes"].as_array().unwrap().len()
        })
        .collect();
    (woodwop, job)
}

#[test]
fn drilled_dowel_holes_are_machined_once_in_every_shop_format() {
    let _turn = crate::integration_support::file_turn();
    let snapshot = doweled_timbers(AssistantPinHoles::Drill {
        first_insertion_mm: None,
    });
    let projection = fabrication(&snapshot);
    let (woodwop, job) = woodwop_and_job_holes(&snapshot, &projection);
    assert_eq!(woodwop, [PINS, PINS], "one woodWOP drilling per hole");
    assert_eq!(job, [PINS, PINS], "one job drilling per hole");
    let btlx = String::from_utf8(projection.btlx_2_3_1_export(&snapshot).unwrap()).unwrap();
    assert_eq!(btlx.matches("<Drilling ").count(), 2 * PINS, "{btlx}");
    let drawing = String::from_utf8(projection.drawing_svg(&snapshot).unwrap()).unwrap();
    assert_eq!(
        drawing.matches(">circular-drill: center=").count(),
        2 * PINS,
        "{drawing}"
    );
}

#[test]
fn logical_dowel_holes_are_drilled_by_the_machine_formats_and_refused_by_name_by_the_rest() {
    let _turn = crate::integration_support::file_turn();
    let snapshot = doweled_timbers(AssistantPinHoles::Logical);
    let projection = fabrication(&snapshot);
    let (woodwop, job) = woodwop_and_job_holes(&snapshot, &projection);
    assert_eq!(woodwop, [PINS, PINS]);
    assert_eq!(job, [PINS, PINS]);
    let element = ShopElement::PinHole("End dowels".into());
    for format in [ShopFormat::Btlx, ShopFormat::Drawing] {
        let Carriage::Refuses(reason) = carriage(format, &element) else {
            panic!("{format} carries {element}")
        };
        let refused = Err(GeneralFabricationError::CannotCarry {
            format,
            part: "Left timber".into(),
            element: element.clone(),
            reason,
        });
        let exported = match format {
            ShopFormat::Btlx => projection.btlx_2_3_1_export(&snapshot),
            _ => projection.drawing_svg(&snapshot),
        };
        assert_eq!(exported, refused);
    }
}
