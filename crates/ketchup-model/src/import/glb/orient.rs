//! Closed shells are turned to face out by their geometry, whatever way the source
//! wound their faces (review 2026-11-13 R1113-SKP-600: a SketchUp box painted from
//! inside, or with a face SketchUp shows reversed, came as no solid when winding
//! followed the paint).

use super::ParsedPrimitive;
use ketchup_geometry::linalg;
use std::collections::BTreeMap;

/// Turns the triangles of `piece` so that each closed, orientable shell of it is wound
/// alike, neighbour by neighbour across every edge, and faces out of the solid: an
/// outer shell encloses a positive volume, one bounding a cavity a negative one. It
/// answers which triangles turned. A piece with an edge not shared by exactly two
/// triangles, or one no winding makes consistent, stays as it is and turns none.
pub(super) fn face_out(piece: &mut ParsedPrimitive) -> Vec<bool> {
    let triangles = &piece.triangles;
    let none = vec![false; triangles.len()];
    let edges = edge_uses(triangles);
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
    let mut shells = Vec::new();
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
            flip(&mut turned, &shell);
        }
        shells.push(shell);
    }
    // A shell inside an odd number of others bounds a cavity and faces into the
    // solid around it (review 2026-11-13 r95 R1113r95-SKP-700).
    let inside: Vec<bool> = shells
        .iter()
        .enumerate()
        .map(|(index, shell)| {
            let point = centroid(piece, shell[0]);
            let around = shells
                .iter()
                .enumerate()
                .filter(|(other, around)| {
                    *other != index && encloses(piece, around, &turned, point)
                })
                .count();
            around % 2 == 1
        })
        .collect();
    for (shell, inside) in shells.iter().zip(inside) {
        if inside {
            flip(&mut turned, shell);
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

/// Splits off the closed shells of `piece` from the faces that close none: its
/// triangles joined across edges two of them share, each group closed when every edge
/// of it runs along two of its own triangles. A piece all closed or all open stays
/// whole; otherwise it comes as its closed shells, then its open rest (review
/// 2026-11-13 r95 P3: a closed box with a face more of its material, apart or on one of
/// its edges, came in all as a sheet).
pub(super) fn split_closed(piece: ParsedPrimitive) -> Vec<ParsedPrimitive> {
    let edges = edge_uses(&piece.triangles);
    let mut group: Vec<usize> = (0..piece.triangles.len()).collect();
    for uses in edges.values().filter(|uses| uses.len() == 2) {
        let (first, second) = (root(&mut group, uses[0].0), root(&mut group, uses[1].0));
        group[first] = second;
    }
    let roots: Vec<usize> = (0..piece.triangles.len())
        .map(|item| root(&mut group, item))
        .collect();
    let mut open = BTreeMap::<usize, bool>::new();
    for uses in edges.values() {
        let mut by_group = BTreeMap::<usize, usize>::new();
        for (item, _) in uses {
            *by_group.entry(roots[*item]).or_default() += 1;
        }
        for (owner, count) in by_group {
            *open.entry(owner).or_default() |= count != 2;
        }
    }
    let closed: Vec<bool> = roots.iter().map(|owner| !open[owner]).collect();
    if closed.iter().all(|shut| *shut) || !closed.iter().any(|shut| *shut) {
        return vec![piece];
    }
    [true, false]
        .into_iter()
        .map(|wanted| part_of(&piece, |item| closed[item] == wanted))
        .collect()
}

/// The first triangle of the group of `item`, the groups' trees shortened on the way.
fn root(group: &mut [usize], mut item: usize) -> usize {
    while group[item] != item {
        group[item] = group[group[item]];
        item = group[item];
    }
    item
}

/// The triangles of `piece` that `keep` keeps, over the corners they use.
fn part_of(piece: &ParsedPrimitive, keep: impl Fn(usize) -> bool) -> ParsedPrimitive {
    let mut renumbered = BTreeMap::<u32, u32>::new();
    let mut vertices_mm = Vec::new();
    let mut triangles = Vec::new();
    let mut look = piece.look.as_ref().map(|_| Vec::new());
    for (item, triangle) in piece.triangles.iter().enumerate() {
        if !keep(item) {
            continue;
        }
        triangles.push(triangle.map(|index| {
            *renumbered.entry(index).or_insert_with(|| {
                vertices_mm.push(piece.vertices_mm[usize::try_from(index).unwrap_or(usize::MAX)]);
                u32::try_from(vertices_mm.len() - 1).unwrap_or(u32::MAX)
            })
        }));
        if let (Some(look), Some(own)) = (look.as_mut(), piece.look.as_ref()) {
            look.push(own[item]);
        }
    }
    ParsedPrimitive {
        name: piece.name.clone(),
        vertices_mm,
        triangles,
        color: piece.color,
        look,
        sheet: piece.sheet,
        origin_mm: piece.origin_mm,
    }
}

/// Each edge of `triangles`, lesser corner first, with the triangles along it and
/// whether each runs it from the lesser corner.
fn edge_uses(triangles: &[[u32; 3]]) -> BTreeMap<(u32, u32), Vec<(usize, bool)>> {
    let mut edges = BTreeMap::<(u32, u32), Vec<(usize, bool)>>::new();
    for (index, [a, b, c]) in triangles.iter().copied().enumerate() {
        for (from, to) in [(a, b), (b, c), (c, a)] {
            edges
                .entry((from.min(to), from.max(to)))
                .or_default()
                .push((index, from < to));
        }
    }
    edges
}

/// Turns the pictures of the triangles `turned` as [`face_out`] turned them.
pub(super) fn turn_look(look: &mut [Option<super::TexturedTriangle>], turned: &[bool]) {
    for (triangle, turn) in look.iter_mut().zip(turned) {
        if let Some(triangle) = triangle.as_mut().filter(|_| *turn) {
            triangle.uv.swap(1, 2);
        }
    }
}

fn flip(turned: &mut [Option<bool>], shell: &[usize]) {
    for item in shell {
        turned[*item] = turned[*item].map(|turn| !turn);
    }
}

/// The corners of triangle `item` of `piece`, wound as `turned` says.
fn corners(piece: &ParsedPrimitive, item: usize, turned: &[Option<bool>]) -> [[f64; 3]; 3] {
    // The parser checked every index against the vertices.
    let corner = |index: u32| piece.vertices_mm[usize::try_from(index).unwrap_or(usize::MAX)];
    let [a, b, c] = piece.triangles[item];
    let order = if turned.get(item).copied().flatten().unwrap_or(false) {
        [a, c, b]
    } else {
        [a, b, c]
    };
    order.map(corner)
}

fn centroid(piece: &ParsedPrimitive, item: usize) -> [f64; 3] {
    let [a, b, c] = corners(piece, item, &[]);
    std::array::from_fn(|axis| (a[axis] + b[axis] + c[axis]) / 3.0)
}

/// Six times the volume the triangles `shell` of `piece` enclose, wound as `turned`
/// says, about the shell's first corner.
fn volume(piece: &ParsedPrimitive, shell: &[usize], turned: &[Option<bool>]) -> f64 {
    let origin = corners(piece, shell[0], turned)[0];
    shell
        .iter()
        .map(|item| {
            let [p, q, r] = corners(piece, *item, turned).map(|at| linalg::sub(at, origin));
            linalg::dot(p, linalg::cross(q, r))
        })
        .sum()
}

/// Whether the closed `shell` of `piece` encloses `point`: the solid angle it
/// spans about the point, its winding number, is a whole turn, not none.
fn encloses(
    piece: &ParsedPrimitive,
    shell: &[usize],
    turned: &[Option<bool>],
    point: [f64; 3],
) -> bool {
    let angle: f64 = shell
        .iter()
        .map(|item| {
            let [a, b, c] = corners(piece, *item, turned).map(|at| linalg::sub(at, point));
            let [la, lb, lc] = [a, b, c].map(linalg::length);
            let above = linalg::dot(a, linalg::cross(b, c));
            let below = la * lb * lc
                + linalg::dot(a, b) * lc
                + linalg::dot(a, c) * lb
                + linalg::dot(b, c) * la;
            2.0 * above.atan2(below)
        })
        .sum();
    angle.abs() > 2.0 * std::f64::consts::PI
}
