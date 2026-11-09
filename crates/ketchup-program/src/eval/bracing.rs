//! Bracing a program declares where the geometry cannot show it: the edge of
//! a load-path member a brace holds, and sheets that are one anchored stiff plane
//! (review 2026-11-08 PG-240).
use super::*;

/// The least spacing `brace()` takes, in mm.
const MIN_SPACING_MM: f64 = 10.0;

/// A part's attribute `key` given as `value`, none when it is given as None,
/// the statics' own checked: a `length_axis` naming no local axis was dropped
/// without a word, the part's longest axis taken instead (review 2026-11-09
/// PG-305).
pub(super) fn attribute(
    key: String,
    value: Option<String>,
) -> anyhow::Result<Option<(String, String)>> {
    let Some(value) = value else {
        return Ok(None);
    };
    if key == crate::model::LENGTH_AXIS
        && !matches!(value.as_str(), "x" | "y" | "z" | "X" | "Y" | "Z")
    {
        anyhow::bail!(
            "attribute {key:?} must name a local axis, \"x\", \"y\" or \"z\", got {value:?}"
        );
    }
    Ok(Some((key, value)))
}

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
        // A spacing of 0.0001 mm placed billions of points and brought the
        // process down (review 2026-11-09 PG-307); no fastening is that close.
        if let Some(value) = spacing_mm
            && !(value.is_finite() && value >= MIN_SPACING_MM)
        {
            anyhow::bail!("brace(): spacing must be at least {MIN_SPACING_MM} mm, got {value}");
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
}
