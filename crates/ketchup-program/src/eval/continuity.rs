//! Explicit identity claims do not change or constrain a part's geometry.
use super::*;

pub(super) fn validate(model: &ProgramModel) -> anyhow::Result<()> {
    for (name, previous) in &model.continuations {
        if model.part(name).is_none() {
            anyhow::bail!(
                "continue_part({name:?}): the declared part no longer exists; declare continuity on a surviving part"
            );
        }
        if name != previous && model.part(previous).is_some() {
            anyhow::bail!(
                "continue_part({name:?}, was={previous:?}): the previous name is still used; only one part may keep its identity"
            );
        }
    }
    Ok(())
}

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Chooses the part that keeps a previous part's identity on the next program apply.
    fn continue_part<'v>(
        #[starlark(require = pos)] part: Value<'v>,
        #[starlark(require = named)] was: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        let name = part_name(part, heap)?;
        check_part_name(was)?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if model.part(&name).is_none() {
            anyhow::bail!(
                "continue_part({name:?}): declare a visible part first; groups, components and tools cannot claim a part's identity"
            );
        }
        if model.continuations.contains_key(&name) {
            anyhow::bail!(
                "continue_part({name:?}) is declared twice; give each part one previous name"
            );
        }
        if let Some((other, _)) = model
            .continuations
            .iter()
            .find(|(_, previous)| previous.as_str() == was)
        {
            anyhow::bail!(
                "continue_part({name:?}, was={was:?}): {other:?} already claims that identity; choose only one continuation"
            );
        }
        model.continuations.insert(name.clone(), was.to_owned());
        record_source(eval, &state, &[&name]);
        Ok(part)
    }
}
