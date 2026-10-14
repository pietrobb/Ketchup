//! What a comment's view shows, written in the fewest paths. An isolation
//! keeps the few parts still shown, a hidden layer names the scene that
//! hides it, and a hidden assembly is one path instead of every part in it.

use crate::{CommentView, InstancePath, Manifest};
use std::collections::{BTreeMap, BTreeSet};

/// Whether `path` lies under one of `prefixes`: a prefix may be an assembly
/// above the bodies.
pub(crate) fn covered(prefixes: &[InstancePath], path: &InstancePath) -> bool {
    prefixes.iter().any(|prefix| {
        prefix.root_occurrence_id == path.root_occurrence_id
            && path.steps.starts_with(&prefix.steps)
    })
}

impl CommentView {
    /// Whether the occurrence at `path` of `manifest` is shown in this view.
    #[must_use]
    pub fn shows(&self, manifest: &Manifest, path: &InstancePath) -> bool {
        let base = match (&self.shown, self.scene) {
            (Some(shown), _) => covered(shown, path),
            (None, Some(id)) => manifest
                .scenes
                .iter()
                .find(|scene| scene.id == id)
                .is_none_or(|scene| !covered(&scene.hidden, path)),
            (None, None) => true,
        };
        base && !covered(&self.hidden, path)
    }

    /// Describe `visible`, one flag per occurrence of `manifest`, in the
    /// fewest paths. A file without `scene` or `shown` stays readable by
    /// Viewers that know only `hidden`, so a tie keeps that form.
    pub fn set_visibility(&mut self, manifest: &Manifest, visible: &[bool]) {
        let paths: Vec<&InstancePath> = manifest.occurrences.iter().map(|o| &o.path).collect();
        let hidden_now: Vec<bool> = visible.iter().map(|shown| !shown).collect();
        let hidden = cover(&paths, &hidden_now, |i| hidden_now[i]);
        let mut best = (hidden.len(), None, hidden, None);
        let shown = cover(&paths, visible, |i| visible[i]);
        if shown.len() + 1 < best.0 {
            best = (shown.len() + 1, None, Vec::new(), Some(shown));
        }
        for scene in &manifest.scenes {
            let in_scene: Vec<bool> = paths
                .iter()
                .map(|path| covered(&scene.hidden, path))
                .collect();
            if in_scene
                .iter()
                .zip(visible)
                .any(|(hid, shown)| *hid && *shown)
            {
                continue;
            }
            let hidden = cover(&paths, &hidden_now, |i| hidden_now[i] && !in_scene[i]);
            if hidden.len() + 1 < best.0 {
                best = (hidden.len() + 1, Some(scene.id), hidden, None);
            }
        }
        (_, self.scene, self.hidden, self.shown) = best;
    }
}

/// The shortest prefixes under which every occurrence is `inside`, together
/// covering each occurrence `wanted` names.
fn cover(
    paths: &[&InstancePath],
    inside: &[bool],
    wanted: impl Fn(usize) -> bool,
) -> Vec<InstancePath> {
    let mut whole: BTreeMap<InstancePath, bool> = BTreeMap::new();
    for (path, inside) in paths.iter().zip(inside) {
        for depth in 0..=path.steps.len() {
            whole
                .entry(prefix(path, depth))
                .and_modify(|all| *all &= *inside)
                .or_insert(*inside);
        }
    }
    let mut chosen = BTreeSet::new();
    for (_, path) in paths.iter().enumerate().filter(|(i, _)| wanted(*i)) {
        // A body with shown parts below it falls back to its own path.
        let depth = (0..=path.steps.len())
            .find(|depth| whole[&prefix(path, *depth)])
            .unwrap_or(path.steps.len());
        chosen.insert(prefix(path, depth));
    }
    chosen.into_iter().collect()
}

fn prefix(path: &InstancePath, depth: usize) -> InstancePath {
    InstancePath {
        root_occurrence_id: path.root_occurrence_id,
        steps: path.steps[..depth].to_vec(),
    }
}
