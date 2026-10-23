//! Hatches of a DXF drawing read as an underlay: a solid one paints the area of its
//! boundary, islands left out by its style; a pattern is drawn from the line definitions
//! the hatch carries, clipped to that area; both in the color the drawing gives them, also
//! where a mirrored insert places them. A hatch it cannot draw is left out alone and named.
use ketchup_model::import::{DxfImportOptions, DxfUnderlay, read_dxf_underlay};
use ketchup_model::underlay::UnderlayLines;
use ketchup_model::underlay::fill::UnderlayFill;

fn read(blocks: &str, entities: &str) -> DxfUnderlay {
    let source = format!(
        "0\nSECTION\n2\nHEADER\n9\n$INSUNITS\n70\n4\n0\nENDSEC\n\
         0\nSECTION\n2\nTABLES\n0\nTABLE\n2\nLAYER\n70\n1\n\
         0\nLAYER\n2\nfloor\n70\n0\n62\n3\n0\nENDTAB\n0\nENDSEC\n\
         0\nSECTION\n2\nBLOCKS\n{blocks}0\nENDSEC\n\
         0\nSECTION\n2\nENTITIES\n{entities}0\nENDSEC\n0\nEOF\n"
    );
    read_dxf_underlay(source.as_bytes(), DxfImportOptions::new(None)).unwrap()
}

fn layer<'a>(underlay: &'a DxfUnderlay, name: &str) -> &'a ketchup_model::underlay::LayerLines {
    let index = underlay
        .layers
        .iter()
        .position(|layer| layer == name)
        .unwrap_or_else(|| panic!("no layer {name} in {:?}", underlay.layers));
    &underlay.lines.layers()[index]
}

fn area(fill: &UnderlayFill) -> f64 {
    fill.triangles()
        .iter()
        .map(|[a, b, c]| {
            ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() / 2.0
        })
        .sum()
}

/// A closed polyline path of `points`, flagged `flags` besides polyline (2).
fn polyline_path(flags: u32, points: &[[f64; 3]]) -> String {
    let vertices: String = points
        .iter()
        .map(|[x, y, bulge]| format!("10\n{x}\n20\n{y}\n42\n{bulge}\n"))
        .collect();
    format!(
        "92\n{}\n72\n1\n73\n1\n93\n{}\n{vertices}97\n0\n",
        flags | 2,
        points.len()
    )
}

/// An edge path of straight lines through `points`, closed back to the first.
fn line_path(points: &[[f64; 2]]) -> String {
    let edges: String = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .map(|([x0, y0], [x1, y1])| format!("72\n1\n10\n{x0}\n20\n{y0}\n11\n{x1}\n21\n{y1}\n"))
        .collect();
    format!("92\n0\n93\n{}\n{edges}97\n0\n", points.len())
}

fn square(low: f64, high: f64) -> Vec<[f64; 2]> {
    vec![[low, low], [high, low], [high, high], [low, high]]
}

/// A solid hatch of `paths` in the style `style` (75), with `own` groups after its layer.
fn solid(handle: &str, layer: &str, own: &str, style: u32, paths: &[String]) -> String {
    format!(
        "0\nHATCH\n5\n{handle}\n8\n{layer}\n{own}10\n0\n20\n0\n30\n0\n2\nSOLID\n70\n1\n71\n0\n\
         91\n{}\n{}75\n{style}\n76\n1\n98\n0\n",
        paths.len(),
        paths.concat()
    )
}

/// One pattern line: its angle, base point, offset and dashes.
type PatternLine<'a> = (f64, [f64; 2], [f64; 2], &'a [f64]);

/// A pattern hatch named `name` of `paths` and the pattern lines `lines`.
fn patterned(
    handle: &str,
    layer: &str,
    name: &str,
    paths: &[String],
    lines: &[PatternLine<'_>],
) -> String {
    let definitions: String = lines
        .iter()
        .map(|(angle, [x, y], [dx, dy], dashes)| {
            let count = dashes.len();
            let dashes: String = dashes.iter().map(|dash| format!("49\n{dash}\n")).collect();
            format!("53\n{angle}\n43\n{x}\n44\n{y}\n45\n{dx}\n46\n{dy}\n79\n{count}\n{dashes}")
        })
        .collect();
    format!(
        "0\nHATCH\n5\n{handle}\n8\n{layer}\n10\n0\n20\n0\n30\n0\n2\n{name}\n70\n0\n71\n0\n\
         91\n{}\n{}75\n0\n76\n1\n52\n0\n41\n1\n77\n0\n78\n{}\n{definitions}98\n0\n",
        paths.len(),
        paths.concat(),
        lines.len()
    )
}

fn subjects(underlay: &DxfUnderlay, code: &str) -> Vec<String> {
    underlay
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code() == code)
        .filter_map(|diagnostic| diagnostic.subject().map(str::to_owned))
        .collect()
}

#[test]
fn a_solid_hatch_paints_its_area_and_its_islands_by_its_style() {
    // A square of 1 m with a half circle bulging from its top edge, an island of 400 mm
    // and in that one of 200 mm: polyline outer path, line edge paths inside.
    let outer = polyline_path(
        1,
        &[
            [0.0, 0.0, 0.0],
            [1000.0, 0.0, 0.0],
            [1000.0, 1000.0, 1.0],
            [0.0, 1000.0, 0.0],
        ],
    );
    let paths = [
        outer,
        line_path(&square(300.0, 700.0)),
        line_path(&square(400.0, 600.0)),
    ];
    let underlay = read(
        "",
        &[
            solid("A1", "normal", "", 0, &paths),
            solid("A2", "outer", "", 1, &paths),
            solid("A3", "ignore", "", 2, &paths),
        ]
        .concat(),
    );
    let half_circle = std::f64::consts::PI * 500.0 * 500.0 / 2.0;
    for (name, expected) in [
        // Even-odd: the island is a hole, the area in it painted again.
        ("normal", 1_000_000.0 + half_circle - 160_000.0 + 40_000.0),
        // The outermost and the islands directly in it.
        ("outer", 1_000_000.0 + half_circle - 160_000.0),
        // The outermost alone.
        ("ignore", 1_000_000.0 + half_circle),
    ] {
        let fills = layer(&underlay, name).fills();
        assert_eq!(fills.len(), 1, "{name}");
        let painted = area(&fills[0]);
        // The arc is drawn in chords: a little of it is cut off.
        assert!(
            painted <= expected + 1e-6 && painted > expected - 300.0,
            "{name}: {painted} of {expected}"
        );
        // A hatch draws no line of its own.
        assert_eq!(layer(&underlay, name).segment_count(), 0, "{name}");
    }
    assert!(subjects(&underlay, "dxf.hatch-skipped").is_empty());
    // The fills travel in the blob, and the zoom of the drawing reaches the arc.
    let decoded = UnderlayLines::decode(&underlay.lines.encode()).unwrap();
    assert_eq!(decoded, underlay.lines);
    let [low, high] = underlay.lines.bounds_mm().unwrap();
    assert!(low[1] <= 0.0 && high[1] > 1499.0, "{low:?} {high:?}");
}

/// Where a point of a clipped dash may lie: on the square of 1 m or the half circle of
/// 500 mm right of it, within the chords of the arc.
fn distance_to_boundary([x, y]: [f64; 2]) -> f64 {
    let to_line = |from: [f64; 2], to: [f64; 2]| {
        let along = [to[0] - from[0], to[1] - from[1]];
        let length = along[0].hypot(along[1]);
        let share = (((x - from[0]) * along[0] + (y - from[1]) * along[1]) / (length * length))
            .clamp(0.0, 1.0);
        (x - from[0] - along[0] * share).hypot(y - from[1] - along[1] * share)
    };
    let arc = if x >= 1000.0 {
        ((x - 1000.0).hypot(y - 500.0) - 500.0).abs()
    } else {
        f64::INFINITY
    };
    [
        to_line([0.0, 0.0], [1000.0, 0.0]),
        to_line([1000.0, 1000.0], [0.0, 1000.0]),
        to_line([0.0, 1000.0], [0.0, 0.0]),
        arc,
    ]
    .into_iter()
    .fold(f64::INFINITY, f64::min)
}

#[test]
fn a_crossing_pattern_is_drawn_from_its_own_lines_clipped_to_a_boundary_with_an_arc() {
    // A square with a half circle on its right edge: three lines and an arc edge.
    let boundary = concat!(
        "92\n1\n93\n4\n",
        "72\n1\n10\n0\n20\n0\n11\n1000\n21\n0\n",
        "72\n2\n10\n1000\n20\n500\n40\n500\n50\n270\n51\n90\n73\n1\n",
        "72\n1\n10\n1000\n20\n1000\n11\n0\n21\n1000\n",
        "72\n1\n10\n0\n20\n1000\n11\n0\n21\n0\n",
        "97\n0\n",
    )
    .to_owned();
    let step = 100.0 / std::f64::consts::SQRT_2;
    // Named ANSI37, but its own lines are 100 mm apart, not the pattern file's 3.175 mm.
    let underlay = read(
        "",
        &patterned(
            "P1",
            "cross",
            "ANSI37",
            &[boundary],
            &[
                (45.0, [0.0, 0.0], [-step, step], &[]),
                (135.0, [0.0, 0.0], [-step, -step], &[]),
            ],
        ),
    );
    let cross = layer(&underlay, "cross");
    assert!(cross.fills().is_empty());
    let dashes = cross.polylines().collect::<Vec<_>>();
    let [mut rising, mut falling] = [0, 0];
    for dash in &dashes {
        assert_eq!(dash.len(), 2, "{dash:?}");
        let [from, to] = [dash[0], dash[1]];
        // Each runs from boundary to boundary.
        for end in [from, to] {
            assert!(distance_to_boundary(end) < 0.2, "{end:?} of {dash:?}");
        }
        let [dx, dy] = [to[0] - from[0], to[1] - from[1]];
        let length = dx.hypot(dy);
        if (dx - dy).abs() < 1e-9 * length.max(1.0) {
            rising += 1;
        } else if (dx + dy).abs() < 1e-9 * length.max(1.0) {
            falling += 1;
        } else {
            panic!("a dash at neither 45° nor 135°: {dash:?}");
        }
    }
    // Lines 100 mm apart over a diagonal of 1.5 m by 1 m: some 17 to 18 each way.
    assert!((15..=19).contains(&rising), "{rising}");
    assert!((15..=19).contains(&falling), "{falling}");
    // Some reach into the half circle.
    assert!(
        dashes
            .iter()
            .flat_map(|dash| dash.iter())
            .any(|point| point[0] > 1300.0)
    );
}

#[test]
fn a_dashed_pattern_draws_its_dashes_and_a_true_color_is_kept() {
    // Horizontal lines 100 mm apart, 50 mm drawn and 50 mm left out.
    let underlay = read(
        "",
        &[
            patterned(
                "D1",
                "dashed",
                "MINE",
                &[line_path(&square(0.0, 1000.0))],
                &[(0.0, [0.0, 0.0], [0.0, 100.0], &[50.0, -50.0])],
            ),
            solid(
                "T1",
                "colored",
                "62\n1\n420\n1193046\n",
                0,
                &[line_path(&square(0.0, 100.0))],
            ),
        ]
        .concat(),
    );
    let dashed = layer(&underlay, "dashed");
    let dashes = dashed.polylines().collect::<Vec<_>>();
    // Ten lines (the top edge bounds no line), ten dashes each.
    assert_eq!(dashes.len(), 100);
    for dash in dashes {
        let length = (dash[1][0] - dash[0][0]).hypot(dash[1][1] - dash[0][1]);
        assert!((length - 50.0).abs() < 1e-6, "{dash:?}");
    }
    let colored = &layer(&underlay, "colored").fills()[0];
    assert_eq!(
        underlay.lines.style(colored.style()).color,
        Some([0x12, 0x34, 0x56])
    );
}

#[test]
fn a_hatch_of_a_mirrored_block_is_mirrored_in_the_color_of_its_insert() {
    let blocks = format!(
        "0\nBLOCK\n8\n0\n2\ntile\n70\n0\n10\n0\n20\n0\n{}{}0\nENDBLK\n",
        // A triangle painted ByBlock (color 0) on layer 0.
        solid(
            "B1",
            "0",
            "62\n0\n",
            0,
            &[line_path(&[[0.0, 0.0], [100.0, 0.0], [0.0, 100.0]])]
        ),
        // Lines rising to the right inside a square, on layer 0 ByBlock too.
        patterned(
            "B2",
            "0",
            "MINE",
            &[line_path(&[
                [0.0, 200.0],
                [100.0, 200.0],
                [100.0, 300.0],
                [0.0, 300.0]
            ])],
            &[(45.0, [0.0, 0.0], [-10.0, 10.0], &[])],
        )
        .replace(
            "8\n0\n10\n0\n20\n0\n30\n0\n",
            "8\n0\n62\n0\n10\n0\n20\n0\n30\n0\n"
        ),
    );
    // Inserted blue on floor, mirrored in x, at x 1000.
    let underlay = read(
        &blocks,
        "0\nINSERT\n8\nfloor\n2\ntile\n62\n5\n10\n1000\n20\n0\n41\n-1\n42\n1\n",
    );
    let floor = layer(&underlay, "floor");
    let [fill] = floor.fills() else {
        panic!("{:?}", floor.fills());
    };
    assert_eq!(underlay.lines.style(fill.style()).color, Some([0, 0, 255]));
    assert!((area(fill) - 5000.0).abs() < 1e-6);
    // Mirrored: the triangle lies left of the insert.
    for point in fill.points_mm() {
        assert!((900.0..=1000.0).contains(&point[0]), "{point:?}");
    }
    let dashes = floor.styled_polylines().collect::<Vec<_>>();
    assert!(!dashes.is_empty());
    for (style, dash) in dashes {
        assert_eq!(underlay.lines.style(style).color, Some([0, 0, 255]));
        // Rising to the right in the block, falling to the right mirrored.
        let [dx, dy] = [dash[1][0] - dash[0][0], dash[1][1] - dash[0][1]];
        assert!((dx + dy).abs() < 1e-9 * dx.hypot(dy).max(1.0), "{dash:?}");
        assert!(
            dash.iter()
                .all(|point| (900.0..=1000.0).contains(&point[0]))
        );
    }
}

#[test]
fn a_hatch_it_cannot_draw_is_left_out_alone_and_named_where_it_lies() {
    let gradient = solid(
        "G1",
        "shade",
        "",
        0,
        &[line_path(&[
            [0.0, 0.0],
            [1000.0, 0.0],
            [1000.0, 500.0],
            [0.0, 500.0],
        ])],
    )
    .replace(
        "98\n0\n",
        "98\n0\n450\n1\n451\n0\n452\n0\n453\n2\n470\nLINEAR\n",
    );
    // Lines half a micrometre apart over 1 m: more than the drawing's budget.
    let dense = patterned(
        "N1",
        "dense",
        "MINE",
        &[line_path(&square(2000.0, 3000.0))],
        &[(0.0, [0.0, 0.0], [0.0, 0.0005], &[])],
    );
    let underlay = read(
        "",
        &[
            "0\nLINE\n5\nL1\n8\nwalls\n10\n0\n20\n0\n11\n4000\n21\n0\n".to_owned(),
            gradient,
            dense,
            solid("S1", "floor", "", 0, &[line_path(&square(0.0, 100.0))]),
        ]
        .concat(),
    );
    assert_eq!(layer(&underlay, "walls").segment_count(), 1);
    // The hatch that is drawn, in its layer's color (ACI 3), ByLayer.
    let [floor] = layer(&underlay, "floor").fills() else {
        panic!("{:?}", underlay.lines);
    };
    assert_eq!(underlay.lines.style(floor.style()).color, Some([0, 255, 0]));
    assert!(
        !underlay
            .layers
            .iter()
            .any(|name| name == "shade" || name == "dense")
    );
    let skipped = subjects(&underlay, "dxf.hatch-skipped");
    let [dense, shade] = skipped.as_slice() else {
        panic!("{skipped:?}");
    };
    assert!(
        dense.starts_with("HATCH, layer dense: a pattern of more lines (")
            && dense.ends_with(
                ") than the 1000000 dashes left of the drawing's 1000000 for entities, \
                 within x 2000..3000, y 2000..3000 mm; handles: N1"
            ),
        "{dense}"
    );
    assert_eq!(
        shade,
        "HATCH, layer shade: a gradient fill, within x 0..1000, y 0..500 mm; handles: G1"
    );
}
