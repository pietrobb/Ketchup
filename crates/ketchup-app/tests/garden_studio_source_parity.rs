//! Independent parity against meshes exported by background Blender from the original
//! generator. No model geometry is constructed here. Requires local artifacts and a
//! native exact worker built beside the test executable (or in target/debug).
//! Run: cargo test -p ketchup-app --test garden_studio_source_parity -- --ignored --nocapture

use ketchup_application::{DocumentSession, SessionSettings};
use ketchup_core::exact_product::ExactBodyPackage;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::time::Duration;

type Point = [f64; 3];
type Triangle = [Point; 3];
const OBJECT_COUNT: usize = 140;
const SURFACE_TOLERANCE_MM: f64 = 0.002;
const SOURCE_SHA256: &str = "4809cc1cbcc1da8c522cb6eeedbbfa3a00e35eb81bcccc19c7f6a0a233f28e31";

#[derive(Deserialize)]
struct Reference {
    source: Source,
    objects: BTreeMap<String, ReferenceObject>,
}

#[derive(Deserialize)]
struct Source {
    sha256: String,
}

#[derive(Deserialize)]
struct ReferenceObject {
    bounds_mm: [Point; 2],
    volume_mm3: f64,
    vertices_mm: Vec<Point>,
    triangles: Vec<[usize; 3]>,
}

#[test]
#[ignore = "requires local Blender reference, Python-authored garden model, and native exact worker"]
fn garden_studio_source_parity() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let artifact = |variable: &str, default: &str| {
        std::env::var_os(variable)
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(default))
    };
    let model = artifact(
        "KETCHUP_GARDEN_MODEL",
        "examples/garden-studio-exact.ketchup",
    );
    let reference_path = artifact(
        "KETCHUP_BLENDER_REFERENCE",
        "examples/garden-studio-exact.blender-reference.json",
    );
    let reference: Reference = serde_json::from_slice(
        &std::fs::read(&reference_path)
            .unwrap_or_else(|error| panic!("{}: {error}", reference_path.display())),
    )
    .expect("valid Blender reference JSON");
    assert_eq!(
        reference.source.sha256, SOURCE_SHA256,
        "wrong Blender source revision"
    );
    assert_eq!(reference.objects.len(), OBJECT_COUNT);

    // Same colocated-worker discovery and debug fallback as timber_frame_house.rs.
    let worker_name = if cfg!(windows) {
        "ketchup-exact-worker.exe"
    } else {
        "ketchup-exact-worker"
    };
    let colocated = std::env::current_exe()
        .unwrap()
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .join(worker_name);
    let worker = if colocated.is_file() {
        colocated
    } else {
        root.join("target/debug").join(worker_name)
    };
    assert!(
        worker.is_file(),
        "missing exact worker: {}",
        worker.display()
    );
    let mut session = DocumentSession::open(
        &model,
        SessionSettings {
            exact_worker_path: Some(worker),
            evaluation_timeout: Duration::from_secs(180),
        },
    )
    .unwrap_or_else(|error| panic!("{}: {error}", model.display()));
    let snapshot = session.snapshot();
    let digest = snapshot.canonical_digest();
    let scene = snapshot.scene_query(); // Includes composed parent/world transforms.
    assert_eq!(snapshot.occurrences().count(), OBJECT_COUNT);
    assert_eq!(scene.len(), OBJECT_COUNT);
    let names = scene
        .iter()
        .map(|occurrence| occurrence.occurrence_name.as_str())
        .collect::<BTreeSet<_>>();
    assert_eq!(names.len(), OBJECT_COUNT, "duplicate occurrence names");
    assert_eq!(
        names,
        reference.objects.keys().map(String::as_str).collect(),
        "named occurrence coverage: no omissions or additions allowed"
    );

    let report = session.evaluate().expect("native exact worker evaluation");
    assert!(report.complete && report.topology_complete, "{report:#?}");
    let meshes = session.exact_results().body_values(&snapshot).unwrap();
    let topology = session.topology_results().body_values(&snapshot).unwrap();
    let mut worst_surface_mm = 0.0_f64;
    for occurrence in scene {
        let name = &occurrence.occurrence_name;
        let expected = &reference.objects[name];
        let bodies = meshes
            .iter()
            .filter(|(key, _)| key.definition_id == occurrence.definition_id)
            .collect::<Vec<_>>();
        assert_eq!(bodies.len(), 1, "{name}: expected one evaluated body");
        let (key, package) = bodies[0];
        assert!(package.is_current(&snapshot), "{name}: stale mesh");
        let exact = topology
            .get(key)
            .unwrap_or_else(|| panic!("{name}: no exact topology"));
        assert!(exact.is_current(&snapshot), "{name}: stale topology");
        let ExactBodyPackage::Graph(brep) = exact.as_ref() else {
            panic!("{name}: expected native B-Rep graph evidence, not mesh-only evidence");
        };
        // Worker topology order is vertices, edges, faces, shells, solids.
        assert_eq!(brep.topology_counts[4], 1, "{name}: exact solid count");
        if name == "Pendant_Shade" {
            assert_eq!(
                brep.topology_counts,
                [64, 96, 34, 1, 1],
                "{name}: exact frustum"
            );
        }

        let matrix = occurrence.transform.matrix();
        let determinant = matrix[0] * (matrix[5] * matrix[10] - matrix[6] * matrix[9])
            - matrix[1] * (matrix[4] * matrix[10] - matrix[6] * matrix[8])
            + matrix[2] * (matrix[4] * matrix[9] - matrix[5] * matrix[8]);
        assert!(
            determinant.is_finite() && determinant.abs() > f64::EPSILON,
            "{name}: singular transform"
        );
        let world_vertices = package
            .vertices()
            .iter()
            .map(|vertex| {
                let p = vertex.position_mm;
                std::array::from_fn(|axis| {
                    let row = axis * 4;
                    matrix[row] * p[0]
                        + matrix[row + 1] * p[1]
                        + matrix[row + 2] * p[2]
                        + matrix[row + 3]
                })
            })
            .collect::<Vec<Point>>();
        let indices = package
            .triangles()
            .iter()
            .map(|triangle| triangle.vertex_indices.map(|index| index as usize))
            .collect::<Vec<_>>();
        let actual = triangles(name, &world_vertices, &indices);
        let baseline = triangles(name, &expected.vertices_mm, &expected.triangles);
        let forward = sampled_surface_distance(&actual, &baseline);
        let reverse = sampled_surface_distance(&baseline, &actual);
        let distance = forward.max(reverse);
        assert!(
            distance <= SURFACE_TOLERANCE_MM,
            "{name}: mesh->Blender {forward:.9} mm, Blender->mesh {reverse:.9} mm exceeds {SURFACE_TOLERANCE_MM} mm"
        );
        worst_surface_mm = worst_surface_mm.max(distance);

        for axis in 0..3 {
            let low = world_vertices
                .iter()
                .map(|p| p[axis])
                .fold(f64::INFINITY, f64::min);
            let high = world_vertices
                .iter()
                .map(|p| p[axis])
                .fold(f64::NEG_INFINITY, f64::max);
            for (side, actual_bound) in [low, high].into_iter().enumerate() {
                assert!(
                    (actual_bound - expected.bounds_mm[side][axis]).abs() <= SURFACE_TOLERANCE_MM,
                    "{name}: world bound side {side}, axis {axis}: {actual_bound} vs {}",
                    expected.bounds_mm[side][axis]
                );
            }
        }
        assert_volume(
            name,
            "world mesh",
            mesh_volume(&actual),
            expected.volume_mm3,
        );
        assert_volume(
            name,
            "exact B-Rep",
            brep.volume_mm3 * determinant.abs(),
            expected.volume_mm3,
        );
    }
    assert_eq!(
        session.snapshot().canonical_digest(),
        digest,
        "evaluation mutated model"
    );
    eprintln!(
        "garden parity: {OBJECT_COUNT} named occurrences, worst bidirectional surface distance {worst_surface_mm:.9} mm; source={SOURCE_SHA256}"
    );
}

fn triangles(name: &str, vertices: &[Point], indices: &[[usize; 3]]) -> Vec<Triangle> {
    assert!(
        !vertices.is_empty() && !indices.is_empty(),
        "{name}: empty mesh"
    );
    assert!(
        vertices.iter().flatten().all(|x| x.is_finite()),
        "{name}: nonfinite mesh"
    );
    indices
        .iter()
        .map(|triangle| {
            triangle.map(|index| {
                *vertices
                    .get(index)
                    .unwrap_or_else(|| panic!("{name}: invalid triangle index {index}"))
            })
        })
        .collect()
}

fn assert_volume(name: &str, kind: &str, actual: f64, expected: f64) {
    // Blender's world coordinates are stored as float32 metres before conversion to mm.
    let tolerance = 0.1_f64.max(expected.abs() * 3e-5);
    assert!(
        expected.is_finite()
            && expected > 0.0
            && actual.is_finite()
            && actual > 0.0
            && (actual - expected).abs() <= tolerance,
        "{name}: {kind} volume {actual:.9} vs Blender {expected:.9} mm^3 (tolerance {tolerance:.9})"
    );
}

fn mesh_volume(mesh: &[Triangle]) -> f64 {
    // A nearby origin avoids cancellation for small parts far from the world origin.
    let origin = mesh[0][0];
    mesh.iter()
        .map(|[a, b, c]| dot(sub(*a, origin), cross(sub(*b, origin), sub(*c, origin))) / 6.0)
        .sum::<f64>()
        .abs()
}

fn sampled_surface_distance(source: &[Triangle], target: &[Triangle]) -> f64 {
    source
        .iter()
        .flat_map(|[a, b, c]| {
            [
                *a,
                *b,
                *c,
                blend(*a, *b, 0.5),
                blend(*b, *c, 0.5),
                blend(*c, *a, 0.5),
                std::array::from_fn(|i| (a[i] + b[i] + c[i]) / 3.0),
            ]
        })
        .map(|point| {
            target
                .iter()
                .map(|triangle| point_triangle_distance_squared(point, *triangle))
                .fold(f64::INFINITY, f64::min)
        })
        .fold(0.0, f64::max)
        .sqrt()
}

// Closest point is either the orthogonal projection inside the triangle or on an
// edge. Degenerate Blender triangles are treated as segments/points, not omitted.
fn point_triangle_distance_squared(p: Point, [a, b, c]: Triangle) -> f64 {
    let ab = sub(b, a);
    let ac = sub(c, a);
    let ap = sub(p, a);
    let normal = cross(ab, ac);
    let normal_squared = dot(normal, normal);
    if normal_squared > 0.0 {
        let u = dot(cross(ap, ac), normal) / normal_squared;
        let v = dot(cross(ab, ap), normal) / normal_squared;
        if u >= 0.0 && v >= 0.0 && u + v <= 1.0 {
            return dot(ap, normal).powi(2) / normal_squared;
        }
    }
    [(a, b), (b, c), (c, a)]
        .into_iter()
        .map(|(start, end)| {
            let edge = sub(end, start);
            let length_squared = dot(edge, edge);
            let t = if length_squared > 0.0 {
                (dot(sub(p, start), edge) / length_squared).clamp(0.0, 1.0)
            } else {
                0.0
            };
            let delta = sub(p, blend(start, end, t));
            dot(delta, delta)
        })
        .fold(f64::INFINITY, f64::min)
}

fn sub(a: Point, b: Point) -> Point {
    std::array::from_fn(|i| a[i] - b[i])
}

fn blend(a: Point, b: Point, t: f64) -> Point {
    std::array::from_fn(|i| a[i] + t * (b[i] - a[i]))
}

fn dot(a: Point, b: Point) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}

fn cross(a: Point, b: Point) -> Point {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
