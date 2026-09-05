use ketchup_exact::{
    BoxSpec, CircleExtrudeSpec, ExactBackend, ExactBodyBooleanOperation, ExactPairRelation, Point3,
    Size3,
};

#[test]
fn native_self_pair_has_positive_volume_and_zero_distance_without_trusting_fingerprints() {
    let backend = ExactBackend::new();
    let cylinder = |x| {
        backend
            .extrude_circle(CircleExtrudeSpec {
                center_mm: [x, 0.0],
                radius_mm: 1.0,
                height_mm: 2.0,
            })
            .unwrap()
    };
    let left = cylinder(0.0);
    let before = left.body.result_fingerprint.clone();
    for _ in 0..2 {
        let query = backend
            .query_body_pair(&left.body, &left.body, 0.0)
            .unwrap();
        assert_eq!(query.relation, ExactPairRelation::Penetrating);
        assert!(query.common_volume_mm3.is_finite() && query.common_volume_mm3 > 0.0);
        assert!((query.common_volume_mm3 - 2.0 * std::f64::consts::PI).abs() < 1e-7);
        assert_eq!(query.distance_mm, 0.0);
    }
    // Equal public metadata is not native-handle identity.
    let mut separated = cylinder(4.0);
    separated.body.result_fingerprint = before.clone();
    let query = backend
        .query_body_pair(&left.body, &separated.body, 0.0)
        .unwrap();
    assert_eq!(query.relation, ExactPairRelation::Separated);
    assert_eq!(query.common_volume_mm3, 0.0);
    assert!((query.distance_mm - 2.0).abs() < 1e-7);
    assert_eq!(left.body.result_fingerprint, before);
}

#[test]
fn native_pair_common_distance_contact_containment_and_failure() {
    let backend = ExactBackend::new();
    let circle = |x, y| {
        backend
            .extrude_circle(CircleExtrudeSpec {
                center_mm: [x, y],
                radius_mm: 1.0,
                height_mm: 2.0,
            })
            .unwrap()
    };
    let left = circle(0.0, 0.0);
    let separated = circle(1.5, 1.5); // AABBs overlap on all axes, circles do not.
    let before = left.body.result_fingerprint.clone();
    let query = backend
        .query_body_pair(&left.body, &separated.body, 1e-7)
        .unwrap();
    assert_eq!(query.relation, ExactPairRelation::Separated);
    assert_eq!(query.common_volume_mm3, 0.0);
    assert!((query.distance_mm - (4.5_f64.sqrt() - 2.0)).abs() < 1e-7);
    // Modeling Intersect still refuses an empty solid; the read-only query does not.
    assert!(
        backend
            .boolean_bodies(
                &left.body,
                &separated.body,
                ExactBodyBooleanOperation::Intersect
            )
            .is_err()
    );
    let penetrating = circle(1.0, 0.0);
    let query = backend
        .query_body_pair(&left.body, &penetrating.body, 1e-7)
        .unwrap();
    assert_eq!(query.relation, ExactPairRelation::Penetrating);
    assert!(query.common_volume_mm3 > 0.0);
    assert_eq!(query.distance_mm, 0.0);
    let far = circle(4.0, 3.0);
    let far_result = backend
        .query_body_pair(&left.body, &far.body, 1e-7)
        .unwrap();
    assert_eq!(far_result.relation, ExactPairRelation::Separated);
    assert_eq!(far_result.common_volume_mm3, 0.0);
    assert!((far_result.distance_mm - 3.0).abs() < 1e-7);
    let near = circle(2.00001, 0.0);
    let near_result = backend
        .query_body_pair(&left.body, &near.body, 1e-4)
        .unwrap();
    assert_eq!(near_result.relation, ExactPairRelation::Touching);
    assert_eq!(near_result.common_volume_mm3, 0.0);
    assert!((near_result.distance_mm - 1e-5).abs() < 1e-7);
    let touching = circle(2.0, 0.0);
    assert_eq!(
        backend
            .query_body_pair(&left.body, &touching.body, 1e-7)
            .unwrap()
            .relation,
        ExactPairRelation::Touching
    );
    let outer = backend
        .make_box(BoxSpec {
            origin_mm: Point3 {
                x: -2.0,
                y: -2.0,
                z: -1.0,
            },
            size_mm: Size3 {
                x: 4.0,
                y: 4.0,
                z: 4.0,
            },
        })
        .unwrap();
    let contained = backend
        .query_body_pair(&outer.body, &left.body, 1e-7)
        .unwrap();
    assert_eq!(contained.relation, ExactPairRelation::Penetrating);
    assert_eq!(contained.distance_mm, 0.0);
    assert!((contained.common_volume_mm3 - 2.0 * std::f64::consts::PI).abs() < 1e-7);
    for tolerance in [-1.0, f64::NAN, f64::INFINITY] {
        assert!(
            backend
                .query_body_pair(&left.body, &outer.body, tolerance)
                .is_err()
        );
        assert!(
            backend
                .query_body_pair(&left.body, &left.body, tolerance)
                .is_err()
        );
    }
    assert_eq!(before, left.body.result_fingerprint);
    assert_eq!(
        query,
        backend
            .query_body_pair(&left.body, &penetrating.body, 1e-7)
            .unwrap()
    );
}
