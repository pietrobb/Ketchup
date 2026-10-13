//! Named variants of a program: each keeps the value of every parameter with
//! the program's source, so one part can be switched between its sizes.
use std::collections::BTreeMap;
use std::fmt;

use ketchup_model::document::{MAX_RULE_PROGRAM_VARIANTS, RuleProgramSource};
use ketchup_program::model::Param;

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

fn unknown(program: &RuleProgramSource, name: &str) -> VariantError {
    VariantError::Unknown {
        name: name.to_owned(),
        known: program.variants.keys().cloned().collect(),
    }
}
