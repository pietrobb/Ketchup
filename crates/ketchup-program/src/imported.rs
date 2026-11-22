//! Bodies a program reads from STEP files (`imported()`, and the library's
//! `purchased()` for bought parts). An evaluation cannot read files: it runs in
//! a worker without them. So the program only names the file, and the host
//! reads it after the evaluation and records here what it read: the file's own
//! name, the part of an assembly it took and the exact kernel's account of the
//! body, its bounds among it. Until then the body has no extent and no check
//! takes it into account.

use crate::model::{Part, ProgramModel, ProgramOperation, ProgramPartBody};
use ketchup_model::import::StepImportEvidence;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ProgramImport {
    /// The file as the program names it: relative to the folder of the
    /// program's own file and inside it (`check_inside_program_folder`).
    pub file: String,
    /// The part of an assembly file to take, by the name the file gives it;
    /// the whole file, every solid in its place, when none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part: Option<String>,
    /// What the host read of the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<ImportedBody>,
}

/// What the host read of a program's STEP file.
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
pub struct ImportedBody {
    /// The file's own name, without its folder.
    pub file_name: String,
    /// Where the part lies among the parts of the file, as the exact kernel
    /// numbers them; none for the whole file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_index: Option<u32>,
    pub evidence: StepImportEvidence,
}

impl ProgramImport {
    /// Bounds of the body in its own frame (the file's coordinates); a point
    /// at the origin until the file is read.
    #[must_use]
    pub fn bounds(&self) -> ([f64; 3], [f64; 3]) {
        self.read.as_ref().map_or(([0.0; 3], [0.0; 3]), |read| {
            (read.evidence.bounds_mm[0], read.evidence.bounds_mm[1])
        })
    }

    /// Records what the host read and sizes `part` by it.
    pub fn record(part: &mut Part, read: ImportedBody) {
        let [min, max] = read.evidence.bounds_mm;
        if let ProgramPartBody::Imported(import) = &mut part.body {
            import.read = Some(read);
            part.size_mm = std::array::from_fn(|axis| max[axis] - min[axis]);
        }
    }
}

impl Part {
    /// The file this part's body is read from, when it is.
    #[must_use]
    pub fn imported(&self) -> Option<&ProgramImport> {
        match &self.body {
            ProgramPartBody::Imported(import) => Some(import),
            _ => None,
        }
    }
}

impl ProgramModel {
    /// Every part whose body a file holds, in the model and in its components.
    pub fn imported_parts_mut(&mut self) -> impl Iterator<Item = &mut Part> {
        self.parts
            .iter_mut()
            .chain(
                self.components
                    .iter_mut()
                    .flat_map(|component| component.parts.iter_mut()),
            )
            .filter(|part| part.imported().is_some())
    }
}

/// Why the program of part `name` may not read `file`: only files in the
/// folder of the program's own file or below it, so a program someone sends
/// reads nothing else of the computer it runs on. The host checks the real
/// path again when it reads, links included.
///
/// # Errors
/// `file` is absolute, or a `..` takes it out of the program's folder.
pub fn check_inside_program_folder(name: &str, file: &str) -> anyhow::Result<()> {
    use std::path::Component;
    let path = std::path::Path::new(file);
    let mut depth = 0_usize;
    for component in path.components() {
        match component {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => anyhow::bail!(
                "{name:?}: {file:?} is outside the folder of the program's file; copy the STEP file into that folder (or below it) and give its path from there, e.g. \"supplier/article.step\""
            ),
        }
    }
    Ok(())
}

/// The name of `operation` on `part`, or why a part read from a file cannot
/// take it: its body is the file's, so it is neither shaped, drilled nor
/// used as another part's tool.
///
/// # Errors
/// `part` is read from a file, or the operation's tool is.
pub(crate) fn operation_name<'a>(
    part: &Part,
    operation: &'a ProgramOperation,
) -> anyhow::Result<&'a str> {
    let name = operation.name();
    if let Some(import) = part.imported() {
        anyhow::bail!(
            "{name:?} on {:?}: the part is read from {:?} and takes no operations; shape the parts it fits instead",
            part.name,
            import.file
        );
    }
    if let ProgramOperation::Boolean(boolean) = operation
        && let Some(import) = boolean.tool.imported()
    {
        anyhow::bail!(
            "{name:?} on {:?}: {:?} is read from {:?} and cannot be a tool; cut with a tool body of the room it needs",
            part.name,
            boolean.tool.name,
            import.file
        );
    }
    Ok(name)
}
