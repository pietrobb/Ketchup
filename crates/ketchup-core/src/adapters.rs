use crate::document::{
    COMMAND_SCHEMA_V1, CanonicalCommand, CanonicalError, CommandBatch, DefinitionId, Dimension,
    FeatureId,
};
use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiAction {
    SetFeatureDimension {
        target: FeatureId,
        value_text: String,
    },
    RenameDefinition {
        target: DefinitionId,
        name: String,
    },
}
pub struct UiAdapter;
impl UiAdapter {
    pub fn canonicalize(action: UiAction) -> Result<CommandBatch, AdapterError> {
        Ok(match action {
            UiAction::SetFeatureDimension { target, value_text } => {
                CommandBatch::new(vec![CanonicalCommand::SetFeatureDimension {
                    id: target,
                    dimension: Dimension::from_decimal(value_text)?,
                }])
            }
            UiAction::RenameDefinition { target, name } => {
                CommandBatch::new(vec![CanonicalCommand::RenameDefinition {
                    id: target,
                    name,
                }])
            }
        })
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RpcRequestV1 {
    pub schema: String,
    pub method: String,
    pub target: u64,
    pub value: String,
}
pub struct RpcAdapter;
impl RpcAdapter {
    pub fn canonicalize(request: RpcRequestV1) -> Result<CommandBatch, AdapterError> {
        if request.schema != COMMAND_SCHEMA_V1 {
            return Err(AdapterError::UnsupportedSchema(request.schema));
        }
        match request.method.as_str() {
            "set_feature_dimension" => UiAdapter::canonicalize(UiAction::SetFeatureDimension {
                target: FeatureId(request.target),
                value_text: request.value,
            }),
            "rename_definition" => UiAdapter::canonicalize(UiAction::RenameDefinition {
                target: DefinitionId(request.target),
                name: request.value,
            }),
            method => Err(AdapterError::UnsupportedAction(method.to_owned())),
        }
    }
}
pub struct CliAdapter;
impl CliAdapter {
    pub fn canonicalize(arguments: &[&str]) -> Result<CommandBatch, AdapterError> {
        match arguments {
            ["set-feature-dimension", target, value] => {
                UiAdapter::canonicalize(UiAction::SetFeatureDimension {
                    target: FeatureId(target.parse().map_err(|_| AdapterError::InvalidEntityId)?),
                    value_text: (*value).to_owned(),
                })
            }
            ["rename-definition", target, name] => {
                UiAdapter::canonicalize(UiAction::RenameDefinition {
                    target: DefinitionId(
                        target.parse().map_err(|_| AdapterError::InvalidEntityId)?,
                    ),
                    name: (*name).to_owned(),
                })
            }
            [action, ..] => Err(AdapterError::UnsupportedAction((*action).to_owned())),
            [] => Err(AdapterError::MissingAction),
        }
    }
}
#[derive(Debug, PartialEq, Eq)]
pub enum AdapterError {
    UnsupportedSchema(String),
    UnsupportedAction(String),
    MissingAction,
    InvalidEntityId,
    InvalidCanonicalValue(CanonicalError),
}
impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(v) => write!(f, "unsupported schema {v}"),
            Self::UnsupportedAction(v) => write!(f, "unsupported action {v}"),
            Self::MissingAction => f.write_str("protocol action is missing"),
            Self::InvalidEntityId => f.write_str("entity ID is invalid"),
            Self::InvalidCanonicalValue(e) => e.fmt(f),
        }
    }
}
impl std::error::Error for AdapterError {}
impl From<CanonicalError> for AdapterError {
    fn from(error: CanonicalError) -> Self {
        Self::InvalidCanonicalValue(error)
    }
}
