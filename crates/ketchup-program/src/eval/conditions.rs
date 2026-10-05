//! Library conditions report repairable design problems without aborting evaluation.
use super::*;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    fn check<'v>(
        #[starlark(require = pos)] condition: bool,
        #[starlark(require = pos)] message: &str,
        #[starlark(require = named)] parts: Value<'v>,
        #[starlark(require = named)] hint: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<bool> {
        let heap = eval.heap();
        let parts = items(parts, heap, "check parts")?
            .into_iter()
            .map(|part| part_name(part, heap))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        for part in &parts {
            known_part(&model, part)?;
        }
        if !condition {
            model.declared_issues.push(crate::validate::Issue {
                severity: crate::validate::Severity::Error,
                kind: "program_condition_failed",
                parts,
                message: message.to_owned(),
                where_mm: None,
                hint: hint.to_owned(),
            });
        }
        Ok(condition)
    }

    /// Keeps the space of the tool body `zone` free: a part (with one of the
    /// tags `only`, when given; not one of `ignore`) reaching into it on the
    /// final model is a `free_space_occupied` error.
    fn keep_clear<'v>(
        #[starlark(require = pos)] zone: Value<'v>,
        #[starlark(require = named)] name: Option<Value<'v>>,
        #[starlark(require = named)] only: Option<Value<'v>>,
        #[starlark(require = named)] ignore: Option<Value<'v>>,
        #[starlark(require = named)] hint: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let heap = eval.heap();
        let zone = part_name(zone, heap)?;
        let ignore = given(ignore)
            .map(|value| {
                items(value, heap, "keep_clear ignore")?
                    .into_iter()
                    .map(|part| part_name(part, heap))
                    .collect::<anyhow::Result<BTreeSet<_>>>()
            })
            .transpose()?
            .unwrap_or_default();
        let space = crate::clearance::FreeSpace {
            name: text(name, "keep_clear name")?.unwrap_or_else(|| zone.clone()),
            only_tags: given(only)
                .map(|value| tag_names(value, heap))
                .transpose()?
                .unwrap_or_default(),
            ignore,
            hint: text(hint, "keep_clear hint")?.unwrap_or_else(|| {
                "Move or shrink the part so it stays out of the space, or move the space's owner."
                    .to_owned()
            }),
            zone,
        };
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if model.tool(&space.zone).is_none() {
            anyhow::bail!(
                "keep_clear({:?}): the space must be a tool body (box(..., tool=True)), not a part or an unknown name",
                space.zone
            );
        }
        model.free_spaces.push(space);
        Ok(NoneType)
    }
}
