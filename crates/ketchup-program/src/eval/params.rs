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
        let min = given(min)
            .map(|value| number(value, &format!("param({name:?}) min")))
            .transpose()?;
        let max = given(max)
            .map(|value| number(value, &format!("param({name:?}) max")))
            .transpose()?;
        if let (Some(min), Some(max)) = (min, max)
            && min > max
        {
            anyhow::bail!("param({name:?}): min {min} is above max {max}; no value is inside");
        }
        let state = state(eval)?;
        let mut overridden = state.overrides.get(name).copied();
        // NaN passes every range check, as each comparison with it is false
        // (review 2026-11-06 PG-182).
        if let Some(value) = overridden
            && !value.is_finite()
        {
            anyhow::bail!("param {name} = {value} is not a finite number; choose a finite one");
        }
        // An override outside the range is never used: the program builds with
        // the default (a count set to 5000 for `max = 20` built 5000 parts
        // first, review 2026-11-04 r19 WB-42), and the model reports it, so a
        // document saved before its program narrowed the range still opens
        // (review 2026-11-06 P3). A default outside it is the program's own.
        // So is one no whole number of a count, or too large to count with
        // (review 2026-11-02 WB-42: range() failed on it naming no parameter;
        // r23 P3: it failed the program, so a document opened no more once its
        // program said whole = True).
        let mut refused_override = None;
        if let Some(value) = overridden
            && (min.is_some_and(|min| value < min)
                || max.is_some_and(|max| value > max)
                || whole && !whole_number(value))
        {
            refused_override = Some(value);
            overridden = None;
        }
        let value = overridden.unwrap_or(default);
        // A whole number the program can count with: 1e300 is whole but no int,
        // so range() failed on it naming no parameter (review 2026-11-04, rest of WB-42).
        if whole && value.abs() >= WHOLE_LIMIT {
            anyhow::bail!(
                "param {name} = {value:e} is too large for a whole number; choose one between -{WHOLE_LIMIT} and {WHOLE_LIMIT}"
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
                refused_override,
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

/// What a count refusing a value says of it.
pub const WHOLE_ONLY: &str = "takes whole numbers under 1e9 only";

/// Whether `value` is a whole number a count can be: no fraction, below
/// [`WHOLE_LIMIT`].
fn whole_number(value: f64) -> bool {
    value.fract() == 0.0 && value.abs() < WHOLE_LIMIT
}

/// Whether `value`, refused for `param`, is a number it could take but for its
/// range; false when `param` counts and `value` is no whole number.
#[must_use]
pub fn is_whole_override(param: &Param, value: f64) -> bool {
    !param.whole || whole_number(value)
}

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
