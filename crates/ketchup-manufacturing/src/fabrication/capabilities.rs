//! What each shop format carries of a part, and the one gate every export
//! passes before it writes a part: an element the format cannot carry is
//! refused by the part's name and the element, never as a generic block.

use super::*;

/// A file the projection writes for the shop.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShopFormat {
    /// The production drawing (SVG).
    Drawing,
    /// The neutral production job (JSON).
    ProductionJob,
    /// BTLx 2.3.1.
    Btlx,
    /// woodWOP MPR 4.0; a profile cut needs the vertical pocket tool.
    Woodwop { vertical_pocket_tool: bool },
}

impl fmt::Display for ShopFormat {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Drawing => "the production drawing",
            Self::ProductionJob => "the production job",
            Self::Btlx => "BTLx",
            Self::Woodwop { .. } => "a woodWOP program",
        })
    }
}

impl ShopFormat {
    /// Whether the format writes a part of this kind: BTLx prismatic members
    /// only, the drawing every part, the others every part that is made.
    #[must_use]
    pub fn writes(self, kind: GeneralBomItemKind) -> bool {
        match self {
            Self::Drawing => true,
            Self::Btlx => kind.is_member(),
            Self::ProductionJob | Self::Woodwop { .. } => kind != GeneralBomItemKind::Purchased,
        }
    }
}

/// An element of a part a format carries, loses or refuses.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ShopElement {
    /// A copy placed mirrored: the opposite hand of its part.
    MirroredCopy,
    /// A machining operation of this kind.
    Operation(GeneralManufacturingKind),
    /// A thread tapped into a drilling, by its label.
    Thread(String),
    /// A pin hole only the pin joint of this name holds, not drilled into the part.
    PinHole(String),
    /// A hole drilled through all: its interval is the padded Boolean cutter,
    /// not a depth a machine may drill.
    ThroughHole,
}

impl fmt::Display for ShopElement {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MirroredCopy => formatter.write_str("a mirrored copy"),
            Self::Operation(kind) => write!(formatter, "a {} operation", kind.token()),
            Self::Thread(label) => write!(formatter, "a tapped hole ({label})"),
            Self::PinHole(joint) => write!(formatter, "a pin hole of pin joint {joint}"),
            Self::ThroughHole => formatter.write_str("a hole drilled through all"),
        }
    }
}

/// What a format does with an element.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Carriage {
    Carries,
    /// Written, but the machine may not make it: the loss report says why.
    Loses(&'static str),
    /// Not written, for this reason and with this way out.
    Refuses(&'static str),
}

const MIRRORED_COPY: &str = "its machining is given for a proper placement of the part, so the \
     opposite hand would be made as the original; make the mirrored part its own definition";
pub(super) const BTLX_THREAD_LOSS: &str = "BTLx 2.3.1 has no tapping processing; each thread is \
     only the Drilling's Comment and its ThreadSize, ThreadPitch and ThreadDepth user attributes, \
     and a machine that ignores them drills the core hole without the thread";
const WOODWOP_THREAD: &str = "its drilling macro drills the core hole only, so the part would be \
     made without the thread; export BTLx or the production job, which carry the thread, or \
     remove the thread";
const UNDRILLED_PIN_HOLE: &str = "it takes only the machining modelled in the part, and the \
     joint's holes are not drilled into it; drill them into the parts (pin joint holes: drill), \
     or export a woodWOP program or the production job, which drill the joint's holes";
const THROUGH_HOLE: &str = "it drills only holes with a depth that stops inside the part, and a \
     through-all hole has no machine depth, only the Boolean cutter that runs past the part; export \
     the production job or the drawing, which carry the hole as through-all";
const WOODWOP_PROFILE_WITHOUT_TOOL: &str =
    "a profile cut is milled as a vertical pocket; choose the vertical pocket tool number";
const WOODWOP_OTHER_CUT: &str =
    "woodWOP takes drillings and vertical pockets only; export BTLx or the production job";

/// The capability table: what `format` does with `element`.
#[must_use]
pub fn carriage(format: ShopFormat, element: &ShopElement) -> Carriage {
    use GeneralManufacturingKind as Kind;
    match (format, element) {
        (ShopFormat::Drawing | ShopFormat::Btlx, ShopElement::PinHole(_)) => {
            Carriage::Refuses(UNDRILLED_PIN_HOLE)
        }
        (ShopFormat::ProductionJob | ShopFormat::Woodwop { .. }, ShopElement::PinHole(_))
        | (ShopFormat::Drawing, _)
        | (ShopFormat::ProductionJob | ShopFormat::Btlx, ShopElement::Operation(_))
        | (ShopFormat::ProductionJob, ShopElement::Thread(_) | ShopElement::ThroughHole)
        | (ShopFormat::Woodwop { .. }, ShopElement::Operation(Kind::Stock | Kind::CircularDrill))
        | (
            ShopFormat::Woodwop {
                vertical_pocket_tool: true,
            },
            ShopElement::Operation(Kind::ProfileCut),
        ) => Carriage::Carries,
        (_, ShopElement::MirroredCopy) => Carriage::Refuses(MIRRORED_COPY),
        (ShopFormat::Btlx | ShopFormat::Woodwop { .. }, ShopElement::ThroughHole) => {
            Carriage::Refuses(THROUGH_HOLE)
        }
        (ShopFormat::Btlx, ShopElement::Thread(_)) => Carriage::Loses(BTLX_THREAD_LOSS),
        (ShopFormat::Woodwop { .. }, ShopElement::Thread(_)) => Carriage::Refuses(WOODWOP_THREAD),
        (ShopFormat::Woodwop { .. }, ShopElement::Operation(Kind::ProfileCut)) => {
            Carriage::Refuses(WOODWOP_PROFILE_WITHOUT_TOOL)
        }
        (ShopFormat::Woodwop { .. }, ShopElement::Operation(_)) => {
            Carriage::Refuses(WOODWOP_OTHER_CUT)
        }
    }
}

/// An element a format writes but a machine may not make.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShopLoss {
    pub part: String,
    pub element: ShopElement,
    pub reason: &'static str,
}

impl GeneralFabricationProjection {
    /// The gate every export passes before it writes a part: the copies,
    /// operations and threads of each part `format` writes are looked up in
    /// the capability table for `format`. The first element refused is named with its part;
    /// the elements written at a loss are returned for the loss report.
    ///
    /// # Errors
    /// [`GeneralFabricationError::CannotCarry`] for a refused element, or
    /// the placement error of a copy that does not resolve or is scaled.
    pub fn admit(
        &self,
        snapshot: &Snapshot,
        format: ShopFormat,
    ) -> Result<Vec<ShopLoss>, GeneralFabricationError> {
        let mut losses = Vec::new();
        let pin_holes = undrilled_pin_holes(snapshot)?;
        for row in self
            .bom
            .rows
            .iter()
            .filter(|row| format.writes(row.item_kind))
        {
            let part = snapshot
                .definition(row.definition_id)
                .map_or_else(String::new, |definition| definition.name().to_owned());
            let mut elements = Vec::new();
            if row.mirrored || copies_are_mirrored(snapshot, row)? {
                elements.push(ShopElement::MirroredCopy);
            }
            for operation in self
                .manufacturing
                .operations
                .iter()
                .filter(|operation| is_row_operation(row, operation))
            {
                elements.push(ShopElement::Operation(operation.kind));
                if let GeneralMachiningGeometry::CircularDrill { through: true, .. } =
                    operation.machining
                {
                    elements.push(ShopElement::ThroughHole);
                }
                if let Some(thread) = &operation.thread {
                    elements.push(ShopElement::Thread(thread.label()));
                }
            }
            if let Some((joint, _)) = pin_holes
                .iter()
                .find(|(_, hole)| row.instances.contains(&hole.instance_path))
            {
                elements.push(ShopElement::PinHole(joint.name.clone()));
            }
            for element in elements {
                match carriage(format, &element) {
                    Carriage::Carries => {}
                    Carriage::Loses(reason) => losses.push(ShopLoss {
                        part: part.clone(),
                        element,
                        reason,
                    }),
                    Carriage::Refuses(reason) => {
                        return Err(GeneralFabricationError::CannotCarry {
                            format,
                            part,
                            element,
                            reason,
                        });
                    }
                }
            }
        }
        Ok(losses)
    }
}

/// The pin holes only their pin joint holds, with the joint. A joint whose
/// holes are drilled into its parts adds none: those pockets are the parts'
/// own drilling operations, so every format machines them once.
pub(super) fn undrilled_pin_holes(
    snapshot: &Snapshot,
) -> Result<Vec<(&PinJointContract, PinHole)>, GeneralFabricationError> {
    let mut holes = Vec::new();
    for joint in snapshot
        .pin_joints()
        .filter(|joint| joint.physical_hole_pairs.is_none())
    {
        let projection = project_pin_joint_contract(snapshot, joint)
            .map_err(GeneralFabricationError::PinJoint)?;
        for pair in projection.pairs {
            holes.push((joint, pair.first));
            holes.push((joint, pair.second));
        }
    }
    Ok(holes)
}

/// Whether a copy of `row` is placed mirrored; a scaled or sheared copy is
/// refused by its own error.
fn copies_are_mirrored(
    snapshot: &Snapshot,
    row: &GeneralBomRow,
) -> Result<bool, GeneralFabricationError> {
    let mut mirrored = false;
    for instance_path in &row.instances {
        let resolved = snapshot
            .resolve_instance_path(instance_path)
            .map_err(GeneralFabricationError::UnresolvedInstance)?;
        if resolved.definition_id != row.definition_id {
            return Err(GeneralFabricationError::ExportBlocked);
        }
        if !is_production_transform(resolved.world_transform) {
            ScaledPlacement::check(instance_path.root_occurrence(), resolved.world_transform)
                .map_err(GeneralFabricationError::ScaledPlacement)?;
            mirrored = true;
        }
    }
    Ok(mirrored)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn woodwop_mills_profiles_only_with_its_pocket_tool_and_cuts_nothing_else() {
        use GeneralManufacturingKind as Kind;
        let carries = |tool, kind| {
            carriage(
                ShopFormat::Woodwop {
                    vertical_pocket_tool: tool,
                },
                &ShopElement::Operation(kind),
            ) == Carriage::Carries
        };
        assert!(carries(false, Kind::CircularDrill) && carries(false, Kind::Stock));
        assert!(!carries(false, Kind::ProfileCut) && carries(true, Kind::ProfileCut));
        assert!(!carries(true, Kind::ThroughCut) && !carries(true, Kind::BooleanCut));
        for kind in [Kind::ThroughCut, Kind::BooleanCut, Kind::ProfileCut] {
            assert_eq!(
                carriage(ShopFormat::Btlx, &ShopElement::Operation(kind)),
                Carriage::Carries
            );
        }
    }

    #[test]
    fn only_the_job_and_the_drawing_carry_a_through_all_hole() {
        for (format, carried) in [
            (ShopFormat::Drawing, true),
            (ShopFormat::ProductionJob, true),
            (ShopFormat::Btlx, false),
            (
                ShopFormat::Woodwop {
                    vertical_pocket_tool: true,
                },
                false,
            ),
        ] {
            let carriage = carriage(format, &ShopElement::ThroughHole);
            assert_eq!(carriage == Carriage::Carries, carried, "{format}");
            assert!(
                carried || carriage == Carriage::Refuses(THROUGH_HOLE),
                "{format}"
            );
        }
    }
}
