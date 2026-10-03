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
}
