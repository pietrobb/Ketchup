//! Whether the braces of a member hold it as EN 1995-1-1 9.2.5.3 asks: their
//! paths to the ground at least as stiff as C and buckling under no less than
//! F_d, the force at the design strength f_c,0,d = k_mod f_c,0,k / gamma_M
//! (review 2026-11-24 R1124og-PG-902: a strip 14 x 14 buckling at
//! k_c f_c,0,k A = 293 N held against F_d = 263 N, where at f_c,0,d it buckles
//! at 180).

use super::{BracingDemand, PRODUCTS, TimberClass};
use crate::loads::Brace;

/// k_mod / gamma_M of `class` for the load-duration class `duration`; of
/// solid softwood in `service_class` for a part of no strength class, whose
/// path the anchorage takes as softwood C24.
pub(super) fn design_share(class: Option<&TimberClass>, service_class: u8, duration: usize) -> f64 {
    match class {
        Some(class) => class.kmod(duration) / class.product().gamma_m,
        None => {
            let solid = &PRODUCTS[0];
            solid.kmod[usize::from(service_class) - 1].map_or(0.0, |row| row[duration])
                / solid.gamma_m
        }
    }
}

/// `braces` with the force their paths buckle under at the design strength:
/// k_mod / gamma_M (`share`, of the part buckling and the load-duration class
/// of the combination governing F_d of its edge) times k_c f_c,0,k A.
pub(super) fn designed(
    braces: &[Brace],
    demands: &[BracingDemand],
    share: impl Fn(&str, usize) -> f64,
) -> Vec<Brace> {
    braces
        .iter()
        .map(|brace| {
            let mut brace = brace.clone();
            let duration = demands
                .iter()
                .find(|demand| demand.edge == brace.edge)
                .map(|demand| demand.duration);
            if let (Some(anchorage), Some(duration)) = (brace.anchorage.as_mut(), duration) {
                let part = anchorage
                    .buckling_in
                    .clone()
                    .unwrap_or_else(|| anchorage.through.clone());
                anchorage.capacity_n = anchorage
                    .capacity_n
                    .map(|capacity| super::round3(capacity * share(&part, duration)));
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
            let buckles = held.capacity_n.filter(|capacity| *capacity < demand.force_n);
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
                short.push(format!(
                    "{} buckles at k_c f_c,0,d A = {capacity} N against F_d = {} N",
                    held.buckling_in.as_deref().unwrap_or(&held.through),
                    demand.force_n
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
