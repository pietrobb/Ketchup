//! Generic sheet and annotation declarations; library helpers provide the convenient calls.
use super::*;
use crate::sheet::{ProgramSheet, ProgramSheetAnnotation, SheetMark};

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Creates a program-owned sheet. `hole_callouts=True` groups equal intact circular
    /// cuts and cosmetic metric threads; unsupported cuts are reported, never omitted.
    /// Depth is the exact cylindrical span, not a blind/through claim.
    fn drawing_sheet<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] source: &str,
        #[starlark(require = named, default = false)] hole_callouts: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        check_name("drawing sheet name", name)?;
        check_name("drawing sheet source", source)?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        anyhow::ensure!(
            !model.drawing_sheets.iter().any(|sheet| sheet.name == name),
            "drawing_sheet({name:?}): duplicate sheet name"
        );
        crate::execution_budget::hold_outside(
            std::mem::size_of::<ProgramSheet>() + name.len() + source.len(),
            "drawing_sheet",
        )?;
        model.drawing_sheets.push(ProgramSheet {
            name: name.to_owned(),
            source: source.to_owned(),
            hole_callouts,
            annotations: Vec::new(),
        });
        Ok(NoneType)
    }

    fn sheet_annotation<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = named)] sheet: &str,
        #[starlark(require = named)] view: &str,
        #[starlark(require = named)] mark: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        check_name("sheet annotation name", name)?;
        let encoded = mark.to_json()?;
        let mark: SheetMark = serde_json::from_str(&encoded)?;
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        let target = model
            .drawing_sheets
            .iter_mut()
            .find(|target| target.name == sheet)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "sheet_annotation({name:?}): declare drawing_sheet({sheet:?}) first"
                )
            })?;
        anyhow::ensure!(
            !target
                .annotations
                .iter()
                .any(|annotation| annotation.name == name),
            "duplicate annotation name {name:?} on sheet {sheet:?}"
        );
        crate::execution_budget::hold_outside(
            std::mem::size_of::<ProgramSheetAnnotation>() + name.len() + view.len() + encoded.len(),
            "sheet_annotation",
        )?;
        target.annotations.push(ProgramSheetAnnotation {
            name: name.to_owned(),
            view: view.to_owned(),
            mark,
        });
        Ok(NoneType)
    }
}
