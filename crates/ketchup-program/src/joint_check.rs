//! Bearing-joint check: every joint declared `bearing=True` is listed with
//! its published rating and the load it passes, compared on the rating's
//! basis, or as not verified with what is missing (a rating, the load on it).
//! A joint that holds a member down against lifting is checked for that pull
//! against the rating's tension_n; one that is not bearing is listed as not
//! verified.

use crate::loads::{JointLoad, LoadReport, Loads, Tension};
use crate::member_check::{GAMMA_M_CONNECTION, action, kmod, ultimate};
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
    /// The pull it takes holding a member down, when it does.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tension: Option<Tension>,
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
pub const NO_TENSION_RATING: &str = "it holds the carried part down, but its rating gives no tension_n: the load it may carry down is no capacity to pull";

/// Why a joint that is not bearing and holds a member down is not verified.
#[must_use]
pub fn not_bearing(design_n: f64) -> String {
    format!(
        "it holds a member down against {design_n} N of pull (EQU), but only a bearing joint is checked: declare it bearing=True with connector_rating(tension_n=)"
    )
}

/// Utilization and the combination it comes from, on the rating's basis:
/// an allowable load against the characteristic sum, a characteristic one
/// as kmod R_k / gamma_M and a design one directly against the design load.
/// Downward a kind that pulls up relieves the joint and is left out; a pull
/// is checked against the rating's tension_n. Also what is missing.
fn utilization(
    model: &ProgramModel,
    joint: &Joint,
    rating: &JointRating,
    load: &JointLoad,
) -> (Option<(f64, String)>, Option<&'static str>) {
    let service_class = if rating.basis == "characteristic" {
        let class = joint.parts.iter().find_map(|name| {
            let material = model.part(name)?.material.as_ref()?;
            model.strength_classes.get(material)
        });
        match class {
            Some(class) => Some(class.service_class),
            None => return (None, Some(NO_SERVICE_CLASS)),
        }
    } else {
        None
    };
    let capacity = |load_n: f64, duration: usize| {
        service_class.map_or(load_n, |class| {
            kmod(class, duration) * load_n / GAMMA_M_CONNECTION
        })
    };
    let mut best: Option<(f64, String)> = None;
    let mut offer = |u: f64, label: String| {
        if best.as_ref().is_none_or(|(worst, _)| u > *worst) {
            best = Some((u, label));
        }
    };
    let down: Loads = load
        .loads_n
        .iter()
        .map(|(kind, value)| (kind.clone(), value.max(0.0)))
        .collect();
    if rating.basis == "allowable" {
        let total: f64 = down.values().sum();
        offer(total / rating.load_n, "characteristic sum".to_owned());
    } else {
        for (value, duration, label) in ultimate(&down) {
            offer(value.max(0.0) / capacity(rating.load_n, duration), label);
        }
    }
    let mut missing = None;
    if let Some(tension) = &load.tension {
        match rating.tension_n {
            None => missing = Some(NO_TENSION_RATING),
            Some(tension_n) if rating.basis == "allowable" => offer(
                tension.characteristic_n / tension_n,
                "pull, characteristic: permanent + variable where it lifts".to_owned(),
            ),
            Some(tension_n) => {
                // kmod of the longest of the loads that lift (the smallest).
                let duration = tension
                    .lifted_by
                    .iter()
                    .map(|kind| action(kind).0)
                    .min()
                    .unwrap_or(0);
                offer(
                    tension.design_n / capacity(tension_n, duration),
                    "pull, EQU: 0.9 permanent holding, 1.1 permanent and 1.5 variable lifting"
                        .to_owned(),
                );
            }
        }
    }
    (best, missing)
}

/// Every bearing joint, and every other joint that holds a member down.
#[must_use]
pub fn joint_checks(model: &ProgramModel, loads: &LoadReport) -> Vec<JointCheck> {
    model
        .joints
        .iter()
        .filter_map(|joint| {
            let load = loads.joints.get(&joint.name);
            let tension = load.and_then(|load| load.tension.clone());
            if !joint.bearing && tension.is_none() {
                return None;
            }
            let mut missing = Vec::new();
            if let (false, Some(tension)) = (joint.bearing, &tension) {
                missing.push(not_bearing(tension.design_n));
            } else if joint.rating.is_none() {
                missing.push(NO_RATING.to_owned());
            }
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
                let (found, lacking) = utilization(model, joint, rating, load);
                match found {
                    Some((u, _)) if u.is_nan() => {
                        missing.push("the utilization is not a number".to_owned());
                    }
                    found => checked = found,
                }
                missing.extend(lacking.map(str::to_owned));
            }
            let utilization = checked.as_ref().map(|(u, _)| (u * 1000.0).round() / 1000.0);
            let status = match utilization {
                Some(u) if u > 1.0 => "fail",
                Some(_) if missing.is_empty() => "pass",
                _ => "not_verified",
            };
            Some(JointCheck {
                joint: joint.name.clone(),
                kind: joint.kind.clone(),
                parts: joint.parts.clone(),
                fastener: joint.fastener.clone(),
                rating: joint.rating.clone(),
                load_n: load
                    .filter(|_| joint.bearing)
                    .map(|load| load.loads_n.clone()),
                tension,
                utilization,
                combination: checked.map(|(_, label)| label),
                status,
                missing,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Review cbec147 ST-N3: on the allowable basis the kinds were summed with
    /// their signs, so an imposed load pulling up hid the permanent load.
    #[test]
    fn a_kind_pulling_up_does_not_relieve_the_allowable_load_downward() {
        let rating = JointRating {
            load_n: 900.0,
            basis: "allowable".to_owned(),
            source: "test".to_owned(),
            note: String::new(),
            tension_n: None,
        };
        let joint = Joint {
            name: "hanger".to_owned(),
            kind: "hanger".to_owned(),
            parts: ["a".to_owned(), "b".to_owned()],
            links: Vec::new(),
            volume_mm: None,
            fasteners_mm: Vec::new(),
            fastener: None,
            max_gap_mm: 0.0,
            bearing: true,
            rating: Some(rating.clone()),
        };
        let load = JointLoad {
            loads_n: Loads::from([
                ("permanent".to_owned(), 1000.0),
                ("imposed".to_owned(), -800.0),
            ]),
            missing: Vec::new(),
            tension: None,
        };
        let (found, missing) = utilization(&ProgramModel::default(), &joint, &rating, &load);
        assert_eq!(missing, None);
        // 1000 / 900, not (1000 - 800) / 900.
        let (u, label) = found.expect("rated and loaded");
        assert!((u - 1000.0 / 900.0).abs() < 1e-9, "{u} {label}");
    }
}
