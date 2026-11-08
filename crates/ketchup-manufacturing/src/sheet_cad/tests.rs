use std::collections::BTreeMap;

use ketchup_dwg::sheet::{SheetEntity, sheet_dxf};

use super::*;
use crate::project_drawings::{
    DrawingSet, DrawingSetOptions, DrawingSolid, PlanSheet, SheetKind, SheetTitles, Storey,
    drawing_set,
};
use crate::title_block::{TitleBlock, TitleField};

/// The twelve triangles of an axis-aligned box.
fn cuboid(name: &str, min: [f64; 3], max: [f64; 3]) -> DrawingSolid {
    let corner = |index: usize| -> [f64; 3] {
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
    DrawingSolid {
        name: name.to_owned(),
        color: [200, 160, 110],
        triangles: quads
            .iter()
            .flat_map(|[a, b, c, d]| {
                [
                    [corner(*a), corner(*b), corner(*c)],
                    [corner(*a), corner(*c), corner(*d)],
                ]
            })
            .collect(),
        faces: (0..6).flat_map(|face| [face, face]).collect(),
        threads: Vec::new(),
    }
}

/// A room of `length` along x and `width` along y, placed at `at`: a floor slab and four
/// 150 mm walls 2.5 m high.
fn room(at: [f64; 2], length: f64, width: f64) -> Vec<DrawingSolid> {
    let [x, y] = at;
    vec![
        cuboid("floor", [x, y, 0.0], [x + length, y + width, 200.0]),
        cuboid("south", [x, y, 200.0], [x + length, y + 150.0, 2700.0]),
        cuboid(
            "north",
            [x, y + width - 150.0, 200.0],
            [x + length, y + width, 2700.0],
        ),
        cuboid(
            "west",
            [x, y + 150.0, 200.0],
            [x + 150.0, y + width - 150.0, 2700.0],
        ),
        cuboid(
            "east",
            [x + length - 150.0, y + 150.0, 200.0],
            [x + length, y + width - 150.0, 2700.0],
        ),
    ]
}

fn set_of(solids: &[DrawingSolid]) -> DrawingSet {
    let options = DrawingSetOptions {
        plans: vec![PlanSheet {
            storey: Storey {
                floor_mm: None,
                cut_mm: 1200.0,
            },
            view_title: "Pôdorys prízemia".to_owned(),
            sheet_title: "Pôdorys".to_owned(),
        }],
        sheet_titles: SheetTitles {
            sections: "Rezy".to_owned(),
            elevations: "Pohľady".to_owned(),
        },
        title_labels: BTreeMap::from([(TitleField::Project, "Stavba".to_owned())]),
        title_block: TitleBlock::from([(TitleField::Project, "Rodinný dom".to_owned())]),
        ..DrawingSetOptions::default()
    };
    drawing_set(solids, &options).expect("the set is drawn")
}

fn polylines<'a>(entities: &'a [SheetEntity], layer: &str) -> Vec<&'a Vec<[f64; 2]>> {
    entities
        .iter()
        .filter_map(|entity| match entity {
            SheetEntity::Polyline {
                layer: on, points, ..
            } if on == layer => Some(points),
            _ => None,
        })
        .collect()
}

fn bounds<'a>(points: impl IntoIterator<Item = &'a [f64; 2]>) -> [[f64; 2]; 2] {
    let mut low = [f64::INFINITY; 2];
    let mut high = [f64::NEG_INFINITY; 2];
    for point in points {
        for axis in 0..2 {
            low[axis] = low[axis].min(point[axis]);
            high[axis] = high[axis].max(point[axis]);
        }
    }
    [low, high].map(|corner| corner.map(|value| (value * 1000.0).round() / 1000.0))
}

#[test]
fn a_plan_lies_in_model_space_where_the_model_lies_and_its_viewport_shows_it_at_the_set_scale() {
    // Far from the origin, as a house placed on a survey is.
    let solids = room([12_000.0, 5_000.0], 6000.0, 3000.0);
    let set = set_of(&solids);
    let sheets = set.cad();
    let plan = &sheets[set
        .sheets
        .iter()
        .position(|sheet| sheet.kind == SheetKind::Plan { storey: 0 })
        .expect("a plan sheet")];
    assert_eq!(plan.layout, "01 Pôdorys");
    let cut = polylines(&plan.model, CUT);
    assert_eq!(cut.len(), 4, "the four walls the plane cuts");
    assert_eq!(
        bounds(cut.into_iter().flatten()),
        [[12_000.0, 5_000.0], [18_000.0, 8_000.0]],
        "at 1:1 where the walls stand"
    );
    assert!(
        plan.model.iter().any(|entity| matches!(
            entity,
            SheetEntity::Fill { layer, color: Some(_), .. } if layer == CUT_FILL
        )),
        "the cut walls are filled"
    );
    assert_eq!(plan.viewports.len(), 1);
    let viewport = plan.viewports[0];
    assert_eq!(viewport.scale, f64::from(set.scale));
    assert_eq!(viewport.twist_deg, 0.0);
    // The viewport's window on paper shows the walls' middle.
    let [x, y] = viewport.view_center;
    assert!(
        (14_000.0..16_000.0).contains(&x) && (5_500.0..7_500.0).contains(&y),
        "{x} {y}"
    );
    assert!(
        plan.paper.iter().any(|entity| matches!(
            entity,
            SheetEntity::Text { layer, value, .. } if layer == VIEW_TITLE && value == "Pôdorys prízemia"
        )),
        "the view's title is on paper"
    );
    assert!(
        plan.paper.iter().any(|entity| matches!(
            entity,
            SheetEntity::Text { layer, value, .. } if layer == SHEET_TEXT && value == "Rodinný dom"
        )),
        "the title block is on paper"
    );
    assert!(
        polylines(&plan.model, DIMENSION).len() > 2,
        "the overall dimensions go with the view"
    );
}

#[test]
fn a_plan_the_sheet_turns_lies_unturned_in_model_space_and_its_viewport_turns_it() {
    // Longer along y: the sheet draws its length to the right.
    let solids = room([0.0, 0.0], 3000.0, 6000.0);
    let set = set_of(&solids);
    let plan = &set.cad()[0];
    let cut = polylines(&plan.model, CUT);
    assert_eq!(
        bounds(cut.into_iter().flatten()),
        [[0.0, 0.0], [3000.0, 6000.0]],
        "the walls where they stand, not turned"
    );
    assert_eq!(plan.viewports[0].twist_deg, -90.0);
}

#[test]
fn sections_keep_their_heights_and_lie_beside_each_other() {
    let set = set_of(&room([0.0, 0.0], 6000.0, 3000.0));
    let index = set
        .sheets
        .iter()
        .position(|sheet| sheet.kind == SheetKind::Sections)
        .expect("a sections sheet");
    let sections = &set.cad()[index];
    assert_eq!(sections.viewports.len(), 2);
    let cut = polylines(&sections.model, CUT);
    let [low, high] = bounds(cut.iter().copied().flatten());
    assert_eq!([low[1], high[1]], [0.0, 2700.0], "y is the height");
    // The longitudinal section 6 m long, the cross section beside it, not over it.
    assert!(high[0] - low[0] > 6000.0 + 3000.0, "{low:?} {high:?}");
}

#[test]
fn a_sheet_of_the_set_reads_back_as_a_dxf_2013_with_its_layout_and_viewports() {
    let set = set_of(&room([0.0, 0.0], 6000.0, 3000.0));
    let files = set.dxf().expect("the sheets are written");
    assert_eq!(files.len(), set.sheets.len());
    let text = String::from_utf8(files[0].clone()).expect("UTF-8");
    assert!(text.contains("AC1027"));
    assert!(
        text.contains("01 Pôdorys"),
        "the layout is named after the sheet"
    );
    for layer in [CUT, CUT_FILL, DIMENSION, VIEW_TITLE, SHEET] {
        assert!(text.contains(&format!("\r\n{layer}\r\n")), "{layer}");
    }
    let dwg = set.dwg().expect("the sheets are written as DWG");
    assert_eq!(dwg.len(), set.sheets.len());
    // The same sheet written once more is the same drawing.
    assert_eq!(sheet_dxf(&set.cad()[0]).expect("written again"), files[0]);
}

#[test]
fn a_long_text_goes_on_over_the_next_lines_and_one_of_spaces_is_none() {
    assert!(text_lines("   ").is_empty());
    let long = format!("{} {}", "a".repeat(200), "b".repeat(100));
    let lines = text_lines(&long);
    assert_eq!(lines, ["a".repeat(200), "b".repeat(100)]);
    let unbroken = "ž".repeat(600);
    assert_eq!(
        text_lines(&unbroken)
            .iter()
            .map(|line| line.chars().count())
            .collect::<Vec<_>>(),
        [250, 250, 100]
    );
}
