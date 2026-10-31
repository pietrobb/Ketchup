//! The quote (topic quote): what the bill of materials counts at the unit
//! prices a program declares, with whatever has no usable price listed and
//! left out of the total. Every amount is worked by hand in its test.

use ketchup_program::bom::bom;
use ketchup_program::quote::{ItemKind, Quote, Unit, quote_table};
use ketchup_program::table::DecimalSeparator;
use std::collections::BTreeMap;

fn quote(source: &str) -> Option<Quote> {
    let model = ketchup_program::evaluate("quote.star", source, &BTreeMap::new())
        .unwrap_or_else(|error| panic!("{error}"))
        .model;
    bom(&model).quote
}

fn refused(source: &str) -> String {
    match ketchup_program::evaluate("quote.star", source, &BTreeMap::new()) {
        Ok(_) => panic!("accepted: {source}"),
        Err(error) => format!("{error:#}"),
    }
}

const CABINET: &str = r#"
side = board("side", (800, 400, 18), material = "Board")
board("top", (600, 400, 18), at = (0, 0, 1000), material = "Board")
hole(side, "z+", at = (100, 100), diameter = 8, depth = 12)
hole(side, "z+", at = (300, 100), diameter = 8, depth = 12)
member("rail", (0, 1000, 0), (2000, 1000, 0), (60, 120), material = "KVH")
member("post", (0, 2000, 0), (1000, 2000, 0), (60, 120), material = "KVH")
sheet_stock("OSB", [(2500, 1250)], kerf = 4, trim = 10)
board("osb 1", (1200, 600, 15), at = (0, 3000, 0), material = "OSB")
board("osb 2", (1200, 600, 15), at = (0, 4000, 0), material = "OSB")
board("pane", (100, 100, 100), at = (0, 6000, 0), material = "Glass")
bolt("b1", "M8", 40, at = (0, 5000, 0))
bolt("b2", "M8", 40, at = (100, 5000, 0))
price_list({
    "KVH": (500, "m3"),
    "Board": (20, "m2"),
    "OSB": (8, "piece"),
    "ISO 4017 M8x40": (0.25, "piece"),
    "drill": (0.5, "operation"),
    "Plywood": (30, "m2"),
}, currency = "EUR", source = "offer 2026-10")
"#;

/// Bars by their blanks' volume (2000 + 1000 mm of 120 x 60 = 0.0216 m3),
/// boards by area (800 x 400 + 600 x 400 = 0.56 m2), OSB by the one sheet
/// both pieces are cut from (1200 + 4 + 1200 <= 2500 - 2 x 10), two bolts and
/// two holes: 10.80 + 11.20 + 8.00 + 0.50 + 1.00 = 31.50 EUR.
#[test]
fn every_counted_item_is_priced_by_its_unit_and_summed() {
    let quote = quote(CABINET).expect("the program declares prices");
    assert_eq!(quote.currency.as_deref(), Some("EUR"));
    let lines: Vec<_> = quote
        .lines
        .iter()
        .map(|line| {
            (
                line.kind,
                line.item.as_str(),
                line.quantity,
                line.unit,
                line.amount,
                line.basis.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        lines,
        [
            (
                ItemKind::Material,
                "Board",
                0.56,
                Unit::SquareMetre,
                11.2,
                Some("blanks")
            ),
            (
                ItemKind::Material,
                "KVH",
                0.0216,
                Unit::CubicMetre,
                10.8,
                Some("blanks")
            ),
            (
                ItemKind::Material,
                "OSB",
                1.0,
                Unit::Piece,
                8.0,
                Some("stock")
            ),
            (
                ItemKind::Hardware,
                "ISO 4017 M8x40",
                2.0,
                Unit::Piece,
                0.5,
                None
            ),
            (
                ItemKind::Operation,
                "drill",
                2.0,
                Unit::Operation,
                1.0,
                None
            ),
        ]
    );
    assert_eq!(quote.total, 31.5);
    assert!(
        quote
            .lines
            .iter()
            .all(|line| line.source.as_deref() == Some("offer 2026-10"))
    );
    assert_eq!(quote.unpriced.len(), 1, "{:?}", quote.unpriced);
    assert_eq!(
        (quote.unpriced[0].kind, quote.unpriced[0].item.as_str()),
        (ItemKind::Material, "Glass")
    );
    assert_eq!(quote.unpriced[0].reason, "no price()");
    assert_eq!(quote.unused_prices, ["Plywood"]);
}

/// A material with stock is bought as the stock its plan uses, by any unit:
/// one 2500 x 1250 sheet is 3.125 m2, not the pieces' 1.44 m2.
#[test]
fn stock_is_priced_by_the_sheets_used_not_the_pieces() {
    let source = CABINET.replace(r#""OSB": (8, "piece")"#, r#""OSB": (4, "m2")"#);
    let quote = quote(&source).unwrap();
    let osb = quote.lines.iter().find(|line| line.item == "OSB").unwrap();
    assert_eq!((osb.quantity, osb.amount), (3.125, 12.5));
}

/// Bars by mass: two Ø12 x 1000 B500B bars of 0.888 kg at 1.20 a kg are
/// 1.776 kg, 2.13; a bolt priced per m2 cannot be counted so and is listed.
#[test]
fn bars_by_mass_and_a_unit_an_item_is_not_counted_in_is_listed_not_guessed() {
    let quote = quote(
        r#"
rebar("r1", 12, 1000)
rebar("r2", 12, 1000, at = (0, 100, 0))
bolt("b", "M8", 40, at = (0, 500, 0))
price("B500B", 1.2, "kg")
price("ISO 4017 M8x40", 2, "m2")
"#,
    )
    .unwrap();
    assert_eq!(quote.lines.len(), 1, "{:?}", quote.lines);
    let bars = &quote.lines[0];
    assert_eq!(
        (bars.kind, bars.item.as_str(), bars.unit),
        (ItemKind::Bars, "B500B", Unit::Kilogram)
    );
    assert_eq!((bars.quantity, bars.amount), (1.776, 2.13));
    assert_eq!(quote.total, 2.13);
    assert_eq!(quote.currency, None);
    assert_eq!(quote.unpriced.len(), 1);
    assert_eq!(
        quote.unpriced[0].reason,
        "priced per m2, but hardware is counted per piece"
    );
}

#[test]
fn a_program_without_prices_has_no_quote() {
    assert_eq!(
        quote("board(\"a\", (100, 100, 18), material = \"Board\")\n"),
        None
    );
}

#[test]
fn a_bad_unit_a_negative_price_or_a_second_currency_is_refused() {
    let error = refused("price(\"KVH\", 500, \"hour\")\n");
    assert!(error.contains("per must be one of"), "{error}");
    assert!(error.contains("\"m3\""), "{error}");
    let error = refused("price(\"KVH\", -1, \"m3\")\n");
    assert!(error.contains("zero or more"), "{error}");
    let error = refused(
        "price(\"KVH\", 500, \"m3\", currency = \"EUR\")\n\
         price(\"OSB\", 9, \"m2\", currency = \"CZK\")\n",
    );
    assert!(error.contains("one currency"), "{error}");
    assert!(error.contains("EUR"), "{error}");
}

/// Review 2026-10-29 PG-66: a second price() of an item silently replaced the first, and
/// a price without a currency passed beside one in EUR, in either order.
#[test]
fn a_second_price_of_an_item_or_a_price_without_the_currency_of_the_others_is_refused() {
    let error = refused(
        "price(\"KVH\", 500, \"m3\")\n\
         price(\"KVH\", 450, \"m3\")\n",
    );
    assert!(error.contains("priced before"), "{error}");
    assert!(error.contains("500"), "{error}");
    for source in [
        "price(\"KVH\", 500, \"m3\")\nprice(\"OSB\", 9, \"m2\", currency = \"EUR\")\n",
        "price(\"KVH\", 500, \"m3\", currency = \"EUR\")\nprice(\"OSB\", 9, \"m2\")\n",
    ] {
        let error = refused(source);
        assert!(error.contains("one currency"), "{error}");
        assert!(error.contains("names none"), "{error}");
    }
}

/// Review 2026-10-29 PG-66: a piece no stock holds was priced at its blank with nothing
/// to say so, and nothing said the total has no VAT in it.
#[test]
fn a_piece_no_stock_holds_is_named_on_its_line_and_the_total_says_no_vat_was_added() {
    let source = CABINET.replace(
        "board(\"pane\"",
        "board(\"osb long\", (2600, 600, 15), at = (0, 7000, 0), material = \"OSB\")\nboard(\"pane\"",
    );
    let quote = quote(&source).unwrap();
    let osb = quote.lines.iter().find(|line| line.item == "OSB").unwrap();
    // The sheet, and the long piece at its blank.
    assert_eq!(osb.quantity, 2.0);
    let note = osb.note.as_deref().unwrap_or_default();
    assert!(note.contains("osb long"), "{osb:?}");
    assert!(note.contains("blank"), "{osb:?}");
    let csv = quote_table(&quote).csv(DecimalSeparator::Point);
    assert!(csv.contains("osb long"), "{csv}");
    let total = csv.lines().find(|line| line.starts_with(",total")).unwrap();
    assert!(total.contains("VAT not added"), "{total}");
}

/// The spreadsheet: one row per priced item, the total, then what has no
/// price and the prices that matched nothing.
#[test]
fn the_quote_table_lists_lines_total_unpriced_and_unused_prices() {
    let csv = quote_table(&quote(CABINET).unwrap()).csv(DecimalSeparator::Point);
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(
        lines[0],
        "\u{feff}kind,item,quantity,unit,unit_price,amount,currency,basis,source,note"
    );
    assert_eq!(
        lines[2],
        "material,KVH,0.0216,m3,500,10.8,EUR,blanks,offer 2026-10,"
    );
    assert_eq!(
        lines[6],
        ",total,,,,31.5,EUR,,,VAT not added; not in the total: 1 without a price"
    );
    assert_eq!(lines[7], "material,Glass,,,,,,,,not priced: no price()");
    assert_eq!(lines[8], ",Plywood,,,,,,,,price() matches nothing counted");
    assert_eq!(lines.len(), 9);
}
