//! Bearing-joint check: every joint declared `bearing=True` is listed with
//! its published rating and the load it passes, or as not verified with what
//! is missing (a rating, the load on it).

use crate::loads::{LoadReport, Loads};
use crate::model::{JointRating, ProgramModel};
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
    /// "not_verified" until a rating and the joint's complete load are both known.
    pub status: &'static str,
    /// Why the joint is not verified.
    pub missing: Vec<String>,
}

/// What a bearing joint lacks before it can be checked.
pub const NO_RATING: &str = "no published rating with a source";
pub const NO_LOAD: &str = "the load on the joint is not computed";

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
            JointCheck {
                joint: joint.name.clone(),
                kind: joint.kind.clone(),
                parts: joint.parts.clone(),
                fastener: joint.fastener.clone(),
                rating: joint.rating.clone(),
                load_n: load.map(|load| load.loads_n.clone()),
                status: "not_verified",
                missing,
            }
        })
        .collect()
}
