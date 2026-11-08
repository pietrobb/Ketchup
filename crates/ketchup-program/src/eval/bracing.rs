//! Bracing a program declares where the geometry cannot show it: the edge of
//! a load-path member a brace holds, and sheets that are one anchored stiff plane
//! (review 2026-11-08 PG-240); and the building site its snow comes from (§5.3).
use super::*;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Declares that the parts (or tags) `by` hold the `edge` ("top" or
    /// "bottom") of the load-path `members` sideways, every `spacing` mm along
    /// where each touches a member, or at the joint's fasteners there. A brace
    /// still has to be fastened to the member and anchored: grounded, fastened
    /// to a grounded part, or a sheet of an anchored diaphragm().
    fn brace<'v>(
        #[starlark(require = pos)] members: Value<'v>,
        #[starlark(require = named)] by: Value<'v>,
        #[starlark(require = named)] edge: &str,
        #[starlark(require = named)] spacing: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        if !matches!(edge, "top" | "bottom") {
            anyhow::bail!("brace(): edge must be \"top\" or \"bottom\", got {edge:?}");
        }
        let spacing_mm = given(spacing)
            .map(|value| number(value, "spacing"))
            .transpose()?;
        if spacing_mm.is_some_and(|value| !value.is_finite() || value <= 0.0) {
            anyhow::bail!("brace(): spacing must be positive");
        }
        let brace = crate::loads::DeclaredBrace {
            members: tag_names(members, heap)?.into_iter().collect(),
            by: tag_names(by, heap)?.into_iter().collect(),
            edge: edge.to_owned(),
            spacing_mm,
        };
        if brace.members.is_empty() || brace.by.is_empty() {
            anyhow::bail!("brace(): name at least one member and one brace");
        }
        state(eval)?.model.borrow_mut().declared_braces.push(brace);
        Ok(NoneType)
    }

    /// Declares the parts (or tags) `sheets` one stiff plane anchored to the
    /// grounded parts `anchored_to`: each sheet then holds the load-path
    /// members it is fastened to on the edge it lies on, when every anchor is
    /// declared grounded and fastened to one of the sheets.
    fn diaphragm<'v>(
        #[starlark(require = pos)] sheets: Value<'v>,
        #[starlark(require = named)] anchored_to: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let diaphragm = crate::loads::Diaphragm {
            sheets: tag_names(sheets, heap)?.into_iter().collect(),
            anchored_to: tag_names(anchored_to, heap)?.into_iter().collect(),
        };
        if diaphragm.sheets.is_empty() || diaphragm.anchored_to.is_empty() {
            anyhow::bail!("diaphragm(): name at least one sheet and one anchor");
        }
        state(eval)?.model.borrow_mut().diaphragms.push(diaphragm);
        Ok(NoneType)
    }

    /// The building site: country "SK" or "CZ", municipality, altitude in m, and
    /// the snow zone of the national annex map or s_k read off it (with its
    /// source). Returns struct(country, sk, source, altitude); sk is None when
    /// the annex gives no value for the inputs. The design basis names the site.
    fn site<'v>(
        #[starlark(require = pos)] country: &str,
        #[starlark(require = named)] municipality: Option<Value<'v>>,
        #[starlark(require = named)] altitude: Option<Value<'v>>,
        #[starlark(require = named)] snow_zone: Option<Value<'v>>,
        #[starlark(require = named)] sk: Option<Value<'v>>,
        #[starlark(require = named)] source: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let snow_zone = given(snow_zone)
            .map(|zone| match zone.unpack_i32() {
                Some(n) => Ok(n.to_string()),
                None => zone.unpack_str().map(ToOwned::to_owned).ok_or_else(|| {
                    anyhow::anyhow!(
                        "site(): snow_zone must be a string or a number, got {}",
                        zone.get_type()
                    )
                }),
            })
            .transpose()?;
        let site = crate::site::Site::resolve(crate::site::SiteInput {
            country,
            municipality: text(municipality, "site municipality")?,
            altitude_m: given(altitude).map(|v| number(v, "altitude")).transpose()?,
            snow_zone,
            sk: given(sk).map(|v| number(v, "sk")).transpose()?,
            source: text(source, "site source")?,
        })?;
        let heap = eval.heap();
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if model.site.is_some() {
            anyhow::bail!("site(): a program has one site; it is already given");
        }
        let value = heap.alloc(AllocStruct([
            ("country", heap.alloc(site.country.as_str())),
            (
                "sk",
                site.sk_kn_m2
                    .map_or_else(Value::new_none, |sk| heap.alloc(sk)),
            ),
            ("source", heap.alloc(site.source.as_str())),
            (
                "altitude",
                site.altitude_m
                    .map_or_else(Value::new_none, |a| heap.alloc(a)),
            ),
        ]));
        model.site = Some(site);
        Ok(value)
    }
}
