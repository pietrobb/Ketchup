//! Closed shells are turned to face out by their geometry, whatever way the source
//! wound their faces (review 2026-11-13 R1113-SKP-600: a SketchUp box painted from
//! inside, or with a face SketchUp shows reversed, came as no solid when winding
//! followed the paint).

use super::ParsedPrimitive;
use std::collections::BTreeMap;

/// Turns the triangles of `piece` so that each closed, orientable shell of it is wound
/// alike, neighbour by neighbour across every edge, and encloses a positive volume, and
/// answers which triangles turned. A piece with an edge not shared by exactly two
/// triangles, or one no winding makes consistent, stays as it is and turns none.
pub(super) fn face_out(piece: &mut ParsedPrimitive) -> Vec<bool> {
    let triangles = &piece.triangles;
    let none = vec![false; triangles.len()];
    let mut edges = BTreeMap::<(u32, u32), Vec<(usize, bool)>>::new();
    for (index, [a, b, c]) in triangles.iter().copied().enumerate() {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges
                .entry((from.min(to), from.max(to)))
                .or_default()
                .push((index, from < to));
        }
    }
    if edges.values().any(|uses| uses.len() != 2) {
        return none;
    }
    let mut neighbours = vec![Vec::new(); triangles.len()];
    for uses in edges.values() {
        let [(first, forward), (second, along)] = [uses[0], uses[1]];
        // Neighbours wound alike run along their shared edge in opposite directions.
        let differ = forward == along;
        neighbours[first].push((second, differ));
        neighbours[second].push((first, differ));
    }
    let mut turned: Vec<Option<bool>> = vec![None; triangles.len()];
    for start in 0..triangles.len() {
        if turned[start].is_some() {
            continue;
        }
        turned[start] = Some(false);
        let mut shell = vec![start];
        let mut queue = vec![start];
        while let Some(item) = queue.pop() {
            let own = turned[item].unwrap_or(false);
            for &(next, differ) in &neighbours[item] {
                let wanted = own ^ differ;
                match turned[next] {
                    Some(already) if already != wanted => return none,
                    Some(_) => {}
                    None => {
                        turned[next] = Some(wanted);
                        shell.push(next);
                        queue.push(next);
                    }
                }
            }
        }
        if volume(piece, &shell, &turned) < 0.0 {
            for item in shell {
                turned[item] = turned[item].map(|turn| !turn);
            }
        }
    }
    let turned: Vec<bool> = turned.into_iter().map(Option::unwrap_or_default).collect();
    for (triangle, turn) in piece.triangles.iter_mut().zip(&turned) {
        if *turn {
            triangle.swap(1, 2);
        }
    }
    if let Some(look) = piece.look.as_mut() {
        turn_look(look, &turned);
    }
    turned
}

/// Turns the pictures of the triangles `turned` as [`face_out`] turned them.
pub(super) fn turn_look(look: &mut [Option<super::TexturedTriangle>], turned: &[bool]) {
    for (triangle, turn) in look.iter_mut().zip(turned) {
        if let Some(triangle) = triangle.as_mut().filter(|_| *turn) {
            triangle.uv.swap(1, 2);
        }
    }
}

/// Six times the volume the triangles `shell` of `piece` enclose, wound as `turned`
/// says, about the shell's first corner.
fn volume(piece: &ParsedPrimitive, shell: &[usize], turned: &[Option<bool>]) -> f64 {
    let corner = |index: u32| piece.vertices_mm[index as usize];
    let origin = corner(piece.triangles[shell[0]][0]);
    shell
        .iter()
        .map(|item| {
            let [a, b, c] = piece.triangles[*item];
            let [a, b, c] = if turned[*item].unwrap_or(false) {
                [a, c, b]
            } else {
                [a, b, c]
            };
            let [p, q, r] = [a, b, c].map(|index| {
                let point = corner(index);
                std::array::from_fn::<f64, 3, _>(|axis| point[axis] - origin[axis])
            });
            p[0] * (q[1] * r[2] - q[2] * r[1])
                + p[1] * (q[2] * r[0] - q[0] * r[2])
                + p[2] * (q[0] * r[1] - q[1] * r[0])
        })
        .sum()
}
