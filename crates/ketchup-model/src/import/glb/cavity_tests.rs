//! Shells nested in a solid face out by their nesting, a cavity into the solid's
//! inside (review 2026-11-13 r95 R1113r95-SKP-700: every closed shell was turned to a
//! positive volume, so a cube with a cavity came in as its outside plus the cavity).

use super::*;

/// The corners and triangles of a cube of side `side` about `centre`, wound to face
/// out, or in when `inward`, numbered from `first`.
fn cube(centre: [f64; 3], side: f64, inward: bool, first: u32) -> (Vec<[f64; 3]>, Vec<[u32; 3]>) {
    let half = side / 2.0;
    let corners = (0..8)
        .map(|corner| {
            std::array::from_fn(|axis| {
                centre[axis] + if corner >> axis & 1 == 1 { half } else { -half }
            })
        })
        .collect();
    let faces = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let triangles = faces
        .iter()
        .flat_map(|[a, b, c, d]| [[*a, *b, *c], [*a, *c, *d]])
        .map(|triangle: [u32; 3]| {
            let [a, b, c] = triangle.map(|corner| corner + first);
            if inward { [a, c, b] } else { [a, b, c] }
        })
        .collect();
    (corners, triangles)
}

/// One piece of the cubes `(side, inward)` about the origin.
fn nested(cubes: &[(f64, bool)]) -> ParsedPrimitive {
    let mut piece = ParsedPrimitive {
        name: "nested".to_owned(),
        vertices_mm: Vec::new(),
        triangles: Vec::new(),
        color: None,
        look: None,
        sheet: false,
        origin_mm: [0.0; 3],
    };
    for (side, inward) in cubes {
        let first = u32::try_from(piece.vertices_mm.len()).unwrap();
        let (corners, triangles) = cube([0.0; 3], *side, *inward, first);
        piece.vertices_mm.extend(corners);
        piece.triangles.extend(triangles);
    }
    piece
}

fn volume(piece: &ParsedPrimitive) -> f64 {
    piece
        .triangles
        .iter()
        .map(|[a, b, c]| {
            let [p, q, r] = [a, b, c].map(|index| piece.vertices_mm[*index as usize]);
            ketchup_geometry::linalg::dot(p, ketchup_geometry::linalg::cross(q, r)) / 6.0
        })
        .sum()
}

/// A cube 30 with a cavity 10, wound any way, holds 30³ − 10³; a cube 10 within a
/// cavity 20 of a cube 30 adds back its own.
#[test]
fn a_cavity_faces_into_the_solid_however_it_was_wound() {
    for (outer, inner) in [(false, true), (false, false), (true, true), (true, false)] {
        let mut piece = nested(&[(30.0, outer), (10.0, inner)]);
        orient::face_out(&mut piece);
        let held = volume(&piece);
        assert!((held - 26_000.0).abs() < 1e-6, "{outer} {inner}: {held}");
        assert!(is_solid(&piece), "{outer} {inner}");
    }
    let mut island = nested(&[(30.0, true), (20.0, false), (10.0, true)]);
    orient::face_out(&mut island);
    let held = volume(&island);
    assert!(
        (held - (27_000.0 - 8_000.0 + 1_000.0)).abs() < 1e-6,
        "{held}"
    );
}

/// Review 2026-11-13 r95 P3: a closed box with one face more of its material, apart from
/// it or a fin on one of its edges, came in all as a sheet. The box comes in as a solid
/// and the face as a sheet of its own.
#[test]
fn a_closed_box_with_a_face_more_comes_in_as_a_solid_and_a_sheet() {
    // Corners 8 - 10 lie apart from the box; 1 and 3 end one of its edges.
    for (case, fin) in [("apart", [8, 9, 10]), ("on an edge", [1, 3, 10])] {
        let mut piece = nested(&[(30.0, false)]);
        piece
            .vertices_mm
            .extend([[40.0, 0.0, 0.0], [50.0, 0.0, 0.0], [25.0, 0.0, -15.0]]);
        piece.triangles.push(fin);
        let pieces =
            closed_shells("box", vec![piece], &mut Vec::new(), &mut BTreeMap::new()).unwrap();
        let solids: Vec<_> = pieces.iter().filter(|piece| is_solid(piece)).collect();
        assert_eq!(solids.len(), 1, "{case}");
        assert!((volume(solids[0]) - 27_000.0).abs() < 1e-6, "{case}");
        let sheets: Vec<_> = pieces.iter().filter(|piece| piece.sheet).collect();
        assert_eq!(sheets.len(), 1, "{case}");
        assert_eq!(sheets[0].triangles.len(), 1, "{case}");
    }
}
