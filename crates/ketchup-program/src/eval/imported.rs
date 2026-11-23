//! `imported()`: a part whose body a STEP file holds (`crate::imported`).
use super::*;
use crate::imported::ProgramImport;

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// A part whose body is the STEP file `file` (.step or .stp), relative to
    /// the folder of the program's own file and inside it. `part=` takes the
    /// one part of an assembly file the file names so; without it the whole
    /// file, every solid where the file places it. The file's origin and axes
    /// are the part's own, so `at=`, `rotate()` and `place()` move it like
    /// any part. The host reads the file after the evaluation: the program
    /// does not know the body's size, and nothing shapes the part or uses it
    /// as a tool.
    fn imported<'v>(
        #[starlark(require = pos)] name: &str,
        #[starlark(require = pos)] file: Value<'v>,
        #[starlark(require = named)] part: Option<Value<'v>>,
        #[starlark(require = named)] at: Option<Value<'v>>,
        #[starlark(require = named)] material: Option<Value<'v>>,
        #[starlark(require = named)] color: Option<Value<'v>>,
        #[starlark(require = named)] attributes: Option<Value<'v>>,
        #[starlark(require = named)] tags: Option<Value<'v>>,
        #[starlark(require = named, default = false)] grounded: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<Value<'v>> {
        let heap = eval.heap();
        check_part_name(name)?;
        let Some(file) = text(Some(file), "file")?.map(|file| file.trim().to_owned()) else {
            anyhow::bail!("imported({name:?}): give the path of its .step or .stp file");
        };
        let extension = std::path::Path::new(&file)
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(extension.as_deref(), Some("step" | "stp")) {
            anyhow::bail!(
                "imported({name:?}): {file:?} is not a STEP file; give the path of a .step or .stp file"
            );
        }
        crate::imported::check_inside_program_folder(name, &file)?;
        let part = text(part, "part")?
            .map(|part| part.trim().to_owned())
            .filter(|part| !part.is_empty());
        let at_mm = given(at).map_or(Ok([0.0; 3]), |at| numbers::<3>(at, heap, "at"))?;
        let look = Look::read(material, color, attributes, tags, grounded, heap)?;
        let state = state(eval)?;
        let body = ProgramPartBody::Imported(Box::new(ProgramImport {
            file,
            part,
            read: None,
        }));
        let part = insert_part(
            &state,
            look.dress(new_part(name, [0.0; 3], at_mm, body)),
            false,
        )?;
        record_source(eval, &state, &[name]);
        Ok(part_value(&part, heap))
    }
}
