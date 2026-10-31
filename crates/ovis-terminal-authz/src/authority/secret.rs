//! Credential secrets and the `ovt1.<credential>.<hex secret>` bearer text that carries them.

use std::fmt;

use crate::error::AuthzError;
use crate::identity::CredentialId;

const BEARER_PREFIX: &str = "ovt1.";
const SECRET_BYTES: usize = 32;

/// 32 bytes drawn from two operating-system random UUIDs (244 random bits).
pub(super) struct CredentialSecret([u8; SECRET_BYTES]);

impl CredentialSecret {
    pub(super) fn generate() -> Self {
        let (high, low) = (uuid::Uuid::new_v4(), uuid::Uuid::new_v4());
        let mut bytes = [0_u8; SECRET_BYTES];
        for (slot, byte) in bytes
            .iter_mut()
            .zip(high.as_bytes().iter().chain(low.as_bytes()))
        {
            *slot = *byte;
        }
        Self(bytes)
    }

    /// Compares in time independent of where the first differing byte is.
    pub(super) fn matches(&self, presented: &Self) -> bool {
        let difference = self
            .0
            .iter()
            .zip(&presented.0)
            .fold(0_u8, |acc, (left, right)| acc | (left ^ right));
        difference == 0
    }

    pub(super) fn bearer(&self, credential: CredentialId) -> String {
        format!("{BEARER_PREFIX}{credential}.{}", LowerHex(&self.0))
    }
}

impl fmt::Debug for CredentialSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("CredentialSecret(<redacted>)")
    }
}

struct LowerHex<'a>(&'a [u8]);

impl fmt::Display for LowerHex<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

#[derive(Debug)]
pub(super) struct PresentedBearer {
    pub(super) credential: CredentialId,
    pub(super) secret: CredentialSecret,
}

impl PresentedBearer {
    pub(super) fn parse(text: &str) -> Result<Self, AuthzError> {
        if text.is_empty() {
            return Err(AuthzError::Unauthenticated);
        }
        let body = text
            .strip_prefix(BEARER_PREFIX)
            .ok_or(AuthzError::InvalidCredential)?;
        let (credential, hex) = body.split_once('.').ok_or(AuthzError::InvalidCredential)?;
        Ok(Self {
            credential: CredentialId::parse(credential)?,
            secret: decode_secret(hex)?,
        })
    }
}

fn decode_secret(hex: &str) -> Result<CredentialSecret, AuthzError> {
    let (pairs, rest) = hex.as_bytes().as_chunks::<2>();
    if pairs.len() != SECRET_BYTES || !rest.is_empty() {
        return Err(AuthzError::InvalidCredential);
    }
    let mut bytes = [0_u8; SECRET_BYTES];
    for (slot, [high, low]) in bytes.iter_mut().zip(pairs) {
        *slot = (nibble(*high)? << 4) | nibble(*low)?;
    }
    Ok(CredentialSecret(bytes))
}

fn nibble(digit: u8) -> Result<u8, AuthzError> {
    let lowercase_hex = digit.is_ascii_digit() || matches!(digit, b'a'..=b'f');
    char::from(digit)
        .to_digit(16)
        .filter(|_| lowercase_hex)
        .and_then(|value| u8::try_from(value).ok())
        .ok_or(AuthzError::InvalidCredential)
}

#[cfg(test)]
mod tests {
    use super::{CredentialSecret, PresentedBearer};
    use crate::identity::CredentialId;

    #[test]
    fn bearer_text_round_trips_the_credential_and_secret() -> Result<(), crate::AuthzError> {
        let secret = CredentialSecret::generate();
        let text = secret.bearer(CredentialId::new(7));

        let presented = PresentedBearer::parse(&text)?;

        assert_eq!(presented.credential, CredentialId::new(7));
        assert!(secret.matches(&presented.secret));
        assert!(!CredentialSecret::generate().matches(&presented.secret));
        assert_eq!(format!("{secret:?}"), "CredentialSecret(<redacted>)");
        Ok(())
    }
}
