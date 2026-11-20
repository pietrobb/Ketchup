use super::*;
use ketchup_model::cam::{
    CamContourSide, CamHole, CamMotion, CamMotionPath, CamPlannerError, CamRegion2d,
};

const DEFINITION: DefinitionId = DefinitionId(96);
const PLATE_PROFILE: FeatureId = FeatureId(960);
const PLATE: FeatureId = FeatureId(961);
const WORKPLANE: FeatureId = FeatureId(962);
const SKETCH: FeatureId = FeatureId(963);
const POCKET: FeatureId = FeatureId(964);
const TOOL_RADIUS: f64 = 1.0;
const ISLAND_CENTER: [f64; 2] = [20.0, 15.0];
const ISLAND_RADIUS: f64 = 4.0;

/// A 40 x 30 x 10 plate with a 30 x 20 pocket 4 deep that leaves a round island
/// of radius 4 standing in its middle. Nothing in it is named for CAM.
fn plate_with_island_pocket() -> (DocumentStore, CamPlan) {
    plate_with_island_pocket_scaled(1.0)
}

/// The island-pocket plate with every length in the XY plane multiplied by
/// `scale`; depth, tool and cut parameters stay the same.
fn plate_with_island_pocket_scaled(scale: f64) -> (DocumentStore, CamPlan) {
    let corners =
        [[5.0, 5.0], [35.0, 5.0], [35.0, 25.0], [5.0, 25.0]].map(|[x, y]| [x * scale, y * scale]);
    let mut entities: Vec<SketchEntity> = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect();
    entities.push(SketchEntity::Circle {
        id: SketchEntityId(5),
        center_mm: ISLAND_CENTER.map(|value| value * scale),
        radius_mm: ISLAND_RADIUS * scale,
    });
    let sketch = SketchSpec {
        workplane: WORKPLANE,
        entities,
        constraints: Vec::new(),
    };
    let ring = sketch
        .solved_regions()
        .unwrap()
        .into_iter()
        .find(|region| !region.holes.is_empty())
        .unwrap()
        .id;
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Plate with an island pocket".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE_PROFILE,
                definition_id: DEFINITION,
                name: "Plate outline".into(),
                kind: FeatureKind::polygon(&[
                    [0.0, 0.0],
                    [40.0 * scale, 0.0],
                    [40.0 * scale, 30.0 * scale],
                    [0.0, 30.0 * scale],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE,
                definition_id: DEFINITION,
                name: "Plate".into(),
                kind: FeatureKind::extrusion(PLATE_PROFILE, dimension(10.0)),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Pocket outline around the island".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            CanonicalCommand::CreateFeature {
                id: POCKET,
                definition_id: DEFINITION,
                name: "Pocket".into(),
                kind: FeatureKind::Pad(PadSpec {
                    profile: PadProfile::SketchRegion {
                        sketch: SKETCH,
                        region: ring,
                    },
                    direction: FeatureDirection::AlongNormal,
                    extent: FeatureExtent::Blind(dimension(4.0)),
                    operation: PadOperation::Cut {
                        target: PLATE,
                        start: CutStart::TargetFace,
                    },
                }),
            },
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &document.current(),
        CamPlanId(96),
        "Island pocket",
        CamStock {
            minimum_mm: [-5.0, -5.0, 0.0],
            maximum_mm: [40.0 * scale + 5.0, 30.0 * scale + 5.0, 10.0],
        },
        CamTool {
            number: 2,
            kind: CamToolKind::FlatEndMill,
            diameter_mm: 2.0 * TOOL_RADIUS,
            flute_length_mm: 6.0,
            overall_length_mm: 20.0,
            holder_diameter_mm: 6.0,
            holder_length_mm: 20.0,
            spindle_rpm: 12_000,
            feed_mm_per_min: 600.0,
            plunge_mm_per_min: 150.0,
            point_angle_deg: None,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 15.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        DEFINITION,
        POCKET,
    )
    .unwrap();
    (document, plan)
}

fn planar_face_at(package: &ExactBRepGraphPackage, z_mm: f64, normal_z: f64) -> u32 {
    let faces: Vec<_> = package
        .face_evidence
        .iter()
        .filter(|face| {
            face.surface_kind == "plane"
                && (face.unit_normal[2] - normal_z).abs() <= 1.0e-9
                && (face.centroid_mm[2] - z_mm).abs() <= 1.0e-6
        })
        .collect();
    assert_eq!(faces.len(), 1, "expected one planar face at z = {z_mm}");
    faces[0].face_ordinal
}

/// Points along a cutting motion, arcs walked by angle, so a check sees the
/// whole swept centre line and not only its ends.
fn cut_samples(motion: &CamMotion) -> Vec<[f64; 3]> {
    match &motion.path {
        CamMotionPath::Line { start_mm, end_mm } => (0..=16)
            .map(|step| {
                let t = f64::from(step) / 16.0;
                [0, 1, 2].map(|axis| start_mm[axis] + (end_mm[axis] - start_mm[axis]) * t)
            })
            .collect(),
        CamMotionPath::Arc {
            start_mm,
            end_mm,
            center_mm,
            clockwise,
        } => {
            let radius = (start_mm[0] - center_mm[0]).hypot(start_mm[1] - center_mm[1]);
            let start = (start_mm[1] - center_mm[1]).atan2(start_mm[0] - center_mm[0]);
            let end = (end_mm[1] - center_mm[1]).atan2(end_mm[0] - center_mm[0]);
            let mut sweep = end - start;
            if *clockwise {
                while sweep >= 0.0 {
                    sweep -= std::f64::consts::TAU;
                }
            } else {
                while sweep <= 0.0 {
                    sweep += std::f64::consts::TAU;
                }
            }
            (0..=32)
                .map(|step| {
                    let angle = start + sweep * f64::from(step) / 32.0;
                    [
                        center_mm[0] + radius * angle.cos(),
                        center_mm[1] + radius * angle.sin(),
                        start_mm[2],
                    ]
                })
                .collect()
        }
    }
}

fn cut_points(toolpath: &CamToolpath) -> Vec<[f64; 3]> {
    toolpath
        .motions
        .iter()
        .filter(|motion| motion.kind == CamMotionKind::Cut)
        .flat_map(cut_samples)
        .collect()
}

fn segment_points(path: &CamPath2d) -> Vec<[f64; 2]> {
    std::iter::once(path.start_mm)
        .chain(path.segments.iter().map(|segment| match segment {
            CamPathSegment2d::Line { to_mm } | CamPathSegment2d::Arc { to_mm, .. } => *to_mm,
        }))
        .collect()
}

#[test]
fn a_pocket_from_an_exact_floor_face_clears_around_its_island_without_gouging() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan) = plate_with_island_pocket();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();

    // The pocket floor at z = 6: its outer loop is the pocket wall, its inner
    // loop the foot of the island, both read from the exact face.
    let floor = planar_face_at(&package, 6.0, 1.0);
    let region = CamRegion2d::from_planar_face(&package, floor, plan.setup()).unwrap();
    assert_eq!(region.outer.segments.len(), 4);
    assert!(
        region
            .outer
            .segments
            .iter()
            .all(|segment| matches!(segment, CamPathSegment2d::Line { .. }))
    );
    for corner in [[5.0, 5.0], [35.0, 5.0], [35.0, 25.0], [5.0, 25.0]] {
        assert!(
            segment_points(&region.outer)
                .iter()
                .any(|point| distance(*point, corner) <= 1.0e-6),
            "pocket corner {corner:?} missing from {region:?}"
        );
    }
    assert_eq!(region.islands.len(), 1);
    let island = &region.islands[0];
    assert!(island.segments.iter().all(|segment| matches!(
        segment,
        CamPathSegment2d::Arc { center_mm, .. }
            if distance(*center_mm, ISLAND_CENTER) <= 1.0e-6
    )));
    assert!((distance(island.start_mm, ISLAND_CENTER) - ISLAND_RADIUS).abs() <= 1.0e-6);

    // A face that is not planar is refused, not approximated.
    let island_wall = package
        .face_evidence
        .iter()
        .find(|face| face.surface_kind == "cylinder")
        .unwrap()
        .face_ordinal;
    assert!(CamRegion2d::from_planar_face(&package, island_wall, plan.setup()).is_err());

    let operations = [CamOperation::RegionPocket {
        id: 1,
        region,
        top_z_mm: 10.0,
        bottom_z_mm: 6.0,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &plan, &operations).unwrap();
    assert_eq!(toolpath.operations.len(), 1);
    let points = cut_points(&toolpath);
    assert!(!points.is_empty());
    for point in &points {
        // The cutter centre never comes closer than its radius to a pocket wall
        // or to the island, and never cuts below the floor.
        assert!(point[0] >= 5.0 + TOOL_RADIUS - 1.0e-6, "{point:?}");
        assert!(point[0] <= 35.0 - TOOL_RADIUS + 1.0e-6, "{point:?}");
        assert!(point[1] >= 5.0 + TOOL_RADIUS - 1.0e-6, "{point:?}");
        assert!(point[1] <= 25.0 - TOOL_RADIUS + 1.0e-6, "{point:?}");
        assert!(
            distance([point[0], point[1]], ISLAND_CENTER) >= ISLAND_RADIUS + TOOL_RADIUS - 1.0e-6,
            "{point:?}"
        );
        assert!(point[2] >= 6.0 - 1.0e-9, "{point:?}");
    }
    let floor_points: Vec<_> = points
        .iter()
        .filter(|point| (point[2] - 6.0).abs() <= 1.0e-9)
        .collect();
    // On the floor the cutter runs along every wall and all the way around the island.
    for (axis, wall) in [(0, 6.0), (0, 34.0), (1, 6.0), (1, 24.0)] {
        assert!(
            floor_points
                .iter()
                .any(|point| (point[axis] - wall).abs() <= 1.0e-6),
            "the cutter never reaches the wall at {wall} on axis {axis}"
        );
    }
    for quadrant in 0..4 {
        let angle = std::f64::consts::FRAC_PI_2 * f64::from(quadrant) + 0.3;
        let probe = [
            ISLAND_CENTER[0] + (ISLAND_RADIUS + TOOL_RADIUS) * angle.cos(),
            ISLAND_CENTER[1] + (ISLAND_RADIUS + TOOL_RADIUS) * angle.sin(),
        ];
        assert!(
            floor_points
                .iter()
                .any(|point| distance([point[0], point[1]], probe) <= 0.2),
            "the cutter never passes the island near {probe:?}"
        );
    }

    let cancelled = AtomicBool::new(false);
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    let pocket_mm3 = 30.0 * 20.0 * 4.0 - std::f64::consts::PI * ISLAND_RADIUS.powi(2) * 4.0;
    // A round cutter cannot reach into the four square pocket corners.
    let corners_mm3 = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4) * TOOL_RADIUS.powi(2) * 4.0;
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.collisions.is_empty(), "{simulation:?}");
    assert!(
        (simulation.removed_stock_mm3 - (pocket_mm3 - corners_mm3)).abs() <= 1.0e-3,
        "{simulation:?}"
    );
}

#[test]
fn an_outside_contour_from_an_exact_face_runs_one_tool_radius_off_the_part() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan) = plate_with_island_pocket();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();

    // The bottom face carries the plate outline as its only loop.
    let bottom = planar_face_at(&package, 0.0, -1.0);
    let region = CamRegion2d::from_planar_face(&package, bottom, plan.setup()).unwrap();
    assert!(region.islands.is_empty());

    let operations = [CamOperation::RegionContour {
        id: 1,
        region,
        side: CamContourSide::Outside,
        top_z_mm: 10.0,
        bottom_z_mm: 6.0,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &plan, &operations).unwrap();
    let points = cut_points(&toolpath);
    assert!(!points.is_empty());
    for point in &points {
        // Distance from the cutter centre to the 40 x 30 outline is the tool radius.
        let dx = (-point[0]).max(point[0] - 40.0).max(0.0);
        let dy = (-point[1]).max(point[1] - 30.0).max(0.0);
        assert!(
            (dx.hypot(dy) - TOOL_RADIUS).abs() <= 1.0e-6,
            "cutter centre {point:?} is not one radius off the outline"
        );
    }
    // The cutter rolls around each outside corner on an arc of the tool radius.
    for corner in [[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]] {
        assert!(toolpath.motions.iter().any(|motion| matches!(
            motion.path,
            CamMotionPath::Arc { center_mm, .. }
                if distance([center_mm[0], center_mm[1]], corner) <= 1.0e-6
        )));
    }

    let cancelled = AtomicBool::new(false);
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.collisions.is_empty(), "{simulation:?}");
    // A band one cutter wide around the 140 mm outline, rounded at the corners, 4 deep.
    let band_mm3 =
        (140.0 * 2.0 * TOOL_RADIUS + std::f64::consts::PI * (2.0 * TOOL_RADIUS).powi(2)) * 4.0;
    assert!(
        (simulation.removed_stock_mm3 - band_mm3).abs() <= 1.0e-3,
        "{simulation:?}"
    );
}

const DRILL_RADIUS: f64 = 3.0;
const THROUGH_HOLE: [f64; 2] = [10.0, 15.0];
const BLIND_HOLE: [f64; 2] = [30.0, 15.0];
const WIDE_HOLE: [f64; 2] = [20.0, 22.0];

/// A 40 x 30 x 10 plate with a 6 mm hole through it, a 6 mm hole 4 deep and a
/// 4 mm-radius hole 4 deep. Nothing in it is named for CAM.
fn plate_with_holes() -> (DocumentStore, CamPlan, FeatureId) {
    let sketch = SketchSpec {
        workplane: WORKPLANE,
        entities: vec![
            SketchEntity::Circle {
                id: SketchEntityId(1),
                center_mm: THROUGH_HOLE,
                radius_mm: DRILL_RADIUS,
            },
            SketchEntity::Circle {
                id: SketchEntityId(2),
                center_mm: BLIND_HOLE,
                radius_mm: DRILL_RADIUS,
            },
            SketchEntity::Circle {
                id: SketchEntityId(3),
                center_mm: WIDE_HOLE,
                radius_mm: 4.0,
            },
        ],
        constraints: Vec::new(),
    };
    let regions = sketch.solved_regions().unwrap();
    let region_of = |circle: u64| {
        regions
            .iter()
            .find(|region| region.outer_entity_ids == [SketchEntityId(circle)])
            .unwrap()
            .id
    };
    // A through hole runs from the sketch plane through all; a blind one
    // starts at the target's face.
    let cut = |id: u64, name: &str, circle: u64, depth: Option<f64>, target: FeatureId| {
        let (extent, start) = match depth {
            Some(depth) => (FeatureExtent::Blind(dimension(depth)), CutStart::TargetFace),
            None => (FeatureExtent::ThroughAll, CutStart::ProfilePlane),
        };
        CanonicalCommand::CreateFeature {
            id: FeatureId(id),
            definition_id: DEFINITION,
            name: name.into(),
            kind: FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion {
                    sketch: SKETCH,
                    region: region_of(circle),
                },
                direction: FeatureDirection::AlongNormal,
                extent,
                operation: PadOperation::Cut { target, start },
            }),
        }
    };
    let last = FeatureId(967);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Plate with holes".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE_PROFILE,
                definition_id: DEFINITION,
                name: "Plate outline".into(),
                kind: FeatureKind::polygon(&[[0.0, 0.0], [40.0, 0.0], [40.0, 30.0], [0.0, 30.0]]),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE,
                definition_id: DEFINITION,
                name: "Plate".into(),
                kind: FeatureKind::extrusion(PLATE_PROFILE, dimension(10.0)),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: SKETCH,
                definition_id: DEFINITION,
                name: "Hole centres".into(),
                kind: FeatureKind::Sketch(sketch),
            },
            cut(965, "Through hole", 1, None, PLATE),
            cut(966, "Blind hole", 2, Some(4.0), FeatureId(965)),
            cut(967, "Wide hole", 3, Some(4.0), FeatureId(966)),
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &document.current(),
        CamPlanId(97),
        "Drill the plate",
        CamStock {
            minimum_mm: [0.0, 0.0, 0.0],
            maximum_mm: [40.0, 30.0, 10.0],
        },
        CamTool {
            number: 3,
            kind: CamToolKind::Drill,
            diameter_mm: 2.0 * DRILL_RADIUS,
            flute_length_mm: 14.0,
            overall_length_mm: 30.0,
            holder_diameter_mm: 10.0,
            holder_length_mm: 20.0,
            spindle_rpm: 3_000,
            feed_mm_per_min: 300.0,
            plunge_mm_per_min: 100.0,
            point_angle_deg: None,
        },
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 15.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        DEFINITION,
        last,
    )
    .unwrap();
    (document, plan, last)
}

/// `plan` with a drill whose point is `point_angle_deg`.
fn plan_with_drill_point(
    document: &DocumentStore,
    plan: &CamPlan,
    point_angle_deg: Option<f64>,
) -> CamPlan {
    let mut tool = plan.tool().clone();
    tool.point_angle_deg = point_angle_deg;
    CamPlan::new(
        &document.current(),
        CamPlanId(98),
        "Drill with another point",
        plan.stock().clone(),
        tool,
        plan.setup().clone(),
        plan.cut_parameters().clone(),
        plan.target().definition_id,
        plan.target().feature_id,
    )
    .unwrap()
}

const POINTED_RIM_Z: f64 = 4.0;

/// A 40 x 30 x 10 plate centred on the origin with a 6 mm hole drilled into its
/// top to z = 4 and ending there in the cone of a 118° drill point, as a
/// revolved drill body cut out of the plate.
fn plate_with_pointed_hole() -> (DocumentStore, CamPlan, FeatureId) {
    let apex = POINTED_RIM_Z - DRILL_RADIUS / 59.0_f64.to_radians().tan();
    let section = [
        [0.0, 12.0],
        [DRILL_RADIUS, 12.0],
        [DRILL_RADIUS, POINTED_RIM_Z],
        [0.0, apex],
    ];
    let cut = FeatureId(975);
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Plate with a pointed hole".into(),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE_PROFILE,
                definition_id: DEFINITION,
                name: "Plate outline".into(),
                kind: FeatureKind::polygon(&[
                    [-20.0, -15.0],
                    [20.0, -15.0],
                    [20.0, 15.0],
                    [-20.0, 15.0],
                ]),
            },
            CanonicalCommand::CreateFeature {
                id: PLATE,
                definition_id: DEFINITION,
                name: "Plate".into(),
                kind: FeatureKind::extrusion(PLATE_PROFILE, dimension(10.0)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(972),
                definition_id: DEFINITION,
                name: "YZ".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Yz)),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(973),
                definition_id: DEFINITION,
                name: "Drill section".into(),
                kind: FeatureKind::Sketch(SketchSpec {
                    workplane: FeatureId(972),
                    entities: (0..4)
                        .map(|index| SketchEntity::Line {
                            id: SketchEntityId(index as u64 + 1),
                            start_mm: section[index],
                            end_mm: section[(index + 1) % 4],
                        })
                        .collect(),
                    constraints: Vec::new(),
                }),
            },
            CanonicalCommand::CreateFeature {
                id: FeatureId(974),
                definition_id: DEFINITION,
                name: "Drill body".into(),
                kind: FeatureKind::Revolve {
                    profile: FeatureId(973),
                    axis_start_mm: [0.0, 0.0],
                    axis_end_mm: [0.0, 1.0],
                    angle_degrees: 360.0,
                },
            },
            CanonicalCommand::CreateFeature {
                id: cut,
                definition_id: DEFINITION,
                name: "Pointed hole".into(),
                kind: FeatureKind::Boolean {
                    operation: BooleanOperation::Cut,
                    target: PLATE,
                    tool: FeatureId(974),
                },
            },
        ]))
        .unwrap();
    let (_, holes_plan, _) = plate_with_holes();
    let plan = CamPlan::new(
        &document.current(),
        CamPlanId(97),
        "Drill the pointed hole",
        CamStock {
            minimum_mm: [-20.0, -15.0, 0.0],
            maximum_mm: [20.0, 15.0, 10.0],
        },
        holes_plan.tool().clone(),
        holes_plan.setup().clone(),
        holes_plan.cut_parameters().clone(),
        DEFINITION,
        cut,
    )
    .unwrap();
    (document, plan, cut)
}

/// R1120ac-CAM-901: a blind hole ending in a drill point is drilled until the
/// full diameter reaches the far rim of its wall, so the drill's point makes the
/// modelled cone; a drill with another point is refused, not run to the rim.
#[test]
fn a_pointed_blind_hole_is_drilled_to_full_diameter_by_a_drill_of_its_point() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan, last) = plate_with_pointed_hole();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, last).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let wall = package
        .face_evidence
        .iter()
        .find(|face| face.surface_kind == "cylinder")
        .unwrap()
        .face_ordinal;
    let hole = CamHole::from_cylindrical_face(&package, wall, plan.setup()).unwrap();
    assert!(distance(hole.center_mm, [0.0, 0.0]) <= 1.0e-6, "{hole:?}");
    assert!((hole.top_z_mm - 10.0).abs() <= 1.0e-6, "{hole:?}");
    assert!(
        (hole.bottom_z_mm - POINTED_RIM_Z).abs() <= 1.0e-6,
        "{hole:?}"
    );
    assert!(!hole.through);
    let angle = hole.floor_point_angle_deg.unwrap();
    assert!((angle - 118.0).abs() <= 1.0e-6, "{angle}");

    let drill = CamOperation::drill_hole(1, &hole, plan.tool()).unwrap();
    let point = plan.tool().tip_length_mm();
    let CamOperation::Drill { bottom_z_mm, .. } = drill else {
        panic!("{drill:?}");
    };
    assert!((bottom_z_mm - (POINTED_RIM_Z - point)).abs() <= 1.0e-6);
    let toolpath = CamToolpath::plan(&snapshot, &plan, &[drill]).unwrap();
    let cancelled = AtomicBool::new(false);
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    let area = std::f64::consts::PI * DRILL_RADIUS.powi(2);
    let hole_mm3 = area * (10.0 - POINTED_RIM_Z + point / 3.0);
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.residual_stock_mm3 <= 1.0e-3, "{simulation:?}");
    assert!(
        (simulation.removed_stock_mm3 - hole_mm3).abs() <= 1.0e-3,
        "{simulation:?}"
    );

    for other in [Some(140.0), Some(180.0), Some(90.0)] {
        let other_plan = plan_with_drill_point(&document, &plan, other);
        assert_eq!(
            CamOperation::drill_hole(1, &hole, other_plan.tool()),
            Err(CamPlannerError::DrillPointDoesNotFitFloor),
            "{other:?}"
        );
    }
}

#[test]
fn drilling_reads_each_hole_from_its_exact_cylinder_and_refuses_what_the_drill_cannot_make() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan, last) = plate_with_holes();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, last).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();

    let holes: Vec<CamHole> = package
        .face_evidence
        .iter()
        .filter(|face| face.surface_kind == "cylinder")
        .map(|face| {
            CamHole::from_cylindrical_face(&package, face.face_ordinal, plan.setup()).unwrap()
        })
        .collect();
    assert_eq!(holes.len(), 3);
    let hole_at = |center: [f64; 2]| {
        holes
            .iter()
            .find(|hole| distance(hole.center_mm, center) <= 1.0e-6)
            .unwrap()
    };
    let through = hole_at(THROUGH_HOLE);
    assert!((through.radius_mm - DRILL_RADIUS).abs() <= 1.0e-6);
    assert!((through.top_z_mm - 10.0).abs() <= 1.0e-6);
    assert!(through.bottom_z_mm.abs() <= 1.0e-6);
    assert!(through.through);
    let blind = hole_at(BLIND_HOLE);
    assert!((blind.top_z_mm - 10.0).abs() <= 1.0e-6);
    assert!((blind.bottom_z_mm - 6.0).abs() <= 1.0e-6);
    assert!(!blind.through);
    let wide = hole_at(WIDE_HOLE);
    assert!((wide.radius_mm - 4.0).abs() <= 1.0e-6);

    assert_eq!(through.floor_point_angle_deg, None);
    assert_eq!(blind.floor_point_angle_deg, Some(180.0));

    // A drill makes only a hole of its own diameter.
    assert!(CamOperation::drill_hole(3, wide, plan.tool()).is_err());
    // R1120ac-CAM-901: a pointed drill cannot make a flat floor; stopped at it, it
    // leaves its cone, and driven on until the full diameter reaches the floor, it
    // cuts below it. A flat-bottom drill makes it.
    assert_eq!(
        CamOperation::drill_hole(2, blind, plan.tool()),
        Err(CamPlannerError::DrillPointDoesNotFitFloor)
    );
    let flat = plan_with_drill_point(&document, &plan, Some(180.0));
    let operations = [
        CamOperation::drill_hole(1, through, flat.tool()).unwrap(),
        CamOperation::drill_hole(2, blind, flat.tool()).unwrap(),
    ];
    let toolpath = CamToolpath::plan(&snapshot, &flat, &operations).unwrap();
    let cancelled = AtomicBool::new(false);
    let simulation = worker
        .simulate_cam(&snapshot, &flat, &toolpath, &[], &cancelled)
        .unwrap();
    let area = std::f64::consts::PI * DRILL_RADIUS.powi(2);
    let wide_mm3 = std::f64::consts::PI * 16.0 * 4.0;
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.collisions.is_empty(), "{simulation:?}");
    assert!(
        (simulation.removed_stock_mm3 - area * (10.0 + 4.0)).abs() <= 1.0e-3,
        "{simulation:?}"
    );
    assert!(
        (simulation.residual_stock_mm3 - wide_mm3).abs() <= 1.0e-3,
        "{simulation:?}"
    );

    // R1119og-MF-800: the standard 118° point is simulated as it is. It passes the
    // through hole's exit with the full diameter, so that hole is made whole.
    let point = plan.tool().tip_length_mm();
    assert!((point - DRILL_RADIUS / 59.0_f64.to_radians().tan()).abs() <= 1.0e-12);
    let toolpath = CamToolpath::plan(
        &snapshot,
        &plan,
        &[CamOperation::drill_hole(1, through, plan.tool()).unwrap()],
    )
    .unwrap();
    let deepest = toolpath
        .motions
        .iter()
        .map(|motion| match &motion.path {
            CamMotionPath::Line { end_mm, .. } | CamMotionPath::Arc { end_mm, .. } => end_mm[2],
        })
        .fold(f64::INFINITY, f64::min);
    assert!((deepest - (-point - 1.0)).abs() <= 1.0e-9, "{deepest}");
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(
        (simulation.removed_stock_mm3 - area * 10.0).abs() <= 1.0e-3,
        "{simulation:?}"
    );

    // The round island of the pocket part stands up out of its floor: it is a
    // boss, not a hole, and no drill makes it.
    let (pocket_document, pocket_plan) = plate_with_island_pocket();
    let pocket_graph =
        ExactBRepGraph::from_snapshot(&pocket_document.current(), DEFINITION, POCKET).unwrap();
    let pocket = worker.evaluate_exact_brep_graph(&pocket_graph).unwrap();
    let island_wall = pocket
        .face_evidence
        .iter()
        .find(|face| face.surface_kind == "cylinder")
        .unwrap()
        .face_ordinal;
    assert!(CamHole::from_cylindrical_face(&pocket, island_wall, pocket_plan.setup()).is_err());
}

#[test]
fn an_operation_naming_a_face_reads_it_from_the_evaluated_target_before_it_plans() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan) = plate_with_island_pocket();
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let floor = planar_face_at(&package, 6.0, 1.0);
    let stock_top = plan.stock().maximum_mm[2];

    // Unread, a named face cannot be planned; read, it is the floor's region
    // cleared from the stock top down to the floor.
    let named = CamOperation::FacePocket {
        id: 1,
        face_ordinal: floor,
    };
    assert_eq!(
        CamToolpath::plan(&snapshot, &plan, std::slice::from_ref(&named)),
        Err(CamPlannerError::UnresolvedFace)
    );
    let resolved = named.resolve_face(&package, &plan).unwrap();
    assert_eq!(
        resolved,
        CamOperation::RegionPocket {
            id: 1,
            region: CamRegion2d::from_planar_face(&package, floor, plan.setup()).unwrap(),
            top_z_mm: stock_top,
            bottom_z_mm: 6.0,
        }
    );
    assert!(CamToolpath::plan(&snapshot, &plan, &[resolved]).is_ok());

    // The plate's underside faces away from the tool: no pocket has it as a floor.
    let underside = planar_face_at(&package, 0.0, -1.0);
    let upside_down = CamOperation::FacePocket {
        id: 2,
        face_ordinal: underside,
    };
    assert_eq!(
        upside_down.resolve_face(&package, &plan),
        Err(CamPlannerError::UnsupportedRegion)
    );

    // A drill names a hole by its wall and gets the hole the wall bounds.
    let (holes_document, holes_plan, last) = plate_with_holes();
    let holes_graph =
        ExactBRepGraph::from_snapshot(&holes_document.current(), DEFINITION, last).unwrap();
    let holes = worker.evaluate_exact_brep_graph(&holes_graph).unwrap();
    let through_wall = holes
        .face_evidence
        .iter()
        .find(|face| {
            face.surface_kind == "cylinder"
                && CamHole::from_cylindrical_face(&holes, face.face_ordinal, holes_plan.setup())
                    .is_ok_and(|hole| distance(hole.center_mm, THROUGH_HOLE) <= 1.0e-6)
        })
        .unwrap()
        .face_ordinal;
    let drill = CamOperation::FaceDrill {
        id: 3,
        face_ordinal: through_wall,
    };
    let hole = CamHole::from_cylindrical_face(&holes, through_wall, holes_plan.setup()).unwrap();
    assert_eq!(
        drill.resolve_face(&holes, &holes_plan),
        CamOperation::drill_hole(3, &hole, holes_plan.tool())
    );

    // Faces are read only from the evaluated target of the plan itself.
    assert_eq!(
        drill.resolve_face(&holes, &plan),
        Err(CamPlannerError::StaleToolpath)
    );
}

const OUTLINE: [[f64; 2]; 2] = [[0.0, 0.0], [80.0, 50.0]];
const OUTLINE_RADIUS: f64 = 5.0;
const ROUND_POCKET: [[f64; 2]; 2] = [[10.0, 15.0], [40.0, 35.0]];
const ROUND_POCKET_RADIUS: f64 = 4.0;
const PLATE_HOLE: [f64; 2] = [60.0, 32.0];
const SLOT_ENDS: [[f64; 2]; 2] = [[52.0, 12.0], [68.0, 12.0]];
const HOLE_RADIUS: f64 = 3.3;

/// A rectangle with its corners rounded to `radius`, counter-clockwise.
fn rounded_rectangle(first_id: u64, [min, max]: [[f64; 2]; 2], radius: f64) -> Vec<SketchEntity> {
    let (r, mut entities) = (radius, Vec::new());
    let corners = [
        [min[0] + r, min[1] + r],
        [max[0] - r, min[1] + r],
        [max[0] - r, max[1] - r],
        [min[0] + r, max[1] - r],
    ];
    // Leaving corner `index` the side runs along this direction.
    let along = [[1.0, 0.0], [0.0, 1.0], [-1.0, 0.0], [0.0, -1.0]];
    for index in 0..4 {
        let center = corners[index];
        let next = corners[(index + 1) % 4];
        let [dx, dy] = along[index];
        // Outward normal of the side leaving this corner.
        let out = [dy, -dx];
        let side_start = [center[0] + out[0] * r, center[1] + out[1] * r];
        let side_end = [next[0] + out[0] * r, next[1] + out[1] * r];
        let [nx, ny] = along[(index + 1) % 4];
        let next_out = [ny, -nx];
        entities.push(SketchEntity::Line {
            id: SketchEntityId(first_id + 2 * index as u64),
            start_mm: side_start,
            end_mm: side_end,
        });
        entities.push(SketchEntity::Arc {
            id: SketchEntityId(first_id + 2 * index as u64 + 1),
            start_mm: side_end,
            end_mm: [next[0] + next_out[0] * r, next[1] + next_out[1] * r],
            center_mm: next,
            clockwise: false,
        });
    }
    entities
}

/// Signed distance from `point` to a rounded rectangle: negative inside.
fn rounded_rectangle_distance(point: [f64; 2], [min, max]: [[f64; 2]; 2], radius: f64) -> f64 {
    let center = [(min[0] + max[0]) * 0.5, (min[1] + max[1]) * 0.5];
    let q = [0, 1]
        .map(|axis| (point[axis] - center[axis]).abs() - ((max[axis] - min[axis]) * 0.5 - radius));
    q[0].max(0.0).hypot(q[1].max(0.0)) + q[0].max(q[1]).min(0.0) - radius
}

/// An 80 x 50 x 8 plate with R5 corners, a 30 x 20 pocket 3 deep with R4
/// corners, an obround belt slot and a 6.6 mm hole through it: the shapes of
/// a real machine plate. Nothing in it is named for CAM.
fn rounded_machine_plate(tool: CamTool) -> (DocumentStore, CamPlan, FeatureId) {
    let outline_sketch = FeatureId(970);
    let feature_sketch = FeatureId(971);
    let plate = FeatureId(972);
    let last = FeatureId(975);
    let outline = SketchSpec {
        workplane: WORKPLANE,
        entities: rounded_rectangle(1, OUTLINE, OUTLINE_RADIUS),
        constraints: Vec::new(),
    };
    let outline_region = outline.solved_regions().unwrap()[0].id;
    let mut entities = rounded_rectangle(1, ROUND_POCKET, ROUND_POCKET_RADIUS);
    let [left, right] = SLOT_ENDS;
    let r = HOLE_RADIUS;
    entities.extend([
        SketchEntity::Line {
            id: SketchEntityId(20),
            start_mm: [left[0], left[1] - r],
            end_mm: [right[0], right[1] - r],
        },
        SketchEntity::Arc {
            id: SketchEntityId(21),
            start_mm: [right[0], right[1] - r],
            end_mm: [right[0], right[1] + r],
            center_mm: right,
            clockwise: false,
        },
        SketchEntity::Line {
            id: SketchEntityId(22),
            start_mm: [right[0], right[1] + r],
            end_mm: [left[0], left[1] + r],
        },
        SketchEntity::Arc {
            id: SketchEntityId(23),
            start_mm: [left[0], left[1] + r],
            end_mm: [left[0], left[1] - r],
            center_mm: left,
            clockwise: false,
        },
        SketchEntity::Circle {
            id: SketchEntityId(30),
            center_mm: PLATE_HOLE,
            radius_mm: HOLE_RADIUS,
        },
    ]);
    let features = SketchSpec {
        workplane: WORKPLANE,
        entities,
        constraints: Vec::new(),
    };
    let regions = features.solved_regions().unwrap();
    let region_with = |entity: u64| {
        regions
            .iter()
            .find(|region| region.outer_entity_ids.contains(&SketchEntityId(entity)))
            .unwrap()
            .id
    };
    let pad =
        |id: u64, name: &str, sketch, region, extent, operation| CanonicalCommand::CreateFeature {
            id: FeatureId(id),
            definition_id: DEFINITION,
            name: name.into(),
            kind: FeatureKind::Pad(PadSpec {
                profile: PadProfile::SketchRegion { sketch, region },
                direction: FeatureDirection::AlongNormal,
                extent,
                operation,
            }),
        };
    let through = |target: FeatureId| PadOperation::Cut {
        target,
        start: CutStart::ProfilePlane,
    };
    let mut document = DocumentStore::new();
    document
        .apply_batch(&CommandBatch::new(vec![
            CanonicalCommand::CreateDefinition {
                id: DEFINITION,
                name: "Rounded machine plate".into(),
            },
            CanonicalCommand::CreateFeature {
                id: WORKPLANE,
                definition_id: DEFINITION,
                name: "XY".into(),
                kind: FeatureKind::Workplane(WorkplaneSpec::principal(PrincipalPlane::Xy)),
            },
            CanonicalCommand::CreateFeature {
                id: outline_sketch,
                definition_id: DEFINITION,
                name: "Outline".into(),
                kind: FeatureKind::Sketch(outline),
            },
            CanonicalCommand::CreateFeature {
                id: feature_sketch,
                definition_id: DEFINITION,
                name: "Pocket, slot and hole".into(),
                kind: FeatureKind::Sketch(features),
            },
            pad(
                972,
                "Plate",
                outline_sketch,
                outline_region,
                FeatureExtent::Blind(dimension(8.0)),
                PadOperation::NewBody,
            ),
            pad(
                973,
                "Pocket",
                feature_sketch,
                region_with(1),
                FeatureExtent::Blind(dimension(3.0)),
                PadOperation::Cut {
                    target: plate,
                    start: CutStart::TargetFace,
                },
            ),
            pad(
                974,
                "Belt slot",
                feature_sketch,
                region_with(20),
                FeatureExtent::ThroughAll,
                through(FeatureId(973)),
            ),
            pad(
                975,
                "Bolt hole",
                feature_sketch,
                region_with(30),
                FeatureExtent::ThroughAll,
                through(FeatureId(974)),
            ),
        ]))
        .unwrap();
    let plan = CamPlan::new(
        &document.current(),
        CamPlanId(98),
        "Rounded plate",
        CamStock {
            minimum_mm: [-5.0, -5.0, 0.0],
            maximum_mm: [85.0, 55.0, 8.0],
        },
        tool,
        CamSetup {
            work_offset: CamWorkOffset::G54,
            origin_mm: [0.0, 0.0, 0.0],
            x_axis: [1.0, 0.0, 0.0],
            y_axis: [0.0, 1.0, 0.0],
            safe_height_mm: 15.0,
        },
        CamCutParameters {
            maximum_stepdown_mm: 2.0,
            stepover_ratio: 0.5,
            radial_allowance_mm: 0.0,
            axial_allowance_mm: 0.0,
        },
        DEFINITION,
        last,
    )
    .unwrap();
    (document, plan, last)
}

fn end_mill() -> CamTool {
    CamTool {
        number: 2,
        kind: CamToolKind::FlatEndMill,
        diameter_mm: 2.0 * TOOL_RADIUS,
        flute_length_mm: 10.0,
        overall_length_mm: 25.0,
        holder_diameter_mm: 6.0,
        holder_length_mm: 20.0,
        spindle_rpm: 12_000,
        feed_mm_per_min: 600.0,
        plunge_mm_per_min: 150.0,
        point_angle_deg: None,
    }
}

/// Arcs of `radius` in `path` centred on each of `centers`.
fn has_arcs_at(path: &CamPath2d, centers: &[[f64; 2]], radius: f64) -> bool {
    let points = segment_points(path);
    centers.iter().all(|center| {
        path.segments.iter().enumerate().any(|(index, segment)| {
            matches!(segment, CamPathSegment2d::Arc { center_mm, .. }
                if distance(*center_mm, *center) <= 1.0e-6
                    && (distance(points[index + 1], *center) - radius).abs() <= 1.0e-6)
        })
    })
}

fn rounded_corner_centers([min, max]: [[f64; 2]; 2], radius: f64) -> [[f64; 2]; 4] {
    [
        [min[0] + radius, min[1] + radius],
        [max[0] - radius, min[1] + radius],
        [max[0] - radius, max[1] - radius],
        [min[0] + radius, max[1] - radius],
    ]
}

#[test]
fn faces_bounded_by_partial_arcs_give_contour_pocket_and_hole_of_a_rounded_machine_plate() {
    let _turn = crate::integration_support::file_turn();
    let (document, plan, last) = rounded_machine_plate(end_mill());
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, last).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let cancelled = AtomicBool::new(false);

    // The underside: the R5 outline, with the slot and the hole as islands.
    let bottom = planar_face_at(&package, 0.0, -1.0);
    let outline = CamRegion2d::from_planar_face(&package, bottom, plan.setup()).unwrap();
    assert!(has_arcs_at(
        &outline.outer,
        &rounded_corner_centers(OUTLINE, OUTLINE_RADIUS),
        OUTLINE_RADIUS
    ));
    assert_eq!(outline.islands.len(), 2, "{outline:?}");
    assert!(
        outline
            .islands
            .iter()
            .any(|island| has_arcs_at(island, &SLOT_ENDS, HOLE_RADIUS)),
        "{outline:?}"
    );

    let contour = [CamOperation::RegionContour {
        id: 1,
        region: outline,
        side: CamContourSide::Outside,
        top_z_mm: 8.0,
        bottom_z_mm: 4.0,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &plan, &contour).unwrap();
    let points = cut_points(&toolpath);
    assert!(!points.is_empty());
    for point in &points {
        let off = rounded_rectangle_distance([point[0], point[1]], OUTLINE, OUTLINE_RADIUS);
        assert!(
            (off - TOOL_RADIUS).abs() <= 1.0e-6,
            "cutter centre {point:?} is {off} off the rounded outline"
        );
    }
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.collisions.is_empty(), "{simulation:?}");
    let perimeter =
        2.0 * (80.0 + 50.0 - 4.0 * OUTLINE_RADIUS) + std::f64::consts::TAU * OUTLINE_RADIUS;
    let band_mm3 =
        (perimeter * 2.0 * TOOL_RADIUS + std::f64::consts::PI * (2.0 * TOOL_RADIUS).powi(2)) * 4.0;
    assert!(
        (simulation.removed_stock_mm3 - band_mm3).abs() <= 1.0e-3,
        "{simulation:?}"
    );

    // The pocket floor at z = 5: four R4 corners the cutter clears completely.
    let floor = planar_face_at(&package, 5.0, 1.0);
    let pocket = CamRegion2d::from_planar_face(&package, floor, plan.setup()).unwrap();
    assert!(pocket.islands.is_empty());
    assert!(has_arcs_at(
        &pocket.outer,
        &rounded_corner_centers(ROUND_POCKET, ROUND_POCKET_RADIUS),
        ROUND_POCKET_RADIUS
    ));
    let clear = [CamOperation::RegionPocket {
        id: 2,
        region: pocket,
        top_z_mm: 8.0,
        bottom_z_mm: 5.0,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &plan, &clear).unwrap();
    for point in cut_points(&toolpath) {
        let inside =
            rounded_rectangle_distance([point[0], point[1]], ROUND_POCKET, ROUND_POCKET_RADIUS);
        assert!(
            inside <= -TOOL_RADIUS + 1.0e-6,
            "{point:?} is {inside} inside"
        );
        assert!(point[2] >= 5.0 - 1.0e-9, "{point:?}");
    }
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    let pocket_area = 30.0 * 20.0 - (4.0 - std::f64::consts::PI) * ROUND_POCKET_RADIUS.powi(2);
    assert!(
        (simulation.removed_stock_mm3 - pocket_area * 3.0).abs() <= 1.0e-3,
        "{simulation:?}"
    );

    // Of all the cylindrical faces (corner fillets, slot ends, pocket corners
    // and the hole) only the hole's wall is a drillable hole; it is read
    // although the top face around it is bounded by arcs.
    let holes: Vec<CamHole> = package
        .face_evidence
        .iter()
        .filter(|face| face.surface_kind == "cylinder")
        .filter_map(|face| {
            CamHole::from_cylindrical_face(&package, face.face_ordinal, plan.setup()).ok()
        })
        .collect();
    assert_eq!(holes.len(), 1, "{holes:?}");
    assert!(distance(holes[0].center_mm, PLATE_HOLE) <= 1.0e-6);
    assert!((holes[0].radius_mm - HOLE_RADIUS).abs() <= 1.0e-6);
    assert!((holes[0].top_z_mm - 8.0).abs() <= 1.0e-6);
    assert!(holes[0].bottom_z_mm.abs() <= 1.0e-6);
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}

/// The island pocket scaled twice in X and Y: about twice the motions and four
/// times the floor. Cutting motion by motion it needed 55 s in release, just
/// under the worker's 60 s request deadline, so an export of a pocket this size
/// would fail; cut as one fused sweep it stays well clear of the deadline.
#[test]
fn a_pocket_twice_as_large_simulates_inside_the_request_deadline() {
    let _turn = crate::integration_support::file_turn();
    let scale = 2.0;
    let (document, plan) = plate_with_island_pocket_scaled(scale);
    let snapshot = document.current();
    let graph = ExactBRepGraph::from_snapshot(&snapshot, DEFINITION, POCKET).unwrap();
    let mut worker =
        ExactWorkerSupervisor::spawn(env!("CARGO_BIN_EXE_ketchup-exact-worker")).unwrap();
    let package = worker.evaluate_exact_brep_graph(&graph).unwrap();
    let floor = planar_face_at(&package, 6.0, 1.0);
    let region = CamRegion2d::from_planar_face(&package, floor, plan.setup()).unwrap();
    let operations = [CamOperation::RegionPocket {
        id: 1,
        region,
        top_z_mm: 10.0,
        bottom_z_mm: 6.0,
    }];
    let toolpath = CamToolpath::plan(&snapshot, &plan, &operations).unwrap();
    assert!(toolpath.motions.len() > 350, "{}", toolpath.motions.len());
    let cancelled = AtomicBool::new(false);
    let simulation = worker
        .simulate_cam(&snapshot, &plan, &toolpath, &[], &cancelled)
        .unwrap();
    let pocket_mm3 =
        (30.0 * 20.0 * 4.0 - std::f64::consts::PI * ISLAND_RADIUS.powi(2) * 4.0) * scale * scale;
    let corners_mm3 = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4) * TOOL_RADIUS.powi(2) * 4.0;
    assert!(simulation.gouge_mm3 <= 1.0e-5, "{simulation:?}");
    assert!(simulation.collisions.is_empty(), "{simulation:?}");
    assert!(
        (simulation.removed_stock_mm3 - (pocket_mm3 - corners_mm3)).abs() <= 1.0e-3,
        "{simulation:?}"
    );
}
