//! A priced quote of what the bill of materials counts: each material of the
//! made parts by volume, area, length or piece (the stock used when `stock()`
//! is declared for it, else the parts' blanks), each bought item by piece,
//! bars by mass, length or piece, and each machining operation, at the unit
//! prices the program declares with `price()`. Whatever has no price is
//! listed, never counted as free.

use crate::bom::Bom;
use crate::cutting::StockKind;
use crate::model::ProgramModel;
use crate::table::{Cell, Table};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

/// What one unit price is per.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
pub enum Unit {
    #[serde(rename = "m3")]
    CubicMetre,
    #[serde(rename = "m2")]
    SquareMetre,
    #[serde(rename = "m")]
    Metre,
    #[serde(rename = "piece")]
    Piece,
    #[serde(rename = "kg")]
    Kilogram,
    #[serde(rename = "operation")]
    Operation,
}

impl Unit {
    pub const ALL: [Self; 6] = [
        Self::CubicMetre,
        Self::SquareMetre,
        Self::Metre,
        Self::Piece,
        Self::Kilogram,
        Self::Operation,
    ];

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::CubicMetre => "m3",
            Self::SquareMetre => "m2",
            Self::Metre => "m",
            Self::Piece => "piece",
            Self::Kilogram => "kg",
            Self::Operation => "operation",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|unit| unit.as_str() == text)
    }
}

/// One unit price (`price()`).
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Price {
    pub value: f64,
    pub per: Unit,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// What a quote line counts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemKind {
    /// The made parts of a material.
    Material,
    /// A bought item of the hardware list.
    Hardware,
    /// Bars of a grade cut to length (bar schedule).
    Bars,
    /// A machining operation, e.g. "drill".
    Operation,
}

impl ItemKind {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Material => "material",
            Self::Hardware => "hardware",
            Self::Bars => "bars",
            Self::Operation => "operation",
        }
    }

    /// The units its quantity can be counted in.
    const fn units(self) -> &'static [Unit] {
        match self {
            Self::Material => &[
                Unit::CubicMetre,
                Unit::SquareMetre,
                Unit::Metre,
                Unit::Piece,
            ],
            Self::Hardware => &[Unit::Piece],
            Self::Bars => &[Unit::Kilogram, Unit::Metre, Unit::Piece],
            Self::Operation => &[Unit::Operation],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct QuoteLine {
    pub kind: ItemKind,
    pub item: String,
    pub quantity: f64,
    pub unit: Unit,
    pub unit_price: f64,
    /// Quantity × unit price, rounded to cents.
    pub amount: f64,
    /// Materials: "stock" (the bars or sheets the cutting plan uses, plus any
    /// piece no stock holds) or "blanks" (the parts' bounding sizes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub basis: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// What the quantity holds that its basis does not say: the pieces of a material
    /// with stock that no stock holds, priced at their blanks (review 2026-10-29 PG-66).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Something counted that the quote could not price.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Unpriced {
    pub kind: ItemKind,
    pub item: String,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct Quote {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub currency: Option<String>,
    pub lines: Vec<QuoteLine>,
    /// Sum of the lines; the unpriced items are not in it.
    pub total: f64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unpriced: Vec<Unpriced>,
    /// Declared prices that match nothing counted (a typo, or nothing made).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unused_prices: Vec<String>,
}

/// Quantities of one item, by unit; a unit it cannot be counted in is absent.
#[derive(Default)]
struct Amounts {
    by_unit: BTreeMap<Unit, f64>,
    basis: Option<&'static str>,
    /// Pieces of a material with stock that no stock holds, counted at their blanks.
    at_blanks: Vec<String>,
    /// Why a unit it could be counted in has no quantity.
    missing: BTreeMap<Unit, &'static str>,
}

impl Amounts {
    fn add(&mut self, unit: Unit, quantity: f64) {
        *self.by_unit.entry(unit).or_default() += quantity;
    }

    /// A blank or a stock piece: count, longest side, largest face, volume.
    fn add_piece(&mut self, length_mm: f64, width_mm: f64, thickness_mm: f64) {
        self.add(Unit::Piece, 1.0);
        self.add(Unit::Metre, length_mm / 1.0e3);
        self.add(Unit::SquareMetre, length_mm * width_mm / 1.0e6);
        self.add(
            Unit::CubicMetre,
            length_mm * width_mm * thickness_mm / 1.0e9,
        );
    }
}

fn round(value: f64, digits: i32) -> f64 {
    let scale = 10_f64.powi(digits);
    (value * scale).round() / scale + 0.0
}

/// A count as a quantity.
fn count(pieces: usize) -> f64 {
    f64::from(u32::try_from(pieces).unwrap_or(u32::MAX))
}

fn quantity_digits(unit: Unit) -> u8 {
    match unit {
        Unit::CubicMetre => 4,
        Unit::SquareMetre | Unit::Metre | Unit::Kilogram => 3,
        Unit::Piece | Unit::Operation => 0,
    }
}

/// Every counted item of `bom` with its quantities.
fn items(bom: &Bom) -> BTreeMap<(ItemKind, String), Amounts> {
    let mut items: BTreeMap<(ItemKind, String), Amounts> = BTreeMap::new();
    let stocked: BTreeSet<&str> = bom
        .cutting
        .iter()
        .filter(|plan| !plan.layouts.is_empty())
        .map(|plan| plan.material.as_str())
        .collect();
    let unplaced: BTreeSet<&str> = bom
        .cutting
        .iter()
        .flat_map(|plan| plan.unplaced.iter().map(|piece| piece.part.as_str()))
        .collect();
    for row in &bom.cut_list {
        let amounts = items
            .entry((ItemKind::Material, row.material.clone()))
            .or_default();
        let [length, width, thickness] = row.dimensions_mm;
        // A material cut from stock is bought as the stock its plan uses; a
        // piece no stock holds is still bought, at its blank.
        let pieces = if stocked.contains(row.material.as_str()) {
            amounts.basis = Some("stock");
            let at_blanks = row
                .parts
                .iter()
                .filter(|part| unplaced.contains(part.as_str()))
                .cloned()
                .collect::<Vec<_>>();
            let count = at_blanks.len();
            amounts.at_blanks.extend(at_blanks);
            count
        } else {
            amounts.basis = Some("blanks");
            row.count
        };
        for _ in 0..pieces {
            amounts.add_piece(length, width, thickness);
        }
    }
    for plan in &bom.cutting {
        let Some(amounts) = items.get_mut(&(ItemKind::Material, plan.material.clone())) else {
            continue;
        };
        for layout in &plan.layouts {
            match (
                plan.kind,
                layout.stock_mm.as_slice(),
                plan.section_mm.as_slice(),
            ) {
                (StockKind::Lengths, [length], [width, thickness])
                | (StockKind::Sheets, [length, width], [thickness]) => {
                    amounts.add_piece(*length, *width, *thickness);
                }
                _ => {}
            }
        }
    }
    for row in &bom.hardware {
        items
            .entry((ItemKind::Hardware, row.item.clone()))
            .or_default()
            .add(Unit::Piece, count(row.count));
    }
    for row in &bom.bar_schedule {
        let amounts = items
            .entry((ItemKind::Bars, row.grade.clone()))
            .or_default();
        amounts.add(Unit::Piece, count(row.count));
        amounts.add(Unit::Metre, row.total_length_m);
        match row.mass_kg {
            Some(kg) if !amounts.missing.contains_key(&Unit::Kilogram) => {
                amounts.add(Unit::Kilogram, kg);
            }
            _ => {
                amounts.by_unit.remove(&Unit::Kilogram);
                amounts
                    .missing
                    .insert(Unit::Kilogram, "a bar of it names no mass_kg");
            }
        }
    }
    for part in &bom.machining {
        for operation in &part.operations {
            items
                .entry((ItemKind::Operation, operation.kind.to_string()))
                .or_default()
                .add(Unit::Operation, 1.0);
        }
    }
    items
}

/// The quote of `bom` at the program's prices, or `None` when it declares
/// none.
#[must_use]
pub fn quote(model: &ProgramModel, bom: &Bom) -> Option<Quote> {
    if model.prices.is_empty() {
        return None;
    }
    let mut lines = Vec::new();
    let mut unpriced = Vec::new();
    let mut used = BTreeSet::new();
    for ((kind, item), amounts) in items(bom) {
        let Some(price) = model.prices.get(&item) else {
            unpriced.push(Unpriced {
                kind,
                item,
                reason: "no price()".to_owned(),
            });
            continue;
        };
        used.insert(item.clone());
        let quantity = amounts.by_unit.get(&price.per).copied();
        let Some(quantity) = quantity.filter(|_| kind.units().contains(&price.per)) else {
            let reason = match amounts.missing.get(&price.per) {
                Some(missing) => format!("priced per {}, but {missing}", price.per.as_str()),
                None => format!(
                    "priced per {}, but {} is counted per {}",
                    price.per.as_str(),
                    kind.as_str(),
                    kind.units()
                        .iter()
                        .map(|unit| unit.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            unpriced.push(Unpriced { kind, item, reason });
            continue;
        };
        let quantity = round(quantity, i32::from(quantity_digits(price.per)));
        lines.push(QuoteLine {
            kind,
            item,
            quantity,
            unit: price.per,
            unit_price: price.value,
            amount: round(quantity * price.value, 2),
            basis: amounts.basis.map(str::to_owned),
            source: price.source.clone(),
            note: (!amounts.at_blanks.is_empty()).then(|| {
                format!(
                    "no stock holds {}, priced at its blank",
                    amounts.at_blanks.join(", ")
                )
            }),
        });
    }
    let total = round(lines.iter().map(|line| line.amount).sum(), 2);
    Some(Quote {
        currency: model.currency.clone(),
        lines,
        total,
        unpriced,
        unused_prices: model
            .prices
            .keys()
            .filter(|name| !used.contains(*name))
            .cloned()
            .collect(),
    })
}

/// The quote as a table for a spreadsheet: one row per priced item, the
/// total, then the items without a price and the prices nothing used.
#[must_use]
pub fn quote_table(quote: &Quote) -> Table {
    let header = [
        "kind",
        "item",
        "quantity",
        "unit",
        "unit_price",
        "amount",
        "currency",
        "basis",
        "source",
        "note",
    ]
    .map(str::to_owned)
    .into();
    let currency = || Cell::Text(quote.currency.clone().unwrap_or_default());
    let empty = || Cell::Text(String::new());
    let mut rows: Vec<Vec<Cell>> = quote
        .lines
        .iter()
        .map(|line| {
            vec![
                Cell::Text(line.kind.as_str().to_owned()),
                Cell::Text(line.item.clone()),
                Cell::Decimal(line.quantity, quantity_digits(line.unit)),
                Cell::Text(line.unit.as_str().to_owned()),
                Cell::Decimal(line.unit_price, 4),
                Cell::Decimal(line.amount, 2),
                currency(),
                Cell::Text(line.basis.clone().unwrap_or_default()),
                Cell::Text(line.source.clone().unwrap_or_default()),
                Cell::Text(line.note.clone().unwrap_or_default()),
            ]
        })
        .collect();
    let mut total = vec![empty(); 10];
    total[1] = Cell::Text("total".to_owned());
    total[5] = Cell::Decimal(quote.total, 2);
    total[6] = currency();
    // The prices are summed as given: no VAT is added to them (review 2026-10-29 PG-66).
    let mut note = "VAT not added".to_owned();
    if !quote.unpriced.is_empty() {
        note += &format!(
            "; not in the total: {} without a price",
            quote.unpriced.len()
        );
    }
    total[9] = Cell::Text(note);
    rows.push(total);
    for item in &quote.unpriced {
        let mut row = vec![empty(); 10];
        row[0] = Cell::Text(item.kind.as_str().to_owned());
        row[1] = Cell::Text(item.item.clone());
        row[9] = Cell::Text(["not priced: ", item.reason.as_str()].concat());
        rows.push(row);
    }
    for name in &quote.unused_prices {
        let mut row = vec![empty(); 10];
        row[1] = Cell::Text(name.clone());
        row[9] = Cell::Text("price() matches nothing counted".to_owned());
        rows.push(row);
    }
    Table { header, rows }
}
