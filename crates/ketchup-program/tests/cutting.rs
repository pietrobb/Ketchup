//! Cutting plans (topic cutting): the made parts of a material with stock()
//! laid on bars of stock lengths or on sheets, with the kerf between pieces,
//! the trimmed edges and what no stock holds. The 1D plans are worked by hand
//! in each test.

use ketchup_program::bom::bom;
use ketchup_program::cutting::{CuttingPlan, Layout, StockKind, cutting_table};
use ketchup_program::table::DecimalSeparator;
use std::collections::BTreeMap;

fn plans(source: &str) -> Vec<CuttingPlan> {
    let model = ketchup_program::evaluate("cutting.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    bom(&model).cutting
}

fn refused(source: &str) -> String {
    match ketchup_program::evaluate("cutting.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("accepted: {source}"),
        Err(error) => format!("{error:#}"),
    }
}

fn names(layout: &Layout) -> Vec<&str> {
    layout
        .pieces
        .iter()
        .map(|piece| piece.part.as_str())
        .collect()
}

/// A bar along x of `section` (width, height) and `length`.
fn bar(name: &str, length: f64, section: (f64, f64), y: f64) -> String {
    format!(
        "member(\"{name}\", (0, {y}, 0), ({length}, {y}, 0), ({}, {}), material = \"KVH\")\n",
        section.0, section.1
    )
}

#[test]
fn bars_are_cut_longest_first_into_the_fullest_bar_that_still_holds_the_piece() {
    let mut source = "bar_stock(\"KVH\", [3000, 6000], kerf = 4)\n".to_owned();
    for (index, length) in [1000.0, 2500.0, 1000.0, 3200.0, 1000.0, 2500.0, 7000.0]
        .into_iter()
        .enumerate()
    {
        source += &bar(
            &format!("s{index}"),
            length,
            (60.0, 120.0),
            200.0 * index as f64,
        );
    }
    source += &bar("rail", 2000.0, (60.0, 200.0), 5000.0);
    let plans = plans(&source);
    assert_eq!(plans.len(), 2, "one plan per cross-section: {plans:#?}");
    // Sections sort by their keys: 120 x 60 before 200 x 60.
    let studs = &plans[0];
    assert_eq!(studs.kind, StockKind::Lengths);
    assert_eq!(studs.section_mm, [120.0, 60.0]);
    // 3200 opens bar 1 (3200); 2500 joins it (3200 + 4 + 2500 = 5704 <= 6000);
    // the second 2500 opens bar 2; each 1000 fits bar 1 no more (6708), so bar
    // 2 takes all three: 2500 + 3 x (4 + 1000) = 5512.
    assert_eq!(studs.layouts.len(), 2);
    assert_eq!(names(&studs.layouts[0]), ["s3", "s1"]);
    assert_eq!(names(&studs.layouts[1]), ["s5", "s0", "s2", "s4"]);
    assert_eq!(studs.layouts[0].stock_mm, [6000.0]);
    assert_eq!(studs.layouts[0].pieces[1].at_mm, [3204.0]);
    // 6000 - (3200 + 4 + 2500 + 4) = 292 left after the cut behind the last piece.
    assert_eq!(studs.layouts[0].offcut_mm, Some(292.0));
    assert_eq!(studs.layouts[1].offcut_mm, Some(484.0));
    assert_eq!(studs.layouts[0].used_percent, 95.0);
    assert_eq!(studs.layouts[1].used_percent, 91.7);
    // 11200 of 12000 mm.
    assert_eq!(studs.used_percent, 93.3);
    assert_eq!(studs.stock.len(), 1);
    assert_eq!(
        (studs.stock[0].size_mm.clone(), studs.stock[0].count),
        (vec![6000.0], 2)
    );
    assert_eq!(studs.unplaced.len(), 1);
    assert_eq!(studs.unplaced[0].part, "s6");
    assert!(
        studs.unplaced[0]
            .reason
            .contains("longer than the longest stock length 6000")
    );
    // A bar of 2000 is cut from the shorter stock length.
    let rail = &plans[1];
    assert_eq!(rail.layouts.len(), 1);
    assert_eq!(rail.layouts[0].stock_mm, [3000.0]);
    assert_eq!(rail.layouts[0].offcut_mm, Some(996.0));
}

#[test]
fn trim_comes_off_each_end_of_a_bar_before_the_first_piece() {
    let mut source = "bar_stock(\"KVH\", [4000], kerf = 5, trim = 10)\n".to_owned();
    source += &bar("a", 1990.0, (60.0, 120.0), 0.0);
    source += &bar("b", 1985.0, (60.0, 120.0), 200.0);
    source += &bar("c", 1986.0, (60.0, 120.0), 400.0);
    let plans = plans(&source);
    // 3980 usable: 1986 does not join a (3981); 1985 fits both bars and goes
    // to the fuller one, a's, exactly: 1990 + 5 + 1985 = 3980.
    assert_eq!(plans[0].layouts.len(), 2, "{plans:#?}");
    assert_eq!(names(&plans[0].layouts[0]), ["a", "b"]);
    assert_eq!(names(&plans[0].layouts[1]), ["c"]);
    assert_eq!(plans[0].layouts[0].offcut_mm, Some(0.0));
    assert_eq!(plans[0].layouts[0].pieces[0].at_mm, [10.0]);
    assert_eq!(plans[0].layouts[0].pieces[1].at_mm, [2005.0]);
}

/// Every piece lies inside the trimmed sheet, a kerf clear of the others.
fn assert_sheet_cuttable(layout: &Layout, kerf: f64, trim: f64) {
    let [length, width] = [layout.stock_mm[0], layout.stock_mm[1]];
    for (index, piece) in layout.pieces.iter().enumerate() {
        let [x, y] = [piece.at_mm[0], piece.at_mm[1]];
        let [l, w] = [piece.size_mm[0], piece.size_mm[1]];
        assert!(x >= trim - 1e-6 && y >= trim - 1e-6, "{piece:?}");
        assert!(
            x + l <= length - trim + 1e-6 && y + w <= width - trim + 1e-6,
            "{piece:?} on {length} x {width}"
        );
        for other in &layout.pieces[index + 1..] {
            let [ox, oy] = [other.at_mm[0], other.at_mm[1]];
            let [ol, ow] = [other.size_mm[0], other.size_mm[1]];
            let apart = x + l + kerf <= ox + 1e-6
                || ox + ol + kerf <= x + 1e-6
                || y + w + kerf <= oy + 1e-6
                || oy + ow + kerf <= y + 1e-6;
            assert!(apart, "{piece:?} and {other:?} are less than a kerf apart");
        }
    }
}

#[test]
fn sheets_take_the_pieces_with_kerf_and_trim_and_shrink_to_the_smallest_stock_that_holds_them() {
    let mut source =
        "sheet_stock(\"OSB\", [(2500, 1250), (1250, 1250)], kerf = 4, trim = 10)\n".to_owned();
    // 2 x 1204 = 2408 along and 2 x 604 = 1208 across fit 2484 x 1234: four a sheet.
    for index in 0..5 {
        source += &format!(
            "board(\"p{index}\", (1200, 600, 18), at = (0, 0, {}), material = \"OSB\")\n",
            30 * index
        );
    }
    source += "board(\"thin\", (800, 300, 12), at = (0, 0, 500), material = \"OSB\")\n";
    source += "board(\"big\", (2600, 1000, 18), at = (0, 0, 600), material = \"OSB\")\n";
    let plans = plans(&source);
    assert_eq!(plans.len(), 2, "one plan per thickness: {plans:#?}");
    let thin = &plans[0];
    assert_eq!(thin.section_mm, [12.0]);
    assert_eq!(thin.layouts[0].stock_mm, [1250.0, 1250.0]);
    let thick = &plans[1];
    assert_eq!(thick.kind, StockKind::Sheets);
    assert_eq!(thick.layouts.len(), 2);
    assert_eq!(thick.layouts[0].pieces.len(), 4);
    assert_eq!(thick.layouts[0].stock_mm, [2500.0, 1250.0]);
    // 4 x 0.72 of 3.125 m2.
    assert_eq!(thick.layouts[0].used_percent, 92.2);
    // The fifth piece alone fits the smaller sheet.
    assert_eq!(thick.layouts[1].stock_mm, [1250.0, 1250.0]);
    for layout in plans.iter().flat_map(|plan| &plan.layouts) {
        assert_sheet_cuttable(layout, 4.0, 10.0);
    }
    assert_eq!(thick.unplaced.len(), 1);
    assert_eq!(thick.unplaced[0].part, "big");
    assert!(
        thick.unplaced[0]
            .reason
            .contains("does not fit the largest stock sheet 2500 x 1250")
    );
}

#[test]
fn many_sheet_pieces_stay_cuttable_and_never_overlap() {
    let mut source = "sheet_stock(\"MDF\", [(2800, 2070)], kerf = 3.2, trim = 8)\n".to_owned();
    let mut index = 0;
    for (l, w, count) in [
        (720.0, 560.0, 9),
        (560.0, 300.0, 14),
        (1800.0, 400.0, 5),
        (450.0, 450.0, 7),
    ] {
        for _ in 0..count {
            source += &format!(
                "board(\"m{index}\", ({l}, {w}, 19), at = (0, 0, {}), material = \"MDF\")\n",
                25 * index
            );
            index += 1;
        }
    }
    let plans = plans(&source);
    let placed: usize = plans[0]
        .layouts
        .iter()
        .map(|layout| layout.pieces.len())
        .sum();
    assert_eq!(placed, index, "{plans:#?}");
    assert!(plans[0].unplaced.is_empty());
    for layout in &plans[0].layouts {
        assert_sheet_cuttable(layout, 3.2, 8.0);
    }
    // 11.0 m2 of pieces on 5.796 m2 sheets: two would need 95 %.
    assert_eq!(plans[0].layouts.len(), 3, "{plans:#?}");
}

#[test]
fn grain_keeps_a_part_along_the_sheet_and_refuses_what_cannot_lie_that_way() {
    let source = "sheet_stock(\"OSB\", [(2500, 1250)], kerf = 4, trim = 10, grain = True)
board(\"along\", (600, 1200, 18), material = \"OSB\", grain = \"y\")
board(\"across\", (1200, 600, 18), at = (0, 0, 30), material = \"OSB\", grain = \"y\")
board(\"free\", (500, 400, 18), at = (0, 0, 60), material = \"OSB\")
board(\"flat\", (500, 400, 18), at = (0, 0, 90), material = \"OSB\", grain = \"z\")
board(\"wide\", (1240, 2400, 18), at = (0, 0, 120), material = \"OSB\", grain = \"x\")
";
    let plans = plans(source);
    let plan = &plans[0];
    let piece = |name: &str| {
        plan.layouts
            .iter()
            .flat_map(|layout| &layout.pieces)
            .find(|piece| piece.part == name)
            .unwrap_or_else(|| panic!("{name}: {plan:#?}"))
    };
    // Its grain (y, 1200) is its length: it lies along the sheet.
    assert_eq!(
        (piece("along").size_mm.clone(), piece("along").turned),
        (vec![1200.0, 600.0], false)
    );
    // Its grain (y, 600) is its width: it lies turned, 600 along the sheet.
    assert_eq!(
        (piece("across").size_mm.clone(), piece("across").turned),
        (vec![600.0, 1200.0], true)
    );
    piece("free");
    let unplaced: BTreeMap<&str, &str> = plan
        .unplaced
        .iter()
        .map(|piece| (piece.part.as_str(), piece.reason.as_str()))
        .collect();
    assert!(
        unplaced["flat"].contains("runs through its thickness"),
        "{unplaced:?}"
    );
    // 2400 across a 1250 sheet: it would fit only turned against its grain.
    assert!(
        unplaced["wide"].contains("laid with its grain along the sheet"),
        "{unplaced:?}"
    );
    for layout in &plan.layouts {
        assert_sheet_cuttable(layout, 4.0, 10.0);
    }
}

#[test]
fn parts_of_a_material_without_stock_are_not_planned() {
    let source = "bar_stock(\"KVH\", [5000], kerf = 4)
member(\"stud\", (0, 0, 0), (2400, 0, 0), (60, 120), material = \"KVH\")
member(\"post\", (0, 300, 0), (2400, 300, 0), (100, 100), material = \"BSH\")
";
    let plans = plans(source);
    assert_eq!(plans.len(), 1);
    assert_eq!(names(&plans[0].layouts[0]), ["stud"]);
}

#[test]
fn the_cutting_table_lists_every_piece_on_its_stock_then_what_did_not_fit() {
    let mut source = "bar_stock(\"KVH\", [6000], kerf = 4)\n".to_owned();
    source += &bar("a", 2500.0, (60.0, 120.0), 0.0);
    source += &bar("b", 6500.0, (60.0, 120.0), 200.0);
    let table = cutting_table(&plans(&source));
    assert_eq!(
        table
            .csv(DecimalSeparator::Comma)
            .lines()
            .collect::<Vec<_>>(),
        [
            "\u{feff}material;kind;section_mm;stock_no;stock_mm;part;at_mm;size_mm;turned;used_percent;offcut_mm;note",
            "KVH;lengths;120 x 60;1;6000;a;0;2500;;41,7;3496;",
            "KVH;lengths;120 x 60;;;b;;;;;;not planned: 6500 mm is longer than the longest stock length 6000 mm less its trim",
        ]
    );
}

#[test]
fn stock_refuses_what_cannot_be_cut() {
    for (source, message) in [
        ("stock(\"\", lengths = [5000])", "name the material"),
        ("stock(\"KVH\")", "give either lengths= or sheets="),
        (
            "stock(\"KVH\", lengths = [5000], sheets = [(2500, 1250)])",
            "give either lengths= or sheets=",
        ),
        ("stock(\"KVH\", lengths = [])", "at least one size"),
        (
            "stock(\"KVH\", lengths = [5000], kerf = -1)",
            "kerf must be zero or more",
        ),
        (
            "stock(\"KVH\", lengths = [15], trim = 10)",
            "longer than twice the trim",
        ),
        (
            "stock(\"KVH\", lengths = [5000], along = \"grain\")",
            "along= applies to sheets",
        ),
    ] {
        let error = refused(source);
        assert!(error.contains(message), "{source}: {error}");
    }
}

/// A wall framed at 625 with OSB boards 1250 x 2500, grain up, on the
/// documented stock with its default kerf 4 and trim 10.
fn sheathed_wall(length: f64, height: f64, openings: &str) -> String {
    format!(
        "buildup('wall', (0, 0, 0), (1, 0, 0), (0, 0, 1), {length}, height = {height},\n\
         layers = [{{'name': 'frame', 'thickness': 140, 'material': 'C24', 'spacing': 625}},\n\
                   {{'name': 'osb', 'thickness': 15, 'material': 'OSB', 'sheet': (1250, 2500), 'grain': 'v'}}],\n\
         openings = [{openings}])\n\
         sheet_stock('OSB', [(2500, 1250)], grain = True)\n"
    )
}

/// Every piece lies on its sheet a kerf clear of the others and the trim
/// clear of the sheet's edges, except along a side it spans whole: a whole
/// board, or one only cut to length, keeps the factory edges there.
fn assert_cuttable_keeping_factory_edges(layout: &Layout, kerf: f64, trim: f64) {
    let stock = [layout.stock_mm[0], layout.stock_mm[1]];
    for (index, piece) in layout.pieces.iter().enumerate() {
        for axis in 0..2 {
            let (at, size) = (piece.at_mm[axis], piece.size_mm[axis]);
            if (size - stock[axis]).abs() < 1e-6 {
                assert!(at.abs() < 1e-6, "{piece:?}");
            } else {
                assert!(at >= trim - 1e-6, "{piece:?}");
                assert!(
                    at + size <= stock[axis] - trim + 1e-6,
                    "{piece:?} on {stock:?}"
                );
            }
        }
        for other in &layout.pieces[index + 1..] {
            let apart = (0..2).any(|axis| {
                piece.at_mm[axis] + piece.size_mm[axis] + kerf <= other.at_mm[axis] + 1e-6
                    || other.at_mm[axis] + other.size_mm[axis] + kerf <= piece.at_mm[axis] + 1e-6
            });
            assert!(apart, "{piece:?} and {other:?} are less than a kerf apart");
        }
    }
}

/// Capability review runs 349 and 352: a buildup's sheathing came as
/// wall-high columns wider than any sheet and was all "not planned"; then,
/// laid from boards, the boards as long as the stock sheet still did not fit
/// it less its default trim. With "sheet" the layer is laid from boards with
/// their joints on stud centres, and the plan saws every one of them from the
/// documented stock, whole boards and boards cut only to length keeping the
/// factory edges.
#[test]
fn a_sheathed_buildup_is_cut_from_whole_sheets_with_joints_on_studs() {
    for (length, height, openings, window) in [
        (3000.0, 2500.0, "(1000, 800, 900, 1200)", 900.0 * 1200.0),
        (4000.0, 2500.0, "", 0.0),
        (2500.0, 2500.0, "", 0.0),
        (3000.0, 2400.0, "(1000, 800, 900, 1200)", 900.0 * 1200.0),
    ] {
        let source = sheathed_wall(length, height, openings);
        let model = ketchup_program::evaluate("cutting.star", &source, &BTreeMap::new())
            .unwrap_or_else(|error| panic!("{error}"))
            .model;
        let osb = bom(&model)
            .cutting
            .into_iter()
            .find(|plan| plan.material == "OSB")
            .unwrap();
        assert_eq!(osb.unplaced, [], "{length}");
        let placed = osb
            .layouts
            .iter()
            .flat_map(|layout| &layout.pieces)
            .map(|piece| piece.size_mm[0] * piece.size_mm[1])
            .sum::<f64>();
        assert!(
            (placed - (length * height - window)).abs() < 1e-3,
            "{length}: {placed}"
        );
        for layout in &osb.layouts {
            assert_cuttable_keeping_factory_edges(layout, 4.0, 10.0);
        }
        // Each whole 1250 x 2500 board is one whole stock sheet.
        let whole = (length * height / (1250.0 * 2500.0)).floor();
        if height == 2500.0 && window == 0.0 {
            let full = osb
                .layouts
                .iter()
                .filter(|layout| layout.used_percent == 100.0)
                .count();
            assert_eq!(full as f64, whole, "{length}: {osb:#?}");
        }
        // Grain up the wall runs along the sheet's length: every piece lies
        // with its height on the wall along the sheet.
        for piece in osb.layouts.iter().flat_map(|layout| &layout.pieces) {
            let (min, max) = model.part(&piece.part).unwrap().world_bounds();
            assert!(
                (piece.size_mm[0] - (max[2] - min[2])).abs() < 1e-6,
                "{piece:?}"
            );
        }
        let studs = model
            .parts
            .iter()
            .filter(|part| part.name.starts_with("wall/frame/stud"))
            .map(|part| {
                let (min, max) = part.world_bounds();
                (min[0], max[0])
            })
            .collect::<Vec<_>>();
        let mut joints = Vec::new();
        for part in model
            .parts
            .iter()
            .filter(|part| part.name.starts_with("wall/osb"))
        {
            assert_eq!(part.attributes.get("grain").map(String::as_str), Some("y"));
            let (min, max) = part.world_bounds();
            assert!(max[0] - min[0] <= 1250.0 + 1e-6, "{}", part.name);
            joints.extend([min[0], max[0]]);
        }
        // A board edge that is not a panel end or an opening side sits on a
        // stud, the stud's centre on the joint.
        for u in joints {
            if u < 1.0 || u > length - 1.0 || (window > 0.0 && [1000.0, 1900.0].contains(&u)) {
                continue;
            }
            assert!(
                studs
                    .iter()
                    .any(|(low, high)| ((low + high) / 2.0 - u).abs() < 1e-6),
                "{length}: joint at u = {u} has no stud under it: {studs:?}"
            );
        }
    }
}
