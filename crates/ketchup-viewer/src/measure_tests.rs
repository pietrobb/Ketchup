//! Snapping and the distance of the Measure tool, on a box (flat faces) and a
//! half cylinder (one curved face) seen from the top at 1 mm per screen point.

use crate::{
    camera::Camera,
    measure::{Measure, Measured, Pointer, Snap, SnapKind},
    model::Model,
};
use ketchup_view_format::{
    Camera as SceneCamera, Definition, InstancePath, Manifest, Occurrence, Package, Projection,
    Source,
};
use serde_json::json;

/// One display mesh: source points (mm), triangle and line indices.
type Mesh = (Vec<[f64; 3]>, Vec<u32>, Vec<u32>);

/// A box 100 × 100 × 50 mm at the origin, four vertices per face as the
/// exporter writes it, its twelve edges as lines.
pub(crate) fn block() -> Mesh {
    let corner = |i: u32| {
        [
            f64::from(i & 1) * 100.0,
            f64::from((i >> 1) & 1) * 100.0,
            f64::from((i >> 2) & 1) * 50.0,
        ]
    };
    let faces = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let mut points = Vec::new();
    let mut triangles = Vec::new();
    let mut lines = Vec::new();
    for (face, quad) in (0_u32..).zip(faces) {
        points.extend(quad.map(corner));
        let base = face * 4;
        triangles.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        for k in 0..4 {
            lines.extend([base + k, base + (k + 1) % 4]);
        }
    }
    (points, triangles, lines)
}

/// Half a cylinder of radius 50 mm about the Y axis through x = 350, from
/// y = 0 to 100, in six facets of 30°: its rims and straight sides are lines,
/// the sides between facets are not.
pub(crate) fn half_cylinder() -> Mesh {
    let mut points = Vec::new();
    for step in 0..=6 {
        let angle = f64::from(step) * std::f64::consts::PI / 6.0;
        for y in [0.0, 100.0] {
            points.push([350.0 + 50.0 * angle.cos(), y, 50.0 * angle.sin()]);
        }
    }
    let mut triangles = Vec::new();
    let mut lines = vec![0, 1, 12, 13];
    for step in 0..6_u32 {
        let (a, b) = (step * 2, step * 2 + 2);
        triangles.extend([a, b, b + 1, a, b + 1, a + 1]);
        lines.extend([a, b, a + 1, b + 1]);
    }
    (points, triangles, lines)
}

/// A package with one component per mesh, each mesh placed as written.
pub(crate) fn package(meshes: &[Mesh]) -> Package {
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    let mut binary = Vec::new();
    let (mut views, mut accessors, mut gltf_meshes, mut nodes) = (vec![], vec![], vec![], vec![]);
    let mut append = |binary: &mut Vec<u8>, bytes: Vec<u8>, count: usize, kind: &str, component| {
        views.push(json!({"buffer":0,"byteOffset":binary.len(),"byteLength":bytes.len()}));
        accessors.push(
            json!({"bufferView":views.len() - 1,"componentType":component,
            "count":count,"type":kind}),
        );
        binary.extend(bytes);
        accessors.len() - 1
    };
    for (index, (points, triangles, lines)) in meshes.iter().enumerate() {
        // Source Z-up millimetres to GLB Y-up metres.
        let bytes = points
            .iter()
            .flat_map(|p| [p[0] / 1000.0, p[2] / 1000.0, -p[1] / 1000.0])
            .flat_map(|v| (v as f32).to_le_bytes())
            .collect();
        let position = append(&mut binary, bytes, points.len(), "VEC3", 5126);
        let indices = |list: &[u32]| list.iter().flat_map(|i| i.to_le_bytes()).collect();
        let faces = append(
            &mut binary,
            indices(triangles),
            triangles.len(),
            "SCALAR",
            5125,
        );
        let edges = append(&mut binary, indices(lines), lines.len(), "SCALAR", 5125);
        gltf_meshes.push(json!({"primitives":[
            {"attributes":{"POSITION":position},"indices":faces,"mode":4},
            {"attributes":{"POSITION":position},"indices":edges,"mode":1}]}));
        nodes.push(json!({"matrix":identity,"mesh":index}));
        nodes.push(json!({"matrix":identity,"children":[index * 2],
            "extras":{"ketchupEntity":"occurrence","ketchupDefinitionId":1,
                "ketchupInstancePath":format!("occurrence:{}", 10 + index)}}));
    }
    let doc = json!({
        "asset":{"version":"2.0","generator":"Ketchup","extras":{
            "ketchupSourceDigest":"aaaaaaaaaaaaaaaa","ketchupSourceUnit":"millimetre",
            "unit":"metre","upAxis":"Y"}},
        "scene":0,"scenes":[{"nodes":(0..meshes.len()).map(|i| i * 2 + 1).collect::<Vec<_>>()}],
        "nodes":nodes,"meshes":gltf_meshes,"buffers":[{"byteLength":binary.len()}],
        "bufferViews":views,"accessors":accessors
    });
    let mut json = serde_json::to_vec(&doc).expect("JSON");
    while !json.len().is_multiple_of(4) {
        json.push(b' ');
    }
    let length = |n: usize| u32::try_from(n).expect("length").to_le_bytes();
    let mut glb = b"glTF".to_vec();
    glb.extend_from_slice(&2_u32.to_le_bytes());
    glb.extend_from_slice(&length(28 + json.len() + binary.len()));
    glb.extend_from_slice(&length(json.len()));
    glb.extend_from_slice(b"JSON");
    glb.extend(json);
    glb.extend_from_slice(&length(binary.len()));
    glb.extend_from_slice(b"BIN\0");
    glb.extend(binary);
    Package {
        glb,
        manifest: Manifest {
            source: Source {
                document_id: 1,
                revision: 1,
                canonical_digest: "a".repeat(16),
            },
            definitions: vec![Definition {
                id: 1,
                name: "Part".into(),
                note: None,
                material: None,
                color_srgb: None,
                local_dimensions: None,
            }],
            occurrences: (0..meshes.len())
                .map(|index| Occurrence {
                    name: format!("Part {index}"),
                    definition_id: 1,
                    glb_node: index * 2 + 1,
                    path: InstancePath {
                        root_occurrence_id: 10 + u64::try_from(index).expect("index"),
                        steps: vec![],
                    },
                    note: None,
                    material: None,
                    color_srgb: None,
                    attributes: Default::default(),
                })
                .collect(),
            scenes: vec![],
            dimensions: vec![],
            start_scene: None,
        },
    }
}

/// The box and the half cylinder, and a top view 500 × 500 points wide
/// centred on x = 200, y = 50: one point is one millimetre.
pub(crate) fn scene() -> (Model, Camera) {
    let model = Model::from_package(package(&[block(), half_cylinder()])).expect("model");
    let camera = Camera::from_scene(
        &model,
        &SceneCamera {
            eye_mm: [200.0, 50.0, 1000.0],
            target_mm: [200.0, 50.0, 25.0],
            up: [0.0, 1.0, 0.0],
            projection: Projection::Orthographic {
                short_span_mm: 500.0,
            },
        },
    )
    .expect("camera");
    (model, camera)
}

const SIZE: [f32; 2] = [500.0, 500.0];
const BOTH: [bool; 2] = [true, true];

/// Snap with the pointer over plan point (`x`, `y`) mm, `right` and `down`
/// screen points off it.
fn snap_at(
    measure: &mut Measure,
    (model, camera): &(Model, Camera),
    visible: &[bool],
    [x, y]: [f32; 2],
    [right, down]: [f32; 2],
) -> Option<Snap> {
    let p = camera.project([x, y, 0.0], 1.0);
    let pointer = Pointer {
        ndc: [p[0] + right / 250.0, p[1] - down / 250.0],
        size: SIZE,
        visible,
        section: None,
    };
    measure.snap(model, camera, &pointer)
}

fn assert_at(snap: Option<Snap>, kind: SnapKind, point: [f64; 3], approximate: bool) {
    let snap = snap.expect("a snap");
    assert_eq!(snap.kind, kind, "{snap:?}");
    assert!(
        (0..3).all(|i| (snap.point[i] - point[i]).abs() < 1e-3),
        "{snap:?} is not at {point:?}"
    );
    assert_eq!(snap.approximate, approximate, "{snap:?}");
}

#[test]
fn a_vertex_beats_the_edges_and_the_face_near_it_and_the_top_corner_hides_the_bottom_one() {
    let scene = scene();
    let mut measure = Measure::default();
    // 5 points from the corner, 3 and 4 from its two top edges, over the top face.
    let snap = snap_at(&mut measure, &scene, &BOTH, [100.0, 100.0], [-3.0, 4.0]);
    assert_at(snap, SnapKind::Vertex, [100.0, 100.0, 50.0], false);
    // Off the model the pointer still catches the corner.
    let snap = snap_at(&mut measure, &scene, &BOTH, [100.0, 100.0], [6.0, -6.0]);
    assert_at(snap, SnapKind::Vertex, [100.0, 100.0, 50.0], false);
}

/// Review 2026-10-24 VW-23: only four candidates were checked for being in
/// sight, so vertices hidden under the face the pointer is over, nearer the
/// pointer than a corner of that face, kept the corner from being caught.
#[test]
fn hidden_vertices_under_the_face_do_not_keep_its_corner_from_the_pointer() {
    // A 2 × 2 × 10 mm block inside the box, under its top face, below the pointer.
    let (points, triangles, lines) = block();
    let small = points
        .iter()
        .map(|p| [93.0 + p[0] / 50.0, 93.0 + p[1] / 50.0, 10.0 + p[2] / 5.0])
        .collect();
    let model = Model::from_package(package(&[block(), (small, triangles, lines)])).expect("model");
    let (_, camera) = scene();
    let scene = (model, camera);
    let mut measure = Measure::default();
    // 8.5 points from the box's corner, 1.4 from the block's corners under it.
    let snap = snap_at(&mut measure, &scene, &BOTH, [94.0, 94.0], [0.0, 0.0]);
    assert_at(snap, SnapKind::Vertex, [100.0, 100.0, 50.0], false);
}

#[test]
fn an_edge_gives_its_nearest_point_inside_and_outside_the_model() {
    let scene = scene();
    let mut measure = Measure::default();
    let inside = snap_at(&mut measure, &scene, &BOTH, [50.0, 100.0], [0.0, 5.0]);
    assert_at(inside, SnapKind::Edge, [50.0, 100.0, 50.0], false);
    let outside = snap_at(&mut measure, &scene, &BOTH, [30.0, 0.0], [0.0, 8.0]);
    assert_at(outside, SnapKind::Edge, [30.0, 0.0, 50.0], false);
}

#[test]
fn away_from_vertices_and_edges_the_face_under_the_pointer_is_taken() {
    let scene = scene();
    let mut measure = Measure::default();
    let snap = snap_at(&mut measure, &scene, &BOTH, [50.0, 50.0], [0.0, 0.0]);
    assert_at(snap, SnapKind::Face, [50.0, 50.0, 50.0], false);
    assert_eq!(
        snap_at(&mut measure, &scene, &BOTH, [200.0, 50.0], [0.0, 0.0]),
        None,
        "nothing between the parts"
    );
    assert_eq!(
        snap_at(
            &mut measure,
            &scene,
            &[false, true],
            [100.0, 100.0],
            [-3.0, 4.0]
        ),
        None,
        "a hidden part catches nothing"
    );
}

#[test]
fn points_on_the_curved_face_are_approximate_and_on_flat_faces_exact() {
    let scene = scene();
    let mut measure = Measure::default();
    // Facet between 90° and 120°: its points sit up to 1.7 mm below the true surface.
    let face =
        snap_at(&mut measure, &scene, &BOTH, [340.0, 50.0], [0.0, 0.0]).expect("the curved face");
    assert_eq!(face.kind, SnapKind::Face);
    assert!(face.approximate, "{face:?}");
    let top = snap_at(&mut measure, &scene, &BOTH, [350.0, 0.0], [2.0, -2.0]);
    assert_at(top, SnapKind::Vertex, [350.0, 0.0, 50.0], true);
    // Over SNAP_PX (12) points from both rim vertices, 4 from the rim between them.
    let rim = snap_at(&mut measure, &scene, &BOTH, [337.0, 0.0], [0.0, -4.0]).expect("the rim");
    assert_eq!(rim.kind, SnapKind::Edge);
    assert!(rim.approximate, "{rim:?}");
}

#[test]
fn two_points_give_the_distance_and_the_axis_differences_and_a_third_starts_over() {
    let point = |point, approximate| Snap {
        point,
        kind: SnapKind::Vertex,
        approximate,
    };
    let mut measure = Measure::default();
    measure.place(point([0.0, 0.0, 50.0], false));
    assert_eq!(measure.done, None);
    measure.place(point([100.0, 100.0, 0.0], false));
    let done = measure.done.expect("measured");
    assert_eq!(done.delta(), [100.0, 100.0, -50.0]);
    assert_eq!(done.distance(), 150.0);
    assert!(!done.approximate());
    assert_eq!(measure.first, None);
    measure.place(point([1.0, 2.0, 3.0], true));
    assert_eq!(measure.done, None, "a new measurement replaces the old one");
    measure.place(point([4.0, 6.0, 3.0], false));
    assert_eq!(
        measure.done,
        Some(Measured {
            a: point([1.0, 2.0, 3.0], true),
            b: point([4.0, 6.0, 3.0], false),
        })
    );
    assert_eq!(measure.done.map(|m| m.distance()), Some(5.0));
    assert_eq!(measure.done.map(|m| m.approximate()), Some(true));
}
