//! A component stretched with SketchUp's Scale tool (review 2026-11-19 og SKP-800). Its
//! group came in with a transform that scales, so every part under it was a scaled
//! placement: the workshop export refused the whole model, the mass had no volume and
//! collisions no box. Each group keeps only the move, turn or mirror of where it lies;
//! the stretch goes into the geometry of the parts under it, a definition of their own
//! beside the one unstretched placements share. Placed in the model, nothing moves.

use std::collections::BTreeMap;

use super::{
    GlbImportError, GlbMeasure, MAX_PRIMITIVES, MAX_TOTAL_TRIANGLES, MAX_TOTAL_VERTICES,
    ParsedNode, ParsedPrimitive, document_transform, recentre, too_large, translation,
};
use crate::tolerance::ROUNDING;
use ketchup_geometry::linalg::{Affine3, Mat3, Singular, Vec3};

/// Takes the stretch out of the placed transforms of `nodes` into own definitions in
/// `primitives`, and answers how many parts were stretched; a scene without a stretch
/// stays as it is.
pub(super) fn bake_stretches(
    nodes: &mut [ParsedNode],
    traversal: &[usize],
    primitives: &mut Vec<ParsedPrimitive>,
) -> Result<usize, GlbImportError> {
    let mut parent_by_node = vec![None; nodes.len()];
    for index in traversal {
        for child in &nodes[*index].children {
            parent_by_node[*child] = Some(*index);
        }
    }
    // Where each group lies in the model, and the move, turn or mirror of that alone.
    let mut world = vec![Affine3::IDENTITY; nodes.len()];
    let mut rigid = vec![Affine3::IDENTITY; nodes.len()];
    let mut reframed = vec![false; nodes.len()];
    // The own definition of a piece under a stretch, and where its origin moved.
    let mut baked = BTreeMap::<(usize, [u64; 9]), (usize, Vec3)>::new();
    let mut stretched_parts = 0_usize;
    // What the copies alone hold: past what one import stores, [`check_stored`] refuses
    // them anyway, so they stop here before they are made (R1120ac-SKP-900).
    let mut copied_vertices = 0_usize;
    let mut copied_triangles = 0_usize;
    for index in traversal.iter().copied() {
        let parent = parent_by_node[index];
        let placed = parent
            .map_or(Affine3::IDENTITY, |parent| world[parent])
            .compose(nodes[index].transform.affine());
        world[index] = placed;
        let stretched = !placed.linear.is_orthonormal(ROUNDING);
        if !stretched && !parent.is_some_and(|parent| reframed[parent]) {
            rigid[index] = placed;
            continue;
        }
        let (turn, stretch) = polar(placed.linear)?;
        rigid[index] = Affine3 {
            linear: turn,
            translation: placed.translation,
        };
        reframed[index] = true;
        let parent_rigid = parent.map_or(Affine3::IDENTITY, |parent| rigid[parent]);
        nodes[index].transform =
            document_transform(rigid_inverse(parent_rigid).compose(rigid[index]))?;
        if !stretched {
            continue;
        }
        let key_of_stretch: [u64; 9] =
            std::array::from_fn(|entry| stretch.rows[entry / 3][entry % 3].to_bits());
        let node = &mut nodes[index];
        for (primitive, placement) in node.primitives.iter_mut().zip(&mut node.placements) {
            let (own, origin) = match baked.get(&(*primitive, key_of_stretch)) {
                Some(found) => *found,
                None => {
                    let source = &primitives[*primitive];
                    copied_vertices = copied_vertices.saturating_add(source.vertices_mm.len());
                    copied_triangles = copied_triangles.saturating_add(source.triangles.len());
                    if copied_vertices > MAX_TOTAL_VERTICES {
                        return Err(too_large(
                            GlbMeasure::StretchedVertices,
                            copied_vertices,
                            MAX_TOTAL_VERTICES,
                        ));
                    }
                    if copied_triangles > MAX_TOTAL_TRIANGLES {
                        return Err(too_large(
                            GlbMeasure::StretchedTriangles,
                            copied_triangles,
                            MAX_TOTAL_TRIANGLES,
                        ));
                    }
                    let mut piece = source.clone();
                    for point in &mut piece.vertices_mm {
                        let stretched = stretch.mul_vec(Vec3::from(*point));
                        *point = [stretched.x, stretched.y, stretched.z];
                    }
                    let origin = Vec3::from(recentre(std::slice::from_mut(&mut piece))?);
                    piece.origin_mm = [origin.x, origin.y, origin.z];
                    primitives.push(piece);
                    let found = (primitives.len() - 1, origin);
                    baked.insert((*primitive, key_of_stretch), found);
                    found
                }
            };
            let offset = placement.affine().translation;
            *placement = document_transform(translation(stretch.mul_vec(offset) + origin))?;
            *primitive = own;
            stretched_parts += 1;
        }
    }
    if stretched_parts > 0 {
        drop_unplaced(nodes, traversal, primitives);
    }
    Ok(stretched_parts)
}

/// Checks the pieces against what one import stores, and answers their triangles.
pub(super) fn check_stored(primitives: &[ParsedPrimitive]) -> Result<usize, GlbImportError> {
    if primitives.len() > MAX_PRIMITIVES {
        return Err(too_large(
            GlbMeasure::MeshParts,
            primitives.len(),
            MAX_PRIMITIVES,
        ));
    }
    let vertices = primitives
        .iter()
        .map(|piece| piece.vertices_mm.len())
        .sum::<usize>();
    if vertices > MAX_TOTAL_VERTICES {
        return Err(too_large(
            GlbMeasure::Vertices,
            vertices,
            MAX_TOTAL_VERTICES,
        ));
    }
    let triangles = primitives
        .iter()
        .map(|piece| piece.triangles.len())
        .sum::<usize>();
    if triangles > MAX_TOTAL_TRIANGLES {
        return Err(too_large(
            GlbMeasure::Triangles,
            triangles,
            MAX_TOTAL_TRIANGLES,
        ));
    }
    Ok(triangles)
}

/// Removes the pieces no placement shows any more: those only stretched parts showed.
fn drop_unplaced(
    nodes: &mut [ParsedNode],
    traversal: &[usize],
    primitives: &mut Vec<ParsedPrimitive>,
) {
    let mut placed = vec![false; primitives.len()];
    for index in traversal {
        for primitive in &nodes[*index].primitives {
            placed[*primitive] = true;
        }
    }
    let mut renumbered = vec![0; primitives.len()];
    let mut kept = 0;
    for (index, is_placed) in placed.iter().enumerate() {
        renumbered[index] = kept;
        kept += usize::from(*is_placed);
    }
    let mut keep = placed.iter().copied();
    primitives.retain(|_piece| keep.next().unwrap_or(true));
    for node in traversal {
        for primitive in &mut nodes[*node].primitives {
            *primitive = renumbered[*primitive];
        }
    }
}

/// The inverse of a move, turn or mirror.
fn rigid_inverse(rigid: Affine3) -> Affine3 {
    let linear = rigid.linear.transpose();
    Affine3 {
        linear,
        translation: -linear.mul_vec(rigid.translation),
    }
}

/// `linear` as a turn or mirror after a stretch along three square axes:
/// `linear = turn · stretch` (the polar decomposition, by Newton's iteration).
fn polar(linear: Mat3) -> Result<(Mat3, Mat3), GlbImportError> {
    let mut turn = linear;
    for _ in 0..100 {
        let inverse = turn
            .inverse()
            .map_err(|Singular| GlbImportError::InvalidTransform)?;
        let next = Mat3::from_rows(std::array::from_fn(|row| {
            std::array::from_fn(|column| {
                f64::midpoint(turn.rows[row][column], inverse.rows[column][row])
            })
        }));
        let settled = (0..9).all(|entry| {
            (next.rows[entry / 3][entry % 3] - turn.rows[entry / 3][entry % 3]).abs()
                <= 4.0 * f64::EPSILON
        });
        turn = next;
        if settled {
            break;
        }
    }
    if !turn.is_orthonormal(ROUNDING) {
        return Err(GlbImportError::InvalidTransform);
    }
    Ok((turn, turn.transpose() * linear))
}
