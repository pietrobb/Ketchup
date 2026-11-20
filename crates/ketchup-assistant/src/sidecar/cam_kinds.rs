//! The kinds of tool and work offset a CAM plan names.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCamToolKind {
    FlatEndMill,
    BallEndMill,
    Drill,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AssistantCamWorkOffset {
    G54,
    G55,
    G56,
    G57,
    G58,
    G59,
}
