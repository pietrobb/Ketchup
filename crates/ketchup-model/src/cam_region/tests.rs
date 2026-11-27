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
/// cuts already pass the cap, so it is refused while they are built.
#[test]
fn a_pocket_whose_cuts_pass_the_motion_cap_is_refused_before_its_lanes_are_joined() {
    let refused = pocket_paths(&plate_of_121_holes(), 3.0, 0.06);
    assert_eq!(
        refused.as_ref().map(Vec::len).err(),
        Some(&CamPlannerError::InvalidOperations)
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
