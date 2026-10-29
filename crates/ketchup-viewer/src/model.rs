//! An opened package prepared for display: bodies with source Z-up/mm
//! placements, their owning components and bounds.

use ketchup_rejection::{Rejection, RejectionPhase};
use ketchup_view_format::{Geometry, IdentifiedPackage, Manifest, Package, PrimitiveTopology};
use num_traits::ToPrimitive;
use std::{collections::BTreeMap, io::Read, ops::Range, sync::Arc};

/// `[min, max]` corners in source millimetres.
type Bounds = [[f64; 3]; 2];

/// Read-only package and placements, not duplicated meshes per occurrence.
#[derive(Debug)]
pub struct Model {
    /// Shared with a phone's session writer, which keeps it on a thread of
    /// its own (VW-10).
    package: Arc<IdentifiedPackage>,
    pub bodies: Vec<Body>,
    pub bounds_mm: Bounds,
    /// Per manifest occurrence: column-major source Z-up/mm placement.
    pub occurrence_matrices_mm: Vec<[f64; 16]>,
    /// Per mesh: where each triangle primitive's positions and indices lie in
    /// the validated GLB, so picking reads them without validating it again.
    triangles: Vec<Vec<(Range<usize>, Range<usize>)>>,
    /// The same for each mesh's presentation edges (line primitives).
    lines: Vec<Vec<(Range<usize>, Range<usize>)>>,
}

/// One mesh placed in the scene, with the component it belongs to.
#[derive(Clone, Debug)]
pub struct Body {
    pub glb_node: usize,
    pub mesh: usize,
    /// Manifest occurrence index of the nearest enclosing component, not its name.
    pub occurrence: usize,
    /// Column-major placement for source Z-up/mm local vertex buffers.
    pub world_matrix_mm: [f32; 16],
    pub bounds_mm: Bounds,
    /// The mesh's own bounds before placement.
    pub mesh_bounds_mm: Bounds,
    /// Column-major source placement in full precision.
    pub world_matrix_f64_mm: [f64; 16],
}

impl Model {
    /// Read a `.ketchup-view` package and prepare it for display.
    pub fn read(reader: impl Read) -> Result<Self, Rejection> {
        Self::from_package(Package::read(reader)?)
    }

    /// Place every mesh node and assign it to its nearest enclosing component.
    pub fn from_package(package: Package) -> Result<Self, Rejection> {
        Self::from_identified(IdentifiedPackage::new(package)?)
    }

    /// [`Self::from_package`] for a package whose identity is already known.
    pub fn from_identified(identified: IdentifiedPackage) -> Result<Self, Rejection> {
        let package = identified.package();
        let geometry = package.geometry()?;
        let bounds = mesh_bounds(&geometry);
        let owner: BTreeMap<_, _> = package
            .manifest
            .occurrences
            .iter()
            .enumerate()
            .map(|(i, occurrence)| (occurrence.glb_node, i))
            .collect();
        let mut enclosing = vec![None; geometry.nodes.len()];
        let mut bodies = Vec::new();
        let mut total_bounds = empty_bounds();
        for &index in &geometry.traversal {
            let node = &geometry.nodes[index];
            enclosing[index] = owner
                .get(&index)
                .copied()
                .or_else(|| node.parent.and_then(|parent| enclosing[parent]));
            let Some(mesh) = node.mesh else {
                continue;
            };
            let occurrence = enclosing[index].ok_or_else(|| {
                invalid(
                    "geometry owner",
                    "A display body has no enclosing source component.",
                )
            })?;
            let matrix = source_matrix(node.world_matrix);
            let world_bounds = transformed_bounds(bounds[mesh], matrix)?;
            let mut world_matrix_mm = [0.0; 16];
            for (output, value) in world_matrix_mm.iter_mut().zip(matrix) {
                *output = value.to_f32().filter(|v| v.is_finite()).ok_or_else(|| {
                    invalid(
                        "placement",
                        "A placement exceeds the renderer's finite numeric range.",
                    )
                })?;
            }
            merge_bounds(&mut total_bounds, world_bounds);
            bodies.push(Body {
                glb_node: index,
                mesh,
                occurrence,
                world_matrix_mm,
                bounds_mm: world_bounds,
                mesh_bounds_mm: bounds[mesh],
                world_matrix_f64_mm: matrix,
            });
        }
        if bodies.is_empty() {
            return Err(invalid("geometry", "The package has no display bodies."));
        }
        let occurrence_matrices_mm = package
            .manifest
            .occurrences
            .iter()
            .map(|occurrence| {
                geometry
                    .nodes
                    .get(occurrence.glb_node)
                    .map(|node| source_matrix(node.world_matrix))
                    .ok_or_else(|| invalid("occurrence", "A component has no geometry node."))
            })
            .collect::<Result<_, _>>()?;
        let ranges = |topology: PrimitiveTopology| -> Vec<Vec<_>> {
            geometry
                .meshes
                .iter()
                .map(|primitives| {
                    primitives
                        .iter()
                        .filter(|p| p.topology == topology)
                        .map(|p| (p.position_range.clone(), p.index_range.clone()))
                        .collect()
                })
                .collect()
        };
        let triangles = ranges(PrimitiveTopology::Triangles);
        let lines = ranges(PrimitiveTopology::Lines);
        Ok(Self {
            package: Arc::new(identified),
            bodies,
            bounds_mm: total_bounds,
            occurrence_matrices_mm,
            triangles,
            lines,
        })
    }

    /// The position and index bytes of each triangle primitive of `mesh`.
    pub fn triangle_buffers(&self, mesh: usize) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.triangles[mesh].iter().map(|(positions, indices)| {
            (
                &self.package.package().glb[positions.clone()],
                &self.package.package().glb[indices.clone()],
            )
        })
    }

    /// The position and index bytes of each line primitive (presentation
    /// edges) of `mesh`, two indices per segment.
    pub fn line_buffers(&self, mesh: usize) -> impl Iterator<Item = (&[u8], &[u8])> {
        self.lines[mesh].iter().map(|(positions, indices)| {
            (
                &self.package.package().glb[positions.clone()],
                &self.package.package().glb[indices.clone()],
            )
        })
    }

    /// The package manifest, unchanged.
    pub fn manifest(&self) -> &Manifest {
        &self.package.package().manifest
    }

    /// The opened package, unchanged.
    pub fn package(&self) -> &Package {
        self.package.package()
    }

    /// The opened package with its export identity, hashed once.
    pub fn export(&self) -> &IdentifiedPackage {
        &self.package
    }

    /// [`Self::export`], to be written elsewhere while the model is shown.
    pub fn shared_export(&self) -> Arc<IdentifiedPackage> {
        Arc::clone(&self.package)
    }

    /// The package geometry, borrowed and revalidated.
    pub fn geometry(&self) -> Result<Geometry<'_>, Rejection> {
        self.package.package().geometry()
    }
}

/// The exporter uses [x,z,-y] in metres; restore source axes and units once.
pub fn source_point(point: [f32; 3]) -> [f64; 3] {
    [
        f64::from(point[0]) * 1000.0,
        -f64::from(point[2]) * 1000.0,
        f64::from(point[1]) * 1000.0,
    ]
}

/// The GLB position (Y-up metres) that the `index`-th index of a primitive's
/// validated buffers names.
pub fn indexed_point(positions: &[u8], indices: &[u8], index: usize) -> [f32; 3] {
    let at = index * 4;
    let vertex = u32::from_le_bytes(indices[at..at + 4].try_into().expect("validated index"));
    let offset = usize::try_from(vertex).expect("bounded index") * 12;
    std::array::from_fn(|k| {
        f32::from_le_bytes(
            positions[offset + k * 4..offset + k * 4 + 4]
                .try_into()
                .expect("validated point"),
        )
    })
}

/// A GLB Y-up/metre matrix in source Z-up/millimetre axes.
fn source_matrix(matrix: [f64; 16]) -> [f64; 16] {
    let axes = [0, 2, 1];
    let signs = [1.0, -1.0, 1.0];
    let mut result = [0.0; 16];
    for col in 0..3 {
        for row in 0..3 {
            result[col * 4 + row] = signs[row] * signs[col] * matrix[axes[col] * 4 + axes[row]];
        }
        result[12 + col] = signs[col] * matrix[12 + axes[col]] * 1000.0;
    }
    result[15] = 1.0;
    result
}

/// Local bounds of every mesh, in source millimetres.
fn mesh_bounds(geometry: &Geometry<'_>) -> Vec<Bounds> {
    // Accessor IDs reuse the GLB's sharing; scan each position buffer only once.
    let mut accessors = BTreeMap::new();
    geometry
        .meshes
        .iter()
        .map(|mesh| {
            let mut bounds = empty_bounds();
            for primitive in mesh {
                let local = accessors
                    .entry(primitive.position_accessor)
                    .or_insert_with(|| {
                        let mut bounds = empty_bounds();
                        for point in primitive.positions().map(source_point) {
                            merge_bounds(&mut bounds, [point, point]);
                        }
                        bounds
                    });
                merge_bounds(&mut bounds, *local);
            }
            bounds
        })
        .collect()
}

/// Bounds of the eight placed corners; refused when not finite.
fn transformed_bounds(bounds: Bounds, matrix: [f64; 16]) -> Result<Bounds, Rejection> {
    let mut world = empty_bounds();
    for bits in 0..8 {
        let local: [f64; 3] = std::array::from_fn(|i| bounds[(bits >> i) & 1][i]);
        let point = std::array::from_fn(|i| {
            matrix[12 + i] + (0..3).map(|k| matrix[k * 4 + i] * local[k]).sum::<f64>()
        });
        if point.iter().any(|v| !v.is_finite() || v.abs() > 1e9) {
            return Err(invalid(
                "placement",
                "World display bounds must be finite and within ±1e9 mm.",
            ));
        }
        merge_bounds(&mut world, [point, point]);
    }
    Ok(world)
}

fn empty_bounds() -> Bounds {
    [[f64::INFINITY; 3], [f64::NEG_INFINITY; 3]]
}

fn merge_bounds(bounds: &mut Bounds, other: Bounds) {
    for i in 0..3 {
        bounds[0][i] = bounds[0][i].min(other[0][i]);
        bounds[1][i] = bounds[1][i].max(other[1][i]);
    }
}

fn invalid(target: &str, reason: &str) -> Rejection {
    Rejection::new("viewer.model.invalid", RejectionPhase::Request)
        .target(target)
        .reason(reason)
        .fix_hint("Re-export a supported .ketchup-view with bounded source component placements.")
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use ketchup_view_format::{Definition, InstancePath, LocalDimensions, Occurrence, Source};
    use serde_json::json;

    pub(crate) fn package(translation: f64) -> Package {
        let identity = [
            1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.005, 0.009, -0.007, 1.0,
        ];
        let doc = json!({
            "asset":{"version":"2.0","generator":"Ketchup","extras":{
                "ketchupSourceDigest":"aaaaaaaaaaaaaaaa","ketchupSourceUnit":"millimetre",
                "unit":"metre","upAxis":"Y"}},
            "scene":0,"scenes":[{"nodes":[1,3]}],
            "nodes":[{"matrix":identity,"mesh":0},
                {"matrix":[0.0,0.0,-1.0,0.0, 0.0,1.0,0.0,0.0,
                    1.0,0.0,0.0,0.0, translation,0.3,-0.2,1.0],"children":[0],
                    "extras":{"ketchupEntity":"occurrence","ketchupDefinitionId":1,
                        "ketchupInstancePath":"occurrence:10"}},
                {"matrix":identity,"mesh":0},
                {"matrix":[1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0,
                    0.0,0.0,1.0,0.0, 0.5,0.3,-0.2,1.0],"children":[2],
                    "extras":{"ketchupEntity":"occurrence","ketchupDefinitionId":1,
                        "ketchupInstancePath":"occurrence:11"}}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"indices":1,"mode":4},
                {"attributes":{"POSITION":0},"indices":2,"mode":1}]}],
            "buffers":[{"byteLength":72}],
            "bufferViews":[{"buffer":0,"byteLength":36},
                {"buffer":0,"byteOffset":36,"byteLength":12},
                {"buffer":0,"byteOffset":48,"byteLength":24}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"},
                {"bufferView":1,"componentType":5125,"count":3,"type":"SCALAR"},
                {"bufferView":2,"componentType":5125,"count":6,"type":"SCALAR"}]
        });
        let mut json = serde_json::to_vec(&doc).expect("JSON");
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut binary = Vec::new();
        for value in [0.0_f32, 0.0, 0.0, 0.01, 0.0, 0.0, 0.0, 0.01, -0.01] {
            binary.extend_from_slice(&value.to_le_bytes());
        }
        for index in [0_u32, 1, 2, 0, 1, 1, 2, 2, 0] {
            binary.extend_from_slice(&index.to_le_bytes());
        }
        let mut glb = b"glTF".to_vec();
        glb.extend_from_slice(&2_u32.to_le_bytes());
        glb.extend_from_slice(
            &u32::try_from(28 + json.len() + binary.len())
                .expect("length")
                .to_le_bytes(),
        );
        glb.extend_from_slice(
            &u32::try_from(json.len())
                .expect("JSON length")
                .to_le_bytes(),
        );
        glb.extend_from_slice(b"JSON");
        glb.extend(json);
        glb.extend_from_slice(&72_u32.to_le_bytes());
        glb.extend_from_slice(b"BIN\0");
        glb.extend(binary);
        Package {
            glb,
            manifest: Manifest {
                source: Source {
                    document_id: 1,
                    revision: 2,
                    canonical_digest: "a".repeat(16),
                },
                definitions: vec![Definition {
                    id: 1,
                    name: "Shared".into(),
                    note: Some("Definition note".into()),
                    material: Some("Authored material".into()),
                    color_srgb: None,
                    local_dimensions: Some(LocalDimensions::AuthoredAxes { size_mm: [10.0; 3] }),
                }],
                occurrences: [10, 11]
                    .into_iter()
                    .enumerate()
                    .map(|(i, root)| Occurrence {
                        name: "Same name".into(),
                        definition_id: 1,
                        glb_node: i * 2 + 1,
                        path: InstancePath {
                            root_occurrence_id: root,
                            steps: vec![],
                        },
                        note: (root == 11).then(|| "Instance note".into()),
                        material: None,
                        color_srgb: None,
                        attributes: Default::default(),
                    })
                    .collect(),
                scenes: vec![],
                dimensions: vec![],
                start_scene: None,
                fitted_projection: None,
            },
        }
    }

    #[test]
    fn offline_package_model_keeps_shared_geometry_and_correct_source_placements() {
        let package = package(0.1);
        let mut bytes = Vec::new();
        package.write(&mut bytes).expect("write");
        let model = Model::read(bytes.as_slice()).expect("local model");
        assert_eq!(model.manifest(), &package.manifest);
        assert_eq!(model.bodies.len(), 2);
        assert_eq!(model.bodies[0].mesh, model.bodies[1].mesh);
        assert_eq!(model.bodies[0].occurrence, 0);
        assert_eq!(model.bodies[1].occurrence, 1);
        assert_eq!(
            &model.bodies[0].world_matrix_mm[12..15],
            &[93.0, 205.0, 309.0]
        );
        for (actual, expected) in model
            .bounds_mm
            .iter()
            .flatten()
            .zip([83.0, 205.0, 309.0, 515.0, 217.0, 319.0])
        {
            // not a tolerance: precision this test expects of f32 display bounds.
            assert!((actual - expected).abs() < 1e-5);
        }
        let geometry = model.geometry().expect("display buffers");
        assert_eq!(geometry.meshes.len(), 1);
        assert_eq!(geometry.meshes[0][0].positions().len(), 3);
        assert_eq!(
            geometry.meshes[0][0].indices().collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_eq!(source_point([0.01, 0.01, -0.01]).map(f64::round), [10.0; 3]);
    }

    #[test]
    fn package_model_refuses_out_of_range_placements_and_corrupt_input() {
        assert!(Model::from_package(package(1e100)).is_err());
        let mut bytes = Vec::new();
        package(0.1).write(&mut bytes).expect("write");
        assert!(Model::read(&bytes[..bytes.len() - 1]).is_err());
        bytes[11] = 2;
        assert!(Model::read(bytes.as_slice()).is_err());
        assert!(Model::read(&b"{}"[..]).is_err(), "no legacy JSON fallback");
    }
}
