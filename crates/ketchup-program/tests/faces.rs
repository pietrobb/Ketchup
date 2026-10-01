//! Faces of any part are frames found by name: planes and cylinders computed
//! from the body and carried through push_pull, mirror, booleans and the
//! part's placement.
use ketchup_program::contact::contact;
use ketchup_program::faces::{FaceFrame, FaceKind};
use ketchup_program::model::Part;
use ketchup_program::run;

fn part(source: &str, name: &str) -> Part {
    let (evaluated, _) = run("faces.star", source, &Default::default()).expect("evaluates");
    evaluated.model.part(name).expect("part exists").clone()
}

fn world(part: &Part, name: &str) -> FaceFrame {
    part.world_face(&part.face_frame(name).expect("face exists"))
}

fn near(actual: [f64; 3], expected: [f64; 3]) -> bool {
    (0..3).all(|i| (actual[i] - expected[i]).abs() < 1e-6)
}

fn assert_near(actual: [f64; 3], expected: [f64; 3], what: &str) {
    assert!(near(actual, expected), "{what}: {actual:?} vs {expected:?}");
}

fn assert_span(face: &FaceFrame, min: [f64; 2], max: [f64; 2]) {
    let close = |a: [f64; 2], b: [f64; 2]| (0..2).all(|i| (a[i] - b[i]).abs() < 1e-9);
    assert!(
        close(face.min, min) && close(face.max, max),
        "{} spans {:?}..{:?}, expected {min:?}..{max:?}",
        face.name,
        face.min,
        face.max
    );
}

fn radius(face: &FaceFrame) -> f64 {
    match face.kind {
        FaceKind::Cylindrical { radius_mm } => radius_mm,
        FaceKind::Planar => panic!("{} is planar", face.name),
    }
}

#[test]
fn box_faces_are_planes_that_follow_the_part_frame() {
    let panel = part(
        "p = box(\"p\", (400, 300, 18), at = (10, 20, 30))\n\
         rotate(p, axis = (0, 0, 1), angle = 90)\n",
        "p",
    );
    let names: Vec<_> = panel.face_frames().into_iter().map(|f| f.name).collect();
    // A box is its rectangle extruded: caps first, then the sides in order.
    assert_eq!(names, ["z-", "z+", "y-", "x+", "y+", "x-"]);
    let top = world(&panel, "z+");
    assert_eq!(top.kind, FaceKind::Planar);
    assert_near(top.origin_mm, [10.0, 20.0, 48.0], "z+ origin");
    assert_near(top.normal, [0.0, 0.0, 1.0], "z+ normal");
    assert_eq!((top.min, top.max), ([0.0; 2], [400.0, 300.0]));
    // Turned 90 degrees about z, the local +x face looks along world +y.
    let end = world(&panel, "x+");
    assert_near(end.normal, [0.0, 1.0, 0.0], "x+ normal");
    assert_near(end.origin_mm, [10.0, 420.0, 30.0], "x+ origin");
    assert_near(
        end.point([300.0, 18.0]),
        [-290.0, 420.0, 48.0],
        "x+ far corner",
    );
    assert!(
        panel
            .face_frame("top")
            .unwrap_err()
            .contains("z-, z+, y-, x+, y+, x-")
    );
}

#[test]
fn a_box_is_its_rectangle_extruded_and_measured_from_its_minimum_corner() {
    let block = part("box(\"b\", (400, 300, 18))\n", "b");
    assert_eq!(block.body.cuboid_size(), Some([400.0, 300.0, 18.0]));
    // Every side, also the two the rectangle runs backwards along, starts at
    // the minimum corner and runs along +x or +y.
    for (name, origin, u) in [
        ("y-", [0.0, 0.0, 0.0], [1.0, 0.0, 0.0]),
        ("x+", [400.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
        ("y+", [0.0, 300.0, 0.0], [1.0, 0.0, 0.0]),
        ("x-", [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
    ] {
        let face = block.face_frame(name).unwrap();
        assert_near(face.origin_mm, origin, name);
        assert_near(face.u, u, name);
        assert_near(face.v, [0.0, 0.0, 1.0], name);
    }
    // The exact kernel names the faces as it names the panel it builds.
    for (name, label) in [
        ("z-", "start"),
        ("z+", "end"),
        ("y-", "segment_1"),
        ("x+", "segment_2"),
        ("y+", "segment_3"),
        ("x-", "segment_4"),
    ] {
        assert_eq!(block.exact_face_label(name).unwrap(), label);
    }
    // The same rectangle drawn from another corner is an extrusion, not a box.
    let shifted = part(
        "extrude(\"s\", distance = 18, profile = [[400, 0], [400, 300], [0, 300], [0, 0]])\n",
        "s",
    );
    assert_eq!(shifted.body.cuboid_size(), None);
    assert_eq!(shifted.size_mm, [400.0, 300.0, 18.0]);
}

#[test]
fn extruded_sides_are_planes_and_rounded_corners_cylinders() {
    let top = part(
        "extrude(\"top\", distance = 18, \
         profile = round_corners([[0, 0], [800, 0], [800, 500], [0, 500]], 40))",
        "top",
    );
    let front = world(&top, "segment1");
    assert_near(front.origin_mm, [40.0, 0.0, 0.0], "front origin");
    assert_near(front.normal, [0.0, -1.0, 0.0], "front normal");
    assert_near(front.u, [1.0, 0.0, 0.0], "front u");
    assert_eq!(front.max, [720.0, 18.0]);
    let end = world(&top, "end");
    assert_near(end.normal, [0.0, 0.0, 1.0], "end normal");
    assert_span(&end, [0.0, 0.0], [800.0, 500.0]);
    let corner = world(&top, "corner2");
    assert!((radius(&corner) - 40.0).abs() < 1e-9);
    assert_near(corner.origin_mm, [760.0, 40.0, 0.0], "corner axis");
    assert!((corner.max[0] - 90.0).abs() < 1e-9 && corner.max[1] == 18.0);
    // The quarter turns from the front edge to the right side, bulging out.
    assert_near(corner.point([0.0, 0.0]), [760.0, 0.0, 0.0], "corner start");
    assert_near(corner.point([90.0, 0.0]), [800.0, 40.0, 0.0], "corner end");
    let diagonal = std::f64::consts::FRAC_1_SQRT_2;
    assert_near(
        corner.normal_at([45.0, 9.0]),
        [diagonal, -diagonal, 0.0],
        "corner normal",
    );
}

#[test]
fn revolved_sides_are_cylinders_rings_and_turned_caps() {
    let ring = part(
        "revolve(\"ring\", axis = [(0, 0), (0, 1)], angle = 90, profile = [\
         [\"bottom\", [10, 0], [30, 0]], [\"outside\", [30, 0], [30, 100]], \
         [\"top\", [30, 100], [10, 100]], [\"bore\", [10, 100], [10, 0]]])",
        "ring",
    );
    let names: Vec<_> = ring.face_frames().into_iter().map(|f| f.name).collect();
    assert_eq!(names, ["start", "end", "bottom", "outside", "top", "bore"]);
    // Turning about +y carries +x towards -z.
    let outside = world(&ring, "outside");
    assert!((radius(&outside) - 30.0).abs() < 1e-9);
    assert_near(outside.normal, [1.0, 0.0, 0.0], "outside normal");
    assert_span(&outside, [0.0, 0.0], [90.0, 100.0]);
    assert_near(
        outside.point([90.0, 50.0]),
        [0.0, 50.0, -30.0],
        "quarter turn",
    );
    let bore = world(&ring, "bore");
    assert!((radius(&bore) - 10.0).abs() < 1e-9);
    assert_near(bore.normal, [-1.0, 0.0, 0.0], "a bore faces its axis");
    let bottom = world(&ring, "bottom");
    assert_eq!(bottom.kind, FaceKind::Planar);
    assert_near(bottom.normal, [0.0, -1.0, 0.0], "bottom normal");
    assert_span(&bottom, [0.0, 0.0], [30.0, 30.0]);
    assert_near(
        world(&ring, "top").origin_mm,
        [0.0, 100.0, 0.0],
        "top plane",
    );
    assert_near(world(&ring, "start").normal, [0.0, 0.0, 1.0], "start cap");
    assert_near(world(&ring, "end").normal, [-1.0, 0.0, 0.0], "end cap");
    let at = [30.0 * 0.5f64.sqrt(), 50.0, -30.0 * 0.5f64.sqrt()];
    let found = ring.face_frame_at(at, 0.01).expect("on the outside");
    assert_eq!(found.name, "outside");
    let coordinates = found.coordinates(at);
    assert!((coordinates[0] - 45.0).abs() < 1e-9 && (coordinates[1] - 50.0).abs() < 1e-9);
    assert!(
        ring.face_frame_at([30.0, 50.0, 30.0], 0.01).is_none(),
        "outside the quarter"
    );

    let full = part(
        "revolve(\"knob\", axis = [(0, 0), (0, 1)], profile = [(0, 0), (20, 0), (20, 30), (0, 30)])",
        "knob",
    );
    let error = full.face_frame("start").unwrap_err();
    assert!(error.contains("not a flat or cylindrical face"), "{error}");
    assert!(
        full.face_frames()
            .iter()
            .all(|face| face.name != "start" && face.name != "end")
    );
}

#[test]
fn faces_follow_push_pull_mirror_and_booleans() {
    let wedge = part(
        "w = extrude(\"w\", distance = 20, profile = [[\"base\", [0, 0], [100, 0]], \
         [\"slope\", [100, 0], [0, 50]], [\"back\", [0, 50], [0, 0]]])\n\
         push_pull(w, face = \"end\", distance = 5, name = \"thicker\")\n\
         drill = revolve(\"drill\", axis = [(0, 0), (0, 1)], tool = True, at = (80, 10, 0), \
         profile = [[\"tip\", [0, 0], [4, 0]], [\"wall\", [4, 0], [4, 40]], [\"cap\", [4, 40], [0, 40]], [\"axis\", [0, 40], [0, 0]]])\n\
         rotate(drill, axis = (1, 0, 0), angle = 90, pivot = (80, 10, 0))\n\
         subtract(w, drill, name = \"bore\")\n\
         mirror(w, axis = \"x\")\n",
        "w",
    );
    assert_near(
        world(&wedge, "end").origin_mm,
        [100.0, 0.0, 25.0],
        "end moved out, mirrored",
    );
    // Mirrored across x = 50, the slope now rises to the right and the back
    // stands at x = 100.
    let slope = world(&wedge, "slope");
    let n = 1.0 / 5f64.sqrt();
    assert_near(slope.normal, [-n, 2.0 * n, 0.0], "mirrored slope normal");
    assert_near(
        world(&wedge, "back").normal,
        [1.0, 0.0, 0.0],
        "mirrored back",
    );
    assert_eq!(
        wedge
            .face_frame_at([100.0, 20.0, 10.0], 0.01)
            .map(|f| f.name),
        Some("back".to_owned())
    );
    // The bore drilled along world z through (80, 10) is mirrored to
    // (20, 10) and still faces its axis; heights now run along -z.
    let wall = world(&wedge, "bore.wall");
    assert!((radius(&wall) - 4.0).abs() < 1e-9);
    assert_near(
        wall.point([0.0, -10.0]),
        [16.0, 10.0, 10.0],
        "bore wall point",
    );
    assert_near(
        wall.point([90.0, -10.0]),
        [20.0, 14.0, 10.0],
        "bore quarter turn",
    );
    assert_near(
        wall.normal_at([0.0, -10.0]),
        [1.0, 0.0, 0.0],
        "bore wall normal",
    );
    assert_span(&wall, [0.0, -40.0], [360.0, 0.0]);
    assert_eq!(
        wedge.face_frame_at([20.0, 6.0, 30.0], 0.01).map(|f| f.name),
        Some("bore.wall".to_owned())
    );
}

#[test]
fn programs_look_faces_up_by_name_and_by_point() {
    let source = "\
ring = revolve(\"ring\", axis = [(0, 0), (0, 1)], angle = 90, at = (100, 0, 0), profile = [\
[\"bottom\", [10, 0], [30, 0]], [\"outside\", [30, 0], [30, 100]], \
[\"top\", [30, 100], [10, 100]], [\"bore\", [10, 100], [10, 0]]])
outside = face(ring, \"outside\")
if outside.kind != \"cylindrical\" or outside.radius != 30 or outside.name != \"outside\":
    fail(\"outside: %r\" % outside)
if [f.name for f in faces(ring)] != [\"start\", \"end\", \"bottom\", \"outside\", \"top\", \"bore\"]:
    fail(\"faces: %r\" % [f.name for f in faces(ring)])
top = face(ring, \"top\")
if top.kind != \"planar\" or top.radius != None or top.normal != (0.0, 1.0, 0.0) or top.origin != (100.0, 100.0, 0.0):
    fail(\"top: %r\" % top)
hit = face_at(ring, (100 + 30 * math.cos(math.radians(45)), 50, -30 * math.sin(math.radians(45))))
if hit == None or hit.name != \"outside\":
    fail(\"face_at: %r\" % hit)
if face_at(ring, (100, 50, 40)) != None:
    fail(\"a point off the part has no face\")
";
    run("faces.star", source, &Default::default()).expect("face lookups hold");
    let error = run(
        "faces.star",
        "b = box(\"b\", (10, 10, 10))\nface(b, \"top\")\n",
        &Default::default(),
    )
    .unwrap_err()
    .message;
    assert!(error.contains("does not exist on \"b\""), "{error}");
}

const KNOB: &str = "knob = revolve(\"knob\", axis = [(0, 0), (0, 1)], profile = [\
[\"bottom\", [0, 0], [30, 0]], [\"side\", [30, 0], [30, 100]], \
[\"top\", [30, 100], [0, 100]], [\"axis\", [0, 100], [0, 0]]])\n";

#[test]
fn holes_go_into_the_caps_and_the_round_side_of_a_revolved_part() {
    let knob = part(
        &format!(
            "{KNOB}hole(knob, \"top\", at = (0, 0), diameter = 10, depth = 20)\n\
             hole(knob, face(knob, \"side\"), at = (90, 50), diameter = 8, depth = 10)\n\
             pocket(knob, \"bottom\", rect = (-5, -5, 5, 5), depth = 3)\n"
        ),
        "knob",
    );
    let [cap, radial] = [&knob.holes[0], &knob.holes[1]];
    assert_near(cap.entry_mm, [0.0, 100.0, 0.0], "cap entry");
    assert_near(cap.inward, [0.0, -1.0, 0.0], "cap inward");
    // 90 degrees round from +x about +y is -z; the drill points at the axis.
    assert_near(radial.entry_mm, [0.0, 50.0, -30.0], "radial entry");
    assert_near(radial.inward, [0.0, 0.0, 1.0], "radial inward");
    let tools = knob.machining_tools();
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(names, ["hole h1", "hole h2", "pocket p1"]);
    let drill = &tools[1].tool;
    assert_near(drill.at_mm, [0.0, 50.0, -30.0], "drill placed at the entry");
    assert_near(
        std::array::from_fn(|row| drill.rotation[row][2]),
        [0.0, 0.0, 1.0],
        "drill runs along the hole",
    );
    let (min, max) = drill.local_bounds();
    assert!((max[2] - min[2] - 10.0).abs() < 1e-9 && (max[0] - min[0] - 8.0).abs() < 1e-9);
    let error = run(
        "faces.star",
        &format!("{KNOB}pocket(knob, \"side\", rect = (0, 0, 10, 10), depth = 2)\n"),
        &Default::default(),
    )
    .unwrap_err()
    .message;
    assert!(error.contains("curved"), "{error}");
    let error = run(
        "faces.star",
        &format!("{KNOB}b = box(\"b\", (10, 10, 10))\nhole(knob, face(b, \"z+\"), at = (1, 1), diameter = 2, depth = 2)\n"),
        &Default::default(),
    )
    .unwrap_err()
    .message;
    assert!(error.contains("belongs to"), "{error}");
}

#[test]
fn a_hole_drilled_after_a_mirror_lands_where_the_face_now_is() {
    let before = part(
        "b = box(\"b\", (100, 60, 40))\n\
         hole(b, \"x+\", at = (20, 10), diameter = 8, depth = 30)\n\
         mirror(b, axis = \"x\")\n",
        "b",
    );
    let after = part(
        "b = box(\"b\", (100, 60, 40))\n\
         mirror(b, axis = \"x\")\n\
         hole(b, \"x+\", at = (20, 10), diameter = 8, depth = 30)\n",
        "b",
    );
    for b in [&before, &after] {
        let hole = &b.holes[0];
        // Drilled into the body before the mirror, from its x+ end ...
        assert_near(hole.entry_mm, [100.0, 20.0, 10.0], "body entry");
        assert_near(hole.inward, [-1.0, 0.0, 0.0], "body inward");
        // ... which the mirror carries to x = 0, still named x+.
        let (entry, inward) = b.after_operations((hole.entry_mm, hole.inward));
        assert_near(entry, [0.0, 20.0, 10.0], "final entry");
        assert_near(inward, [1.0, 0.0, 0.0], "final inward");
        assert_near(world(b, "x+").origin_mm, [0.0, 0.0, 0.0], "mirrored face");
    }
    assert_eq!(before.holes, after.holes);
}

#[test]
fn contact_finds_a_profile_face_mirrored_in_the_middle_of_the_program() {
    let source = "\
w = extrude(\"w\", distance = 20, profile = [[\"base\", [0, 0], [100, 0]], \
[\"slope\", [100, 0], [0, 50]], [\"back\", [0, 50], [0, 0]]])
hole(w, \"base\", at = (70, 10), diameter = 5, depth = 10)
mirror(w, axis = \"x\")
post = box(\"post\", (30, 50, 20))
on(post, w, face = \"back\")
c = contact(w, post)
if (c.face_a, c.face_b) != (\"back\", \"x-\"):
    fail(\"faces %s %s\" % (c.face_a, c.face_b))
dowels(w, post, dowel = \"6x30\", margin = 10)
";
    let (evaluated, report) = run("faces.star", source, &Default::default()).expect("evaluates");
    assert!(report.ok, "{:#?}", report.issues);
    let (w, post) = (
        evaluated.model.part("w").unwrap(),
        evaluated.model.part("post").unwrap(),
    );
    // The mirror across x = 50 turned the back from x = 0 to x = 100; on()
    // stood the post there, and a bounding box would call the face x+.
    let found = contact(w, post).expect("touching");
    assert_eq!(
        [found.face_a.as_str(), found.face_b.as_str()],
        ["back", "x-"]
    );
    assert_near(found.normal, [1.0, 0.0, 0.0], "out of the back");
    assert_eq!(found.points_mm.len(), 4);
    assert!(
        (found.size_mm[0] * found.size_mm[1] - 1000.0).abs() < 1e-6,
        "{:?}",
        found.size_mm
    );
    assert!(found.points_mm.iter().all(|p| (p[0] - 100.0).abs() < 1e-9));
    // The dowel holes go into that face: after the base hole, two in the back.
    let dowels: Vec<_> = w.holes.iter().filter(|h| h.face == "back").collect();
    assert_eq!(dowels.len(), 2);
    for hole in dowels {
        let (entry, inward) = w.after_operations((hole.entry_mm, hole.inward));
        assert!((entry[0] - 100.0).abs() < 1e-9, "{entry:?}");
        assert_near(inward, [-1.0, 0.0, 0.0], "into the mirrored back");
    }
    assert_eq!(post.holes.iter().filter(|h| h.face == "x-").count(), 2);
    assert!((post.at_mm[0] - 100.0).abs() < 1e-9, "{:?}", post.at_mm);
}

#[test]
fn a_leaning_board_trimmed_flat_joins_a_board_it_is_not_square_to() {
    let source = "\
base = box(\"base\", (300, 200, 30), at = (0, 0, -30))
leg = box(\"leg\", (100, 30, 300), at = (100, 85, -50))
rotate(leg, axis = (1, 0, 0), angle = 20, pivot = (150, 100, 0))
trim = box(\"trim\", (1000, 1000, 200), at = (-350, -400, -200), tool = True)
subtract(leg, trim, name = \"cut\")
c = contact(leg, base)
if (c.face_a, c.face_b) != (\"cut.z+\", \"z+\"):
    fail(\"faces %s %s\" % (c.face_a, c.face_b))
dowels(leg, base, dowel = \"8x30\", margin = 25)
";
    let (evaluated, report) = run("faces.star", source, &Default::default()).expect("evaluates");
    assert!(report.ok, "{:#?}", report.issues);
    let (leg, base) = (
        evaluated.model.part("leg").unwrap(),
        evaluated.model.part("base").unwrap(),
    );
    let found = contact(leg, base).expect("the cut end stands on the base");
    assert_near(found.normal, [0.0, 0.0, -1.0], "down out of the cut");
    // The cut crosses the 30 mm leg leaning 20 degrees: 100 by 30 / cos 20.
    let across = 30.0 / 20f64.to_radians().cos();
    assert!(
        (found.size_mm[0] - 100.0).abs() < 1e-6,
        "{:?}",
        found.size_mm
    );
    assert!(
        (found.size_mm[1] - across).abs() < 1e-6,
        "{:?}",
        found.size_mm
    );
    assert!(found.points_mm.iter().all(|p| p[2].abs() < 1e-9));
    let dowels: Vec<_> = leg.holes.iter().filter(|h| h.face == "cut.z+").collect();
    assert_eq!(dowels.len(), 2);
    for hole in &dowels {
        // Square to the base, so at 20 degrees to the leg's own axes.
        let inward = std::array::from_fn(|row| {
            (0..3)
                .map(|k| leg.rotation[row][k] * hole.inward[k])
                .sum::<f64>()
        });
        assert_near(inward, [0.0, 0.0, 1.0], "up into the leg");
        assert!(leg.to_world(hole.entry_mm)[2].abs() < 1e-9);
    }
    assert_eq!(base.holes.iter().filter(|h| h.face == "z+").count(), 2);
}
