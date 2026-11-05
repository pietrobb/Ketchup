//! param(): the named numbers a caller can override.
use super::*;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// A named number the caller can override. Returns an int when the
    /// default is an int and the value is whole.
    fn param<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] default: Value<'v>,
        #[starlark(require = named)] min: Option<Value<'v>>,
        #[starlark(require = named)] max: Option<Value<'v>>,
        #[starlark(require = named, default = "")] doc: &str,
        #[starlark(require = named, default = "mm")] unit: &str,
        #[starlark(require = named, default = false)] whole: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let plain = plain_unit(name, unit)?;
        let integer_default = default.unpack_i32().is_some();
        let default = number(default, &format!("param({name:?}) default"))?;
        if whole && default.fract() != 0.0 {
            anyhow::bail!("param({name:?}): whole = True needs a whole default, got {default}");
        }
        let min = given(min).map(|value| number(value, "min")).transpose()?;
        let max = given(max).map(|value| number(value, "max")).transpose()?;
        let state = state(eval)?;
        let overridden = state.overrides.get(name).copied();
        let value = overridden.unwrap_or(default);
        // Refused here, before the program uses it (review 2026-11-02 WB-42: as
        // an issue of the finished model, range() failed on it first, naming
        // no parameter).
        if whole && value.fract() != 0.0 {
            anyhow::bail!("param {name} = {value} takes whole numbers only; choose a whole number");
        }
        // A whole number the program can count with: 1e300 is whole but no int,
        // so range() failed on it naming no parameter (review 2026-11-04, rest of WB-42).
        if whole && value.abs() >= WHOLE_LIMIT {
            anyhow::bail!(
                "param {name} = {value:e} is too large for a whole number; choose one between -{WHOLE_LIMIT} and {WHOLE_LIMIT}"
            );
        }
        // An override outside the range is refused before the program builds with it
        // (a count set to 5000 for `max = 20` built 5000 parts first, review 2026-11-04 r19 WB-42);
        // a default outside it is the program's own, reported with the model.
        if overridden.is_some()
            && (min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max))
        {
            anyhow::bail!(
                "param {name} = {value} is outside {}; choose a value inside it",
                range_text(min, max)
            );
        }
        {
            let mut model = state.model.borrow_mut();
            if model.params.iter().any(|param| param.name == name) {
                anyhow::bail!("param {name:?} is defined twice");
            }
            model.params.push(Param {
                name: name.to_owned(),
                value,
                default,
                min,
                max,
                doc: doc.to_owned(),
                plain,
                whole,
            });
        }
        let heap = eval.heap();
        if (integer_default || whole) && value.fract() == 0.0 && value.abs() < WHOLE_LIMIT {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(heap.alloc(value as i32));
        }
        Ok(heap.alloc(value))
    }
}

/// The bound of a parameter returned as an int.
const WHOLE_LIMIT: f64 = 1.0e9;

/// `[min, max]` as the range of a parameter is written, an open side as `-inf` or `inf`.
#[must_use]
pub fn range_text(min: Option<f64>, max: Option<f64>) -> String {
    format!(
        "[{}, {}]",
        min.map_or("-inf".to_owned(), |value| value.to_string()),
        max.map_or("inf".to_owned(), |value| value.to_string()),
    )
}

/// Whether `param(name, unit = unit)` is a count, a factor or an angle (`""`)
/// rather than a length (`"mm"`).
fn plain_unit(name: &str, unit: &str) -> anyhow::Result<bool> {
    match unit {
        "mm" => Ok(false),
        "" => Ok(true),
        other => anyhow::bail!(
            "param({name:?}): unit must be \"mm\" for a length or \"\" for a count, a factor or an angle, got {other:?}"
        ),
    }
}
