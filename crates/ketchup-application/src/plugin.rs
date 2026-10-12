//! A plugin edits the document on the MCP path: the gateway checks its
//! capability, operation allowlist and budgets, and an accepted program is
//! planned and committed by `apply_cad_program` as one Undo step, only while
//! the document is still at the revision the plugin read.

use crate::{DocumentSession, SessionError};
use ketchup_assistant::extension::{
    PluginGateway, PluginGatewayError, PluginGrant, PluginProgram, PluginRequest, PluginResponse,
};
use ketchup_model::document::Snapshot;
use ketchup_scheduler::plugin::{PluginHostError, PluginRunResult, run_plugin_process};
use std::collections::BTreeSet;
use std::ffi::OsString;
use std::fmt;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

impl DocumentSession {
    /// Answers one plugin request against this document. A program the
    /// gateway accepts is committed before the answer goes back.
    pub fn handle_plugin_request(
        &mut self,
        gateway: &mut PluginGateway,
        request: PluginRequest,
    ) -> Result<PluginResponse, PluginApplyError> {
        let response = gateway.handle(&self.snapshot(), request)?;
        if let PluginResponse::Program(program) = &response {
            self.apply_plugin_program(program)?;
        }
        Ok(response)
    }

    /// Runs a plugin process against this document and commits the program
    /// it sent, if any.
    pub fn run_plugin(
        &mut self,
        executable: impl AsRef<Path>,
        arguments: &[OsString],
        grant: PluginGrant,
        timeout: Duration,
        cancelled: &AtomicBool,
    ) -> Result<PluginRunResult, PluginApplyError> {
        let run = run_plugin_process(
            executable,
            arguments,
            &self.snapshot(),
            grant,
            timeout,
            cancelled,
        )?;
        if let Some(program) = &run.program {
            self.apply_plugin_program(program)?;
        }
        Ok(run)
    }

    fn apply_plugin_program(
        &mut self,
        program: &PluginProgram,
    ) -> Result<Snapshot, PluginApplyError> {
        let current_revision = self.snapshot().revision_id();
        if current_revision != program.revision_id {
            return Err(PluginApplyError::Stale {
                read_revision: program.revision_id,
                current_revision,
            });
        }
        Ok(self.apply_cad_program(&program.program, &BTreeSet::new())?)
    }
}

#[derive(Debug)]
pub enum PluginApplyError {
    Gateway(PluginGatewayError),
    Host(PluginHostError),
    /// The document changed after the plugin read it.
    Stale {
        read_revision: u64,
        current_revision: u64,
    },
    Session(SessionError),
}

impl fmt::Display for PluginApplyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Gateway(error) => error.fmt(formatter),
            Self::Host(error) => error.fmt(formatter),
            Self::Stale {
                read_revision,
                current_revision,
            } => write!(
                formatter,
                "plugin read revision {read_revision}, the document is at {current_revision}"
            ),
            Self::Session(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for PluginApplyError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Gateway(error) => Some(error),
            Self::Host(error) => Some(error),
            Self::Stale { .. } => None,
            Self::Session(error) => Some(error),
        }
    }
}

impl From<PluginGatewayError> for PluginApplyError {
    fn from(error: PluginGatewayError) -> Self {
        Self::Gateway(error)
    }
}

impl From<PluginHostError> for PluginApplyError {
    fn from(error: PluginHostError) -> Self {
        Self::Host(error)
    }
}

impl From<SessionError> for PluginApplyError {
    fn from(error: SessionError) -> Self {
        Self::Session(error)
    }
}
