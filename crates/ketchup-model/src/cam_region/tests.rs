use super::*;

fn rectangle([min, max]: [[f64; 2]; 2], clockwise: bool) -> CamPath2d {
    let corners = if clockwise {
        [[min[0], max[1]], [max[0], max[1]], [max[0], min[1]], min]
    } else {
        [[max[0], min[1]], max, [min[0], max[1]], min]
    };
    CamPath2d {
        start_mm: min,
        segments: corners
            .into_iter()
            .map(|to_mm| CamPathSegment2d::Line { to_mm })
            .collect(),
    }
}

/// Review 2026-11-27 CAM-1: lanes far finer than the pocket give more cuts than
/// one toolpath may hold, and the toolpath was refused only after every lane had
/// been joined to the next, which held the window for tens of seconds. The
/// pocket is refused once its paths pass the cap, before the rest is joined.
#[test]
fn a_pocket_whose_lanes_pass_the_motion_cap_is_refused_while_they_are_joined() {
    let region = CamRegion2d {
        outer: rectangle([[0.0, 0.0], [400.0, 400.0]], false),
        islands: vec![
            rectangle([[120.0, 20.0], [140.0, 380.0]], true),
            rectangle([[260.0, 20.0], [280.0, 380.0]], true),
        ],
    };
    assert!(pocket_paths(&region, 1.0, 1.0).is_ok());
    // 0.01 mm lanes: about 40 000 rows, three spans across the islands.
    let refused = pocket_paths(&region, 1.0, 0.01);
    assert_eq!(
        refused.as_ref().map(Vec::len).err(),
        Some(&CamPlannerError::InvalidOperations),
        "{} paths under a cap of {MAX_CAM_MOTIONS} motions",
        refused.as_ref().map_or(0, Vec::len)
    );
}

fn plate_of_121_holes() -> CamRegion2d {
    let hole = |center: [f64; 2]| {
        let start_mm = [center[0] + 10.0, center[1]];
        let arc = |to_mm| CamPathSegment2d::Arc {
            to_mm,
            center_mm: center,
            clockwise: true,
        };
        CamPath2d {
            start_mm,
            segments: vec![arc([center[0] - 10.0, center[1]]), arc(start_mm)],
        }
    };
    CamRegion2d {
        outer: rectangle([[0.0, 0.0], [600.0, 600.0]], false),
        islands: (1..12)
            .flat_map(|row| {
                (1..12).map(move |column| hole([50.0 * f64::from(column), 50.0 * f64::from(row)]))
            })
            .collect(),
    }
}

/// Review 2026-11-27 CAM-1: a 600 mm plate with 121 R10 holes, cleared by a
/// Ø6 cutter at 1 % stepover, was refused only after its lanes had been built
/// and joined: 4.45 s on the window thread in an optimised build. Its lanes'
/// cuts already pass the cap, so it is refused while they are built, by the
/// motion cap or the work limit, whichever it passes first.
#[test]
fn a_pocket_whose_cuts_pass_the_motion_cap_is_refused_before_its_lanes_are_joined() {
    let refused = pocket_paths(&plate_of_121_holes(), 3.0, 0.06);
    assert!(
        matches!(
            refused,
            Err(CamPlannerError::InvalidOperations | CamPlannerError::PocketTooFine)
        ),
        "{:?}",
        refused.map(|paths| paths.len())
    );
}

/// The same refusal within 0.5 s in release; 4.45 s before it was refused
/// while the lanes were built.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_a_pocket_whose_cuts_pass_the_motion_cap_is_refused_within_half_a_second() {
    let plate = plate_of_121_holes();
    let started = std::time::Instant::now();
    let refused = pocket_paths(&plate, 3.0, 0.06);
    let elapsed = started.elapsed();
    assert!(refused.is_err());
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "refused after {elapsed:.2?}"
    );
}

fn two_tall_islands() -> CamRegion2d {
    CamRegion2d {
        outer: rectangle([[0.0, 0.0], [400.0, 400.0]], false),
        islands: vec![
            rectangle([[120.0, 20.0], [140.0, 380.0]], true),
            rectangle([[260.0, 20.0], [280.0, 380.0]], true),
        ],
    }
}

/// Review 2026-11-27 CAM-1 (capability review 1079): planning time follows the
/// boundary segments the lanes and links are tested against, not the motions.
/// Within the limit a pocket is planned as before; one segment short of what it
/// needs, it is refused with its own error.
#[test]
fn a_pocket_is_refused_once_its_lanes_and_links_pass_the_work_limit() {
    let region = two_tall_islands();
    let mut counted = PocketWork {
        used: 0,
        limit: usize::MAX,
    };
    let planned = pocket_paths_within(&region, 1.0, 1.0, &mut counted);
    assert!(planned.is_ok());
    assert_eq!(pocket_paths(&region, 1.0, 1.0), planned);
    let mut just_enough = PocketWork {
        used: 0,
        limit: counted.used,
    };
    assert_eq!(
        pocket_paths_within(&region, 1.0, 1.0, &mut just_enough),
        planned
    );
    let mut one_short = PocketWork {
        used: 0,
        limit: counted.used - 1,
    };
    assert_eq!(
        pocket_paths_within(&region, 1.0, 1.0, &mut one_short),
        Err(CamPlannerError::PocketTooFine)
    );
}

/// Operator 2026-11-27: the review plate at a tenth of the Ø6 cutter, 18 585
/// motions and 0.9 s of planning in an optimised build, was planned by 0.3.3;
/// refusing it is a regression. It is planned within half the work limit.
#[test]
fn a_plate_of_121_holes_at_a_tenth_of_the_cutter_is_planned_within_half_the_work_limit() {
    let plate = plate_of_121_holes();
    let mut work = PocketWork {
        used: 0,
        limit: usize::MAX,
    };
    let planned = pocket_paths_within(&plate, 3.0, 0.6, &mut work);
    assert!(planned.is_ok());
    assert!(
        work.used <= MAX_POCKET_WORK / 2,
        "used {} of {MAX_POCKET_WORK}",
        work.used
    );
    assert_eq!(pocket_paths(&plate, 3.0, 0.6), planned);
}

/// The same plate is planned within 0.5 s in release (0.3 s measured; 0.79 s
/// before a move was tested only against the segments near it).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_a_plate_of_121_holes_at_a_tenth_of_the_cutter_is_planned_within_half_a_second() {
    let plate = plate_of_121_holes();
    let (planned, elapsed) = timed(|| pocket_paths(&plate, 3.0, 0.6));
    assert!(planned.is_ok(), "{:?}", planned.err());
    assert!(
        elapsed < ketchup_test_env::perf_budget(std::time::Duration::from_millis(500)),
        "planned in {elapsed:.2?}"
    );
}

fn timed<T>(run: impl FnOnce() -> T) -> (T, std::time::Duration) {
    let started = std::time::Instant::now();
    let value = run();
    (value, started.elapsed())
}

/// The slowest pocket still planned: the finest of these stepovers the review
/// plate is planned at, using nine tenths of the work limit or more, takes under
/// a second in release (0.65 s measured), and the next finer one is refused
/// within a second.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_the_slowest_pocket_planned_within_the_work_limit_takes_under_a_second() {
    let plate = plate_of_121_holes();
    let stepovers = [0.5, 0.4, 0.35, 0.3, 0.25, 0.2];
    let refused_at = stepovers
        .iter()
        .position(|&stepover| pocket_paths(&plate, 3.0, stepover).is_err())
        .expect("the finest stepover passes the work limit");
    assert!(refused_at > 0, "the coarsest stepover is planned");
    let slowest = stepovers[refused_at - 1];
    let mut work = PocketWork {
        used: 0,
        limit: MAX_POCKET_WORK,
    };
    let (planned, elapsed) = timed(|| pocket_paths_within(&plate, 3.0, slowest, &mut work));
    assert!(planned.is_ok());
    assert!(
        work.used >= MAX_POCKET_WORK / 10 * 9,
        "stepover {slowest} used {} of {MAX_POCKET_WORK}",
        work.used
    );
    assert!(
        elapsed < ketchup_test_env::perf_budget(std::time::Duration::from_secs(1)),
        "stepover {slowest} planned in {elapsed:.2?}"
    );
    let finer = stepovers[refused_at];
    let (refused, elapsed) = timed(|| pocket_paths(&plate, 3.0, finer));
    assert_eq!(refused.err(), Some(CamPlannerError::PocketTooFine));
    assert!(
        elapsed < ketchup_test_env::perf_budget(std::time::Duration::from_secs(1)),
        "stepover {finer} refused after {elapsed:.2?}"
    );
}

/// Capability review 1083: at 0.1 mm the plate's cuts are under the motion cap
/// until they are joined, and it was refused only after 4.6–10 s. Its spans
/// alone pass the work limit, so it is refused before any is joined.
#[test]
fn a_pocket_whose_spans_pass_the_work_limit_is_refused_before_they_are_joined() {
    let mut work = PocketWork {
        used: 0,
        limit: MAX_POCKET_WORK,
    };
    let refused = pocket_paths_within(&plate_of_121_holes(), 3.0, 0.1, &mut work);
    assert_eq!(refused, Err(CamPlannerError::PocketTooFine));
    // Refused while joining, the work would have passed the limit.
    assert!(work.used <= MAX_POCKET_WORK, "used {}", work.used);
}

/// The same refusal within 0.5 s in release (0.09 s measured).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_a_pocket_refused_at_one_tenth_mm_stepover_is_refused_within_half_a_second() {
    let plate = plate_of_121_holes();
    let (refused, elapsed) = timed(|| pocket_paths(&plate, 3.0, 0.1));
    assert!(refused.is_err());
    assert!(
        elapsed < ketchup_test_env::perf_budget(std::time::Duration::from_millis(500)),
        "refused after {elapsed:.2?}"
    );
}

/// Operator 2026-11-27: the plate's cuts are known from its size, the cutter
/// and the stepover before any lane is built. Counting them lane by lane still
/// took 0.14 s; refused from that estimate it takes a few milliseconds.
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_a_pocket_whose_estimated_cuts_pass_the_motion_cap_is_refused_before_any_lane_is_built() {
    let plate = plate_of_121_holes();
    let started = std::time::Instant::now();
    let refused = pocket_paths(&plate, 3.0, 0.06);
    let elapsed = started.elapsed();
    assert_eq!(
        refused.as_ref().map(Vec::len).err(),
        Some(&CamPlannerError::InvalidOperations)
    );
    assert!(
        elapsed < std::time::Duration::from_millis(50),
        "refused after {elapsed:.2?}"
    );
}

/// The estimate a pocket is refused by never counts more cuts than its lanes
/// make, so it refuses no pocket whose lanes fit the cap: here round, square
/// and L-shaped islands at three stepovers.
#[test]
fn the_fewest_pocket_cuts_never_exceed_the_cuts_its_lanes_make() {
    let center = [90.0, 110.0];
    let arc = |to_mm| CamPathSegment2d::Arc {
        to_mm,
        center_mm: center,
        clockwise: true,
    };
    let circle = CamPath2d {
        start_mm: [105.0, 110.0],
        segments: vec![arc([75.0, 110.0]), arc([105.0, 110.0])],
    };
    let l_shape = CamPath2d {
        start_mm: [130.0, 20.0],
        segments: [
            [130.0, 140.0],
            [150.0, 140.0],
            [150.0, 40.0],
            [180.0, 40.0],
            [180.0, 20.0],
            [130.0, 20.0],
        ]
        .into_iter()
        .map(|to_mm| CamPathSegment2d::Line { to_mm })
        .collect(),
    };
    let region = CamRegion2d {
        outer: rectangle([[0.0, 0.0], [200.0, 160.0]], false),
        islands: vec![
            rectangle([[20.0, 20.0], [60.0, 140.0]], true),
            circle,
            l_shape,
        ],
    };
    for stepover in [0.5, 1.0, 2.5] {
        let boundaries = pocket_boundaries(&region, 3.0 + stepover * LANE_WALL_GAP_STEPOVERS)
            .expect("the lanes' boundaries");
        let fewest = fewest_pocket_cuts(&boundaries, stepover);
        let paths = pocket_paths(&region, 3.0, stepover).expect("the pocket is planned");
        // The walls' passes, one per loop, follow the lanes.
        let lanes = &paths[..paths.len() - 1 - region.islands.len()];
        let cuts: usize = lanes
            .iter()
            .map(|path| path.segments.len().div_ceil(2))
            .sum();
        assert!(
            fewest > 0 && fewest <= cuts,
            "{fewest} cuts estimated, {cuts} made at {stepover} mm"
        );
    }
}
