//! The raw-input grant: raw bytes are admitted only under the authority's decision.

use ovis_terminal_authz::{
    AccessRequest, Action, Authority, CommitCheck, Decision, ResolvedTarget, Session,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use super::error::EncodingError;
use crate::registry::RegistryIdentities;

type Registry = RegistryIdentities;

/// The terminal and instant a raw-input grant is asked for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawRequest {
    target: ResolvedTarget<Registry>,
    at: TimestampMs,
}

impl RawRequest {
    /// Asks for raw input on `target` at `at`.
    pub const fn new(target: ResolvedTarget<Registry>, at: TimestampMs) -> Self {
        Self { target, at }
    }
}

/// Proof that the authority granted raw input on one terminal; it cannot be forged or copied.
#[derive(Debug)]
pub struct RawGrant {
    decision: Decision<Registry>,
}

impl RawGrant {
    /// Asks `authority` whether `session` may send raw bytes as `request` describes.
    ///
    /// # Errors
    ///
    /// Returns `raw_not_authorized` when no grant covers raw input on the terminal.
    pub fn decide(
        authority: &Authority<Registry>,
        session: &Session,
        request: RawRequest,
    ) -> Result<Self, EncodingError> {
        let access = AccessRequest::new(Action::RawInput, request.target, request.at);
        authority
            .authorize(session, &access)
            .map(|decision| Self { decision })
            .map_err(EncodingError::RawNotAuthorized)
    }

    /// Rechecks the grant at the write boundary, so a credential withdrawn after admission
    /// fences raw bytes that are still queued.
    ///
    /// # Errors
    ///
    /// Returns `raw_not_authorized` when the grant lapsed or belongs to another authority.
    pub fn recheck(
        &self,
        authority: &Authority<Registry>,
        at: TimestampMs,
    ) -> Result<(), EncodingError> {
        let commit = CommitCheck::new(Action::RawInput, self.decision.target().clone(), at);
        authority
            .recheck(&self.decision, &commit)
            .map_err(EncodingError::RawNotAuthorized)
    }
}
