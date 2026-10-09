//! Group connectivity is independent of grounding and declared joints.
use crate::eval::TOLERANCE_MM;
use crate::exact::{ExactShapes, box_is_solid};
use crate::model::{Part, ProgramModel};
use crate::validate::{Issue, Severity};
use std::collections::{BTreeMap, BTreeSet};

/// Unknown is not contact: a non-box pair needs the native solids to settle it.
fn touching(a: &Part, b: &Part, exact: &ExactShapes) -> Option<bool> {
    if let Some(pair) = exact.pair(&a.name, &b.name) {
        return Some(pair.penetrating() || pair.touching());
    }
    let separated = a.obb().separation(&b.obb()) > TOLERANCE_MM;
    if separated || (box_is_solid(a) && box_is_solid(b)) {
        Some(!separated)
    } else {
        None
    }
}

fn root(parents: &[usize], mut index: usize) -> usize {
    while parents[index] != index {
        index = parents[index];
    }
    index
}

fn join(parents: &mut [usize], a: usize, b: usize) {
    let (a, b) = (root(parents, a), root(parents, b));
    parents[b] = a;
}

fn islands<'a>(parts: &[&'a Part], parents: &[usize]) -> Vec<Vec<&'a str>> {
    let mut islands = BTreeMap::<usize, Vec<&str>>::new();
    for (index, part) in parts.iter().enumerate() {
        islands
            .entry(root(parents, index))
            .or_default()
            .push(&part.name);
    }
    islands.into_values().collect()
}

pub(crate) fn issues(model: &ProgramModel, exact: &ExactShapes, issues: &mut Vec<Issue>) {
    let parts: BTreeMap<_, _> = model
        .parts
        .iter()
        .map(|part| (part.name.as_str(), part))
        .collect();
    let groups: BTreeMap<_, _> = model
        .groups
        .iter()
        .map(|group| (group.name.as_str(), group))
        .collect();
    let mut pairs = BTreeMap::new();
    for group in &model.groups {
        let mut leaves = BTreeSet::new();
        let mut pending = group.members.iter().map(String::as_str).collect::<Vec<_>>();
        while let Some(name) = pending.pop() {
            if parts.contains_key(name) {
                leaves.insert(name);
            } else if let Some(child) = groups.get(name) {
                pending.extend(child.members.iter().map(String::as_str));
            }
        }
        let members = leaves.iter().map(|name| parts[name]).collect::<Vec<_>>();
        if members.len() < 2 {
            continue;
        }
        let mut known = (0..members.len()).collect::<Vec<_>>();
        let mut possible = known.clone();
        let bounds = members
            .iter()
            .map(|part| part.world_bounds())
            .collect::<Vec<_>>();
        let mut order = (0..members.len()).collect::<Vec<_>>();
        order.sort_by(|a, b| bounds[*a].0[0].total_cmp(&bounds[*b].0[0]));
        for (position, &left) in order.iter().enumerate() {
            for &right in &order[position + 1..] {
                if bounds[right].0[0] > bounds[left].1[0] + TOLERANCE_MM {
                    break;
                }
                let (a, b) = (members[left], members[right]);
                let contact = *pairs
                    .entry((a.name.as_str(), b.name.as_str()))
                    .or_insert_with(|| touching(a, b, exact));
                if contact == Some(true) {
                    join(&mut known, left, right);
                }
                if contact != Some(false) {
                    join(&mut possible, left, right);
                }
            }
        }
        let certain = islands(&members, &known);
        if certain.len() == 1 {
            continue;
        }
        let possible = islands(&members, &possible);
        let detached = possible.len() > 1;
        let components = if detached { &possible } else { &certain };
        let bounds = members.iter().map(|part| part.world_bounds()).fold(
            ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]),
            |(min, max), (lo, hi)| {
                (
                    std::array::from_fn(|i| min[i].min(lo[i])),
                    std::array::from_fn(|i| max[i].max(hi[i])),
                )
            },
        );
        issues.push(Issue {
            source_lines: Vec::new(),
            severity: Severity::Warning,
            kind: if detached { "disconnected_group" } else { "group_contact_unverified" },
            parts: leaves.into_iter().map(str::to_owned).collect(),
            message: format!(
                "group {} has {} {}: {}",
                group.name,
                components.len(),
                if detached { "disconnected sets of parts" } else { "sets without verified contact" },
                components.iter().map(|set| format!("[{}]", set.join(", "))).collect::<Vec<_>>().join("; "),
            ),
            where_mm: Some(bounds),
            hint: if detached {
                "Move the separated members into contact or put independent bodies in separate groups. Grounding and joint declarations do not establish physical contact."
            } else {
                "Apply the program to measure native solid contacts; bounding boxes alone cannot prove this group is connected."
            }.to_owned(),
        });
    }
}
