use std::fmt;

use crate::document::{
    COMMAND_SCHEMA_V1, CanonicalCommand, CanonicalError, CommandBatch, Dimension, NodeId,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UiAction {
    SetDimension { target: NodeId, value_text: String },
    RenameNode { target: NodeId, name: String },
}

pub struct UiAdapter;

impl UiAdapter {
    pub fn canonicalize(action: UiAction) -> Result<CommandBatch, AdapterError> {
        match action {
            UiAction::SetDimension { target, value_text } => {
                set_dimension_batch(target, value_text)
            }
            UiAction::RenameNode { target, name } => {
                Ok(CommandBatch::new(vec![CanonicalCommand::RenameNode {
                    id: target,
                    name,
                }]))
            }
        }
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
            "set_dimension" => set_dimension_batch(NodeId(request.target), request.value),
            "rename_node" => Ok(CommandBatch::new(vec![CanonicalCommand::RenameNode {
                id: NodeId(request.target),
                name: request.value,
            }])),
            method => Err(AdapterError::UnsupportedAction(method.to_owned())),
        }
    }
}

pub struct CliAdapter;

impl CliAdapter {
    pub fn canonicalize(arguments: &[&str]) -> Result<CommandBatch, AdapterError> {
        match arguments {
            ["set-dimension", target, value] => {
                let target = target
                    .parse::<u64>()
                    .map_err(|_| AdapterError::InvalidNodeId)?;
                set_dimension_batch(NodeId(target), (*value).to_owned())
            }
            ["rename-node", target, name] => {
                let target = target
                    .parse::<u64>()
                    .map_err(|_| AdapterError::InvalidNodeId)?;
                Ok(CommandBatch::new(vec![CanonicalCommand::RenameNode {
                    id: NodeId(target),
                    name: (*name).to_owned(),
                }]))
            }
            [action, ..] => Err(AdapterError::UnsupportedAction((*action).to_owned())),
            [] => Err(AdapterError::MissingAction),
        }
    }
}

fn set_dimension_batch(target: NodeId, value_text: String) -> Result<CommandBatch, AdapterError> {
    let dimension = Dimension::from_decimal(value_text)?;
    Ok(CommandBatch::new(vec![CanonicalCommand::SetDimension {
        id: target,
        dimension,
    }]))
}

#[derive(Debug, PartialEq, Eq)]
pub enum AdapterError {
    UnsupportedSchema(String),
    UnsupportedAction(String),
    MissingAction,
    InvalidNodeId,
    InvalidCanonicalValue(CanonicalError),
}

impl fmt::Display for AdapterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema(schema) => write!(formatter, "unsupported schema {schema}"),
            Self::UnsupportedAction(action) => write!(formatter, "unsupported action {action}"),
            Self::MissingAction => formatter.write_str("protocol action is missing"),
            Self::InvalidNodeId => formatter.write_str("node ID is invalid"),
            Self::InvalidCanonicalValue(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for AdapterError {}

impl From<CanonicalError> for AdapterError {
    fn from(error: CanonicalError) -> Self {
        Self::InvalidCanonicalValue(error)
    }
}
