//! Keyed proofs that a peer holds a credential's secret, bound to one message.
//!
//! A runtime attests a statement (for example its boot identity and a caller's fresh
//! challenge) with the bearer text of the credential its session authenticated with; the
//! caller, who holds the same bearer, recomputes the proof. Nobody without the secret can
//! produce it, and a proof for one message never verifies another.

use std::fmt;

use ovis_terminal_protocol::schema::scalar::TimestampMs;
use sha2::{Digest as _, Sha256};

use super::{Authority, Session};
use crate::error::AuthzError;
use crate::family::IdentityFamily;

const BLOCK_BYTES: usize = 64;
const DIGEST_BYTES: usize = 32;

/// HMAC-SHA256 (RFC 2104) of `message` under `key`.
fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; DIGEST_BYTES] {
    let hashed;
    let key = if key.len() > BLOCK_BYTES {
        hashed = Sha256::digest(key);
        hashed.as_slice()
    } else {
        key
    };
    let mut block = [0_u8; BLOCK_BYTES];
    for (slot, byte) in block.iter_mut().zip(key) {
        *slot = *byte;
    }
    let inner = Sha256::new()
        .chain_update(block.map(|byte| byte ^ 0x36))
        .chain_update(message)
        .finalize();
    Sha256::new()
        .chain_update(block.map(|byte| byte ^ 0x5c))
        .chain_update(inner)
        .finalize()
        .into()
}

/// A proof of credential possession over one message.
#[derive(Clone, PartialEq, Eq)]
pub struct Attestation([u8; DIGEST_BYTES]);

impl Attestation {
    /// The proof a holder of `bearer` computes over `message`.
    #[must_use]
    pub fn of_bearer(bearer: &str, message: &[u8]) -> Self {
        Self(hmac_sha256(bearer.as_bytes(), message))
    }

    /// Wraps proof bytes received from a peer.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// The proof bytes.
    #[must_use]
    pub const fn to_bytes(&self) -> [u8; DIGEST_BYTES] {
        self.0
    }

    /// Compares in time independent of where the first differing byte is.
    #[must_use]
    pub fn matches(&self, other: &Self) -> bool {
        self.0
            .iter()
            .zip(&other.0)
            .fold(0_u8, |acc, (left, right)| acc | (left ^ right))
            == 0
    }
}

impl fmt::Debug for Attestation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Attestation(..)")
    }
}

/// A message to attest at one instant.
#[derive(Debug, Clone, Copy)]
pub struct AttestRequest<'a> {
    message: &'a [u8],
    at: TimestampMs,
}

impl<'a> AttestRequest<'a> {
    /// Attests `message` at `at`.
    #[must_use]
    pub const fn new(message: &'a [u8], at: TimestampMs) -> Self {
        Self { message, at }
    }
}

impl<F: IdentityFamily> Authority<F> {
    /// Attests the request's message with the credential `session` authenticated with, if
    /// that credential is still valid at the request's instant.
    ///
    /// # Errors
    ///
    /// [`AuthzError::InvalidCredential`] for a session of another authority, and the
    /// revocation and expiry errors of [`Authority::recheck`] for a lapsed credential.
    pub fn attest(
        &self,
        session: &Session,
        request: AttestRequest<'_>,
    ) -> Result<Attestation, AuthzError> {
        self.own(session)?;
        let table = self.read()?;
        let record = table.check_claim(&session.claim, request.at)?;
        let bearer = record.bearer_text(session.claim.credential);
        drop(table);
        Ok(Attestation::of_bearer(&bearer, request.message))
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{Attestation, hmac_sha256};

    fn hex(bytes: [u8; 32]) -> String {
        bytes.iter().fold(String::new(), |mut text, byte| {
            write!(text, "{byte:02x}").expect("writing to a String succeeds");
            text
        })
    }

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        assert_eq!(
            hex(hmac_sha256(b"Jefe", b"what do ya want for nothing?")),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        assert_eq!(
            hex(hmac_sha256(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn a_proof_verifies_only_its_own_key_and_message() {
        let proof = Attestation::of_bearer("ovt1.1.aa", b"statement");

        assert!(proof.matches(&Attestation::of_bearer("ovt1.1.aa", b"statement")));
        assert!(!proof.matches(&Attestation::of_bearer("ovt1.1.ab", b"statement")));
        assert!(!proof.matches(&Attestation::of_bearer("ovt1.1.aa", b"statement2")));
        assert!(proof.matches(&Attestation::from_bytes(proof.to_bytes())));
        assert_eq!(format!("{proof:?}"), "Attestation(..)");
    }
}
