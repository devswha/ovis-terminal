//! Principal, credential and authority identities.

use std::fmt;

use ovis_terminal_protocol::rpc::DecimalU64;
use serde::{Deserialize, Serialize};

use crate::error::AuthzError;

const MAX_PRINCIPAL_BYTES: usize = 128;

/// Who acts: an account, client or program named by the credential it presents.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PrincipalId(String);

impl PrincipalId {
    /// Parses a principal name of 1 to 128 bytes without control characters.
    ///
    /// # Errors
    ///
    /// Returns [`AuthzError::InvalidValue`] for an empty, oversized or control-bearing name.
    pub fn parse(name: &str) -> Result<Self, AuthzError> {
        let printable = !name.chars().any(char::is_control);
        if name.is_empty() || name.len() > MAX_PRINCIPAL_BYTES || !printable {
            return Err(AuthzError::InvalidValue {
                field_path: "principal",
            });
        }
        Ok(Self(name.to_owned()))
    }

    /// Borrows the principal name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PrincipalId {
    type Error = AuthzError;

    fn try_from(name: String) -> Result<Self, Self::Error> {
        Self::parse(&name)
    }
}

impl From<PrincipalId> for String {
    fn from(principal: PrincipalId) -> Self {
        principal.0
    }
}

impl fmt::Display for PrincipalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Identity of one issued credential, minted by its authority as `c<n>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct CredentialId(u64);

impl CredentialId {
    pub(super) const fn new(sequence: u64) -> Self {
        Self(sequence)
    }

    /// Parses the `c<n>` text form.
    ///
    /// # Errors
    ///
    /// Returns [`AuthzError::InvalidCredential`] when the text is not a canonical `c<n>`.
    pub fn parse(text: &str) -> Result<Self, AuthzError> {
        let digits = text
            .strip_prefix('c')
            .ok_or(AuthzError::InvalidCredential)?;
        let canonical = digits.len() == 1 || !digits.starts_with('0');
        if !canonical || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(AuthzError::InvalidCredential);
        }
        digits
            .parse()
            .map(Self)
            .map_err(|_| AuthzError::InvalidCredential)
    }
}

impl fmt::Display for CredentialId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "c{}", self.0)
    }
}

impl Serialize for CredentialId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

/// Epoch of a credential; every revocation advances it, so decisions made under an
/// older epoch fail their commit-time recheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CredentialEpoch(DecimalU64);

impl CredentialEpoch {
    pub(super) const FIRST: Self = Self(DecimalU64::new(1));

    /// Returns the native epoch value.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }

    pub(super) const fn next(self) -> Self {
        Self(DecimalU64::new(self.0.get().saturating_add(1)))
    }
}

#[cfg(test)]
mod tests {
    use super::{CredentialEpoch, CredentialId, PrincipalId};

    #[test]
    fn credential_id_text_round_trips_and_rejects_non_canonical_forms() {
        let parsed = CredentialId::parse(&CredentialId::new(u64::MAX).to_string());
        let rejected = ["", "c", "c01", "x1", "c1a", "c18446744073709551616", "c+1"]
            .map(|text| CredentialId::parse(text).err().map(|error| error.kind()));

        assert_eq!(parsed.ok(), Some(CredentialId::new(u64::MAX)));
        assert_eq!(rejected, [Some("invalid_credential"); 7]);
    }

    #[test]
    fn epoch_is_a_decimal_string_and_saturates_at_the_top() -> Result<(), serde_json::Error> {
        let top = CredentialEpoch(ovis_terminal_protocol::rpc::DecimalU64::new(u64::MAX));

        let text = serde_json::to_string(&top)?;
        let back: CredentialEpoch = serde_json::from_str(&text)?;

        assert_eq!(text, "\"18446744073709551615\"");
        assert_eq!(back, top);
        assert_eq!(top.next(), top);
        assert!(serde_json::from_str::<CredentialEpoch>("1").is_err());
        Ok(())
    }

    #[test]
    fn principal_rejects_empty_oversized_and_control_names() {
        let oversized = "p".repeat(129);
        let kinds = ["", oversized.as_str(), "a\nb"]
            .map(|name| PrincipalId::parse(name).err().map(|error| error.kind()));

        assert_eq!(kinds, [Some("invalid_params"); 3]);
        assert_eq!(
            PrincipalId::parse("p".repeat(128).as_str())
                .map(|id| id.as_str().len())
                .ok(),
            Some(128)
        );
    }
}
