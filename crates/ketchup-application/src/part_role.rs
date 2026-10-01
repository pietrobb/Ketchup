//! What a part does for the validators, as a type.
//!
//! A part's role is the category name it carries in the `ketchup.validator-role.v1`
//! classification. The name is `<function>[.<frame>][:<group>]`, for example
//! `physics.beam.xy` or `manufacturing.hole.z:door-left`. It is parsed here, once, into
//! a [`PartRole`]; validators match on the function and read the frame, never the text.

use std::fmt;

/// The job a part does in a validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleFunction {
    Room,
    Occupant,
    Passage,
    Obstacle,
    Context,
    GravityBody,
    GravityGround,
    StaticLoad,
    StaticSupport,
    /// Any body that carries load across its plane; the plane normal is its depth axis.
    /// A role read from the body's source-frame extents, not a shape constructor.
    Beam,
    Freestanding,
    /// Any body whose thickness is measured along the plane normal (minimum thickness,
    /// hosted holes). A role, not a shape: the body may be any extrusion or sheet. The
    /// role names are stored classification categories, so they stay as written.
    Panel,
    Hole,
    CupBore,
    LinearPair,
}

/// How a role orients its part in the part's own source frame.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoleFrame {
    /// The role has no direction.
    None,
    /// One source axis: a hole's drilling axis, a free-standing body's vertical axis, a
    /// linear pair's travel axis.
    Axis(usize),
    /// A source plane, named by its normal axis: a panel's or beam's thickness axis, a
    /// passage's height axis.
    Plane { normal: usize },
}

impl RoleFrame {
    /// The single axis a role is about: the axis itself, or the plane's normal.
    #[must_use]
    pub const fn axis(self) -> Option<usize> {
        match self {
            Self::None => None,
            Self::Axis(axis) | Self::Plane { normal: axis } => Some(axis),
        }
    }

    /// The two source axes that span a plane.
    #[must_use]
    pub fn plane_axes(self) -> Option<[usize; 2]> {
        let Self::Plane { normal } = self else {
            return None;
        };
        let mut axes = (0..3).filter(|axis| *axis != normal);
        Some([axes.next()?, axes.next()?])
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrameKind {
    None,
    Axis,
    Plane,
}

/// Every role function: its name, the frame it takes, and whether it names a group
/// (a load case, a room, a host panel ...) after `:`.
const FUNCTIONS: [(RoleFunction, &str, FrameKind, bool); 15] = [
    (RoleFunction::Room, "spatial.room", FrameKind::None, true),
    (
        RoleFunction::Occupant,
        "spatial.occupant",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::Passage,
        "spatial.passage",
        FrameKind::Plane,
        true,
    ),
    (
        RoleFunction::Obstacle,
        "spatial.obstacle",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::Context,
        "spatial.context",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::GravityBody,
        "physics.gravity.body",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::GravityGround,
        "physics.gravity.ground",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::StaticLoad,
        "physics.static.load",
        FrameKind::None,
        true,
    ),
    (
        RoleFunction::StaticSupport,
        "physics.static.support",
        FrameKind::None,
        true,
    ),
    (RoleFunction::Beam, "physics.beam", FrameKind::Plane, false),
    (
        RoleFunction::Freestanding,
        "physics.freestanding",
        FrameKind::Axis,
        false,
    ),
    (
        RoleFunction::Panel,
        "manufacturing.panel",
        FrameKind::Plane,
        true,
    ),
    (
        RoleFunction::Hole,
        "manufacturing.hole",
        FrameKind::Axis,
        true,
    ),
    (
        RoleFunction::CupBore,
        "manufacturing.cup-bore",
        FrameKind::Axis,
        true,
    ),
    (
        RoleFunction::LinearPair,
        "hardware.linear-pair",
        FrameKind::Axis,
        true,
    ),
];

const AXIS_NAMES: [&str; 3] = ["x", "y", "z"];
/// Planes by their normal axis.
const PLANE_NAMES: [&str; 3] = ["yz", "xz", "xy"];

/// A parsed validator role. `group` is empty for functions that take none.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PartRole<'a> {
    pub function: RoleFunction,
    pub frame: RoleFrame,
    pub group: &'a str,
}

impl<'a> PartRole<'a> {
    /// Parses a role name; a name that is not a role of this vocabulary is `None`.
    #[must_use]
    pub fn parse(name: &'a str) -> Option<Self> {
        let (head, group) = match name.split_once(':') {
            Some((head, group)) if !group.is_empty() => (head, Some(group)),
            Some(_) => return None,
            None => (name, None),
        };
        FUNCTIONS
            .iter()
            .find_map(|&(function, function_name, frame_kind, grouped)| {
                if grouped != group.is_some() {
                    return None;
                }
                let rest = head.strip_prefix(function_name)?;
                let frame = match (frame_kind, rest.strip_prefix('.')) {
                    (FrameKind::None, _) if rest.is_empty() => RoleFrame::None,
                    (FrameKind::Axis, Some(axis)) => {
                        RoleFrame::Axis(AXIS_NAMES.iter().position(|name| *name == axis)?)
                    }
                    (FrameKind::Plane, Some(plane)) => RoleFrame::Plane {
                        normal: PLANE_NAMES.iter().position(|name| *name == plane)?,
                    },
                    _ => return None,
                };
                Some(Self {
                    function,
                    frame,
                    group: group.unwrap_or_default(),
                })
            })
    }
}

impl fmt::Display for PartRole<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (_, name, _, _) = FUNCTIONS
            .iter()
            .find(|(function, ..)| *function == self.function)
            .expect("every role function is listed");
        formatter.write_str(name)?;
        match self.frame {
            RoleFrame::None => {}
            RoleFrame::Axis(axis) => write!(formatter, ".{}", AXIS_NAMES[axis])?,
            RoleFrame::Plane { normal } => write!(formatter, ".{}", PLANE_NAMES[normal])?,
        }
        if !self.group.is_empty() {
            write!(formatter, ":{}", self.group)?;
        }
        Ok(())
    }
}

impl serde::Serialize for PartRole<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_role_name_round_trips_through_its_type() {
        for (function, name, frame_kind, grouped) in FUNCTIONS {
            let frames: Vec<(String, RoleFrame)> = match frame_kind {
                FrameKind::None => vec![(String::new(), RoleFrame::None)],
                FrameKind::Axis => (0..3)
                    .map(|axis| (format!(".{}", AXIS_NAMES[axis]), RoleFrame::Axis(axis)))
                    .collect(),
                FrameKind::Plane => (0..3)
                    .map(|normal| {
                        (
                            format!(".{}", PLANE_NAMES[normal]),
                            RoleFrame::Plane { normal },
                        )
                    })
                    .collect(),
            };
            for (suffix, frame) in frames {
                let text = format!("{name}{suffix}{}", if grouped { ":g-1" } else { "" });
                let role = PartRole::parse(&text).unwrap_or_else(|| panic!("{text} parses"));
                assert_eq!(role.function, function, "{text}");
                assert_eq!(role.frame, frame, "{text}");
                assert_eq!(role.group, if grouped { "g-1" } else { "" }, "{text}");
                assert_eq!(role.to_string(), text);
                assert_eq!(serde_json::to_value(role).unwrap(), text);
            }
        }
    }

    #[test]
    fn a_plane_names_its_normal_and_spanning_axes() {
        let beam = PartRole::parse("physics.beam.xz").unwrap();
        assert_eq!(beam.frame.axis(), Some(1));
        assert_eq!(beam.frame.plane_axes(), Some([0, 2]));
        assert_eq!(
            PartRole::parse("manufacturing.hole.y:door").unwrap().frame,
            RoleFrame::Axis(1)
        );
    }

    #[test]
    fn names_outside_the_vocabulary_or_its_grammar_are_not_roles() {
        for text in [
            "",
            "physics.beam",
            "physics.beam.xy:group",
            "physics.beam.zz",
            "physics.freestanding.xy",
            "manufacturing.hole.z",
            "manufacturing.hole.z:",
            "spatial.room",
            "spatial.room.x:a",
            "spatial.roomy:a",
            "structure.support",
            "furniture.shelf.xy",
            "manufacturing.hinge-cup.z:door",
            "spatial.furniture:room",
        ] {
            assert_eq!(PartRole::parse(text), None, "{text}");
        }
    }
}
