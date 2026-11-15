use super::*;
use ketchup_model::cam::{CamContourSide, CamMotion, CamMotionPath, CamRegion2d};

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
    let corners = [[5.0, 5.0], [35.0, 5.0], [35.0, 25.0], [5.0, 25.0]];
    let mut entities: Vec<SketchEntity> = (0..4)
        .map(|index| SketchEntity::Line {
            id: SketchEntityId(index as u64 + 1),
            start_mm: corners[index],
            end_mm: corners[(index + 1) % 4],
        })
        .collect();
    entities.push(SketchEntity::Circle {
        id: SketchEntityId(5),
        center_mm: ISLAND_CENTER,
        radius_mm: ISLAND_RADIUS,
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
            maximum_mm: [45.0, 35.0, 10.0],
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
    let band_mm3 = (140.0 * 2.0 * TOOL_RADIUS
        + std::f64::consts::PI * (2.0 * TOOL_RADIUS).powi(2))
        * 4.0;
    assert!(
        (simulation.removed_stock_mm3 - band_mm3).abs() <= 1.0e-3,
        "{simulation:?}"
    );
}

fn distance(left: [f64; 2], right: [f64; 2]) -> f64 {
    (left[0] - right[0]).hypot(left[1] - right[1])
}
