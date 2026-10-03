use ketchup_exact::{BoxSpec, ExactBackend, Point3, Size3};

const IDENTITY: [f64; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

#[test]
fn trimmed_faces_not_solids_and_rigid_placement() {
    let backend = ExactBackend::new();
    let body = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            size_mm: Size3 {
                x: 650.0,
                y: 10.0,
                z: 10.0,
            },
        })
        .unwrap();
    // Opposite faces of the same solid: solid self-distance is zero, face distance is 650.
    let faces = &body.body.topology.faces;
    let left = faces
        .iter()
        .find(|f| f.centroid_mm.x.abs() < 1e-7)
        .unwrap()
        .ordinal;
    let right = faces
        .iter()
        .find(|f| (f.centroid_mm.x - 650.0).abs() < 1e-7)
        .unwrap()
        .ordinal;
    assert_eq!(
        backend
            .query_body_pair(&body.body, &body.body, 0.0)
            .unwrap()
            .distance_mm,
        0.0
    );
    let distance = |a, b, m: &[f64; 16]| {
        backend
            .query_face_pair(&body.body, a, m, &body.body, b, m)
            .unwrap()
    };
    assert!((distance(left, right, &IDENTITY) - 650.0).abs() < 1e-7);
    let rotated = [
        0.0, -1.0, 0.0, 45.0, 1.0, 0.0, 0.0, 23.0, 0.0, 0.0, 1.0, -9.0, 0.0, 0.0, 0.0, 1.0,
    ];
    assert!((distance(left, right, &rotated) - 650.0).abs() < 1e-7);
    let mut shifted = IDENTITY;
    shifted[7] = 30.0;
    // Supporting planes stay 650 apart, but the finite faces have a 20 mm lateral gap.
    let finite = backend
        .query_face_pair(&body.body, left, &IDENTITY, &body.body, right, &shifted)
        .unwrap();
    assert!((finite - (650.0_f64.powi(2) + 20.0_f64.powi(2)).sqrt()).abs() < 1e-7);
    assert!(
        backend
            .query_face_pair(
                &body.body,
                u32::MAX,
                &IDENTITY,
                &body.body,
                right,
                &IDENTITY
            )
            .is_err()
    );
    let mut scaled = IDENTITY;
    scaled[0] = 2.0;
    assert!(
        backend
            .query_face_pair(&body.body, left, &scaled, &body.body, right, &IDENTITY)
            .is_err()
    );
}
