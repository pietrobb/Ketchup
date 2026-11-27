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
