//! Borrowed GPU-ready views of the validated GLB: shared buffers, the node tree
//! with composed placements, and a parent-first traversal.

use super::{Info, accessor_data, source_digest, validated_document};
use ketchup_rejection::Rejection;

/// Borrowed display buffers: instances and material variants never copy vertices.
#[derive(Debug)]
pub struct Geometry<'a> {
    pub meshes: Vec<Vec<GeometryPrimitive<'a>>>,
    /// Indexed exactly like the GLB nodes and manifest occurrence references.
    pub nodes: Vec<GeometryNode>,
    /// Parent-before-child traversal, independent of the GLB node ordering.
    pub traversal: Vec<usize>,
}

/// One GLB node with its parent, optional mesh and composed placement.
#[derive(Clone, Debug)]
pub struct GeometryNode {
    pub parent: Option<usize>,
    pub mesh: Option<usize>,
    /// Column-major Y-up/metre placement, with the full ancestry composed.
    pub world_matrix: [f64; 16],
}

/// Faces or presentation edges.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrimitiveTopology {
    Triangles,
    Lines,
}

/// One primitive borrowing the shared position and index buffers.
#[derive(Debug)]
pub struct GeometryPrimitive<'a> {
    pub position_accessor: usize,
    pub index_accessor: usize,
    positions: &'a [u8],
    indices: &'a [u8],
    pub topology: PrimitiveTopology,
    pub color_linear: Option<[f64; 4]>,
}

impl GeometryPrimitive<'_> {
    /// Vertex positions in GLB Y-up metres.
    pub fn positions(&self) -> impl ExactSizeIterator<Item = [f32; 3]> + '_ {
        self.positions.chunks_exact(12).map(|point| {
            std::array::from_fn(|i| {
                f32::from_le_bytes(point[i * 4..i * 4 + 4].try_into().expect("validated VEC3"))
            })
        })
    }

    /// Vertex indices: three per triangle or two per line.
    pub fn indices(&self) -> impl ExactSizeIterator<Item = u32> + '_ {
        self.indices
            .chunks_exact(4)
            .map(|index| u32::from_le_bytes(index.try_into().expect("validated u32 index")))
    }

    /// Validated, aligned little-endian data ready for a single shared GPU upload.
    pub fn position_bytes(&self) -> &[u8] {
        self.positions
    }

    /// Validated little-endian u32 indices, ready for a GPU upload.
    pub fn index_bytes(&self) -> &[u8] {
        self.indices
    }
}

/// Validate the GLB and borrow its geometry; nothing is copied.
pub(crate) fn decode(bytes: &[u8]) -> Result<(Geometry<'_>, Info), Rejection> {
    let (doc, binary) = validated_document(bytes)?;
    let meshes = doc
        .meshes
        .iter()
        .map(|mesh| {
            mesh.primitives
                .iter()
                .map(|primitive| {
                    Ok(GeometryPrimitive {
                        position_accessor: primitive.attributes.position,
                        index_accessor: primitive.indices,
                        positions: accessor_data(
                            &doc,
                            &doc.accessors[primitive.attributes.position],
                            binary,
                        )?,
                        indices: accessor_data(&doc, &doc.accessors[primitive.indices], binary)?,
                        topology: if primitive.mode == 4 {
                            PrimitiveTopology::Triangles
                        } else {
                            PrimitiveTopology::Lines
                        },
                        color_linear: primitive
                            .material
                            .map(|i| doc.materials[i].pbr_metallic_roughness.base_color_factor),
                    })
                })
                .collect::<Result<Vec<_>, Rejection>>()
        })
        .collect::<Result<Vec<_>, Rejection>>()?;
    let mut nodes: Vec<_> = doc
        .nodes
        .iter()
        .map(|node| GeometryNode {
            parent: None,
            mesh: node.mesh,
            world_matrix: node.matrix,
        })
        .collect();
    for (parent, node) in doc.nodes.iter().enumerate() {
        for &child in &node.children {
            nodes[child].parent = Some(parent);
        }
    }
    let mut traversal = doc.scenes[0].nodes.clone();
    let mut cursor = 0;
    while cursor < traversal.len() {
        let parent = traversal[cursor];
        for &child in &doc.nodes[parent].children {
            nodes[child].world_matrix = std::array::from_fn(|i| {
                (0..4)
                    .map(|k| {
                        nodes[parent].world_matrix[k * 4 + i % 4]
                            * doc.nodes[child].matrix[i / 4 * 4 + k]
                    })
                    .sum()
            });
        }
        traversal.extend_from_slice(&doc.nodes[parent].children);
        cursor += 1;
    }
    let info = Info {
        source_digest: source_digest(&doc)?,
        nodes: doc.nodes,
    };
    Ok((
        Geometry {
            meshes,
            nodes,
            traversal,
        },
        info,
    ))
}
