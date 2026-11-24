//! Joints between nodes, usually of different parts: a rigid link the second
//! node follows, springs between two nodes at one place, or a tie of a node to
//! a point of a triangle of three others.

use crate::fea::{DofLayout, FeaCouplingKind, FeaCouplingProblem, FeaError, FeaModel};
use ketchup_geometry::linalg::{length, sub};
use ketchup_model::tolerance::ROUNDING;
use std::collections::{BTreeMap, BTreeSet};

/// An unknown as a weighted sum of other unknowns.
type Terms = Vec<(usize, f64)>;

fn refused(coupling: usize, problem: FeaCouplingProblem) -> FeaError {
    FeaError::InvalidCoupling { coupling, problem }
}

/// From a coupling's first node to its second.
fn offset(model: &FeaModel, nodes: [usize; 2]) -> [f64; 3] {
    sub(
        model.nodes[nodes[1]].position_mm,
        model.nodes[nodes[0]].position_mm,
    )
}

/// Each rigid coupling's follower with the coupling's index and its leader.
fn rigid_followers(model: &FeaModel) -> BTreeMap<usize, (usize, usize)> {
    model
        .couplings
        .iter()
        .enumerate()
        .filter(|(_, coupling)| coupling.kind == FeaCouplingKind::Rigid)
        .map(|(index, coupling)| (coupling.nodes[1], (index, coupling.nodes[0])))
        .collect()
}

/// The nodes ties make follow.
fn tie_followers(model: &FeaModel) -> BTreeSet<usize> {
    model
        .couplings
        .iter()
        .filter(|coupling| matches!(coupling.kind, FeaCouplingKind::Tie { .. }))
        .map(|coupling| coupling.nodes[1])
        .collect()
}

/// What does not depend on which nodes turn: nodes, stiffness, springs at one
/// place, one leader per follower, no circle of followers, no held follower.
pub(crate) fn validate_couplings(model: &FeaModel) -> Result<(), FeaError> {
    let mut followers = BTreeSet::new();
    for (index, coupling) in model.couplings.iter().enumerate() {
        let [first, second] = coupling.nodes;
        if first == second || first >= model.nodes.len() || second >= model.nodes.len() {
            return Err(refused(index, FeaCouplingProblem::InvalidNode));
        }
        match coupling.kind {
            FeaCouplingKind::Rigid => {
                if !followers.insert(second) {
                    return Err(refused(index, FeaCouplingProblem::FollowsTwice));
                }
                if model
                    .constraints
                    .iter()
                    .any(|constraint| constraint.node == second)
                {
                    return Err(refused(index, FeaCouplingProblem::ConstrainedFollower));
                }
            }
            FeaCouplingKind::Spring {
                translation_n_per_mm,
                rotation_n_mm_per_rad,
            } => {
                let mut stiffness = translation_n_per_mm.iter().chain(&rotation_n_mm_per_rad);
                if stiffness
                    .clone()
                    .any(|value| !value.is_finite() || *value < 0.0)
                    || stiffness.all(|value| *value <= 0.0)
                {
                    return Err(refused(index, FeaCouplingProblem::InvalidStiffness));
                }
                if length(offset(model, coupling.nodes)) > ROUNDING {
                    return Err(refused(index, FeaCouplingProblem::SpringNodesApart));
                }
            }
            FeaCouplingKind::Tie { others, weights } => {
                let triangle = [first, others[0], others[1]];
                if others.iter().any(|node| *node >= model.nodes.len()) {
                    return Err(refused(index, FeaCouplingProblem::InvalidNode));
                }
                if triangle.contains(&second)
                    || others[0] == others[1]
                    || triangle[1..].contains(&first)
                    || weights
                        .iter()
                        .any(|weight| !(-ROUNDING..=1.0 + ROUNDING).contains(weight))
                    || (weights.iter().sum::<f64>() - 1.0).abs() > ROUNDING
                {
                    return Err(refused(index, FeaCouplingProblem::InvalidTie));
                }
                if !followers.insert(second) {
                    return Err(refused(index, FeaCouplingProblem::FollowsTwice));
                }
                if model
                    .constraints
                    .iter()
                    .any(|constraint| constraint.node == second)
                {
                    return Err(refused(index, FeaCouplingProblem::ConstrainedFollower));
                }
            }
        }
    }
    // A tie is set out after every rigid link and from nodes that follow
    // nothing, so its follower may lead no rigid link and its triangle follow
    // nothing.
    let tied = tie_followers(model);
    for (index, coupling) in model.couplings.iter().enumerate() {
        let leads_from_tied =
            coupling.kind == FeaCouplingKind::Rigid && tied.contains(&coupling.nodes[0]);
        let tie_on_follower = match coupling.kind {
            FeaCouplingKind::Tie { others, .. } => [coupling.nodes[0], others[0], others[1]]
                .iter()
                .any(|node| followers.contains(node)),
            _ => false,
        };
        if leads_from_tied || tie_on_follower {
            return Err(refused(index, FeaCouplingProblem::TieChain));
        }
    }
    let leaders = rigid_followers(model);
    for (index, coupling) in model.couplings.iter().enumerate() {
        if coupling.kind != FeaCouplingKind::Rigid {
            continue;
        }
        let mut node = coupling.nodes[0];
        for _ in 0..leaders.len() {
            if node == coupling.nodes[1] {
                return Err(refused(index, FeaCouplingProblem::Cycle));
            }
            match leaders.get(&node) {
                Some(&(_, leader)) => node = leader,
                None => break,
            }
        }
    }
    Ok(())
}

/// Marks the leaders rigid couplings make turn: one whose follower sits away
/// from it (the follower's move depends on the turn) or turns itself.
pub(crate) fn turning_leaders(model: &FeaModel, turning: &mut [bool]) {
    loop {
        let mut changed = false;
        for coupling in &model.couplings {
            let [leader, follower] = coupling.nodes;
            if coupling.kind != FeaCouplingKind::Rigid
                || leader >= turning.len()
                || follower >= turning.len()
                || turning[leader]
            {
                continue;
            }
            if turning[follower] || length(offset(model, coupling.nodes)) > ROUNDING {
                turning[leader] = true;
                changed = true;
            }
        }
        if !changed {
            return;
        }
    }
}

/// A turn spring needs both its nodes to turn.
pub(crate) fn validate_turns(model: &FeaModel, layout: &DofLayout) -> Result<(), FeaError> {
    for (index, coupling) in model.couplings.iter().enumerate() {
        if let FeaCouplingKind::Spring {
            rotation_n_mm_per_rad,
            ..
        } = coupling.kind
            && rotation_n_mm_per_rad.iter().any(|value| *value > 0.0)
            && coupling
                .nodes
                .iter()
                .any(|node| layout.dof(*node, 3).is_none())
        {
            return Err(refused(
                index,
                FeaCouplingProblem::TurnSpringNeedsTurningNodes,
            ));
        }
    }
    Ok(())
}

/// Each spring's stiffness and the unknowns at its two ends.
fn springs(model: &FeaModel, layout: &DofLayout) -> Result<Vec<(f64, usize, usize)>, FeaError> {
    let mut springs = Vec::new();
    for (index, coupling) in model.couplings.iter().enumerate() {
        let FeaCouplingKind::Spring {
            translation_n_per_mm,
            rotation_n_mm_per_rad,
        } = coupling.kind
        else {
            continue;
        };
        for (axis, stiffness) in translation_n_per_mm
            .into_iter()
            .chain(rotation_n_mm_per_rad)
            .enumerate()
        {
            if stiffness <= 0.0 {
                continue;
            }
            let dof = |node| {
                layout.dof(node, axis).ok_or(refused(
                    index,
                    FeaCouplingProblem::TurnSpringNeedsTurningNodes,
                ))
            };
            springs.push((stiffness, dof(coupling.nodes[0])?, dof(coupling.nodes[1])?));
        }
    }
    Ok(springs)
}

/// How many entries [`add_springs`] adds.
pub(crate) fn spring_entry_count(model: &FeaModel, layout: &DofLayout) -> Result<usize, FeaError> {
    Ok(4 * springs(model, layout)?.len())
}

pub(crate) fn add_springs(
    model: &FeaModel,
    layout: &DofLayout,
    entries: &mut Vec<(usize, usize, f64)>,
) -> Result<(), FeaError> {
    for (stiffness, first, second) in springs(model, layout)? {
        entries.extend([
            (first, first, stiffness),
            (second, second, stiffness),
            (first, second, -stiffness),
            (second, first, -stiffness),
        ]);
    }
    Ok(())
}

/// The energy the springs store: half the stiffness times the stretch squared.
pub(crate) fn spring_energy(model: &FeaModel, layout: &DofLayout, displacement: &[f64]) -> f64 {
    springs(model, layout).map_or(0.0, |springs| {
        springs
            .into_iter()
            .map(|(stiffness, first, second)| {
                0.5 * stiffness * (displacement[second] - displacement[first]).powi(2)
            })
            .sum()
    })
}

/// What a convergence study keeps between meshes: the places rigid couplings
/// lead from and each spring's place and stiffness.
pub(crate) fn joint_definition(model: &FeaModel) -> (BTreeSet<[u64; 3]>, Vec<[u64; 9]>) {
    let mut leaders = BTreeSet::new();
    let mut springs = Vec::new();
    for coupling in &model.couplings {
        let Some(place) = model.nodes.get(coupling.nodes[0]) else {
            continue;
        };
        let place = place.position_mm.map(f64::to_bits);
        match coupling.kind {
            FeaCouplingKind::Rigid => {
                leaders.insert(place);
            }
            FeaCouplingKind::Spring {
                translation_n_per_mm,
                rotation_n_mm_per_rad,
            } => {
                let mut values = [0; 9];
                for (value, bits) in place
                    .into_iter()
                    .chain(translation_n_per_mm.map(f64::to_bits))
                    .chain(rotation_n_mm_per_rad.map(f64::to_bits))
                    .zip(&mut values)
                {
                    *bits = value;
                }
                springs.push(values);
            }
            // A tie follows the mesh, which each level refines.
            FeaCouplingKind::Tie { .. } => {}
        }
    }
    springs.sort_unstable();
    (leaders, springs)
}

/// The unknowns of rigid couplings' followers, each as a sum of unknowns that
/// follow nothing.
pub(crate) struct Joints {
    followers: BTreeMap<usize, Terms>,
}

impl Joints {
    pub(crate) fn new(model: &FeaModel, layout: &DofLayout) -> Result<Self, FeaError> {
        let rigid = rigid_followers(model);
        let mut joints = Self {
            followers: BTreeMap::new(),
        };
        let mut resolved = BTreeSet::new();
        for &start in rigid.keys() {
            // A leader that follows another is set out first.
            let mut chain = Vec::new();
            let mut node = start;
            while !resolved.contains(&node)
                && let Some(&(index, leader)) = rigid.get(&node)
            {
                chain.push((node, index, leader));
                node = leader;
            }
            for (follower, index, leader) in chain.into_iter().rev() {
                joints.follow(model, layout, index, [leader, follower])?;
                resolved.insert(follower);
            }
        }
        for (index, coupling) in model.couplings.iter().enumerate() {
            let FeaCouplingKind::Tie { others, weights } = coupling.kind else {
                continue;
            };
            let triangle = [coupling.nodes[0], others[0], others[1]];
            for axis in 0..3 {
                let mut terms = Vec::with_capacity(3);
                for (node, weight) in triangle.into_iter().zip(weights) {
                    let dof = layout
                        .dof(node, axis)
                        .ok_or(refused(index, FeaCouplingProblem::InvalidNode))?;
                    terms.push((dof, weight));
                }
                let follower = layout
                    .dof(coupling.nodes[1], axis)
                    .ok_or(refused(index, FeaCouplingProblem::InvalidNode))?;
                joints.followers.insert(follower, terms);
            }
        }
        Ok(joints)
    }

    /// The follower moves by `u + θ × r` and turns by `θ`, with `u` and `θ`
    /// the leader's move and turn and `r` the offset from leader to follower.
    fn follow(
        &mut self,
        model: &FeaModel,
        layout: &DofLayout,
        index: usize,
        [leader, follower]: [usize; 2],
    ) -> Result<(), FeaError> {
        let dof = |node, axis| {
            layout
                .dof(node, axis)
                .ok_or(refused(index, FeaCouplingProblem::InvalidNode))
        };
        let lever = offset(model, [leader, follower]);
        let swings = length(lever) > ROUNDING;
        for axis in 0..3 {
            let mut terms = vec![(dof(leader, axis)?, 1.0)];
            if swings {
                let (next, after) = ((axis + 1) % 3, (axis + 2) % 3);
                terms.push((dof(leader, 3 + next)?, lever[after]));
                terms.push((dof(leader, 3 + after)?, -lever[next]));
            }
            let terms = self.independent(terms);
            self.followers.insert(dof(follower, axis)?, terms);
        }
        if layout.dof(follower, 3).is_some() {
            for axis in 3..6 {
                let terms = self.independent(vec![(dof(leader, axis)?, 1.0)]);
                self.followers.insert(dof(follower, axis)?, terms);
            }
        }
        Ok(())
    }

    /// `terms` with every unknown that follows replaced by what it follows.
    fn independent(&self, terms: Terms) -> Terms {
        let mut sum = BTreeMap::<usize, f64>::new();
        for (dof, weight) in terms {
            match self.followers.get(&dof) {
                Some(inner) => {
                    for &(inner_dof, inner_weight) in inner {
                        *sum.entry(inner_dof).or_default() += weight * inner_weight;
                    }
                }
                None => *sum.entry(dof).or_default() += weight,
            }
        }
        sum.into_iter().collect()
    }

    fn terms(&self, dof: usize) -> Terms {
        self.followers
            .get(&dof)
            .cloned()
            .unwrap_or_else(|| vec![(dof, 1.0)])
    }

    pub(crate) fn follower_dofs(&self) -> impl Iterator<Item = usize> + '_ {
        self.followers.keys().copied()
    }

    /// How many entries [`Self::reduce`] makes of `entries`; `None` when it
    /// passes them on as they are.
    pub(crate) fn reduced_len(&self, entries: &[(usize, usize, f64)]) -> Option<usize> {
        if self.followers.is_empty() {
            return None;
        }
        let terms = |dof| self.followers.get(&dof).map_or(1, Vec::len);
        Some(
            entries
                .iter()
                .map(|&(row, column, _)| terms(row) * terms(column))
                .sum::<usize>()
                + self.followers.len(),
        )
    }

    /// `Tᵀ·K·T` and `Tᵀ·f`, with `T` writing every unknown in the unknowns
    /// that follow nothing; a follower keeps an empty diagonal entry.
    pub(crate) fn reduce(
        &self,
        entries: Vec<(usize, usize, f64)>,
        forces: &mut [f64],
    ) -> Vec<(usize, usize, f64)> {
        let Some(count) = self.reduced_len(&entries) else {
            return entries;
        };
        let mut reduced = Vec::with_capacity(count);
        for (row, column, value) in entries {
            if !self.followers.contains_key(&row) && !self.followers.contains_key(&column) {
                reduced.push((row, column, value));
                continue;
            }
            for (row, row_weight) in self.terms(row) {
                for (column, column_weight) in self.terms(column) {
                    reduced.push((row, column, row_weight * column_weight * value));
                }
            }
        }
        for (&dof, terms) in &self.followers {
            reduced.push((dof, dof, 0.0));
            let force = std::mem::take(&mut forces[dof]);
            for &(independent, weight) in terms {
                forces[independent] += weight * force;
            }
        }
        reduced
    }

    /// Sets each follower from the unknowns it follows.
    pub(crate) fn recover(&self, displacement: &mut [f64]) {
        for (&dof, terms) in &self.followers {
            displacement[dof] = terms
                .iter()
                .map(|&(independent, weight)| weight * displacement[independent])
                .sum();
        }
    }
}
