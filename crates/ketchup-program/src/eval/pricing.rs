//! Unit prices (`price()`) the report's quote prices the bill of materials at.
use super::*;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// The unit price of `item`: a material of the made parts (per "m3",
    /// "m2", "m" or "piece"; the stock used when `stock()` is declared, else
    /// the blanks), a bought item of the hardware list ("piece"), a bar
    /// grade ("kg", "m", "piece") or a machining operation such as "drill"
    /// ("operation"). Every price is in one `currency`. The report's
    /// bom.quote prices what the bill of materials counts.
    fn price<'v>(
        #[starlark(require = pos)] item: &str,
        #[starlark(require = pos)] value: Value<'v>,
        #[starlark(require = pos)] per: &str,
        #[starlark(require = named)] currency: Option<Value<'v>>,
        #[starlark(require = named)] source: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let item = item.trim();
        if item.is_empty() {
            anyhow::bail!("price(): name the material, item, bar grade or operation");
        }
        let value = number(value, "price")?;
        if !value.is_finite() || value < 0.0 {
            anyhow::bail!("price({item:?}): the price must be zero or more, got {value}");
        }
        let Some(per) = crate::quote::Unit::parse(per.trim()) else {
            let units = crate::quote::Unit::ALL.map(crate::quote::Unit::as_str);
            anyhow::bail!("price({item:?}): per must be one of {units:?}, got {per:?}");
        };
        let currency = text(currency, "price currency")?
            .map(|currency| currency.trim().to_owned())
            .filter(|currency| !currency.is_empty());
        let source = text(source, "price source")?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if let Some(currency) = currency {
            match &model.currency {
                Some(declared) if *declared != currency => anyhow::bail!(
                    "price({item:?}): every price is in one currency; {declared} was given before, not {currency}"
                ),
                _ => model.currency = Some(currency),
            }
        }
        model
            .prices
            .insert(item.to_owned(), crate::quote::Price { value, per, source });
        Ok(NoneType)
    }
}
