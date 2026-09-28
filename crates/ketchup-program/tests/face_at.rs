//! A face picked on a part is named the way the program names it.
use ketchup_program::{frame, model::Part, run};

fn part(source: &str, name: &str) -> Part {
    let (evaluated, _) = run("test.star", source, &Default::default()).expect("evaluates");
    evaluated.model.part(name).expect("part exists").clone()
}

/// World point and outward normal, as the window sees them, in `part`'s frame.
fn picked(
    part: &Part,
    world: [f64; 3],
    normal_world: [f64; 3],
    role: Option<&str>,
) -> Option<String> {
    part.face_at(
        part.to_local(world),
        frame::apply_transposed(&part.rotation, normal_world),
        role,
    )
}

#[test]
fn faces_of_a_rotated_box_are_named_in_its_own_frame() {
    // A 400 x 40 x 20 rail stood on end: local +x points up, local +z along -y.
    let rail = part(
        "rail = box(\"rail\", (400, 40, 20), at = (100, 200, 0))\n\
         rotate(rail, axis = (0, 1, 0), angle = -90)\n\
         rotate(rail, axis = (1, 0, 0), angle = 90)\n",
        "rail",
    );
    let (min, max) = rail.world_bounds();
    let centre: [f64; 3] = std::array::from_fn(|i| 0.5 * (min[i] + max[i]));
    for (local, name) in [
        ([1.0, 0.0, 0.0], "x+"),
        ([-1.0, 0.0, 0.0], "x-"),
        ([0.0, 1.0, 0.0], "y+"),
        ([0.0, -1.0, 0.0], "y-"),
        ([0.0, 0.0, 1.0], "z+"),
        ([0.0, 0.0, -1.0], "z-"),
    ] {
        let world_normal = frame::apply(&rail.rotation, local);
        let world = std::array::from_fn(|i| {
            centre[i]
                + world_normal[i] * (rail.reach(world_normal) - frame::dot(centre, world_normal))
        });
        assert_eq!(
            picked(&rail, world, world_normal, Some("top")).as_deref(),
            Some(name)
        );
    }
    assert_eq!(
        rail.face_at([200.0, 20.0, 5.0], [0.0, 0.0, 1.0], None),
        None,
        "a plane inside the box is no face of it"
    );
}

#[test]
fn profile_faces_are_the_caps_and_segment_names() {
    let top = part(
        "top = extrude(\"top\", profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40), distance = 18)\n",
        "top",
    );
    assert_eq!(
        top.face_at([400.0, 250.0, 18.0], [0.0, 0.0, 1.0], None)
            .as_deref(),
        Some("end")
    );
    assert_eq!(
        top.face_at([400.0, 250.0, 0.0], [0.0, 0.0, -1.0], None)
            .as_deref(),
        Some("start")
    );
    assert_eq!(
        top.face_at([400.0, 0.0, 9.0], [0.0, -1.0, 0.0], None)
            .as_deref(),
        Some("segment1")
    );
    // A point on the rounded corner at (800, 0), slightly inside the arc as a
    // tessellated face would give it.
    let d = 40.0 - 40.0 / 2f64.sqrt() + 0.05;
    let corner = top.face_at([800.0 - d, d, 9.0], [0.7, -0.7, 0.0], None);
    assert!(
        corner
            .as_deref()
            .is_some_and(|name| name.starts_with("corner")),
        "{corner:?}"
    );
    // The solid's own name for a face wins when the program knows it.
    assert_eq!(
        top.face_at([0.0; 3], [0.0, 0.0, 1.0], Some("segment3#2"))
            .as_deref(),
        Some("segment3#2")
    );
    assert_eq!(
        top.face_at([400.0, 250.0, 9.0], [0.0, 0.0, 1.0], Some("nonsense")),
        None
    );
}

#[test]
fn a_revolved_face_is_found_from_its_radius() {
    let seat = part(
        "seat = revolve(\"seat\", profile = [[\"bottom\", [0, 0], [150, 0]], [\"rim\", [150, 0], [150, 20]], \
         [\"top\", [150, 20], [0, 20]], [\"axis\", [0, 20], [0, 0]]], axis = [(0, 0), (0, 1)])\n",
        "seat",
    );
    let rim = seat.face_at(
        [150.0 / 2f64.sqrt(), 10.0, 150.0 / 2f64.sqrt()],
        [0.7, 0.0, 0.7],
        None,
    );
    assert_eq!(rim.as_deref(), Some("rim"));
    assert_eq!(
        seat.face_at([40.0, 20.0, -30.0], [0.0, 1.0, 0.0], None)
            .as_deref(),
        Some("top")
    );
}
