//! Material, not bounding rectangles, drives contacts and drilling rows.
use ketchup_program::{ProgramModel, contact::contact, evaluate, run};
use std::collections::BTreeMap;
fn model(source: &str) -> ProgramModel {
    evaluate("contact.star", source, &BTreeMap::new())
        .expect("evaluate")
        .model
}
fn area(c: &ketchup_program::contact::Contact) -> f64 {
    let xy: Vec<_> = c
        .points_mm
        .iter()
        .map(|p| {
            let d: [f64; 3] = std::array::from_fn(|i| p[i] - c.origin_mm[i]);
            [
                ketchup_geometry::linalg::dot(d, c.u),
                ketchup_geometry::linalg::dot(d, c.v),
            ]
        })
        .collect();
    xy.iter()
        .zip(xy.iter().cycle().skip(1))
        .take(xy.len())
        .map(|(a, b)| a[0] * b[1] - a[1] * b[0])
        .sum::<f64>()
        .abs()
        * 0.5
}
#[test]
fn triangle_empty_corner_and_actual_overlap() {
    let m = model(
        "a=extrude('a',profile=[(0,0),(100,0),(0,100)],distance=30)\nb=box('b',(10,10,30),at=(80,80,30))\n",
    );
    assert!(contact(m.part("a").unwrap(), m.part("b").unwrap()).is_none());
    let m = model(
        "a=extrude('a',profile=[(0,0),(100,0),(0,100)],distance=30)\nb=box('b',(100,100,30),at=(0,0,30))\n",
    );
    let c = contact(m.part("a").unwrap(), m.part("b").unwrap()).unwrap();
    assert!((area(&c) - 5000.0).abs() < 1.0e-6);
    assert!(
        m.part("a")
            .unwrap()
            .face_frame_at([80., 80., 30.], 0.01)
            .is_none()
    );
}
#[test]
fn concave_and_disconnected_overlap_preserves_area_after_rotation() {
    for placement in [
        "",
        "for p in (a,b):\n    rotate(p,axis=(1,2,3),angle=37,pivot=(0,0,0))\n    move(p,by=(73,-21,89))\n",
    ] {
        let m = model(&format!(
            "a=extrude('a',profile=[(0,0),(100,0),(100,100),(80,100),(80,20),(20,20),(20,100),(0,100)],distance=30)\nb=box('b',(100,20,30),at=(0,50,30))\n{placement}"
        ));
        let c = contact(m.part("a").unwrap(), m.part("b").unwrap()).unwrap();
        assert!((area(&c) - 800.0).abs() < 1.0e-6, "{c:?}");
    }
    let m = model(
        "a=extrude('a',profile=[(0,0),(100,0),(100,20),(20,20),(20,100),(0,100)],distance=30)\nb=box('b',(100,100,30),at=(0,0,30))\n",
    );
    assert!(
        (area(&contact(m.part("a").unwrap(), m.part("b").unwrap()).unwrap()) - 3600.0).abs()
            < 1.0e-6
    );
}
#[test]
fn drilled_void_is_not_contact_and_ring_area_excludes_it() {
    let prefix = "a=box('a',(100,100,30))\nhole(a,'z+',at=(50,50),diameter=40,depth=30)\n";
    let m = model(&format!("{prefix}b=box('b',(10,10,30),at=(45,45,30))\n"));
    assert!(contact(m.part("a").unwrap(), m.part("b").unwrap()).is_none());
    let m = model(&format!("{prefix}b=box('b',(100,100,30),at=(0,0,30))\n"));
    let c = contact(m.part("a").unwrap(), m.part("b").unwrap()).unwrap();
    assert!(
        (area(&c) - (10000.0 - 400.0 * std::f64::consts::PI)).abs() < 0.2,
        "{}",
        area(&c)
    );
}
#[test]
fn rotated_row_has_paired_bores_with_end_margin() {
    for placement in [
        "",
        "for p in (a,b):\n    rotate(p,axis=(1,2,3),angle=31,pivot=(0,0,0))\n    move(p,by=(70,-40,90))\n",
    ] {
        let source = format!(
            "a=box('base',(300,300,30))\nb=box('top',(200,20,30),at=(50,100,30))\nrotate(b,axis=(0,0,1),angle=45,pivot=(150,110,0))\n{placement}dowels(a,b,dowel='6x30',count=2,margin=10)\n"
        );
        let (result, report) = run("row.star", &source, &BTreeMap::new()).expect("dowels");
        assert!(report.ok, "{:?}", report.issues);
        let (a, b) = (
            result.model.part("base").unwrap(),
            result.model.part("top").unwrap(),
        );
        let holes: Vec<_> = b.holes().collect();
        assert_eq!(holes.len(), 2);
        let mut xs: Vec<_> = holes.iter().map(|h| h.entry_mm[0]).collect();
        xs.sort_by(f64::total_cmp);
        assert!(
            (xs[0] - 10.0).abs() < 1.0e-6 && (xs[1] - 190.0).abs() < 1.0e-6,
            "{xs:?}"
        );
        for (ha, hb) in a.holes().zip(b.holes()) {
            assert!((hb.entry_mm[1] - 10.0).abs() < 1.0e-6);
            assert!(
                ketchup_geometry::linalg::distance(
                    a.to_world(ha.entry_mm),
                    b.to_world(hb.entry_mm)
                ) < 1.0e-6
            );
        }
    }
}
#[test]
fn row_avoids_concavity_and_hole_and_rejects_impossible_margin() {
    for shape in [
        "a=extrude('a',profile=[(0,0),(100,0),(0,100)],distance=30)\n",
        "a=extrude('a',profile=[(0,0),(100,0),(100,20),(20,20),(20,100),(0,100)],distance=30)\n",
        "a=box('a',(100,100,30))\nhole(a,'z+',at=(50,50),diameter=40,depth=30)\n",
    ] {
        let source = format!(
            "{shape}b=box('b',(100,100,30),at=(0,0,30))\nfor p in (a,b):\n    rotate(p,axis=(1,2,3),angle=37,pivot=(0,0,0))\n    move(p,by=(73,-21,89))\ndowels(a,b,dowel='6x30',count=2,margin=10)\n"
        );
        let m = model(&source);
        let (a, b) = (m.part("a").unwrap(), m.part("b").unwrap());
        assert_eq!(b.holes().count(), 2);
        assert_eq!(a.holes().filter(|h| h.id.starts_with("dowel:")).count(), 2);
        for (other, hole) in a
            .holes()
            .filter(|h| h.id.starts_with("dowel:"))
            .zip(b.holes())
        {
            let [x, y, _] = hole.entry_mm;
            assert!(
                ketchup_geometry::linalg::distance(
                    a.to_world(other.entry_mm),
                    b.to_world(hole.entry_mm)
                ) < 1.0e-6
            );
            assert!((3.0..=97.0).contains(&x) && (3.0..=97.0).contains(&y));
            if shape.contains("(100,20)") {
                assert!(x <= 17.0 || y <= 17.0, "{x},{y}");
            } else if shape.contains("extrude") {
                assert!((100.0 - x - y) / 2.0_f64.sqrt() >= 3.0 - 0.001);
            } else {
                assert!((x - 50.0).hypot(y - 50.0) >= 23.0 - 0.001);
            }
        }
    }
    let source = "a=box('a',(20,20,30))\nb=box('b',(20,20,30),at=(0,0,30))\ndowels(a,b,dowel='6x30',count=2,margin=20)\n";
    assert!(evaluate("impossible.star", source, &BTreeMap::new()).is_err());
}
#[test]
fn repeated_bores_preserve_contact_material_area() {
    let m = model(
        "a=box('a',(100,100,30))\nfor x in (20,40,60,80):\n    for y in (20,40,60,80):\n        hole(a,'z+',at=(x,y),diameter=8,depth=30)\nb=box('b',(100,100,30),at=(0,0,30))\nfor p in (a,b):\n    rotate(p,axis=(1,2,3),angle=31,pivot=(0,0,0))\n",
    );
    let a = m.part("a").unwrap();
    assert_eq!(a.holes().count(), 16);
    let c = contact(a, m.part("b").unwrap()).unwrap();
    let expected = 10000.0 - 16.0 * std::f64::consts::PI * 16.0;
    assert!(
        (area(&c) - expected).abs() < 0.3,
        "{} vs {expected}",
        area(&c)
    );
}
