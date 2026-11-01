//! The typed control-restart interface an activation delegates to.
//!
//! An update never signals, kills or replaces a program: it selects a build and asks
//! control to restart onto it, and the holders keep their programs and drain by build.

use std::fmt::Debug;

use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::server::ServerTransitionResult;
use ovis_terminal_protocol::schema::scalar::Incarnation;

/// Restartable control of the runtime the update service belongs to.
pub trait RestartControl: Debug + Send + Sync {
    /// Restarts control as `op_id` while every held program keeps running.
    ///
    /// # Errors
    ///
    /// The control's own typed failure.
    fn restart(&self, op_id: &OpId) -> Result<ServerTransitionResult, RestartFailure>;

    /// Program incarnations the holders own now.
    ///
    /// # Errors
    ///
    /// The control's own typed failure.
    fn live_incarnations(&self) -> Result<Vec<Incarnation>, RestartFailure>;
}

/// A control failure, carried with the control's own kind and hint.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum RestartFailure {
    /// The control refused or failed the request.
    #[error("the control refused or failed the request")]
    Control {
        /// The control error's stable machine token.
        kind: &'static str,
        /// The control error's recovery guidance.
        hint: &'static str,
        /// The control error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync>,
    },
}

impl RestartFailure {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Control { kind, .. } => kind,
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Control { hint, .. } => hint,
        }
    }
}
