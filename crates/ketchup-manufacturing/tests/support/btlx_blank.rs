//! BTLx checks shared by the BTLx tests of this crate and the release test of
//! the demo house (included there by path).

/// Air a cutter may run past the faces it cuts through: the demo house's
/// imported through cuts end 1-2 mm outside the member (výrez 13 starts at
/// y = -1 and is 422 deep in a 420 beam). Larger overshoots of the house are
/// pinned one by one (house_project_collision.rs).
pub const HOUSE_CUTTER_MARGIN_MM: f64 = 2.0;

/// How far each processing reaches outside its blank, in mm (0 inside):
/// `(part designation, processing ordinal in the part, overshoot)`.
pub fn btlx_machining_overshoots(xml: &str) -> Vec<(String, usize, f64)> {
    let mut overshoots = Vec::new();
    let mut last_header = String::new();
    let mut ordinal = 0;
    for_each_processing(xml, |extents, points, header, _| {
        if header != last_header {
            last_header = header.to_owned();
            ordinal = 0;
        }
        let overshoot = points
            .iter()
            .flat_map(|at| (0..3).map(|axis| (-at[axis]).max(at[axis] - extents[axis])))
            .fold(0.0_f64, f64::max);
        let start = header.find("Designation=\"").unwrap() + 13;
        let designation = &header[start..start + header[start..].find('"').unwrap()];
        overshoots.push((designation.to_owned(), ordinal, overshoot));
        ordinal += 1;
    });
    overshoots
}

/// Every point of every processing, at its reference plane and at its depth
/// along XVector x YVector (into the material, docs/btlx-conventions.md), lies
/// in the part's blank [0, Length] x [0, Width] x [0, Height].
pub fn assert_btlx_machining_lies_in_its_blank(xml: &str, name: &str) {
    assert_btlx_machining_lies_in_its_blank_within(xml, name, 1e-6);
}

/// As [`assert_btlx_machining_lies_in_its_blank`], but a point may lie up to
/// `margin_mm` outside the blank: a cutter drawn with some air past the faces
/// it cuts through. A processing in a wrong frame (a corner moved to the
/// centre, a mirrored axis) leaves the blank by a large part of a member
/// dimension. The failure names the largest overshoot of the whole file.
pub fn assert_btlx_machining_lies_in_its_blank_within(xml: &str, name: &str, margin_mm: f64) {
    let mut worst: Option<(f64, [f64; 3], [f64; 3], String)> = None;
    for_each_processing(xml, |extents, points, header, processing| {
        for &at in points {
            let overshoot = (0..3)
                .map(|axis| (-at[axis]).max(at[axis] - extents[axis]))
                .fold(0.0_f64, f64::max);
            if worst.as_ref().is_none_or(|worst| overshoot > worst.0) {
                worst = Some((
                    overshoot,
                    at,
                    extents,
                    format!("<Part {header}>\n{processing}"),
                ));
            }
        }
    });
    if let Some((overshoot, at, extents, processing)) = worst {
        assert!(
            overshoot <= margin_mm,
            "{name}: processing reaches {at:?}, {overshoot} mm outside the blank {extents:?} (allowed {margin_mm} mm) of {processing}"
        );
    }
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
