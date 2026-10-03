//! Explicit support reference for a program, independent of the part placements.
use super::*;

impl ProgramModel {
    /// Leaf names explicitly anchored themselves or through an enclosing group.
    #[must_use]
    pub fn grounded_parts(&self) -> BTreeSet<String> {
        let mut grounded = self
            .parts
            .iter()
            .filter(|part| part.grounded)
            .map(|part| part.name.clone())
            .collect::<BTreeSet<_>>();
        let groups = self
            .groups
            .iter()
            .map(|group| (group.name.as_str(), group))
            .collect::<BTreeMap<_, _>>();
        let mut pending = self
            .groups
            .iter()
            .filter(|group| group.grounded)
            .map(|group| group.name.as_str())
            .collect::<Vec<_>>();
        let mut visited = BTreeSet::new();
        while let Some(name) = pending.pop() {
            if !visited.insert(name) {
                continue;
            }
            if let Some(group) = groups.get(name) {
                pending.extend(group.members.iter().map(String::as_str));
            } else {
                grounded.insert(name.to_owned());
            }
        }
        grounded
    }
}

#[starlark_module]
pub(super) fn builtins(builder: &mut GlobalsBuilder) {
    /// Sets the world height of the horizontal support plane in millimetres.
    fn floor<'v>(
        #[starlark(require = pos)] z: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let z = number(z, "floor z")?;
        if z.abs() > MAX_COORDINATE_MM {
            anyhow::bail!("floor z must lie within +/-{MAX_COORDINATE_MM} mm");
        }
        let state = state(eval)?;
        let mut model = state.model.borrow_mut();
        if model.floor_z_mm.is_some() {
            anyhow::bail!("floor() is declared twice; set one world floor height for the program");
        }
        model.floor_z_mm = Some(z);
        Ok(NoneType)
    }
}
