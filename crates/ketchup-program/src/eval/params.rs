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
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let plain = plain_unit(name, unit)?;
        let integer_default = default.unpack_i32().is_some();
        let default = number(default, &format!("param({name:?}) default"))?;
        let min = given(min).map(|value| number(value, "min")).transpose()?;
        let max = given(max).map(|value| number(value, "max")).transpose()?;
        let state = state(eval)?;
        let value = state.overrides.get(name).copied().unwrap_or(default);
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
            });
        }
        let heap = eval.heap();
        if integer_default && value.fract() == 0.0 && value.abs() < 1.0e9 {
            #[allow(clippy::cast_possible_truncation)]
            return Ok(heap.alloc(value as i32));
        }
        Ok(heap.alloc(value))
    }
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
