//! What each FEA refusal tells the user, with what to change: the window and
//! the AI show this text, not the error's Rust name (review 2026-11-20 ac P3).

use std::fmt;

use crate::fea::{
    FeaCouplingProblem, FeaError, MAX_FEA_CONSTRAINTS, MAX_FEA_COUPLINGS, MAX_FEA_ELEMENTS,
    MAX_FEA_LOADS, MAX_FEA_MATERIALS, MAX_FEA_MODES, MAX_FEA_NODES,
};

impl fmt::Display for FeaError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema => formatter.write_str("the FEA model has an unknown schema"),
            Self::EmptyCaseId => formatter.write_str("the FEA case has no name"),
            Self::EmptyMesh => formatter.write_str("the mesh has no nodes or no elements"),
            Self::ModelTooLarge => write!(
                formatter,
                "the model is larger than a study allows: at most {MAX_FEA_NODES} nodes (or the \
                 study's own lower limit), {MAX_FEA_ELEMENTS} elements, {MAX_FEA_MATERIALS} \
                 materials, {MAX_FEA_CONSTRAINTS} constraints, {MAX_FEA_LOADS} loads and \
                 {MAX_FEA_COUPLINGS} couplings; mesh it coarser or study a smaller part"
            ),
            Self::InvalidNode => formatter.write_str("a mesh node is not a finite point"),
            Self::DuplicateMaterialId => formatter.write_str("two materials have the same id"),
            Self::InvalidMaterial => formatter.write_str(
                "a material needs a positive Young's modulus, a Poisson's ratio from -1 to under 0.5, \
                 and a positive yield strength and density where it gives them",
            ),
            Self::DuplicateElementId { element } => write!(formatter, "element {element} is listed twice"),
            Self::DuplicateElement { element } => {
                write!(formatter, "element {element} repeats an element on the same nodes")
            }
            Self::InvalidElementNode { element } => {
                write!(formatter, "element {element} names a node the mesh does not have")
            }
            Self::RepeatedElementNode { element } => {
                write!(formatter, "element {element} names one node twice")
            }
            Self::DegenerateElement { element } => write!(
                formatter,
                "element {element} is flat or turned inside out; remesh the part"
            ),
            Self::MissingMaterial { element } => {
                write!(formatter, "element {element} names a material the model does not have")
            }
            Self::InvalidArea { element } => {
                write!(formatter, "element {element} needs a positive cross-section area")
            }
            Self::InvalidFrameSection { element } => write!(
                formatter,
                "frame element {element} needs a positive area, second moments and torsion values"
            ),
            Self::InvalidFrameOrientation { element } => write!(
                formatter,
                "frame element {element} needs an orientation vector that does not run along it"
            ),
            Self::MissingConstraint => {
                formatter.write_str("nothing holds the model; fix at least one face in place")
            }
            Self::InvalidConstraint => formatter.write_str("a constraint names a node the mesh does not have"),
            Self::ConflictingConstraint => {
                formatter.write_str("one node is held twice with different values")
            }
            Self::InvalidCoupling { coupling, problem } => {
                write!(formatter, "coupling {coupling}: {problem}")
            }
            Self::MissingLoad => formatter.write_str("no load acts on the model; add a force or a pressure"),
            Self::InvalidLoad => formatter.write_str(
                "a load is not a finite number, or names a node, face or element the model does not have",
            ),
            Self::NonBoundaryTraction => formatter.write_str("a pressure is put on a face inside the mesh"),
            Self::InvalidSettings => formatter.write_str("the solve settings are out of range"),
            // Not only a missing support: a valid model whose stiffnesses differ by
            // too many orders of magnitude cannot be factored either.
            Self::SingularSystem => formatter.write_str(
                "the stiffness cannot be solved: either a part can still move or turn freely \
                 (add a support or a joint to it), or the model is too badly conditioned, with \
                 stiffnesses many orders of magnitude apart (a tiny element between large ones, \
                 a very soft material beside steel, a very long thin part)",
            ),
            Self::FactorTooLarge { needed_bytes, budget_bytes } => write!(
                formatter,
                "the study would need at least {} MiB at once, more than the {} MiB it may use; \
                 mesh it coarser or study a smaller part",
                needed_bytes.div_ceil(1 << 20),
                budget_bytes / (1 << 20)
            ),
            Self::NonFiniteSolution => formatter.write_str(
                "the solve gave numbers too large to be real; check the loads and the material values",
            ),
            Self::MissingDensity { material } => write!(
                formatter,
                "natural frequencies need the density of material {material}"
            ),
            Self::InvalidModeCount => write!(
                formatter,
                "ask for 1 to {MAX_FEA_MODES} modes, no more than the model has free unknowns with mass"
            ),
            Self::ModesNotConverged => formatter.write_str(
                "the natural frequencies did not settle; ask for fewer modes or mesh it finer",
            ),
            Self::Cancelled => formatter.write_str("the study was cancelled"),
        }
    }
}

impl std::error::Error for FeaError {}

impl fmt::Display for FeaCouplingProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidNode => "a node is outside the mesh, or both nodes are the same",
            Self::InvalidStiffness => {
                "a stiffness is not finite or negative, or every stiffness is zero"
            }
            Self::SpringNodesApart => "a spring joins nodes at different places",
            Self::TurnSpringNeedsTurningNodes => "a turn stiffness joins a node that does not turn",
            Self::FollowsTwice => "a node already follows another rigid coupling",
            Self::Cycle => "rigid couplings follow each other round in a circle",
            Self::ConstrainedFollower => {
                "a constraint holds the node that follows; hold the node it follows"
            }
            Self::InvalidTie => "a tie's weights do not place a point in its triangle",
            Self::TieChain => "a node a tie follows follows another coupling itself",
        })
    }
}
