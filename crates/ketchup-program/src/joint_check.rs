//! Bearing-joint check: every joint declared `bearing=True` is listed with
//! its published rating and the load it passes, compared on the rating's
//! basis, or as not verified with what is missing (a rating, the load on it).

use crate::loads::{LoadReport, Loads};
use crate::member_check::{GAMMA_M_CONNECTION, kmod, ultimate};
use crate::model::{Joint, JointRating, ProgramModel};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct JointCheck {
    pub joint: String,
    pub kind: String,
    /// [carried part, carrying part].
    pub parts: [String; 2],
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fastener: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rating: Option<JointRating>,
    /// Characteristic load the carried part passes through the joint, by kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_n: Option<Loads>,
    /// Load over capacity on the rating's basis, when both are known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub utilization: Option<f64>,
    /// How the load was taken: characteristic sum or the governing combination.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub combination: Option<String>,
    /// "pass" or "fail" once a rating and the joint's complete load are both
    /// known, "not_verified" before.
    pub status: &'static str,
    /// Why the joint is not verified.
    pub missing: Vec<String>,
}

/// What a bearing joint lacks before it can be checked.
pub const NO_RATING: &str = "no published rating with a source";
pub const NO_LOAD: &str = "the load on the joint is not computed";
pub const NO_SERVICE_CLASS: &str = "a characteristic rating needs kmod: neither joined part has a timber strength class (timber_strength)";

/// Utilization and the combination it comes from, on the rating's basis:
/// an allowable load against the characteristic sum, a characteristic one
/// as kmod R_k / gamma_M and a design one directly against the design load.
fn utilization(
    model: &ProgramModel,
    joint: &Joint,
    rating: &JointRating,
    loads: &Loads,
) -> Result<(f64, String), &'static str> {
    if rating.basis == "allowable" {
        let total: f64 = loads.values().sum();
        return Ok((total.abs() / rating.load_n, "characteristic sum".to_owned()));
    }
    let service_class = if rating.basis == "characteristic" {
        let class = joint.parts.iter().find_map(|name| {
            let material = model.part(name)?.material.as_ref()?;
            model.strength_classes.get(material)
        });
        Some(class.ok_or(NO_SERVICE_CLASS)?.service_class)
    } else {
        None
    };
    let mut best: Option<(f64, String)> = None;
    for (value, duration, label) in ultimate(loads) {
        let capacity = service_class.map_or(rating.load_n, |class| {
            kmod(class, duration) * rating.load_n / GAMMA_M_CONNECTION
        });
        let u = value.abs() / capacity;
        if best.as_ref().is_none_or(|(worst, _)| u > *worst) {
            best = Some((u, label));
        }
    }
    Ok(best.unwrap_or((0.0, String::new())))
}

#[must_use]
pub fn joint_checks(model: &ProgramModel, loads: &LoadReport) -> Vec<JointCheck> {
    model
        .joints
        .iter()
        .filter(|joint| joint.bearing)
        .map(|joint| {
            let mut missing = Vec::new();
            if joint.rating.is_none() {
                missing.push(NO_RATING.to_owned());
            }
            let load = loads.joints.get(&joint.name);
            match load {
                None => missing.push(NO_LOAD.to_owned()),
                Some(load) => missing.extend(
                    load.missing
                        .iter()
                        .map(|reason| format!("the load is incomplete: {reason}")),
                ),
            }
            let mut checked = None;
            if let (Some(rating), Some(load)) = (&joint.rating, load) {
                match utilization(model, joint, rating, &load.loads_n) {
                    Ok((u, _)) if u.is_nan() => {
                        missing.push("the utilization is not a number".to_owned())
                    }
                    Ok(found) => checked = Some(found),
                    Err(reason) => missing.push(reason.to_owned()),
                }
            }
            let utilization = checked.as_ref().map(|(u, _)| (u * 1000.0).round() / 1000.0);
            let status = match utilization {
                Some(u) if u > 1.0 => "fail",
                Some(_) if missing.is_empty() => "pass",
                _ => "not_verified",
            };
            JointCheck {
                joint: joint.name.clone(),
                kind: joint.kind.clone(),
                parts: joint.parts.clone(),
                fastener: joint.fastener.clone(),
                rating: joint.rating.clone(),
                load_n: load.map(|load| load.loads_n.clone()),
                utilization,
                combination: checked.map(|(_, label)| label),
                status,
                missing,
            }
        })
        .collect()
}
