//! Program-stable face and edge names survive changed numbers.
//!
//! Each case builds the same program twice with different dimensions, the way
//! a rule program is re-run after an edit, and checks that every named
//! reference still lands on the geometrically right face.

use ketchup_exact::naming::{NamedBody, NamedBoolean, NamedSegment, NamingError};
use ketchup_exact::{EdgeFinish, ExactBackend, FaceEvidence, PlanarProfileSegment};

fn line(name: &str, start: [f64; 2], end: [f64; 2]) -> NamedSegment {
    NamedSegment::new(
        name,
        PlanarProfileSegment::Line {
            start_mm: start,
            end_mm: end,
        },
    )
}

fn rectangle(x0: f64, y0: f64, x1: f64, y1: f64, names: [&str; 4]) -> Vec<NamedSegment> {
    vec![
        line(names[0], [x0, y0], [x1, y0]),
        line(names[1], [x1, y0], [x1, y1]),
        line(names[2], [x1, y1], [x0, y1]),
        line(names[3], [x0, y1], [x0, y0]),
    ]
}

fn face<'a>(body: &'a NamedBody, name: &str) -> &'a FaceEvidence {
    let ordinal = body.face(name).unwrap_or_else(|error| panic!("{error}"));
    body.output
        .body
        .topology
        .faces
        .iter()
        .find(|face| face.ordinal == ordinal)
        .expect("face evidence")
}

fn close(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1.0e-6
}

/// Face bounds carry the OCCT tolerance gap, so compare them more loosely.
fn near(actual: f64, expected: f64) -> bool {
    (actual - expected).abs() <= 1.0e-3
}

fn between(value: f64, low: f64, high: f64) -> bool {
    value > low && value < high
}

// (a) Extrude a profile with an arc, fillet a named edge, change dimensions.
fn filleted_leg(width: f64, height: f64, thickness: f64) -> NamedBody {
    let backend = ExactBackend::new();
    let radius = width / 2.0;
    let profile = vec![
        line("spodok", [0.0, 0.0], [width, 0.0]),
        line("bok", [width, 0.0], [width, height]),
        NamedSegment::new(
            "oblúk",
            PlanarProfileSegment::CircularArc {
                start_mm: [width, height],
                end_mm: [0.0, height],
                center_mm: [radius, height],
                clockwise: false,
            },
        ),
        line("zadok", [0.0, height], [0.0, 0.0]),
    ];
    let leg = backend.named_extrude(&profile, 0.0, thickness).unwrap();
    backend
        .named_finish(
            &leg,
            &[("end", "bok"), ("end", "oblúk")],
            EdgeFinish::Fillet,
            3.0,
        )
        .unwrap()
}

#[test]
fn fillet_stays_on_the_named_edge_when_dimensions_change() {
    for (width, height, thickness) in [(100.0, 60.0, 18.0), (140.0, 90.0, 25.0), (60.0, 30.0, 12.0)]
    {
        let leg = filleted_leg(width, height, thickness);

        // Fillet faces are named after the two faces they run between.
        let side = face(&leg, "fillet(bok,end)");
        assert!(between(side.centroid_mm.x, width - 3.0, width), "{side:?}");
        assert!(
            between(side.centroid_mm.z, thickness - 3.0, thickness),
            "{side:?}"
        );
        assert!(between(side.centroid_mm.y, 0.0, height), "{side:?}");

        let arc = face(&leg, "fillet(end,oblúk)");
        assert!(arc.centroid_mm.y > height, "{arc:?}");
        assert!(
            between(arc.centroid_mm.z, thickness - 3.0, thickness),
            "{arc:?}"
        );

        // "zadok" is tangent to the arc, so OCCT carried the fillet onto it
        // although the program never selected that edge; it still gets a name.
        let back = face(&leg, "fillet(end,zadok)");
        assert!(between(back.centroid_mm.x, 0.0, 3.0), "{back:?}");
        assert!(between(back.centroid_mm.y, 0.0, height), "{back:?}");

        // The faces the fillet trimmed keep their own names.
        assert!(face(&leg, "oblúk").centroid_mm.y > height);
        assert!(close(face(&leg, "end").centroid_mm.z, thickness));
        assert!(close(face(&leg, "bok").centroid_mm.x, width));
    }
}

#[test]
fn a_reference_to_a_missing_edge_is_a_named_error() {
    let backend = ExactBackend::new();
    let leg = filleted_leg(100.0, 60.0, 18.0);
    let error = backend
        .named_finish(&leg, &[("start", "end")], EdgeFinish::Chamfer, 1.0)
        .unwrap_err();
    let NamingError::UnknownEdge {
        neighbours_of_first,
        ..
    } = &error
    else {
        panic!("{error}");
    };
    assert!(
        neighbours_of_first.contains(&"spodok".to_owned()),
        "{error}"
    );
    let error = backend
        .named_finish(&leg, &[("end", "predok")], EdgeFinish::Chamfer, 1.0)
        .unwrap_err();
    assert!(matches!(error, NamingError::UnknownFace { .. }), "{error}");
    assert!(error.to_string().contains("face 'predok' does not exist"));
}

// (b) A groove splits the top face; Push/Pull one half.
fn grooved_board(length: f64, groove_x: f64, groove_width: f64, pull: f64) -> NamedBody {
    let backend = ExactBackend::new();
    let (depth, thickness, groove_depth) = (300.0, 18.0, 6.0);
    let board = backend
        .named_extrude(
            &rectangle(
                0.0,
                0.0,
                length,
                depth,
                ["predok", "pravy", "zadok", "lavy"],
            ),
            0.0,
            thickness,
        )
        .unwrap();
    let tool = backend
        .named_extrude(
            &rectangle(
                groove_x,
                -1.0,
                groove_x + groove_width,
                depth + 1.0,
                ["vstup", "stena_p", "vystup", "stena_l"],
            ),
            thickness - groove_depth,
            groove_depth + 1.0,
        )
        .unwrap();
    let grooved = backend
        .named_boolean(&board, &tool, "drazka", NamedBoolean::Cut)
        .unwrap();
    backend.named_offset_face(&grooved, "end#2", pull).unwrap()
}

#[test]
fn push_pull_on_one_half_of_a_split_face_follows_the_half_when_the_groove_moves() {
    for (length, groove_x, pull) in [
        (500.0, 150.0, 10.0),
        (500.0, 320.0, 10.0),
        (700.0, 40.0, -4.0),
    ] {
        let groove_width = 8.0;
        let board = grooved_board(length, groove_x, groove_width, pull);
        let names = board.face_names();

        let left = face(&board, "end#1");
        assert!(near(left.bounds_mm.min.x, 0.0) && near(left.bounds_mm.max.x, groove_x));
        assert!(close(left.centroid_mm.z, 18.0), "{names:?}");

        let right = face(&board, "end#2");
        assert!(
            near(right.bounds_mm.min.x, groove_x + groove_width),
            "{right:?}"
        );
        assert!(near(right.bounds_mm.max.x, length), "{right:?}");
        assert!(close(right.centroid_mm.z, 18.0 + pull), "{names:?}");

        assert!(close(face(&board, "drazka.start").centroid_mm.z, 12.0));
        let wall = face(&board, "drazka.stena_l");
        assert!(close(wall.centroid_mm.x, groove_x), "{wall:?}");
        let far_wall = face(&board, "drazka.stena_p");
        // The groove wall and the side under the moved half follow it up or down.
        assert!(near(far_wall.bounds_mm.max.z, 18.0 + pull), "{far_wall:?}");
        assert!(near(face(&board, "pravy").bounds_mm.max.z, 18.0 + pull));
        assert!(near(wall.bounds_mm.max.z, 18.0), "{wall:?}");
    }
}

// (c) Revolve, chamfer, change the angle.
fn chamfered_ring(angle: f64, outer: f64) -> NamedBody {
    let backend = ExactBackend::new();
    let ring = backend
        .named_revolve(
            &rectangle(
                40.0,
                0.0,
                outer,
                30.0,
                ["spodok", "vonkajsi", "vrch", "vnutorny"],
            ),
            [0.0, 0.0],
            [0.0, 1.0],
            angle,
        )
        .unwrap();
    backend
        .named_finish(&ring, &[("vrch", "vonkajsi")], EdgeFinish::Chamfer, 2.0)
        .unwrap()
}

#[test]
fn chamfer_on_a_revolved_edge_survives_a_changed_angle() {
    for (angle, outer) in [(90.0, 60.0), (180.0, 60.0), (270.0, 75.0), (360.0, 75.0)] {
        let ring = chamfered_ring(angle, outer);
        let names = ring.face_names();

        let chamfer = face(&ring, "chamfer(vonkajsi,vrch)");
        assert_eq!(chamfer.surface_kind, "other", "{names:?}");
        assert!(near(chamfer.bounds_mm.min.y, 28.0), "{chamfer:?}");
        assert!(near(chamfer.bounds_mm.max.y, 30.0), "{chamfer:?}");
        // It sits between the two faces whose edge it replaced.
        ring.edge("chamfer(vonkajsi,vrch)", "vonkajsi").unwrap();
        ring.edge("chamfer(vonkajsi,vrch)", "vrch").unwrap();

        assert_eq!(face(&ring, "vonkajsi").surface_kind, "cylinder");
        assert!(close(face(&ring, "vrch").centroid_mm.y, 30.0));
        assert_eq!(
            names.iter().any(|name| name == "start"),
            angle < 360.0,
            "{names:?}"
        );
        if angle < 360.0 {
            // The end cap follows the angle; the start cap stays in the XY plane.
            assert!(close(face(&ring, "start").centroid_mm.z, 0.0));
            let end = face(&ring, "end");
            let (sin, cos) = angle.to_radians().sin_cos();
            let radial = end.centroid_mm.x * cos - end.centroid_mm.z * sin;
            let off_plane = end.centroid_mm.x * sin + end.centroid_mm.z * cos;
            assert!(radial > 0.0 && off_plane.abs() < 1.0e-6, "{end:?}");
        }
    }
}

// (d) Booleans keep the names of both bodies, so a fillet can follow them.
#[test]
fn fillet_after_a_cut_and_an_intersect_rounds_the_named_edges() {
    let backend = ExactBackend::new();
    let block = backend
        .named_extrude(
            &rectangle(0.0, 0.0, 100.0, 60.0, ["front", "right", "back", "left"]),
            0.0,
            40.0,
        )
        .unwrap();
    // A through slot across the block, 20 wide and 10 deep from the top.
    let slot = backend
        .named_extrude(
            &rectangle(40.0, -1.0, 60.0, 61.0, ["a", "b", "c", "d"]),
            30.0,
            11.0,
        )
        .unwrap();
    let slotted = backend
        .named_boolean(&block, &slot, "slot", NamedBoolean::Cut)
        .unwrap();
    let removed = 20.0 * 60.0 * 10.0;
    assert!(close(
        slotted.output.body.topology.volume_mm3,
        100.0 * 60.0 * 40.0 - removed
    ));
    // The slot splits the top face in two; the side under it keeps its name.
    assert!(near(face(&slotted, "end#1").bounds_mm.max.x, 40.0));
    assert!(near(face(&slotted, "slot.start").centroid_mm.z, 30.0));

    let radius = 5.0;
    let rounded = backend
        .named_finish(&slotted, &[("right", "front")], EdgeFinish::Fillet, radius)
        .unwrap();
    let corner = radius * radius * (1.0 - std::f64::consts::FRAC_PI_4);
    assert!(
        (rounded.output.body.topology.volume_mm3
            - (100.0 * 60.0 * 40.0 - removed - corner * 40.0))
            .abs()
            < 1.0e-3,
        "{}",
        rounded.output.body.topology.volume_mm3
    );
    assert!(face(&rounded, "fillet(front,right)").centroid_mm.x > 95.0);

    // Intersect with a box that keeps the left half: the kept faces keep
    // their names and the new wall is named after the tool face.
    let half = backend
        .named_extrude(
            &rectangle(-1.0, -1.0, 50.0, 61.0, ["p", "cut_wall", "q", "r"]),
            -1.0,
            42.0,
        )
        .unwrap();
    let kept = backend
        .named_boolean(&block, &half, "half", NamedBoolean::Intersect)
        .unwrap();
    assert!(close(kept.output.body.topology.volume_mm3, 50.0 * 60.0 * 40.0));
    assert!(near(face(&kept, "half.cut_wall").centroid_mm.x, 50.0));
    assert!(close(face(&kept, "left").centroid_mm.x, 0.0));
    kept.edge("half.cut_wall", "front").unwrap();
}
