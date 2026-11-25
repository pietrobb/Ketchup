//! Whether the braces of a member hold it as EN 1995-1-1 9.2.5.3 asks: their
//! paths to the ground at least as stiff as C and buckling under no less than
//! F_d, the force at the design strength f_c,0,d = k_mod f_c,0,k / gamma_M
//! (review 2026-11-24 R1124og-PG-902: a strip 14 x 14 buckling at
//! k_c f_c,0,k A = 293 N held against F_d = 263 N, where at f_c,0,d it buckles
//! at 180), in every ultimate combination at its own k_mod (review 2026-11-25
//! R1125og-PG-1001: k_mod came from the combination of the largest moment, so
//! adding an imposed load passed strips that 1.35 G alone buckles).

use super::{
    Bending, BracingDemand, Combination, Design, GAMMA_G_FAVOURABLE, PRODUCTS, Segment, Shape,
    TimberClass, Zone, extremes, free_stretches, kcrit, round3,
};
use crate::loads::{Brace, MemberLoad};

/// F_d one ultimate combination asks of each brace of an edge.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct CombinationForce {
    label: String,
    duration: usize,
    force_n: f64,
}

/// What the braces of the compressed edge must give (EN 1995-1-1 9.2.5.2,
/// review 2026-11-08 PG-242): over each stretch between the bearings, the
/// compression N_d = (1 - kcrit) M_d / h of the edge as if nothing braced it
/// (9.2.5.2(4)), the force N_d / k_f at a brace and the stiffness k_s N_d / a
/// it needs, k_s = 2 (1 + cos(pi / m)) taken at its bound 4, for each edge
/// something braces where it is compressed; N_d and F_d the largest of the
/// combinations, and F_d of each combination for the k_mod it buckles a brace at.
pub(super) fn demand(
    load: &MemberLoad,
    shape: &Shape,
    design: &Design,
    uls: &[Combination],
    zones: &[Zone],
    segments: &[(Segment, Vec<Option<Bending>>)],
    kinds: &[usize],
) -> Vec<BracingDemand> {
    const K_S: f64 = 4.0;
    let class = design.class;
    let h = shape.h;
    let b = shape.b.min(shape.area / h);
    let k_f = if class.glulam { 80.0 } else { 60.0 };
    let mut demands = Vec::new();
    for top in [true, false] {
        let edge = if top { "top" } else { "bottom" };
        let mut by: Vec<String> = load
            .braced_by
            .iter()
            .filter(|brace| brace.edge == edge)
            .map(|brace| brace.part.clone())
            .collect();
        by.sort();
        by.dedup();
        if by.is_empty() {
            continue;
        }
        // N_d of each combination, the largest over the stretches.
        let mut axial = vec![0.0_f64; uls.len()];
        for (lo, hi) in free_stretches(load, &[], zones, top) {
            let length = (hi - lo) * shape.arc;
            let lef = if top { length + 2.0 * h } else { length };
            if lef <= 1.0 {
                continue;
            }
            let sigma_crit = 0.78 * b * b * class.e0_05 / (h * lef);
            let k = kcrit((class.fm_k / sigma_crit).sqrt());
            let mut moment = vec![0.0_f64; uls.len()];
            for (seg, sections) in segments {
                for (j, x) in seg.xs.iter().enumerate() {
                    if sections[j].is_none() || *x < lo || *x > hi {
                        continue;
                    }
                    let components: Vec<f64> = seg.moments.iter().map(|m| m[j]).collect();
                    for (largest, combination) in moment.iter_mut().zip(uls) {
                        let (sagging, hogging) = extremes(
                            &combination.factors,
                            kinds,
                            &components,
                            Some(GAMMA_G_FAVOURABLE),
                        );
                        *largest = largest.max(if top { sagging } else { -hogging });
                    }
                }
            }
            for (n_d, m_d) in axial.iter_mut().zip(&moment) {
                *n_d = n_d.max((1.0 - k) * m_d / h);
            }
        }
        let largest = axial.iter().copied().fold(0.0_f64, f64::max);
        if largest <= 0.0 {
            continue;
        }
        let longest = free_stretches(load, &load.braced_by, zones, top)
            .into_iter()
            .map(|(lo, hi)| (hi - lo) * shape.arc)
            .fold(0.0, f64::max);
        demands.push(BracingDemand {
            edge: edge.to_owned(),
            by,
            axial_n: round3(largest),
            force_n: round3(largest / k_f),
            stiffness_n_mm: (longest > 1.0).then(|| round3(K_S * largest / longest)),
            by_combination: uls
                .iter()
                .zip(&axial)
                .filter(|(_, n_d)| **n_d > 0.0)
                .map(|(combination, n_d)| CombinationForce {
                    label: combination.label.clone(),
                    duration: combination.duration,
                    force_n: round3(n_d / k_f),
                })
                .collect(),
        });
    }
    demands
}

/// k_mod and gamma_M of `class` for the load-duration class `duration`; of
/// solid softwood in `service_class` for a part of no strength class, whose
/// path the anchorage takes as softwood C24.
pub(super) fn design_factors(
    class: Option<&TimberClass>,
    service_class: u8,
    duration: usize,
) -> (f64, f64) {
    match class {
        Some(class) => (class.kmod(duration), class.product().gamma_m),
        None => {
            let solid = &PRODUCTS[0];
            let kmod = solid.kmod[usize::from(service_class) - 1].map_or(0.0, |row| row[duration]);
            (kmod, solid.gamma_m)
        }
    }
}

/// `braces` with the force their paths buckle under at the design strength,
/// k_mod / gamma_M (`factors`, of the part buckling and the load-duration
/// class) times k_c f_c,0,k A, in the combination of `demands` asking most of
/// that strength, which each names with its k_mod and F_d.
pub(super) fn designed(
    braces: &[Brace],
    demands: &[BracingDemand],
    factors: impl Fn(&str, usize) -> (f64, f64),
) -> Vec<Brace> {
    braces
        .iter()
        .map(|brace| {
            let mut brace = brace.clone();
            let Some(demand) = demands.iter().find(|demand| demand.edge == brace.edge) else {
                return brace;
            };
            let Some(anchorage) = brace.anchorage.as_mut() else {
                return brace;
            };
            let Some(characteristic) = anchorage.capacity_n else {
                return brace;
            };
            let part = anchorage
                .buckling_in
                .clone()
                .unwrap_or_else(|| anchorage.through.clone());
            let governing = demand
                .by_combination
                .iter()
                .map(|asked| {
                    let (kmod, gamma_m) = factors(&part, asked.duration);
                    (asked, kmod, characteristic * kmod / gamma_m)
                })
                .min_by(|a, b| (a.2 / a.0.force_n).total_cmp(&(b.2 / b.0.force_n)));
            if let Some((asked, kmod, capacity)) = governing {
                anchorage.capacity_n = Some(round3(capacity));
                anchorage.combination = Some(asked.label.clone());
                anchorage.k_mod = Some(kmod);
                anchorage.force_n = Some(asked.force_n);
            }
            brace
        })
        .collect()
}

/// The braces of `braces` whose path to the ground holds less stiffly, or
/// buckles under less force, than `demands` ask of a brace of their edge
/// (EN 1995-1-1 9.2.5.3: C = k_s N_d / a, F_d = N_d / k_f), with why.
pub(super) fn weak<'b>(braces: &'b [Brace], demands: &[BracingDemand]) -> Vec<(&'b Brace, String)> {
    braces
        .iter()
        .filter_map(|brace| {
            let held = brace.anchorage.as_ref()?;
            let demand = demands.iter().find(|demand| demand.edge == brace.edge)?;
            let flexible = demand
                .stiffness_n_mm
                .filter(|needed| held.stiffness_n_mm < *needed);
            let force = held.force_n.unwrap_or(demand.force_n);
            let buckles = held.capacity_n.filter(|capacity| *capacity < force);
            if flexible.is_none() && buckles.is_none() {
                return None;
            }
            let mut short = Vec::new();
            if let Some(needed) = flexible {
                short.push(format!(
                    "C = {} N/mm against k_s N_d / a = {needed} N/mm",
                    held.stiffness_n_mm
                ));
            }
            if let Some(capacity) = buckles {
                let of = match (held.k_mod, &held.combination) {
                    (Some(kmod), Some(combination)) => format!(" (k_mod {kmod}, {combination})"),
                    _ => String::new(),
                };
                short.push(format!(
                    "{} buckles at k_c f_c,0,d A = {capacity} N{of} against F_d = {force} N",
                    held.buckling_in.as_deref().unwrap_or(&held.through),
                ));
            }
            Some((
                brace,
                format!(
                    "{} holds its {} edge through {}, too flexible or too slender to hold it sideways ({}; EN 1995-1-1 9.2.5.3), so it holds nothing",
                    brace.part,
                    brace.edge,
                    held.through,
                    short.join(", ")
                ),
            ))
        })
        .collect()
}

/// Why `braces` leave their member not verified (review 2026-11-25
/// R1125og-PG-1004): a brace, or the part of its path buckling, of a material
/// whose strength class names no product, whose k_mod and gamma_M are
/// unknown; as a member of it is not verified (review 2026-11-02 PG-106).
pub(super) fn unknown_products<'m>(
    braces: &[Brace],
    class_of: impl Fn(&str) -> Option<(&'m str, &'m TimberClass)>,
) -> Vec<String> {
    let mut why = Vec::new();
    for brace in braces {
        let buckling = brace
            .anchorage
            .as_ref()
            .and_then(|held| held.capacity_n.and(held.buckling_in.as_deref()));
        for part in std::iter::once(brace.part.as_str()).chain(buckling) {
            if let Some((material, class)) = class_of(part)
                && class.product.is_empty()
                && !class.glulam
            {
                let reason = format!(
                    "{part} braces it, but timber_strength({material:?}) names no product (product=\"solid\", \"osb3\", ...), so the k_mod and gamma_M it buckles at are unknown"
                );
                if !why.contains(&reason) {
                    why.push(reason);
                }
            }
        }
    }
    why
}
