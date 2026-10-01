//! What stops a shared-definition change, an occurrence fork or a component
//! replacement: either while its impact is projected or while it refreshes the
//! assembly mates, drawings and exports that depend on it. Each case names the
//! part it is about by ID, so a caller can point at it.

use crate::assembly::{
    AssemblyMateId, AssemblyRecomputeError, AssemblyRecomputeStatus, AssemblySolveStatus,
};
use crate::document::{
    BodyId, CanonicalError, DefinitionId, FeatureId, InstancePath, OccurrenceId,
    ProposalPrepareError,
};
use crate::drawing::{DrawingError, DrawingSheetId};
use crate::exact_product::{
    ExactProductError, ExactReferenceQuarantineReason, ExactReferenceResolution,
};
use crate::feature_history::{BodyHistoryMutationError, BodyParameterEditError};
use std::fmt;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyBlocker {
    /// The requested parameter edit could not be prepared.
    ParameterEdit(BodyParameterEditError),
    /// The requested history mutation could not be prepared.
    HistoryMutation(BodyHistoryMutationError),
    /// Only existing exact edits and bounded history mutations can fork.
    ChangeKindNotForkable,
    /// A definition the change reads was not found.
    DefinitionMissing(DefinitionId),
    /// Definitions that instance other definitions cannot be replaced.
    NestedInstances,
    /// A body of a definition is hidden or consumed by a later feature.
    BodyHiddenOrConsumed {
        definition: DefinitionId,
        body: BodyId,
    },
    /// A feature the change reads was not found.
    FeatureMissing(FeatureId),
    /// A feature has no record in the dependency graph.
    FeatureDependenciesMissing(FeatureId),
    /// A feature lies outside the selected definition.
    FeatureOutsideDefinition(FeatureId),
    /// A forked feature has no source feature it was copied from.
    ForkFeatureWithoutLineage(FeatureId),
    /// No unused ID is left for a new definition, body or feature.
    IdentitySpaceExhausted,
    /// Recomputing the affected rigid assembly mates failed.
    AssemblyRecompute(AssemblyRecomputeError),
    /// The recomputed rigid assembly does not solve.
    AssemblyNotSolved(AssemblyRecomputeStatus),
    /// The recompute finished without a solve.
    NoSolveResult,
    /// The local rigid solve leaves the assembly under- or over-constrained.
    SolveNotFullyConstrained {
        recompute: AssemblyRecomputeStatus,
        solve: AssemblySolveStatus,
    },
    /// The local rigid solve would move the selected occurrence.
    SolveMovesSelected,
    /// The local rigid solve would move an unchanged occurrence of the source
    /// or target definition.
    SolveMovesSibling,
    /// The refreshed proposal could not be prepared.
    Proposal(ProposalPrepareError),
    /// Applying the staged commands to the document failed.
    Staging(CanonicalError),
    /// The staged candidate changed something the change must keep.
    CandidateChanged(CandidateDrift),
    /// The fork did not keep world placement, collections and drawing sources.
    PlacementNotPreserved,
    /// The change added or removed assembly mates.
    MateSetChanged,
    /// An assembly mate could not follow the change.
    Mate {
        mate: AssemblyMateId,
        problem: MateProblem,
    },
    /// A drawing sheet could not follow the change.
    Sheet {
        sheet: DrawingSheetId,
        problem: SheetProblem,
    },
    /// The export impact reaches beyond the selected occurrence.
    ExportOutsideSelection,
    /// An exported occurrence could not follow the change.
    Export {
        path: InstancePath,
        problem: ExportProblem,
    },
    /// An unchanged exact result could not be carried to the new revision.
    ResultsNotCarried,
    /// Reading or exporting an exact result failed.
    Exact(ExactProductError),
}

/// What a staged candidate changed although the change must keep it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CandidateDrift {
    /// Identity, world placement, collections or drawing sources of the
    /// selected occurrence.
    SelectedOccurrence,
    /// An occurrence the change does not touch.
    Occurrence(OccurrenceId),
    /// A definition the change does not touch.
    Definition(DefinitionId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MateProblem {
    Missing,
    /// The proposal names a mate the source document does not have.
    Introduced,
    /// The mate does not depend on the change, yet it changed.
    UnrelatedChanged,
    KindChanged,
    OtherEndpointChanged,
    /// Only planar and axial mates can follow a replaced component.
    NotPlanarOrAxial,
    /// The mate does not reference the definition being replaced.
    NotSourceDefinition,
    /// The replacement has no subshape the mate reference could move to.
    NoTargetSubshape,
    BrokenReference,
    Quarantined(ExactReferenceQuarantineReason),
    ForkLineageLost,
    NotResolvedToFork,
    NotRebound,
    LineageChanged,
    RoleLost {
        role: String,
    },
    RoleAmbiguous {
        role: String,
    },
    OccurrenceLost(OccurrenceId),
    NotUniquelyCurrent(ExactReferenceResolution),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SheetProblem {
    Missing,
    Projection(DrawingError),
    NotRefreshed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportProblem {
    NotVisible,
    NoExactBody,
    NotForkDefinition,
}

impl fmt::Display for DependencyBlocker {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ParameterEdit(error) => error.fmt(formatter),
            Self::HistoryMutation(error) => error.fmt(formatter),
            Self::ChangeKindNotForkable => formatter.write_str(
                "occurrence fork supports only existing exact edits and bounded history mutation",
            ),
            Self::DefinitionMissing(id) => write!(formatter, "definition {} was not found", id.0),
            Self::NestedInstances => formatter
                .write_str("component replacement does not support nested definition instances"),
            Self::BodyHiddenOrConsumed { definition, body } => write!(
                formatter,
                "definition {} body {} is hidden or consumed",
                definition.0, body.0
            ),
            Self::FeatureMissing(id) => write!(formatter, "feature {} was not found", id.0),
            Self::FeatureDependenciesMissing(id) => {
                write!(formatter, "feature {} has no dependency record", id.0)
            }
            Self::FeatureOutsideDefinition(id) => {
                write!(
                    formatter,
                    "feature {} is outside the selected definition",
                    id.0
                )
            }
            Self::ForkFeatureWithoutLineage(id) => {
                write!(formatter, "fork feature {} has no source lineage", id.0)
            }
            Self::IdentitySpaceExhausted => formatter.write_str("identity space is exhausted"),
            Self::AssemblyRecompute(error) => {
                write!(formatter, "rigid assembly recompute failed: {error}")
            }
            Self::AssemblyNotSolved(status) => {
                write!(formatter, "rigid assembly is not publishable: {status:?}")
            }
            Self::NoSolveResult => formatter.write_str("local rigid solve produced no result"),
            Self::SolveNotFullyConstrained { recompute, solve } => write!(
                formatter,
                "local rigid solve is not fully constrained: {recompute:?}/{solve:?}"
            ),
            Self::SolveMovesSelected => formatter
                .write_str("local rigid solve would move the selected occurrence world placement"),
            Self::SolveMovesSibling => {
                formatter.write_str("local rigid solve would move an unchanged occurrence")
            }
            Self::Proposal(error) => error.fmt(formatter),
            Self::Staging(error) => error.fmt(formatter),
            Self::CandidateChanged(drift) => write!(formatter, "candidate changed {drift}"),
            Self::PlacementNotPreserved => formatter
                .write_str("world placement, collections, and drawing sources were not preserved"),
            Self::MateSetChanged => formatter.write_str("the assembly mate set changed"),
            Self::Mate { mate, problem } => write!(formatter, "assembly mate {} {problem}", mate.0),
            Self::Sheet { sheet, problem } => {
                write!(formatter, "drawing sheet {} {problem}", sheet.0)
            }
            Self::ExportOutsideSelection => {
                formatter.write_str("export impact includes a non-selected branch")
            }
            Self::Export { path, problem } => {
                write!(formatter, "export occurrence path {path:?} {problem}")
            }
            Self::ResultsNotCarried => formatter
                .write_str("an unchanged exact result could not be carried to the new revision"),
            Self::Exact(error) => error.fmt(formatter),
        }
    }
}

impl fmt::Display for CandidateDrift {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SelectedOccurrence => formatter.write_str(
                "identity, world placement, or preserved dependencies of the selected occurrence",
            ),
            Self::Occurrence(id) => write!(formatter, "unrelated occurrence {}", id.0),
            Self::Definition(id) => write!(formatter, "definition {}", id.0),
        }
    }
}

impl fmt::Display for MateProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("disappeared during dependency staging"),
            Self::Introduced => formatter.write_str("was introduced by the proposal"),
            Self::UnrelatedChanged => {
                formatter.write_str("changed although it does not depend on the change")
            }
            Self::KindChanged => formatter.write_str("changed kind"),
            Self::OtherEndpointChanged => formatter.write_str("changed its non-selected endpoint"),
            Self::NotPlanarOrAxial => formatter.write_str("is not planar or axial"),
            Self::NotSourceDefinition => {
                formatter.write_str("does not reference the selected source definition")
            }
            Self::NoTargetSubshape => formatter.write_str("has no target subshape correspondence"),
            Self::BrokenReference => formatter.write_str("has a broken exact reference"),
            Self::Quarantined(reason) => {
                write!(formatter, "exact reference is quarantined: {reason:?}")
            }
            Self::ForkLineageLost => formatter.write_str("lost selected fork lineage"),
            Self::NotResolvedToFork => formatter.write_str("did not resolve to the selected fork"),
            Self::NotRebound => formatter.write_str("did not rebind to a resolved reference"),
            Self::LineageChanged => formatter.write_str("changed stable subshape lineage"),
            Self::RoleLost { role } => {
                write!(
                    formatter,
                    "lost semantic role {role} during producer transition"
                )
            }
            Self::RoleAmbiguous { role } => write!(
                formatter,
                "has an ambiguous semantic role {role} during producer transition"
            ),
            Self::OccurrenceLost(occurrence) => {
                write!(formatter, "lost occurrence {}", occurrence.0)
            }
            Self::NotUniquelyCurrent(resolution) => {
                write!(
                    formatter,
                    "reference is not uniquely current: {resolution:?}"
                )
            }
        }
    }
}

impl fmt::Display for SheetProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => formatter.write_str("disappeared during dependency staging"),
            Self::Projection(error) => write!(formatter, "could not be projected: {error}"),
            Self::NotRefreshed => {
                formatter.write_str("did not refresh from current exact evidence")
            }
        }
    }
}

impl fmt::Display for ExportProblem {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotVisible => "is not visible and current",
            Self::NoExactBody => "has no current exact body",
            Self::NotForkDefinition => "does not resolve to the selected fork definition",
        })
    }
}

impl std::error::Error for DependencyBlocker {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ParameterEdit(error) => Some(error),
            Self::HistoryMutation(error) => Some(error),
            Self::AssemblyRecompute(error) => Some(error),
            Self::Proposal(error) => Some(error),
            Self::Staging(error) => Some(error),
            Self::Sheet {
                problem: SheetProblem::Projection(error),
                ..
            } => Some(error),
            Self::Exact(error) => Some(error),
            _ => None,
        }
    }
}
