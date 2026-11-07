use super::*;

/// The twelve triangles of an axis-aligned box.
fn cuboid(name: &str, min: Point, max: Point) -> DrawingSolid {
    let corner = |index: usize| -> Point {
        std::array::from_fn(|axis| {
            if index & (1 << axis) == 0 {
                min[axis]
            } else {
                max[axis]
            }
        })
    };
    let quads = [
        [0, 2, 3, 1],
        [4, 5, 7, 6],
        [0, 1, 5, 4],
        [2, 6, 7, 3],
        [0, 4, 6, 2],
        [1, 3, 7, 5],
    ];
    let triangles = quads
        .iter()
        .flat_map(|[a, b, c, d]| {
            [
                [corner(*a), corner(*b), corner(*c)],
                [corner(*a), corner(*c), corner(*d)],
            ]
        })
        .collect();
    DrawingSolid {
        name: name.to_owned(),
        color: [200, 160, 110],
        triangles,
        faces: (0..6).flat_map(|face| [face, face]).collect(),
        threads: Vec::new(),
    }
}

/// A closed cylinder about z, its side meshed in `segments` facets: the
/// side is face 0, the bottom face 1, the top face 2.
fn cylinder(radius: f64, height: f64, segments: u32) -> DrawingSolid {
    let rim = |index: u32, z: f64| -> Point {
        let angle = f64::from(index % segments) * std::f64::consts::TAU / f64::from(segments);
        [radius * angle.cos(), radius * angle.sin(), z]
    };
    let (mut triangles, mut faces) = (Vec::new(), Vec::new());
    for index in 0..segments {
        let (a, b) = (rim(index, 0.0), rim(index + 1, 0.0));
        let (c, d) = (rim(index + 1, height), rim(index, height));
        triangles.extend([[a, b, c], [a, c, d]]);
        faces.extend([0, 0]);
        triangles.push([[0.0, 0.0, 0.0], b, a]);
        faces.push(1);
        triangles.push([[0.0, 0.0, height], d, c]);
        faces.push(2);
    }
    DrawingSolid {
        name: "pin".to_owned(),
        color: [200, 160, 110],
        triangles,
        faces,
        threads: Vec::new(),
    }
}

/// P3-V: a cylinder meshed coarser than the old 20° fold limit (the
/// display mesh steps up to 0.35 rad, just over 20°) drew a line at every
/// facet of its side; only its two rims are edges.
#[test]
fn a_coarsely_meshed_cylinder_shows_only_its_rims() {
    let edges = solid_edges(&cylinder(50.0, 100.0, 17)).features;
    assert_eq!(edges.len(), 2 * 17, "{edges:?}");
    assert!(
        edges
            .iter()
            .all(|[a, b]| a[2] == b[2] && (a[2] == 0.0 || a[2] == 100.0)),
        "{edges:?}"
    );
}

/// MF-16: a round column seen from the side shows its two vertical
/// outlines, not only its rims, and seen from above no line of its side.
#[test]
fn a_cylinder_seen_from_the_side_shows_its_outline() {
    let column = cylinder(50.0, 100.0, 17);
    let edges = vec![solid_edges(&column)];
    let frames = view_frames([-50.0, -50.0, 0.0], [50.0, 50.0, 100.0], 200.0, false);
    let (view, front) = frames[3];
    assert_eq!(view, ProjectView::Front);
    let drawn = draw_view(view, front, std::slice::from_ref(&column), &edges);
    let mut sides = drawn
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Edge([a, b]) if (a[0] - b[0]).abs() < 1e-9 => Some(a[0]),
            _ => None,
        })
        .collect::<Vec<_>>();
    sides.sort_by(f64::total_cmp);
    sides.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    assert_eq!(sides.len(), 2, "{sides:?}");
    assert!(
        sides[0] < -49.0 && (sides[1] - 50.0).abs() < 1e-9,
        "{sides:?}"
    );
    assert_eq!(outline_edges(&edges[0], frames[0].1.depth).count(), 0);
}

/// P3-V: two faces of a shallow roof meeting at 10° still show their ridge,
/// because it is an edge of the model, however small the fold.
#[test]
fn a_shallow_fold_between_two_faces_is_drawn() {
    let pitch = 5_f64.to_radians().tan() * 1000.0;
    let (south_west, south_east) = ([0.0, 0.0, 0.0], [3000.0, 0.0, 0.0]);
    let (north_west, north_east) = ([0.0, 2000.0, 0.0], [3000.0, 2000.0, 0.0]);
    let (ridge_west, ridge_east) = ([0.0, 1000.0, pitch], [3000.0, 1000.0, pitch]);
    let roof = DrawingSolid {
        name: "roof".to_owned(),
        color: [200, 160, 110],
        triangles: vec![
            [south_west, south_east, ridge_east],
            [south_west, ridge_east, ridge_west],
            [ridge_west, ridge_east, north_east],
            [ridge_west, north_east, north_west],
        ],
        faces: vec![0, 0, 1, 1],
        threads: Vec::new(),
    };
    let edges = solid_edges(&roof).features;
    assert!(
        edges.contains(&[ridge_west, ridge_east]) || edges.contains(&[ridge_east, ridge_west]),
        "{edges:?}"
    );
    assert_eq!(
        edges.len(),
        7,
        "the ridge and the six outline edges: {edges:?}"
    );
}

/// A 6 × 3 m room: a floor slab and four 150 mm walls 2.5 m high.
fn room() -> Vec<DrawingSolid> {
    vec![
        cuboid("floor", [0.0, 0.0, 0.0], [6000.0, 3000.0, 200.0]),
        cuboid("south", [0.0, 0.0, 200.0], [6000.0, 150.0, 2700.0]),
        cuboid("north", [0.0, 2850.0, 200.0], [6000.0, 3000.0, 2700.0]),
        cuboid("west", [0.0, 150.0, 200.0], [150.0, 2850.0, 2700.0]),
        cuboid("east", [5850.0, 150.0, 200.0], [6000.0, 2850.0, 2700.0]),
    ]
}

use crate::title_block::{TitleBlock, TitleField};
use ketchup_model::sheet_pdf::PdfInfo;

fn plans(storeys: Vec<Storey>) -> Vec<PlanSheet> {
    storeys
        .into_iter()
        .enumerate()
        .map(|(index, storey)| PlanSheet {
            storey,
            view_title: format!("Floor plan {}", index + 1),
            sheet_title: format!("Plan {}", index + 1),
        })
        .collect()
}

fn options(solids: &[DrawingSolid], cut_mm: Option<f64>) -> DrawingSetOptions {
    let storeys = match cut_mm {
        Some(cut_mm) => vec![Storey {
            floor_mm: None,
            cut_mm,
        }],
        None => storeys(solids, 1200.0),
    };
    DrawingSetOptions {
        plans: plans(storeys),
        sheet_titles: SheetTitles {
            sections: "Sections".to_owned(),
            elevations: "Elevations".to_owned(),
        },
        title_labels: BTreeMap::from([(TitleField::Project, "Stavba".to_owned())]),
        title_block: TitleBlock::from([(TitleField::Project, "Room <A>".to_owned())]),
        ..DrawingSetOptions::default()
    }
}

fn room_set(cut_mm: Option<f64>) -> DrawingSet {
    drawing_set(&room(), &options(&room(), cut_mm)).unwrap()
}

fn texts(page: &Page) -> Vec<&str> {
    page.texts().collect()
}

fn sheet_of(set: &DrawingSet, kind: SheetKind) -> &SetSheet {
    set.sheets.iter().find(|sheet| sheet.kind == kind).unwrap()
}

fn summary(set: &DrawingSet, view: ProjectView) -> &ViewSummary {
    set.sheets
        .iter()
        .flat_map(|sheet| &sheet.views)
        .find(|summary| summary.view == view)
        .unwrap()
}

/// Měchenice MECH-6: the sections and elevations mark ±0,000 at the floor,
/// the top above it and the underside of the floor slab below it (review
/// 2026-11-04 MF-141); the plans mark no level.
#[test]
fn sections_and_elevations_mark_their_levels_against_the_floor() {
    let options = DrawingSetOptions {
        decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
        ..options(&room(), None)
    };
    let set = drawing_set(&room(), &options).unwrap();
    for view in [ProjectView::LongitudinalSection, ProjectView::Front] {
        assert_eq!(
            summary(&set, view).levels,
            ["-0,200", "±0,000", "+2,500"],
            "{view:?}"
        );
    }
    assert!(summary(&set, ProjectView::Plan).levels.is_empty());
    for kind in [SheetKind::Sections, SheetKind::Elevations] {
        let sheet = sheet_of(&set, kind);
        assert!(texts(&sheet.page).contains(&"+2,500"), "{kind:?}");
        assert!(texts(&sheet.lines).contains(&"±0,000"), "{kind:?} DXF");
    }
}

/// Review 2026-11-04 MF-141: ±0,000 was the lowest floor, so a basement
/// was ±0,000 and the ground floor +3,000; the foundations were never
/// marked, nor an attic floor or a gallery without a plan of its own, a
/// gallery not even after the first fix, under half the floor's area
/// (review finding run 480). ±0,000 is the
/// ground floor, the lowest at or above z = 0, unless the sheet sets it;
/// every floor and the bottom below ±0,000 are marked.
#[test]
fn zero_is_the_ground_floor_and_every_level_below_and_above_is_marked() {
    let mut house = room();
    for mut solid in room() {
        solid.name.push_str(" basement");
        for corner in solid.triangles.iter_mut().flatten() {
            corner[2] -= 3000.0;
        }
        house.push(solid);
    }
    // An attic floor 200 mm above the walls under a 600 mm knee wall:
    // too low under the top for a plan of its own.
    house.push(cuboid(
        "attic",
        [0.0, 0.0, 2700.0],
        [6000.0, 3000.0, 2900.0],
    ));
    house.push(cuboid("knee", [0.0, 0.0, 2900.0], [6000.0, 150.0, 3500.0]));
    // Galleries and a landing well under half the floor's area, floors all
    // the same: 2 × 3 m, 1.2 m wide along 4 m, and a square of 1.8 m
    // (capability finding 482).
    for gallery_corner in [
        [2000.0, 3000.0, 1600.0],
        [4000.0, 1200.0, 1600.0],
        [1800.0, 1800.0, 1600.0],
    ] {
        let mut house = house.clone();
        house.push(cuboid("gallery", [0.0, 0.0, 1400.0], gallery_corner));
        let mut options = DrawingSetOptions {
            decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
            ..options(&house, None)
        };
        let levels = |options: &DrawingSetOptions| {
            let set = drawing_set(&house, options).unwrap();
            summary(&set, ProjectView::LongitudinalSection)
                .levels
                .clone()
        };
        assert_eq!(
            levels(&options),
            ["-3,200", "-3,000", "±0,000", "+1,400", "+2,700", "+3,300"],
            "gallery to {gallery_corner:?}"
        );
        // The basement floor set as ±0,000.
        options.zero_level_mm = Some(-2800.0);
        assert_eq!(
            levels(&options),
            ["-0,200", "±0,000", "+3,000", "+4,400", "+5,700", "+6,300"],
            "gallery to {gallery_corner:?}"
        );
    }
}

/// Review 2026-11-04 r19 MF-161 and P3: every top 1 m wide was a level,
/// so a bed, a dining table and a kitchen island were marked; a gallery
/// of 140 mm planks or of OSB strips never was, each piece measured on
/// its own, nor a landing 900 mm wide. A raised level is one a person
/// stands on: as wide as a flight of stairs, the pieces that touch
/// measured as one, and high enough over the level below to stand under.
#[test]
fn furniture_is_no_level_and_a_gallery_of_planks_and_a_narrow_landing_are() {
    let levels = |solids: &[DrawingSolid]| {
        let options = DrawingSetOptions {
            decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
            ..options(solids, None)
        };
        let set = drawing_set(solids, &options).unwrap();
        summary(&set, ProjectView::LongitudinalSection)
            .levels
            .clone()
    };
    let bare = ["-0,200", "±0,000", "+2,500"];
    assert_eq!(levels(&room()), bare);
    // On the floor (its top at 200): a bed 2000 × 1400 at +450, a table
    // top 2000 × 1000 at +750 and an island 2400 × 1200 at +900.
    let mut furnished = room();
    furnished.extend([
        cuboid("bed", [300.0, 300.0, 200.0], [2300.0, 1700.0, 650.0]),
        cuboid("table", [2600.0, 300.0, 900.0], [4600.0, 1300.0, 950.0]),
        cuboid("island", [3000.0, 1500.0, 200.0], [5400.0, 2700.0, 1100.0]),
    ]);
    assert_eq!(levels(&furnished), bare, "furniture");
    let raised = ["-0,200", "±0,000", "+1,400", "+2,500"];
    // A gallery of 14 planks 140 mm wide, every other one shorter, their
    // tops at +1,400; a desk on it is furniture again.
    let mut planks = room();
    for index in 0..14_u32 {
        let y = 150.0 + f64::from(index) * 140.0;
        let end = if index % 2 == 0 { 2150.0 } else { 1950.0 };
        planks.push(cuboid(
            "plank",
            [150.0, y, 1500.0],
            [end, y + 140.0, 1600.0],
        ));
    }
    planks.push(cuboid(
        "desk",
        [300.0, 300.0, 2300.0],
        [1900.0, 1200.0, 2350.0],
    ));
    assert_eq!(levels(&planks), raised, "planks");
    // Three OSB strips 625 mm wide.
    let mut strips = room();
    for index in 0..3_u32 {
        let y = 150.0 + f64::from(index) * 625.0;
        strips.push(cuboid(
            "osb",
            [150.0, y, 1578.0],
            [2650.0, y + 625.0, 1600.0],
        ));
    }
    assert_eq!(levels(&strips), raised, "OSB strips");
    // A landing 900 mm wide.
    let mut landing = room();
    landing.push(cuboid(
        "landing",
        [4950.0, 150.0, 1400.0],
        [5850.0, 1350.0, 1600.0],
    ));
    assert_eq!(levels(&landing), raised, "landing");
    // One strip alone, 625 mm, is no more a level than the top of a wall.
    let mut strip = room();
    strip.push(cuboid(
        "osb",
        [150.0, 150.0, 1578.0],
        [2650.0, 775.0, 1600.0],
    ));
    assert_eq!(levels(&strip), bare, "one strip");
    // Review 2026-11-06 P3: planks laid with 5 mm joints did not touch,
    // so each was measured alone and the gallery went unmarked.
    let mut jointed = room();
    for index in 0..14_u32 {
        let y = 150.0 + f64::from(index) * 145.0;
        jointed.push(cuboid(
            "plank",
            [150.0, y, 1500.0],
            [2150.0, y + 140.0, 1600.0],
        ));
    }
    assert_eq!(levels(&jointed), raised, "planks with joints");
}

/// Review 2026-11-06 MF-180: a raised level was measured against the last
/// level marked anywhere in the model, so a landing at +1,800 on the way up
/// unmarked the gallery at +2,700 across the hall, 2,700 over the floor
/// under it. It is measured against the highest marked level under it in
/// plan; a landing 900 over the floor is still none.
#[test]
fn a_gallery_is_measured_against_the_level_under_it_not_a_landing_elsewhere() {
    let hall = vec![
        cuboid("floor", [0.0, 0.0, -200.0], [8000.0, 6000.0, 0.0]),
        cuboid("roof", [0.0, 0.0, 5000.0], [8000.0, 6000.0, 5200.0]),
        cuboid("landing 1", [6000.0, 0.0, 800.0], [7200.0, 1200.0, 900.0]),
        cuboid(
            "landing 2",
            [6000.0, 1500.0, 1700.0],
            [7200.0, 2700.0, 1800.0],
        ),
        cuboid("gallery", [0.0, 0.0, 2500.0], [3000.0, 6000.0, 2700.0]),
    ];
    let options = DrawingSetOptions {
        decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
        ..options(&hall, None)
    };
    let set = drawing_set(&hall, &options).unwrap();
    assert_eq!(
        summary(&set, ProjectView::LongitudinalSection).levels,
        ["-0,200", "±0,000", "+1,800", "+2,700", "+5,200"]
    );
}

/// Review 2026-11-06 r23 MF-200: "under it in plan" was measured by the
/// box around each solid's faces, so an L-shaped gallery round a stair,
/// with the landing at +1,800 in its free corner, or a gallery whose edge
/// lies 100 mm over a landing, was measured against the landing and went
/// unmarked. Its faces are measured, against the level under the most of
/// it. Two tables beside the floor, under none of it, are no level (P3).
#[test]
fn a_gallery_round_a_stair_is_measured_against_the_floor_under_most_of_it() {
    let hall = |gallery: Vec<DrawingSolid>| {
        let mut hall = vec![
            cuboid("floor", [0.0, 0.0, -200.0], [8000.0, 6000.0, 0.0]),
            cuboid("roof", [0.0, 0.0, 5000.0], [8000.0, 6000.0, 5200.0]),
            cuboid(
                "landing",
                [5000.0, 1500.0, 1700.0],
                [6200.0, 2700.0, 1800.0],
            ),
            // Two tables outside the hall, 10 mm apart.
            cuboid("table", [8500.0, 0.0, 700.0], [9100.0, 1200.0, 740.0]),
            cuboid("table", [9110.0, 0.0, 700.0], [9710.0, 1200.0, 740.0]),
        ];
        hall.extend(gallery);
        let options = DrawingSetOptions {
            decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
            ..options(&hall, None)
        };
        let set = drawing_set(&hall, &options).unwrap();
        summary(&set, ProjectView::LongitudinalSection)
            .levels
            .clone()
    };
    let expected = ["-0,200", "±0,000", "+1,800", "+2,700", "+5,200"];
    // One solid in the shape of an L, the landing in its free corner.
    let mut l_shaped = cuboid("gallery", [0.0, 0.0, 2500.0], [4000.0, 6000.0, 2700.0]);
    l_shaped.triangles.extend(
        cuboid(
            "gallery",
            [4000.0, 3000.0, 2500.0],
            [8000.0, 6000.0, 2700.0],
        )
        .triangles,
    );
    assert_eq!(hall(vec![l_shaped]), expected, "L-shaped");
    // Its edge 100 mm over the landing.
    let over_the_edge = cuboid("gallery", [0.0, 0.0, 2500.0], [5100.0, 6000.0, 2700.0]);
    assert_eq!(
        hall(vec![over_the_edge]),
        expected,
        "over the landing's edge"
    );
}

/// Review 2026-11-07 P3: a top was measured against the level under the
/// most of it, so a table 800 mm wide at +3,440, 500 mm of it past the
/// gallery's edge, and a bed over the stair's opening in the gallery were
/// measured against the floor and marked; so was a desk on the gallery
/// over a floor laid twice (finish and board of one top), counted twice.
/// A platform at +2,400 beside the hall, nothing under it, was measured
/// against the landing at +1,800 elsewhere and went unmarked. A top is
/// measured against the highest level under a quarter of it, else the
/// highest floor below it.
#[test]
fn a_top_is_measured_against_the_highest_level_under_a_quarter_of_it() {
    let hall = |extra: Vec<DrawingSolid>| {
        let mut hall = vec![
            cuboid("floor", [0.0, 0.0, -200.0], [8000.0, 6000.0, 0.0]),
            cuboid("roof", [0.0, 0.0, 5000.0], [8000.0, 6000.0, 5200.0]),
            cuboid(
                "landing",
                [5000.0, 1500.0, 1700.0],
                [6200.0, 2700.0, 1800.0],
            ),
            // The stair's opening between y = 2000 and 3000.
            cuboid("gallery", [0.0, 0.0, 2500.0], [3000.0, 2000.0, 2700.0]),
            cuboid("gallery", [0.0, 3000.0, 2500.0], [3000.0, 6000.0, 2700.0]),
        ];
        hall.extend(extra);
        let options = DrawingSetOptions {
            decimal_separator: ketchup_model::annotation::DecimalMark::Comma,
            ..options(&hall, None)
        };
        let set = drawing_set(&hall, &options).unwrap();
        summary(&set, ProjectView::LongitudinalSection)
            .levels
            .clone()
    };
    let expected = ["-0,200", "±0,000", "+1,800", "+2,700", "+5,200"];
    assert_eq!(hall(Vec::new()), expected, "bare");
    let table = cuboid("table", [2700.0, 3500.0, 3400.0], [3500.0, 4500.0, 3440.0]);
    assert_eq!(hall(vec![table]), expected, "table past the edge");
    let bed = cuboid("bed", [500.0, 1800.0, 2900.0], [2500.0, 3200.0, 3150.0]);
    assert_eq!(hall(vec![bed]), expected, "bed over the opening");
    let doubled = vec![
        cuboid("finish", [0.0, 0.0, -20.0], [8000.0, 6000.0, 0.0]),
        cuboid("desk", [500.0, 4000.0, 3400.0], [2000.0, 5000.0, 3440.0]),
    ];
    assert_eq!(hall(doubled), expected, "floor laid twice");
    let platform = cuboid("platform", [9000.0, 0.0, 2300.0], [11000.0, 2000.0, 2400.0]);
    assert_eq!(
        hall(vec![platform]),
        ["-0,200", "±0,000", "+1,800", "+2,400", "+2,700", "+5,200"],
        "platform beside the hall"
    );
}

/// Three floors of 100 × 100 tiles 50 mm square, 20 000 top triangles each.
fn tiled_floors() -> Vec<DrawingSolid> {
    let mut tiles = Vec::new();
    for top in [0.0, 1500.0, 3000.0] {
        for index in 0..10_000_u32 {
            let [x, y] = [f64::from(index % 100) * 50.0, f64::from(index / 100) * 50.0];
            tiles.push(cuboid(
                "tile",
                [x, y, top - 10.0],
                [x + 50.0, y + 50.0, top],
            ));
        }
    }
    tiles
}

/// Review 2026-11-07 P3: the levels' triangles were overlapped each with
/// each, and nothing stopped it.
#[test]
fn the_triangles_of_many_levels_are_overlapped_and_can_be_stopped() {
    let tiles = tiled_floors();
    assert_eq!(
        marked_floor_levels(&tiles, &AtomicBool::new(false)),
        Some(vec![0.0, 1500.0, 3000.0])
    );
    assert_eq!(marked_floor_levels(&tiles, &AtomicBool::new(true)), None);
}

/// Review 2026-11-07 P3: 3 s for three levels of 20 000 triangles, tens
/// for more (50 s in a debug build).
#[test]
#[ignore = "perf: release timing budget, runs nightly"]
fn perf_the_triangles_of_many_levels_are_overlapped_in_a_second() {
    let tiles = tiled_floors();
    let started = std::time::Instant::now();
    assert!(marked_floor_levels(&tiles, &AtomicBool::new(false)).is_some());
    let took = started.elapsed();
    assert!(took < std::time::Duration::from_secs(1), "{took:?}");
}

/// `solids` moved `by` in plan, as a model placed in national coordinates.
fn moved(mut solids: Vec<DrawingSolid>, by: [f64; 2]) -> Vec<DrawingSolid> {
    for corner in solids
        .iter_mut()
        .flat_map(|solid| solid.triangles.iter_mut().flatten())
    {
        corner[0] += by[0];
        corner[1] += by[1];
    }
    solids
}

/// Review 2026-11-07 r26 MF-240: a solid's winding was summed from the
/// world coordinates of its corners, terms of 1e27 at 1e9 mm, so in
/// S-JTSK about half the tiles came out inside out and their undersides
/// became false levels. A model far from the origin is drawn as at it.
#[test]
fn a_model_in_national_coordinates_is_drawn_as_at_the_origin() {
    let near = room();
    let tiles = || {
        (0..400_u32).map(|index| {
            let [x, y] = [f64::from(index % 20) * 50.0, f64::from(index / 20) * 50.0];
            cuboid("tile", [x, y, 190.0], [x + 50.0, y + 50.0, 200.0])
        })
    };
    let near_set = drawing_set(&near, &options(&near, None)).unwrap();
    for by in [[-1.2e9, -5.0e8], [5.0e9, 5.0e9]] {
        let far = moved(near.clone(), by);
        let far_tiles = moved(tiles().collect(), by);
        assert!(
            far.iter()
                .chain(&far_tiles)
                .all(|solid| !winds_inward(solid)),
            "{by:?}"
        );
        assert_eq!(
            marked_floor_levels(&far_tiles, &AtomicBool::new(false)),
            marked_floor_levels(&tiles().collect::<Vec<_>>(), &AtomicBool::new(false)),
            "{by:?}"
        );
        let far_set = drawing_set(&far, &options(&far, None)).unwrap();
        for view in ProjectView::ALL {
            let (far_view, near_view) = (summary(&far_set, view), summary(&near_set, view));
            assert_eq!(far_view.levels, near_view.levels, "{by:?} {view:?}");
            assert_eq!(far_view.cut_solids, near_view.cut_solids, "{by:?} {view:?}");
            for axis in 0..2 {
                assert!(
                    (far_view.extent_mm[axis] - near_view.extent_mm[axis]).abs() < 1e-3,
                    "{by:?} {view:?} {:?} {:?}",
                    far_view.extent_mm,
                    near_view.extent_mm
                );
            }
        }
        let triangle = [[by[0], by[1]], [by[0] + 3.0, by[1]], [by[0], by[1] + 2.0]];
        assert!((polygon_area(&triangle) - 3.0).abs() < 1e-9, "{by:?}");
    }
}

/// Review 2026-11-06 r23 MF-203: with the model's origin on the ground,
/// the terrace 150 mm below it was ±0,000 under the floor at +450, the
/// floor nearest z = 0. A floor at or above z = 0 by less than a storey
/// comes first.
#[test]
fn zero_is_the_floor_over_the_ground_not_the_terrace_below_it() {
    let house = vec![
        cuboid("terrace", [-3000.0, 0.0, -350.0], [0.0, 6000.0, -150.0]),
        cuboid("floor", [0.0, 0.0, 250.0], [6000.0, 6000.0, 450.0]),
        cuboid("roof", [0.0, 0.0, 3000.0], [6000.0, 6000.0, 3200.0]),
    ];
    assert_eq!(ground_floor(&house), 450.0);
}

/// Review 2026-11-06 P3: ±0,000 was the lowest floor at or above z = 0, so
/// a ground floor modelled 1 mm or 150 mm below it made the floor above
/// ±0,000. A floor less than a storey below z = 0 is the ground floor; a
/// basement is still below it.
#[test]
fn a_ground_floor_just_below_zero_is_still_zero() {
    for below in [1.0, 150.0] {
        let mut house = room();
        for corner in house
            .iter_mut()
            .flat_map(|solid| solid.triangles.iter_mut().flatten())
        {
            corner[2] -= 200.0 + below;
        }
        house.push(cuboid(
            "upper floor",
            [0.0, 0.0, 2500.0 - 200.0 - below],
            [6000.0, 3000.0, 2700.0 - 200.0 - below],
        ));
        assert_eq!(ground_floor(&house), -below, "{below} mm below");
    }
    let mut house = room();
    for mut solid in room() {
        for corner in solid.triangles.iter_mut().flatten() {
            corner[2] -= 3000.0;
        }
        house.push(solid);
    }
    assert_eq!(ground_floor(&house), 200.0, "the basement");
}

/// Měchenice MECH-6: a vertical dimension of the program has no length in plan;
/// the sections and elevations draw it where they show its true length, a
/// section only beyond its cut. A horizontal one stays in the plan.
#[test]
fn sections_and_elevations_draw_the_vertical_dimensions() {
    use ketchup_model::annotation::AnnotationMark;
    let dimension = |name: &str, from_mm, to_mm| Annotation {
        name: name.to_owned(),
        tags: std::collections::BTreeSet::new(),
        mark: AnnotationMark::Dimension {
            from_mm,
            to_mm,
            offset_mm: 500.0,
            height_mm: 2.5,
        },
    };
    let mut options = options(&room(), None);
    options.annotations = vec![
        dimension("length", [0.0, -500.0, 0.0], [6000.0, -500.0, 0.0]),
        // Beyond both cuts (y 1500, x 3000).
        dimension("beyond", [6000.0, 2000.0, 200.0], [6000.0, 2000.0, 2350.0]),
        // In front of both cuts.
        dimension("before", [1000.0, 1000.0, 200.0], [1000.0, 1000.0, 1200.0]),
    ];
    let set = drawing_set(&room(), &options).unwrap();
    let counts = |kind| &sheet_of(&set, kind).annotations;
    let plan = counts(SheetKind::Plan { storey: 0 });
    assert_eq!((plan.dimensions, plan.outside_plan), (1, 2));
    assert_eq!(counts(SheetKind::Sections).dimensions, 2);
    // Each on the elevation of its own half, front or back and left or
    // right, not on the opposite one too (review 2026-11-04).
    assert_eq!(counts(SheetKind::Elevations).dimensions, 4);
    let elevations = sheet_of(&set, SheetKind::Elevations);
    assert!(texts(&elevations.page).contains(&"2150"));
    assert!(texts(&elevations.lines).contains(&"2150"), "DXF");
}

/// MF-4: a half millimetre rounds up, as a drawing reads, not to the even neighbour.
#[test]
fn overall_dimensions_round_a_half_millimetre_up() {
    let block = [cuboid("block", [0.0, 0.0, 0.0], [2450.5, 1000.0, 2.5])];
    let set = drawing_set(&block, &options(&block, None)).unwrap();
    let texts = set
        .sheets
        .iter()
        .flat_map(|sheet| sheet.page.texts())
        .collect::<Vec<_>>();
    assert!(texts.contains(&"2451"), "{texts:?}");
    assert!(texts.contains(&"3"), "{texts:?}");
    assert!(!texts.contains(&"2450"), "{texts:?}");
}

#[test]
fn a_plan_cut_below_the_origin_reports_its_height_with_its_sign() {
    // The room lowered by 1 m: its walls stand from -800 to 1700.
    let lowered = room()
        .into_iter()
        .map(|mut solid| {
            for corner in solid.triangles.iter_mut().flatten() {
                corner[2] -= 1000.0;
            }
            solid
        })
        .collect::<Vec<_>>();
    let set = drawing_set(&lowered, &options(&lowered, Some(-300.0))).unwrap();
    assert_eq!(summary(&set, ProjectView::Plan).cut_at_mm, Some(-300.0));
    // Its slab top at -800: the storey is cut 1.2 m above it.
    assert_eq!(storeys(&lowered, 1200.0)[0].cut_mm, 400.0);
}

/// The room is a set of three sheets at 1:50: its plan, its sections and
/// its elevations, numbered 1 / 3 to 3 / 3, each with the frame and the
/// title block, saying what it shows.
#[test]
fn the_set_has_a_plan_a_sections_and_an_elevations_sheet_each_numbered_and_titled() {
    let set = room_set(Some(1200.0));
    assert_eq!(set.scale, 50, "a room is drawn at the building scale");
    assert_eq!(
        set.sheets
            .iter()
            .map(|sheet| (sheet.kind, sheet.number, sheet.title.as_str(), sheet.format))
            .collect::<Vec<_>>(),
        [
            (SheetKind::Plan { storey: 0 }, 1, "Plan 1", SheetFormat::A3),
            (SheetKind::Sections, 2, "Sections", SheetFormat::A3),
            (SheetKind::Elevations, 3, "Elevations", SheetFormat::A3),
        ]
    );
    for sheet in &set.sheets {
        let texts = texts(&sheet.page);
        let number = format!("{} / 3", sheet.number);
        for wanted in [
            sheet.title.as_str(),
            number.as_str(),
            "1:50",
            "A3",
            "Room <A>",
            "Stavba",
        ] {
            assert!(
                texts.contains(&wanted),
                "{wanted} on sheet {}: {texts:?}",
                sheet.number
            );
        }
        // The frame and the title block in its lower right corner on every
        // sheet, in its painted and its line drawing alike.
        for page in [&sheet.page, &sheet.lines] {
            for (min, max, width_mm) in [
                ([20.0, 10.0], [410.0, 287.0], 0.7),
                ([230.0, 238.0], [410.0, 287.0], 0.5),
            ] {
                assert!(
                    page.marks.contains(&Mark::Line {
                        points: vec![min, [max[0], min[1]], max, [min[0], max[1]]],
                        closed: true,
                        stroke: Stroke::solid(width_mm),
                    }),
                    "sheet {}",
                    sheet.number
                );
            }
        }
    }
    let pdf = set.pdf(&PdfInfo::default());
    assert!(pdf.starts_with(b"%PDF-"));
    assert_eq!(
        pdf.windows(b"/Type /Page /".len())
            .filter(|window| *window == b"/Type /Page /")
            .count(),
        3,
        "a page per sheet"
    );
}

#[test]
fn plan_and_sections_fill_the_parts_the_plane_cuts() {
    let set = room_set(Some(1200.0));
    let plan = summary(&set, ProjectView::Plan);
    assert_eq!(plan.cut_solids, 4, "the four walls, not the floor");
    assert_eq!(plan.cut_at_mm, Some(1200.0));
    assert_eq!(plan.title, "Floor plan 1");
    assert_eq!(
        summary(&set, ProjectView::LongitudinalSection).cut_at_mm,
        Some(1500.0),
        "the room's middle across"
    );
    assert_eq!(plan.extent_mm, [6000.0, 3000.0]);
    // Along the 6 m length: floor plus the two end walls; across: floor and side walls.
    assert_eq!(
        summary(&set, ProjectView::LongitudinalSection).cut_solids,
        3
    );
    assert_eq!(summary(&set, ProjectView::CrossSection).cut_solids, 3);
    assert_eq!(
        summary(&set, ProjectView::CrossSection).extent_mm,
        [3000.0, 2700.0]
    );
    assert_eq!(summary(&set, ProjectView::Front).cut_solids, 0);
    assert_eq!(
        summary(&set, ProjectView::Front).extent_mm,
        [6000.0, 2700.0]
    );
    let cut_fills = |sheet: &SetSheet| {
        sheet
            .page
            .marks
            .iter()
            .filter(|mark| {
                matches!(mark, Mark::Fill { outline: Some(stroke), .. } if stroke.width_mm == CUT_STROKE_MM)
            })
            .count()
    };
    assert_eq!(cut_fills(sheet_of(&set, SheetKind::Plan { storey: 0 })), 4);
    assert_eq!(cut_fills(sheet_of(&set, SheetKind::Sections)), 6);
    assert_eq!(cut_fills(sheet_of(&set, SheetKind::Elevations)), 0);
    let elevations = texts(&sheet_of(&set, SheetKind::Elevations).page);
    assert!(elevations.contains(&"6000"));
    assert!(elevations.contains(&"2700"));
}

#[test]
fn the_default_plan_cut_is_measured_from_the_floor_not_from_the_footings() {
    // The room on four footings reaching 1312 mm below the slab.
    let mut house = room()
        .into_iter()
        .map(|mut solid| {
            for triangle in &mut solid.triangles {
                for point in triangle {
                    point[2] += 1312.0;
                }
            }
            solid
        })
        .collect::<Vec<_>>();
    for (x, y) in [(0.0, 0.0), (5400.0, 0.0), (0.0, 2400.0), (5400.0, 2400.0)] {
        house.push(cuboid(
            "footing",
            [x, y, 0.0],
            [x + 600.0, y + 600.0, 1312.0],
        ));
    }
    // Slab top at 1512: the cut is 1.2 m above it, through the walls only.
    assert_eq!(default_plan_cut_z(&house, 1200.0), Some(2712.0));
    // A height set in the sheet settings moves the cut with it.
    assert_eq!(default_plan_cut_z(&house, 900.0), Some(2412.0));
    // A model with no floor below head height is cut through its middle.
    let table = [cuboid("top", [0.0; 3], [1600.0, 800.0, 40.0])];
    assert_eq!(default_plan_cut_z(&table, 1200.0), Some(20.0));
}

/// The room with a second storey on its 200 mm ceiling slab and a flat
/// roof slab: two storeys, the roof deck is none.
fn two_storeys() -> Vec<DrawingSolid> {
    let mut house = room();
    for (name, z) in [("ceiling", 2700.0), ("roof", 5400.0)] {
        house.push(cuboid(name, [0.0, 0.0, z], [6000.0, 3000.0, z + 200.0]));
    }
    for wall in room().into_iter().skip(1) {
        let mut upper = wall;
        upper.name.push_str(" upper");
        for corner in upper.triangles.iter_mut().flatten() {
            corner[2] += 2700.0;
        }
        house.push(upper);
    }
    house
}

#[test]
fn each_storey_has_a_plan_sheet_of_its_own_with_its_annotations() {
    use ketchup_model::annotation::AnnotationMark;
    let house = two_storeys();
    assert_eq!(
        storeys(&house, 1200.0),
        [
            Storey {
                floor_mm: Some(200.0),
                cut_mm: 1400.0
            },
            Storey {
                floor_mm: Some(2900.0),
                cut_mm: 4100.0
            },
        ]
    );
    let dimension = |name: &str, z: f64| Annotation {
        name: name.to_owned(),
        tags: std::collections::BTreeSet::new(),
        mark: AnnotationMark::Dimension {
            from_mm: [0.0, -500.0, z],
            to_mm: [6000.0, -500.0, z],
            offset_mm: -500.0,
            height_mm: 2.5,
        },
    };
    let mut options = options(&house, None);
    options.annotations = vec![dimension("ground", 0.0), dimension("upper", 2900.0)];
    options
        .title_block
        .insert(TitleField::Drawing, "Rodinný dom".to_owned());
    let set = drawing_set(&house, &options).unwrap();
    assert_eq!(
        set.sheets
            .iter()
            .map(|sheet| (sheet.kind, sheet.number))
            .collect::<Vec<_>>(),
        [
            (SheetKind::Plan { storey: 0 }, 1),
            (SheetKind::Plan { storey: 1 }, 2),
            (SheetKind::Sections, 3),
            (SheetKind::Elevations, 4),
        ]
    );
    let cuts = set.sheets[..2]
        .iter()
        .map(|sheet| sheet.views[0].cut_at_mm)
        .collect::<Vec<_>>();
    assert_eq!(cuts, [Some(1400.0), Some(4100.0)]);
    // Each plan draws the dimension of its own storey only.
    for sheet in &set.sheets[..2] {
        assert_eq!(sheet.annotations.dimensions, 1, "sheet {}", sheet.number);
    }
    assert_eq!(set.annotations().dimensions, 2);
    for sheet in &set.sheets {
        let texts = texts(&sheet.page);
        let title = format!("Rodinný dom – {}", sheet.title);
        let number = format!("{} / 4", sheet.number);
        assert!(texts.contains(&title.as_str()), "{texts:?}");
        assert!(texts.contains(&number.as_str()), "{texts:?}");
    }

    // Review 2026-10-24 MF-29: plans given by their cuts alone have no
    // floor, and each drew the annotations of every storey.
    options.plans = plans(
        [1400.0, 4100.0]
            .map(|cut_mm| Storey {
                floor_mm: None,
                cut_mm,
            })
            .to_vec(),
    );
    let set = drawing_set(&house, &options).unwrap();
    for sheet in &set.sheets[..2] {
        assert_eq!(sheet.annotations.dimensions, 1, "sheet {}", sheet.number);
    }
    assert_eq!(set.annotations().dimensions, 2);
}

#[test]
fn every_sheet_is_a_dxf_of_its_visible_lines_with_its_title_block() {
    let set = room_set(Some(1200.0));
    let files = set.dxf();
    assert_eq!(files.len(), 3);
    for (sheet, dxf) in set.sheets.iter().zip(&files) {
        let dxf = String::from_utf8(dxf.clone()).unwrap();
        assert!(dxf.starts_with("0\nSECTION\n2\nHEADER\n9\n$ACADVER\n1\nAC1009"));
        assert!(dxf.contains(&format!("1\n{} / 3\n", sheet.number)), "{dxf}");
        assert!(dxf.contains(&format!("1\n{}\n", sheet.title)));
        assert!(dxf.contains("0\nLINE\n8\nPEN-0_18\n"), "the edges");
        // No face is painted: a fill is only the outline of a cut.
        assert!(
            sheet
                .lines
                .marks
                .iter()
                .all(|mark| !matches!(mark, Mark::Fill { outline: None, .. })),
            "sheet {}",
            sheet.number
        );
    }
    let plan = String::from_utf8(files[0].clone()).unwrap();
    assert_eq!(
        plan.matches("0\nLINE\n8\nCUT\n").count(),
        16,
        "the four walls the plan cuts, outlined"
    );
}

/// A box behind a wall is hidden in the front elevation's lines; the part
/// of a taller box above the wall shows.
#[test]
fn the_lines_of_a_view_leave_out_what_nearer_faces_hide() {
    let wall = cuboid("wall", [0.0, 0.0, 0.0], [4000.0, 200.0, 2500.0]);
    let hidden = cuboid("hidden", [500.0, 1000.0, 0.0], [1500.0, 2000.0, 1000.0]);
    let taller = cuboid("taller", [2000.0, 1000.0, 0.0], [3000.0, 2000.0, 3000.0]);
    let solids = vec![wall, hidden, taller];
    let edges = solids.iter().map(solid_edges).collect::<Vec<_>>();
    let (min, max) = bounds(&solids).unwrap();
    let (view, front) = view_frames(min, max, 1200.0, false)[3];
    assert_eq!(view, ProjectView::Front);
    let drawn = draw_view(view, front, &solids, &edges);
    let painted_inside = drawn.items.iter().any(|item| {
        matches!(item, Item::Edge([a, b]) if a[0] > 500.0 - 1.0 && b[0] < 1500.0 + 1.0 && a[1] < 1000.0 + 1.0 && b[1] < 1000.0 + 1.0)
    });
    assert!(
        painted_inside,
        "the PDF paints the hidden box's edges, then the wall over them"
    );
    let lines = visible::visible_lines(&drawn, &std::sync::atomic::AtomicBool::new(false));
    let on = |value: f64, wanted: f64| (value - wanted).abs() < 1e-6;
    for line in &lines {
        let [a, b] = [line.points[0], line.points[1]];
        let wall_outline = (on(a[0], 0.0) && on(b[0], 0.0))
            || (on(a[0], 4000.0) && on(b[0], 4000.0))
            || (on(a[1], 0.0) && on(b[1], 0.0))
            || (on(a[1], 2500.0) && on(b[1], 2500.0));
        let above_the_wall = a[1] >= 2500.0 - 1e-6 && b[1] >= 2500.0 - 1e-6;
        assert!(
            wall_outline || above_the_wall,
            "{line:?} is hidden behind the wall"
        );
    }
    // The taller box shows its sides above the wall and its top.
    for x in [2000.0, 3000.0] {
        assert!(
            lines.iter().any(|line| on(line.points[0][0], x)
                && on(line.points[1][0], x)
                && on(line.points[0][1].min(line.points[1][1]), 2500.0)
                && on(line.points[0][1].max(line.points[1][1]), 3000.0)),
            "the side at x = {x} from 2500 to 3000: {lines:?}"
        );
    }
    assert!(
        lines
            .iter()
            .any(|line| on(line.points[0][1], 3000.0) && on(line.points[1][1], 3000.0))
    );
    // The bottom edge of the wall, split by depth slabs, is one line again.
    assert_eq!(
        lines
            .iter()
            .filter(|line| on(line.points[0][1], 0.0) && on(line.points[1][1], 0.0))
            .count(),
        1,
        "{lines:?}"
    );
}

/// A cancelled set stops removing hidden lines instead of finishing every
/// view, and is answered as cancelled, not drawn (review 2026-11-02 MCP-121).
#[test]
fn a_cancelled_set_stops_removing_hidden_lines() {
    use std::sync::atomic::AtomicBool;
    let solids = vec![
        cuboid("wall", [0.0, 0.0, 0.0], [4000.0, 200.0, 2500.0]),
        cuboid("box", [500.0, 1000.0, 0.0], [1500.0, 2000.0, 1000.0]),
    ];
    let edges = solids.iter().map(solid_edges).collect::<Vec<_>>();
    let (min, max) = bounds(&solids).unwrap();
    let (view, front) = view_frames(min, max, 1200.0, false)[3];
    let drawn = draw_view(view, front, &solids, &edges);
    assert!(!visible::visible_lines(&drawn, &AtomicBool::new(false)).is_empty());
    assert!(visible::visible_lines(&drawn, &AtomicBool::new(true)).is_empty());
    assert!(matches!(
        drawing_set_until(
            &solids,
            &options(&solids, Some(1200.0)),
            &AtomicBool::new(true)
        ),
        Err(ProjectDrawingError::Cancelled)
    ));
    assert!(
        drawing_set_until(
            &solids,
            &options(&solids, Some(1200.0)),
            &AtomicBool::new(false)
        )
        .is_ok()
    );
}

#[test]
fn every_sheet_is_drawn_at_the_scale_of_the_set_on_the_smallest_format_that_holds_it() {
    // A 24 m long hall: at 1:50 its plan needs an A2, its two sections side
    // by side an A1 and its elevations, two of 524 mm in a row, an A0.
    let hall = [
        cuboid("floor", [0.0, 0.0, 0.0], [24_000.0, 3000.0, 200.0]),
        cuboid("south", [0.0, 0.0, 200.0], [24_000.0, 150.0, 2700.0]),
    ];
    let set = drawing_set(&hall, &options(&hall, Some(1200.0))).unwrap();
    assert_eq!(set.scale, 50);
    let formats = set
        .sheets
        .iter()
        .map(|sheet| (sheet.kind, sheet.format))
        .collect::<Vec<_>>();
    assert_eq!(
        formats,
        [
            (SheetKind::Plan { storey: 0 }, SheetFormat::A2),
            (SheetKind::Sections, SheetFormat::A1),
            (SheetKind::Elevations, SheetFormat::A0),
        ]
    );
    // Allowed 1:100, the sections fit an A3 only at 1:100, so the whole
    // set is drawn at 1:100 and the elevations shrink to an A1.
    let mut coarser = options(&hall, Some(1200.0));
    coarser.coarsest_scale = Some(100);
    let coarser = drawing_set(&hall, &coarser).unwrap();
    assert_eq!(coarser.scale, 100);
    assert_eq!(
        coarser
            .sheets
            .iter()
            .map(|sheet| sheet.format)
            .collect::<Vec<_>>(),
        [SheetFormat::A3, SheetFormat::A3, SheetFormat::A1]
    );
}

#[test]
fn a_plan_cut_above_everything_shows_the_roofless_room_from_above() {
    let set = room_set(Some(5000.0));
    let plan = summary(&set, ProjectView::Plan);
    assert_eq!(plan.cut_solids, 0);
    assert_eq!(plan.extent_mm, [6000.0, 3000.0]);
    let edges = set.sheets[0]
        .page
        .marks
        .iter()
        .filter(|mark| matches!(mark, Mark::Line { stroke, .. } if stroke.color == EDGE_COLOR))
        .count();
    assert!(edges > 0, "the walls are drawn from above");
}

#[test]
fn cut_loops_close_around_each_wall() {
    let frame = ViewFrame {
        right: [1.0, 0.0, 0.0],
        up: [0.0, 1.0, 0.0],
        depth: [0.0, 0.0, -1.0],
        cut: Some(-1200.0),
    };
    let loops = cut_loops(&frame, &room()[1]);
    assert_eq!(loops.len(), 1);
    let area = polygon_area(&loops[0][..loops[0].len() - 1]).abs();
    assert!((area - 6000.0 * 150.0).abs() < 1.0, "{area}");
}

/// The plan sheet of the room on A3 at 1:50, worked out by hand. The frame
/// starts 20 mm from the left and 10 mm from the top and every view is
/// padded by 22 mm, so the plan's 6000 mm run from x = 42 to 42 + 6000/50 =
/// 162 and its 3000 mm from y = 21 to 21 + 60 = 81; the width line lies
/// 8 mm below, the height line 8 mm to the left. On the sections sheet the
/// cross section starts after the longitudinal one, 20 + 120 + 44 + 22 =
/// 206, and is 3000/50 = 60 wide and 2700/50 = 54 high.
#[test]
fn dimension_lines_and_the_title_block_sit_where_the_scale_puts_them() {
    let set = room_set(Some(1200.0));
    let ticks = |page: &Page| {
        page.marks
            .iter()
            .filter_map(|mark| match mark {
                Mark::Line { points, stroke, .. } if stroke.width_mm == 0.35 => Some([
                    f64::midpoint(points[0][0], points[1][0]),
                    f64::midpoint(points[0][1], points[1][1]),
                ]),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let text_at = |page: &Page, text: &str, at: [f64; 2]| {
        page.marks.iter().any(|mark| {
            matches!(mark, Mark::Text { text: t, at: a, .. }
                if t == text && (a[0] - at[0]).abs() < 1e-9 && (a[1] - at[1]).abs() < 1e-9)
        })
    };
    let plan = &set.sheets[0].page;
    let sections = &set.sheets[1].page;
    for (page, wanted) in [
        (
            plan,
            [[42.0, 89.0], [162.0, 89.0], [34.0, 81.0], [34.0, 21.0]],
        ),
        (
            sections,
            [[206.0, 83.0], [266.0, 83.0], [198.0, 75.0], [198.0, 21.0]],
        ),
    ] {
        let ticks = ticks(page);
        for tick in wanted {
            assert!(
                ticks
                    .iter()
                    .any(|t| (t[0] - tick[0]).abs() < 1e-9 && (t[1] - tick[1]).abs() < 1e-9),
                "no tick at {tick:?} in {ticks:?}"
            );
        }
    }
    // Each value sits over the middle of its line, 1 mm clear of it.
    assert!(text_at(plan, "6000", [102.0, 88.0]));
    assert!(text_at(plan, "3000", [33.0, 51.0]));
    assert!(text_at(sections, "3000", [236.0, 82.0]));
    assert!(text_at(sections, "2700", [197.0, 48.0]));
    // The scale and the sheet number in their cells of the 180 × 49 title
    // block in the lower right corner of the 420 × 297 sheet.
    assert!(plan.marks.contains(&Mark::Line {
        points: vec![
            [295.0, 278.0],
            [325.0, 278.0],
            [325.0, 287.0],
            [295.0, 287.0]
        ],
        closed: true,
        stroke: Stroke::solid(0.25),
    }));
    assert!(plan.marks.iter().any(|mark| matches!(mark,
        Mark::Text { text, at, .. } if text == "1:50" && (at[0] - 296.5).abs() < 1e-9)));
    assert!(plan.marks.iter().any(|mark| matches!(mark,
        Mark::Text { text, at, .. } if text == "1 / 3" && (at[0] - 346.5).abs() < 1e-9)));
}

#[test]
fn nothing_visible_is_an_error() {
    assert_eq!(
        drawing_set(&[], &options(&[], Some(0.0))),
        Err(ProjectDrawingError::Empty)
    );
    assert_eq!(
        drawing_set(&room(), &DrawingSetOptions::default()),
        Err(ProjectDrawingError::Empty),
        "no plan asked for"
    );
}

/// Eight points 5 m apart on the south side of a 35 m square: a number at
/// each, a chain of dimensions below them and the square's outline.
fn anchor_plan() -> Vec<Annotation> {
    use ketchup_model::annotation::AnnotationMark;
    let point = |index: u32| [f64::from(index) * 5000.0, 0.0, 0.0];
    let annotation = |name: String, mark| Annotation {
        name,
        tags: std::collections::BTreeSet::new(),
        mark,
    };
    let mut plan = (0..8)
        .map(|index| {
            annotation(
                format!("S-{:02}", index + 1),
                AnnotationMark::Text {
                    at_mm: point(index),
                    text: format!("S-{:02}", index + 1),
                    height_mm: 3.5,
                },
            )
        })
        .collect::<Vec<_>>();
    plan.extend((0..7).map(|index| {
        annotation(
            format!("d{index}"),
            AnnotationMark::Dimension {
                from_mm: point(index),
                to_mm: point(index + 1),
                offset_mm: -1500.0,
                height_mm: 2.5,
            },
        )
    }));
    plan.push(annotation(
        "outline".to_owned(),
        AnnotationMark::Line {
            points_mm: vec![
                [0.0, 0.0, 0.0],
                [35_000.0, 0.0, 0.0],
                [35_000.0, 35_000.0, 0.0],
                [0.0, 35_000.0, 0.0],
            ],
            closed: true,
        },
    ));
    plan
}

fn text_at<'a>(page: &'a Page, wanted: &str) -> Vec<&'a [f64; 2]> {
    page.marks
        .iter()
        .filter_map(|mark| match mark {
            Mark::Text { text, at, .. } if text == wanted => Some(at),
            _ => None,
        })
        .collect()
}

#[test]
fn a_plan_of_its_own_draws_its_annotations_at_the_scale_it_is_given() {
    let marker = cuboid("marker", [-100.0, -100.0, 5.0], [100.0, 100.0, 10.0]);
    let options = DrawingSetOptions {
        plan_only: true,
        scale: Some(250),
        annotations: anchor_plan(),
        ..options(std::slice::from_ref(&marker), None)
    };
    let set = drawing_set(std::slice::from_ref(&marker), &options).unwrap();
    assert_eq!(set.scale, 250);
    assert_eq!(set.sheets.len(), 1, "the plan alone");
    let sheet = &set.sheets[0];
    assert_eq!(
        sheet.views.iter().map(|view| view.view).collect::<Vec<_>>(),
        [ProjectView::Plan]
    );
    assert_eq!(
        sheet.annotations,
        AnnotationCounts {
            texts: 8,
            dimensions: 7,
            lines: 1,
            ..AnnotationCounts::default()
        }
    );
    // Every number at its point: 5 m apart is 20 mm at 1:250.
    let numbers = (1..=8)
        .map(|index| {
            let found = text_at(&sheet.page, &format!("S-{index:02}"));
            assert_eq!(found.len(), 1, "S-{index:02}");
            *found[0]
        })
        .collect::<Vec<_>>();
    for pair in numbers.windows(2) {
        assert!((pair[1][0] - pair[0][0] - 20.0).abs() < 1e-9, "{pair:?}");
        assert!((pair[1][1] - pair[0][1]).abs() < 1e-9, "{pair:?}");
    }
    // Seven measured values, each centred between its two points and below them.
    let values = text_at(&sheet.page, "5000");
    assert_eq!(values.len(), 7, "{values:?}");
    for (value, pair) in values.iter().zip(numbers.windows(2)) {
        assert!((value[0] - f64::midpoint(pair[0][0], pair[1][0])).abs() < 1e-9);
        assert!(value[1] > pair[0][1], "below the points on the sheet");
    }
    // The overall dimensions measure the marker, not the annotations.
    assert_eq!(text_at(&sheet.page, "200").len(), 2);
    assert!(text_at(&sheet.page, "35000").is_empty());
    // The DXF carries the same annotations.
    assert_eq!(text_at(&sheet.lines, "5000").len(), 7);

    let annotations_alone = DrawingSetOptions {
        plans: plans(vec![Storey {
            floor_mm: None,
            cut_mm: 0.0,
        }]),
        ..options.clone()
    };
    let alone = drawing_set(&[], &annotations_alone).unwrap();
    assert_eq!(alone.sheets[0].annotations, sheet.annotations);
    assert_eq!(text_at(&alone.sheets[0].page, "5000").len(), 7);

    let too_fine = DrawingSetOptions {
        scale: Some(10),
        ..options
    };
    assert_eq!(
        drawing_set(&[marker], &too_fine),
        Err(ProjectDrawingError::DoesNotFit {
            scale: 10,
            format: None
        })
    );
}

/// R9-C step 14: the plan marks every window and door it shows, and the
/// last sheet lists each mark with its designation and how many openings
/// carry it; each sheet's DXF carries the same marks and table.
#[test]
fn the_plan_marks_windows_and_doors_and_the_last_sheet_lists_them() {
    let mut house = room();
    // Two equal windows in the south wall, a door in the east wall.
    house.extend([
        cuboid("w1/glass", [1000.0, 40.0, 1100.0], [2200.0, 110.0, 2350.0]),
        cuboid("w2/glass", [3500.0, 40.0, 1100.0], [4700.0, 110.0, 2350.0]),
        cuboid(
            "entry/leaf",
            [5890.0, 1000.0, 200.0],
            [5960.0, 1900.0, 2300.0],
        ),
    ]);
    let opening = |mark: &str, designation: &str, part: &str| DrawingOpening {
        mark: mark.to_owned(),
        designation: designation.to_owned(),
        parts: vec![part.to_owned()],
    };
    let mut options = options(&house, Some(1200.0));
    options.openings = OpeningSchedule {
        openings: vec![
            opening("W1", "window 1200 x 1250", "w1/glass"),
            opening("D1", "door 900 x 2100", "entry/leaf"),
            opening("W1", "window 1200 x 1250", "w2/glass"),
            // Hidden: listed, but no plan shows it.
            opening("W2", "window 600 x 600", "attic/glass"),
        ],
        title: "Windows and doors".to_owned(),
        columns: ["Mark", "Designation", "Count"].map(str::to_owned),
    };
    let set = drawing_set(&house, &options).unwrap();
    assert_eq!(
        set.sheets
            .iter()
            .map(|sheet| (sheet.kind, sheet.number))
            .collect::<Vec<_>>(),
        [
            (SheetKind::Plan { storey: 0 }, 1),
            (SheetKind::Sections, 2),
            (SheetKind::Elevations, 3),
            (SheetKind::Openings, 4),
        ]
    );

    let plan = sheet_of(&set, SheetKind::Plan { storey: 0 });
    assert_eq!(plan.openings, ["W1", "D1", "W1"]);
    for page in [&plan.page, &plan.lines] {
        assert_eq!(text_at(page, "W1").len(), 2);
        assert_eq!(text_at(page, "D1").len(), 1);
        assert!(text_at(page, "W2").is_empty());
    }
    // The door's mark stands in the middle of its leaf, right of the windows'.
    let [door] = text_at(&plan.page, "D1")[..] else {
        panic!("one door mark")
    };
    assert!(text_at(&plan.page, "W1").iter().all(|w| w[0] < door[0]));

    assert_eq!(
        options.openings.rows(),
        [
            ScheduleRow {
                mark: "W1".to_owned(),
                designation: "window 1200 x 1250".to_owned(),
                count: 2
            },
            ScheduleRow {
                mark: "D1".to_owned(),
                designation: "door 900 x 2100".to_owned(),
                count: 1
            },
            ScheduleRow {
                mark: "W2".to_owned(),
                designation: "window 600 x 600".to_owned(),
                count: 1
            },
        ]
    );
    let schedule = sheet_of(&set, SheetKind::Openings);
    let texts = texts(&schedule.page);
    for wanted in [
        "Mark",
        "Designation",
        "Count",
        "window 1200 x 1250",
        "door 900 x 2100",
        "window 600 x 600",
        "2",
        "4 / 4",
        "Windows and doors",
        "Room <A>",
    ] {
        assert!(texts.contains(&wanted), "{wanted}: {texts:?}");
    }
    assert_eq!(schedule.lines, schedule.page);
    let dxf = set.dxf();
    assert_eq!(dxf.len(), 4);
    assert!(String::from_utf8_lossy(&dxf[3]).contains("window 1200 x 1250"));

    // The plans alone have no schedule.
    options.plan_only = true;
    let plans = drawing_set(&house, &options).unwrap();
    assert_eq!(plans.sheets.len(), 1);
    assert_eq!(plans.sheets[0].openings, ["W1", "D1", "W1"]);
}

/// R9-C step 15: a wall panel 2000 x 2500 framed in 60 x 140 timber,
/// a window 600 x 1200 between two of its studs, sheathed with OSB.
#[test]
fn a_shop_drawing_numbers_every_piece_and_dimensions_the_studs_and_the_opening() {
    let stud = |name: &str, x: f64, top: f64| cuboid(name, [x, 0.0, 60.0], [x + 60.0, 140.0, top]);
    let lying =
        |name: &str, x: [f64; 2], z: f64| cuboid(name, [x[0], 0.0, z], [x[1], 140.0, z + 60.0]);
    let panel = vec![
        lying("p/bottom plate", [0.0, 2000.0], 0.0),
        lying("p/top plate", [0.0, 2000.0], 2440.0),
        stud("p/stud 1", 0.0, 2440.0),
        stud("p/stud 2", 600.0, 2440.0),
        stud("p/stud 3", 1940.0, 2440.0),
        stud("p/king 1", 840.0, 2000.0),
        stud("p/king 2", 1500.0, 2000.0),
        lying("p/header", [840.0, 1560.0], 2000.0),
        lying("p/sill", [900.0, 1500.0], 740.0),
        cuboid("p/osb 1", [0.0, -15.0, 0.0], [900.0, 0.0, 2500.0]),
        cuboid("p/osb 2", [1500.0, -15.0, 0.0], [2000.0, 0.0, 2500.0]),
    ];
    let position = |material: &str, dimensions_mm: [f64; 3], solids: &[&str]| ShopPosition {
        material: material.to_owned(),
        dimensions_mm,
        solids: solids.iter().map(|name| format!("p/{name}")).collect(),
        count: solids.len(),
        mirror_of: None,
    };
    let mut options = options(&panel, Some(1200.0));
    options.shop = ShopDrawings {
        sheets: vec![ShopSheet {
            title: "Panel p".to_owned(),
            right: [1.0, 0.0, 0.0],
            up: [0.0, 0.0, 1.0],
            positions: vec![
                position("C24", [2000.0, 140.0, 60.0], &["bottom plate", "top plate"]),
                position(
                    "C24",
                    [2380.0, 140.0, 60.0],
                    &["stud 1", "stud 2", "stud 3"],
                ),
                position("C24", [1940.0, 140.0, 60.0], &["king 1", "king 2"]),
                position("C24", [720.0, 140.0, 60.0], &["header"]),
                position("C24", [600.0, 140.0, 60.0], &["sill"]),
                position("OSB", [2500.0, 900.0, 15.0], &["osb 1"]),
                ShopPosition {
                    mirror_of: Some(6),
                    ..position("OSB", [2500.0, 500.0, 15.0], &["osb 2"])
                },
            ],
        }],
        columns: ["Pos.", "Material", "Size", "Count"].map(str::to_owned),
        mirror_of: "mirror of".to_owned(),
    };
    let set = drawing_set(&panel, &options).unwrap();
    assert_eq!(
        set.sheets
            .iter()
            .map(|sheet| sheet.kind)
            .collect::<Vec<_>>(),
        [
            SheetKind::Plan { storey: 0 },
            SheetKind::Sections,
            SheetKind::Elevations,
            SheetKind::Shop { drawing: 0 },
        ]
    );
    let sheet = sheet_of(&set, SheetKind::Shop { drawing: 0 });
    let shop = sheet.shop.as_ref().unwrap();
    // The studs' edges, and between the kings the window's 600.
    assert_eq!(
        shop.across,
        [
            0.0, 60.0, 600.0, 660.0, 840.0, 900.0, 1500.0, 1560.0, 1940.0, 2000.0
        ]
    );
    // The plates', and the window's sill top at 800 and header at 2000.
    assert_eq!(
        shop.up,
        [0.0, 60.0, 740.0, 800.0, 2000.0, 2060.0, 2440.0, 2500.0]
    );
    assert_eq!(
        shop.marks,
        ["1", "1", "2", "2", "2", "3", "3", "4", "5", "6", "7"]
    );
    for page in [&sheet.page, &sheet.lines] {
        // Its mark on every piece, and its row in the table.
        for (number, pieces) in [("1", 2), ("2", 3), ("3", 2), ("4", 1), ("6", 1)] {
            assert!(text_at(page, number).len() > pieces, "{number}");
        }
        let texts = texts(page);
        for wanted in [
            "Pos.",
            "2380 x 140 x 60",
            "2500 x 900 x 15",
            "OSB",
            "4 / 4",
            "Panel p",
            "1560",
            "7 (mirror of 6)",
        ] {
            assert!(texts.contains(&wanted), "{wanted}: {texts:?}");
        }
    }
    // Sheet and pieces at a shop scale, not the set's.
    assert!(set.scale > 25 && sheet.page.texts().any(|text| text == "1:25"));
    assert!(String::from_utf8_lossy(&set.dxf()[3]).contains("2500 x 500 x 15"));

    // The OSB lies in front of the framing, so it gets a view of its own
    // below: the framing view shows every stud, its edges drawn from plate
    // to plate (as the DXF carries them) and its number on it.
    assert_eq!(
        sheet
            .views
            .iter()
            .map(|view| view.title.as_str())
            .collect::<Vec<_>>(),
        ["Panel p", "Panel p - OSB"]
    );
    let shop = shop::plan_shop(
        &options.shop.sheets[0],
        &panel,
        &options.shop.columns,
        &options.shop.mirror_of,
        None,
    )
    .unwrap()
    .unwrap();
    let views = shop.visible_lines(&std::sync::atomic::AtomicBool::new(false));
    let drawn = |view: usize, x: f64, from: f64, to: f64| {
        views[view]
            .iter()
            .flat_map(|line| line.points.windows(2))
            .any(|pair| {
                (pair[0][0] - x).abs() < 0.01
                    && (pair[1][0] - x).abs() < 0.01
                    && pair[0][1].min(pair[1][1]) <= from + 0.01
                    && pair[0][1].max(pair[1][1]) >= to - 0.01
            })
    };
    for x in [0.0, 60.0, 600.0, 660.0, 1940.0, 2000.0] {
        assert!(drawn(0, x, 60.0, 2440.0), "stud edge {x}");
    }
    for x in [840.0, 900.0, 1500.0, 1560.0] {
        assert!(drawn(0, x, 60.0, 2000.0), "king edge {x}");
    }
    for x in [0.0, 900.0, 1500.0, 2000.0] {
        assert!(drawn(1, x, 0.0, 2500.0), "osb edge {x}");
    }
    let numbers = |view: &DrawnView| {
        view.marks
            .iter()
            .map(|(_, number)| number.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        numbers(&shop.view),
        ["1", "1", "2", "2", "2", "3", "3", "4", "5"]
    );
    assert_eq!(numbers(&shop.layers[0].1), ["6", "7"]);

    options.plan_only = true;
    assert_eq!(drawing_set(&panel, &options).unwrap().sheets.len(), 1);
}

/// MF-62: a strip of the sheathing no wider than the sheet is thick is
/// still sheathing, so the sheathing keeps its own view and does not hide
/// the studs, and the strip is not dimensioned as a member.
#[test]
fn a_strip_of_the_sheathing_is_not_a_member() {
    let stud = |name: &str, x: f64| cuboid(name, [x, 0.0, 60.0], [x + 60.0, 140.0, 2440.0]);
    let lying = |name: &str, z: f64| cuboid(name, [0.0, 0.0, z], [1000.0, 140.0, z + 60.0]);
    let panel = vec![
        lying("p/bottom plate", 0.0),
        lying("p/top plate", 2440.0),
        stud("p/stud 1", 0.0),
        stud("p/stud 2", 470.0),
        stud("p/stud 3", 940.0),
        // Deep, but no board: the studs beside it stay members.
        cuboid("p/header", [60.0, 0.0, 1800.0], [470.0, 140.0, 2040.0]),
        cuboid("p/osb", [0.0, -15.0, 0.0], [988.0, 0.0, 2500.0]),
        cuboid("p/strip", [988.0, -15.0, 0.0], [1000.0, 0.0, 2500.0]),
    ];
    let position = |material: &str, dimensions_mm: [f64; 3], solids: &[&str]| ShopPosition {
        material: material.to_owned(),
        dimensions_mm,
        solids: solids.iter().map(|name| format!("p/{name}")).collect(),
        count: solids.len(),
        mirror_of: None,
    };
    let sheet = ShopSheet {
        title: "Panel p".to_owned(),
        right: [1.0, 0.0, 0.0],
        up: [0.0, 0.0, 1.0],
        positions: vec![
            position("C24", [1000.0, 140.0, 60.0], &["bottom plate", "top plate"]),
            position(
                "C24",
                [2380.0, 140.0, 60.0],
                &["stud 1", "stud 2", "stud 3"],
            ),
            position("C24", [410.0, 240.0, 140.0], &["header"]),
            position("OSB", [2500.0, 988.0, 15.0], &["osb"]),
            position("OSB", [2500.0, 15.0, 12.0], &["strip"]),
        ],
    };
    let columns = ["Pos.", "Material", "Size", "Count"].map(str::to_owned);
    let shop = shop::plan_shop(&sheet, &panel, &columns, "", None)
        .unwrap()
        .unwrap();
    let numbers = |view: &DrawnView| {
        view.marks
            .iter()
            .map(|(_, number)| number.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(numbers(&shop.view), ["1", "1", "2", "2", "2", "3"]);
    assert_eq!(shop.layers.len(), 1);
    assert_eq!(numbers(&shop.layers[0].1), ["4", "5"]);
    assert_eq!(
        shop.summary.across,
        [0.0, 60.0, 470.0, 530.0, 940.0, 1000.0]
    );
    assert_eq!(shop.summary.up, [0.0, 60.0, 2440.0, 2500.0]);
}
