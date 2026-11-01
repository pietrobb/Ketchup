//! Golden shop files (review 2026-10-29 proposal 5.1): the cutting plan, the
//! quote and the CNC drawings of one program are compared byte for byte with
//! `tests/fixtures/shop-golden`. CI runs this on Linux, macOS and Windows, so a
//! shop file that differs between systems fails there (ADR 0010 promises the
//! same files everywhere). `KETCHUP_UPDATE_GOLDEN=1` rewrites the files; review
//! their diff before committing.

use ketchup_program::bom::bom;
use ketchup_program::cnc_dxf::cnc_parts;
use ketchup_program::cutting::cutting_table;
use ketchup_program::quote::quote_table;
use ketchup_program::table::DecimalSeparator;
use std::collections::BTreeMap;
use std::path::Path;

/// Bars and sheets from stock, prices, and flat parts with holes, pockets,
/// rounded corners and arcs on several faces.
const PROGRAM: &str = r#"
side = board("side", [18, 400, 720], material = "DTD")
hole(side, "x+", world = (18, 37, 32), diameter = 5, depth = 13, id = "shelf pin")
hole(side, "x-", world = (0, 200, 360), diameter = 8, depth = 18, through = True, id = "bolt")
hole(side, "y-", world = (9, 0, 300), diameter = 8, depth = 30, id = "dowel")
pocket(side, "x+", rect = (0, 0, 400, 8), depth = 6, id = "back groove")
top = extrude("top", profile = round_corners([(0, 0), (600, 0), (600, 300), (0, 300)], 20),
              distance = 18, at = (0, 0, 720), attributes = {FABRICATION_ROLE: MANUFACTURED_ITEM})
cut(top, profile = [["s1", [200, 150], [100, 150], {"center": (150, 150)}],
                    ["s2", [100, 150], [200, 150], {"center": (150, 150)}]], through = True, name = "grommet")
cut(top, profile = [(400, 100), (500, 100), (500, 200), (400, 200)], depth = 5, name = "inlay")
bar_stock("KVH", [3000, 6000], kerf = 4)
member("rail", (0, 1000, 0), (2500, 1000, 0), (60, 120), material = "KVH")
member("post", (0, 2000, 0), (1000, 2000, 0), (60, 120), material = "KVH")
member("brace", (0, 2500, 0), (1300, 3200, 0), (60, 120), material = "KVH")
sheet_stock("OSB", [(2500, 1250)], kerf = 4, trim = 10)
board("osb 1", (1200, 600, 15), at = (0, 3000, 0), material = "OSB")
board("osb 2", (1200, 600, 15), at = (0, 4000, 0), material = "OSB")
board("osb 3", (700, 333.3, 15), at = (0, 5000, 0), material = "OSB")
bolt("b1", "M8", 40, at = (0, 6000, 0))
price_list({
    "KVH": (500, "m3"),
    "DTD": (20.7, "m2"),
    "OSB": (8, "piece"),
    "ISO 4017 M8x40": (0.25, "piece"),
    "drill": (0.5, "operation"),
}, currency = "EUR", source = "offer 2026-10")
"#;

fn shop_files() -> Vec<(&'static str, Vec<u8>)> {
    let model = ketchup_program::evaluate("shop.star", PROGRAM, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error:#}"))
        .model;
    let bom = bom(&model);
    let quote = bom.quote.as_ref().expect("the program declares prices");
    let cnc = cnc_parts(&model);
    assert!(
        cnc.parts.len() >= 2 && bom.cutting.len() >= 2,
        "the program no longer makes the shop files it pins: {cnc:#?}"
    );
    vec![
        (
            "cutting-plan.csv",
            cutting_table(&bom.cutting)
                .csv(DecimalSeparator::Point)
                .into_bytes(),
        ),
        (
            "quote.csv",
            quote_table(quote).csv(DecimalSeparator::Point).into_bytes(),
        ),
        ("cnc-dxf.zip", cnc.zip()),
    ]
}

#[test]
fn the_shop_files_are_the_same_bytes_on_every_system() {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/shop-golden");
    let update = ketchup_test_env::update_golden();
    let mut changed = Vec::new();
    for (name, bytes) in shop_files() {
        let path = directory.join(name);
        if update {
            std::fs::create_dir_all(&directory).unwrap();
            std::fs::write(&path, &bytes).unwrap();
            continue;
        }
        if std::fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            changed.push(name);
        }
    }
    assert!(
        changed.is_empty(),
        "shop files differ from their golden bytes; review and regenerate with KETCHUP_UPDATE_GOLDEN=1: {changed:?}"
    );
}
