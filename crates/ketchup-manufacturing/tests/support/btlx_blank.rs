//! BTLx checks shared by the BTLx tests of this crate and the release test of
//! the demo house (included there by path).

/// Every point of every processing, at its reference plane and at its depth
/// along XVector x YVector (into the material, docs/btlx-conventions.md), lies
/// in the part's blank [0, Length] x [0, Width] x [0, Height].
pub fn assert_btlx_machining_lies_in_its_blank(xml: &str, name: &str) {
    for_each_processing(xml, |extents, points, header, processing| {
        for at in points {
            assert!(
                (0..3).all(|axis| (-1e-6..=extents[axis] + 1e-6).contains(&at[axis])),
                "{name}: processing reaches {at:?} outside the blank {extents:?} of <Part {header}>\n{processing}"
            );
        }
    });
}

/// Every processing removes material from its own part: the box of its points
/// overlaps the blank with positive volume. A cutter drawn larger than the
/// member (imported models cut through faces with some air) may run past the
/// blank, but a processing placed in a wrong frame misses it.
pub fn assert_btlx_machining_cuts_its_blank(xml: &str, name: &str) {
    for_each_processing(xml, |extents, points, header, processing| {
        let overlaps = (0..3).all(|axis| {
            let low = points
                .iter()
                .map(|at| at[axis])
                .fold(f64::INFINITY, f64::min);
            let high = points
                .iter()
                .map(|at| at[axis])
                .fold(f64::NEG_INFINITY, f64::max);
            low < extents[axis] && high > 0.0
        });
        assert!(
            overlaps,
            "{name}: processing {points:?} misses the blank {extents:?} of <Part {header}>\n{processing}"
        );
    });
}

/// Calls `check(extents, points, part header, processing text)` for every
/// processing: its points at the reference plane and at its depth, in the
/// part's blank frame.
fn for_each_processing(xml: &str, mut check: impl FnMut([f64; 3], &[[f64; 3]], &str, &str)) {
    fn attribute(element: &str, key: &str) -> f64 {
        let start = element.find(&format!(" {key}=\"")).unwrap() + key.len() + 3;
        let end = start + element[start..].find('"').unwrap();
        element[start..end].parse().unwrap()
    }
    fn element<'a>(xml: &'a str, tag: &str) -> &'a str {
        let start = xml.find(&format!("<{tag} ")).unwrap();
        &xml[start..start + xml[start..].find('>').unwrap()]
    }
    fn point(xml: &str, tag: &str) -> [f64; 3] {
        let element = element(xml, tag);
        ["X", "Y", "Z"].map(|key| attribute(element, key))
    }
    fn text(xml: &str, tag: &str) -> f64 {
        let start = xml.find(&format!("<{tag}>")).unwrap() + tag.len() + 2;
        xml[start..start + xml[start..].find('<').unwrap()]
            .parse()
            .unwrap()
    }
    for part in xml.split("<Part ").skip(1) {
        let header = &part[..part.find('>').unwrap()];
        let extents = ["Length", "Width", "Height"].map(|key| attribute(part, key));
        let planes = part
            .split("<UserReferencePlane ")
            .skip(1)
            .map(|plane| {
                let origin = point(plane, "ReferencePoint");
                let x = point(plane, "XVector");
                let y = point(plane, "YVector");
                let normal = ketchup_geometry::linalg::cross(x, y);
                (
                    attribute(&format!(" {plane}"), "ID"),
                    [origin, x, y, normal],
                )
            })
            .collect::<Vec<_>>();
        for processing in part.split("ReferencePlaneID=").skip(1) {
            let id: f64 = processing[1..processing[1..].find('"').unwrap() + 1]
                .parse()
                .unwrap();
            let [origin, x, y, normal] = planes.iter().find(|plane| plane.0 == id).unwrap().1;
            let (local, depth) = if processing.contains("<StartX>") {
                (
                    vec![[text(processing, "StartX"), text(processing, "StartY")]],
                    text(processing, "Depth"),
                )
            } else {
                let contour = element(processing, "Contour");
                let points = ["<StartPoint ", "<EndPoint "]
                    .into_iter()
                    .flat_map(|tag| processing.match_indices(tag))
                    .map(|(at, _)| {
                        let element = &processing[at..at + processing[at..].find('>').unwrap()];
                        [attribute(element, "X"), attribute(element, "Y")]
                    })
                    .collect();
                (points, attribute(contour, "Depth"))
            };
            let points = local
                .iter()
                .flat_map(|&[u, v]| {
                    [0.0, depth].map(|along| {
                        std::array::from_fn(|axis| {
                            origin[axis] + x[axis] * u + y[axis] * v + normal[axis] * along
                        })
                    })
                })
                .collect::<Vec<[f64; 3]>>();
            check(
                extents,
                &points,
                header,
                &processing[..processing.len().min(1500)],
            );
        }
    }
}
