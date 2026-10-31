//! Requests that issue or revoke credentials, and what an issuance returns.

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde::Deserialize;

use crate::error::AuthzError;
use crate::family::IdentityFamily;
use crate::identity::{CredentialId, PrincipalId};
use crate::scope::Grant;

/// The closed request document that asks for a new credential.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields, bound = "")]
pub struct IssueRequest<F: IdentityFamily> {
    pub(crate) principal: PrincipalId,
    pub(crate) grants: Vec<Grant<F>>,
    pub(crate) expires_at_ms: TimestampMs,
}

impl<F: IdentityFamily> IssueRequest<F> {
    /// Asks for a credential of `principal` holding `grants` until `expires_at`.
    #[must_use]
    pub const fn new(
        principal: PrincipalId,
        grants: Vec<Grant<F>>,
        expires_at: TimestampMs,
    ) -> Self {
        Self {
            principal,
            grants,
            expires_at_ms: expires_at,
        }
    }

    /// Parses the request document, rejecting unknown fields, unknown permissions or scope
    /// kinds and numeric counters.
    ///
    /// # Errors
    ///
    /// Returns [`AuthzError::InvalidParams`] naming the offending field.
    pub fn from_json(text: &str) -> Result<Self, AuthzError> {
        let mut decoder = serde_json::Deserializer::from_str(text);
        let request = serde_path_to_error::deserialize(&mut decoder).map_err(|error| {
            AuthzError::InvalidParams {
                field_path: error.path().to_string(),
                source: error.into_inner(),
            }
        })?;
        decoder.end().map_err(|source| AuthzError::InvalidParams {
            field_path: String::new(),
            source,
        })?;
        Ok(request)
    }

    /// When the requested credential stops being valid.
    #[must_use]
    pub const fn expires_at(&self) -> TimestampMs {
        self.expires_at_ms
    }
}

/// An issuance request evaluated at one instant.
#[derive(Debug, Clone)]
pub struct Issuance<F: IdentityFamily> {
    pub(crate) request: IssueRequest<F>,
    pub(crate) at: TimestampMs,
}

impl<F: IdentityFamily> Issuance<F> {
    /// Evaluates `request` at `at`.
    #[must_use]
    pub const fn new(request: IssueRequest<F>, at: TimestampMs) -> Self {
        Self { request, at }
    }
}

/// A revocation of one credential evaluated at one instant.
#[derive(Debug, Clone)]
pub struct Revocation {
    pub(crate) credential: CredentialId,
    pub(crate) at: TimestampMs,
}

impl Revocation {
    /// Revokes `credential` at `at`.
    #[must_use]
    pub const fn new(credential: CredentialId, at: TimestampMs) -> Self {
        Self { credential, at }
    }
}
