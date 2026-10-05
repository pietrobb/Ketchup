//! Bearing-joint check: every joint declared `bearing=True` is listed with
//! its published rating, or as not verified when the rating is missing. A
//! utilization needs the load the joint carries; until that is known the
//! joint stays not verified, even when it has a rating.

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
    /// "not_verified" until a rating and the joint's load are both known.
    pub status: &'static str,
    /// Why the joint is not verified.
    pub missing: Vec<&'static str>,
}

/// What a bearing joint lacks before it can be checked.
pub const NO_RATING: &str = "no published rating with a source";
pub const NO_LOAD: &str = "the load on the joint is not computed";

#[must_use]
pub fn joint_checks(model: &ProgramModel) -> Vec<JointCheck> {
    model
        .joints
        .iter()
        .filter(|joint| joint.bearing)
        .map(|joint| {
            let mut missing = Vec::new();
            if joint.rating.is_none() {
                missing.push(NO_RATING);
            }
            missing.push(NO_LOAD);
            JointCheck {
                joint: joint.name.clone(),
                kind: joint.kind.clone(),
                parts: joint.parts.clone(),
                fastener: joint.fastener.clone(),
                rating: joint.rating.clone(),
                status: "not_verified",
                missing,
            }
        })
        .collect()
}
