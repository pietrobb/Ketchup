//! Named variants of a program: each keeps the value of every parameter with
//! the program's source, so one part can be switched between its sizes.
use std::collections::BTreeMap;
use std::fmt;

use ketchup_model::document::{MAX_RULE_PROGRAM_VARIANTS, RuleProgramSource};
use ketchup_program::cut_list::cut_list_table;
use ketchup_program::model::Param;
use ketchup_program::table::{Cell, Table};
use ketchup_program::takeoff::{Takeoff, material_takeoff_of_all};
use ketchup_program::{CancelToken, ProgramError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VariantError {
    EmptyName,
    /// The program declares no parameter, so a variant would hold nothing.
    NoParameters,
    TooMany,
    Unknown {
        name: String,
        known: Vec<String>,
    },
    /// The program no longer declares these parameters of the variant.
    MissingParameters {
        name: String,
        missing: Vec<String>,
    },
}

impl fmt::Display for VariantError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyName => formatter.write_str("a variant needs a name"),
            Self::NoParameters => {
                formatter.write_str("the program declares no parameter to keep in a variant")
            }
            Self::TooMany => write!(
                formatter,
                "the program already has {MAX_RULE_PROGRAM_VARIANTS} variants"
            ),
            Self::Unknown { name, known } if known.is_empty() => {
                write!(
                    formatter,
                    "the program has no variant {name:?}; it has none"
                )
            }
            Self::Unknown { name, known } => write!(
                formatter,
                "the program has no variant {name:?}; it has {}",
                known.join(", ")
            ),
            Self::MissingParameters { name, missing } => write!(
                formatter,
                "variant {name:?} sets parameters the program no longer declares: {}",
                missing.join(", ")
            ),
        }
    }
}

impl std::error::Error for VariantError {}

/// `program` with variant `name` holding the current value of every
/// parameter in `params`; a variant of that name is replaced.
pub fn save_variant(
    program: &RuleProgramSource,
    params: &[Param],
    name: &str,
) -> Result<RuleProgramSource, VariantError> {
    if name.trim().is_empty() {
        return Err(VariantError::EmptyName);
    }
    if params.is_empty() {
        return Err(VariantError::NoParameters);
    }
    if !program.variants.contains_key(name) && program.variants.len() >= MAX_RULE_PROGRAM_VARIANTS {
        return Err(VariantError::TooMany);
    }
    let mut saved = program.clone();
    saved.variants.insert(
        name.to_owned(),
        params
            .iter()
            .map(|param| (param.name.clone(), param.value))
            .collect(),
    );
    Ok(saved)
}

/// `program` without variant `name`.
pub fn delete_variant(
    program: &RuleProgramSource,
    name: &str,
) -> Result<RuleProgramSource, VariantError> {
    let mut deleted = program.clone();
    if deleted.variants.remove(name).is_none() {
        return Err(unknown(program, name));
    }
    Ok(deleted)
}

/// The values variant `name` gives the parameters, refused when the program
/// no longer declares one of them.
pub fn variant_params(
    program: &RuleProgramSource,
    params: &[Param],
    name: &str,
) -> Result<BTreeMap<String, f64>, VariantError> {
    let values = program
        .variants
        .get(name)
        .ok_or_else(|| unknown(program, name))?;
    let missing = values
        .keys()
        .filter(|parameter| !params.iter().any(|param| &&param.name == parameter))
        .cloned()
        .collect::<Vec<_>>();
    if !missing.is_empty() {
        return Err(VariantError::MissingParameters {
            name: name.to_owned(),
            missing,
        });
    }
    Ok(values.clone())
}

/// The variant every value of which the parameters have now.
#[must_use]
pub fn active_variant<'a>(program: &'a RuleProgramSource, params: &[Param]) -> Option<&'a str> {
    program
        .variants
        .iter()
        .find(|(_, values)| {
            values.iter().all(|(parameter, value)| {
                params
                    .iter()
                    .any(|param| &param.name == parameter && param.value == *value)
            })
        })
        .map(|(name, _)| name.as_str())
}

/// Why one variant has no cut list.
#[derive(Debug)]
pub enum VariantEvaluationError {
    Program(ProgramError),
    Variant(VariantError),
}

impl fmt::Display for VariantEvaluationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Program(error) => error.fmt(formatter),
            Self::Variant(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for VariantEvaluationError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Program(error) => Some(error),
            Self::Variant(error) => Some(error),
        }
    }
}

/// One variant evaluated apart from the window's document: its cut list and
/// the takeoff of all its parts, or why it does not evaluate.
#[derive(Debug)]
pub struct VariantExport {
    pub name: String,
    pub params: BTreeMap<String, f64>,
    pub result: Result<(Table, Takeoff), VariantEvaluationError>,
}

/// Every variant of `program` in name order, each evaluated with its own
/// values; `None` once `cancel` is set.
#[must_use]
pub fn evaluate_variants(
    program: &RuleProgramSource,
    cancel: &CancelToken,
) -> Option<Vec<VariantExport>> {
    program
        .variants
        .iter()
        .map(|(name, values)| {
            let evaluated = crate::program_isolation::evaluate_program(
                &program.file_name,
                &program.source,
                values,
                cancel,
            );
            if cancel.is_cancelled() {
                return None;
            }
            let result = evaluated
                .map_err(VariantEvaluationError::Program)
                .and_then(|evaluated| {
                    let model = evaluated.model;
                    let missing = values
                        .keys()
                        .filter(|parameter| {
                            !model.params.iter().any(|param| &&param.name == parameter)
                        })
                        .cloned()
                        .collect::<Vec<_>>();
                    if !missing.is_empty() {
                        return Err(VariantEvaluationError::Variant(
                            VariantError::MissingParameters {
                                name: name.clone(),
                                missing,
                            },
                        ));
                    }
                    Ok((
                        cut_list_table(&model, |_| true),
                        material_takeoff_of_all(&model),
                    ))
                });
            Some(VariantExport {
                name: name.clone(),
                params: values.clone(),
                result,
            })
        })
        .collect()
}

/// A takeoff as a table: its rows, then one `TOTAL` row per material.
#[must_use]
pub fn takeoff_table(takeoff: &Takeoff) -> Table {
    let header = [
        "category",
        "material",
        "width_mm",
        "thickness_mm",
        "count",
        "length_m",
        "area_m2",
        "volume_m3",
        "volume_basis",
    ]
    .map(str::to_owned)
    .to_vec();
    let rows = takeoff.rows.iter().map(|row| {
        vec![
            Cell::Text(row.category.clone()),
            Cell::Text(row.material.clone()),
            Cell::Number(row.section_mm[0]),
            Cell::Number(row.section_mm[1]),
            Cell::Count(row.count),
            Cell::Number(row.length_m),
            Cell::Number(row.area_m2),
            Cell::Number(row.volume_m3),
            Cell::Text(row.volume_basis.as_str().to_owned()),
        ]
    });
    let totals = takeoff.materials.iter().map(|total| {
        vec![
            Cell::Text("TOTAL".to_owned()),
            Cell::Text(total.material.clone()),
            Cell::Text(String::new()),
            Cell::Text(String::new()),
            Cell::Count(total.count),
            Cell::Number(total.length_m),
            Cell::Number(total.area_m2),
            Cell::Number(total.volume_m3),
            Cell::Text(total.volume_basis.as_str().to_owned()),
        ]
    });
    Table {
        header,
        rows: rows.chain(totals).collect(),
    }
}

/// The tables of several variants as one, the variant's name in the first
/// column; a column only some variants have stays empty in the others.
#[must_use]
pub fn combined_table(tables: &[(&str, &Table)]) -> Table {
    let mut header = vec!["variant".to_owned()];
    for (_, table) in tables {
        for name in &table.header {
            if !header.contains(name) {
                header.push(name.clone());
            }
        }
    }
    let mut rows = Vec::new();
    for (variant, table) in tables {
        let columns = table
            .header
            .iter()
            .map(|name| header.iter().position(|column| column == name))
            .collect::<Vec<_>>();
        for row in &table.rows {
            let mut cells = vec![Cell::Text(String::new()); header.len()];
            cells[0] = Cell::Text((*variant).to_owned());
            for (column, cell) in columns.iter().zip(row) {
                if let Some(column) = column {
                    cells[*column] = cell.clone();
                }
            }
            rows.push(cells);
        }
    }
    Table { header, rows }
}

fn unknown(program: &RuleProgramSource, name: &str) -> VariantError {
    VariantError::Unknown {
        name: name.to_owned(),
        known: program.variants.keys().cloned().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ketchup_program::table::DecimalSeparator;
    use ketchup_program::takeoff::material_takeoff;

    fn model(source: &str) -> ketchup_program::ProgramModel {
        ketchup_program::evaluate("takeoff.star", source, &BTreeMap::new())
            .expect("program evaluates")
            .model
    }

    #[test]
    fn a_takeoff_csv_has_a_row_per_group_and_a_total_per_material() {
        let model = model(concat!(
            "box(\"stena/stĺpik 1\", (60, 140, 2500), material = \"drevo\")\n",
            "box(\"stena/stĺpik 2\", (60, 140, 2500), at = (600, 0, 0), material = \"drevo\")\n",
        ));
        let counted = BTreeMap::from([
            ("stena/stĺpik 1".to_owned(), Some(20_000_000.0)),
            ("stena/stĺpik 2".to_owned(), None),
        ]);
        let csv = takeoff_table(&material_takeoff(&model, &counted)).csv(DecimalSeparator::Comma);
        assert!(
            csv.starts_with("\u{feff}category;material;width_mm"),
            "{csv}"
        );
        assert!(
            csv.contains("\nstena;drevo;140;60;2;5;0,683;0,041;mixed\n"),
            "{csv}"
        );
        assert!(
            csv.contains("\nTOTAL;drevo;;;2;5;0,683;0,041;mixed\n"),
            "{csv}"
        );
    }

    /// Review 2026-10-09 (P3): a name like `=1+1` was written as a formula.
    #[test]
    fn a_takeoff_csv_writes_formula_like_names_as_plain_text() {
        let model = model("box(\"=HYPERLINK(1)/doska\", (1000, 200, 25), material = \"@drevo\")\n");
        let csv = takeoff_table(&material_takeoff_of_all(&model)).csv(DecimalSeparator::Comma);
        assert!(
            csv.contains("\n'=HYPERLINK(1);'@drevo;200;25;1;1;0,2;0,005;blank\n"),
            "{csv}"
        );
        assert!(
            csv.contains("\nTOTAL;'@drevo;;;1;1;0,2;0,005;blank\n"),
            "{csv}"
        );
    }
}
