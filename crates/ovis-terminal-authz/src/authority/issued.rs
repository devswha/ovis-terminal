//! What an issuance returns to the issuer.

use std::fmt;

use ovis_terminal_protocol::schema::ids::RuntimeLifetimeId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde::{Serialize, Serializer};

use crate::identity::{CredentialEpoch, CredentialId, PrincipalId};
use crate::scope::Grant;

/// The bearer text, kept out of `Debug` output.
#[derive(Clone)]
pub(super) struct Bearer(pub(super) String);

impl fmt::Debug for Bearer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

impl Serialize for Bearer {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// A newly issued credential; the bearer text is shown to the issuer exactly here.
#[derive(Debug, Clone, Serialize)]
pub struct IssuedCredential {
    pub(super) credential: CredentialId,
    pub(super) principal: PrincipalId,
    pub(super) runtime_lifetime: RuntimeLifetimeId,
    pub(super) epoch: CredentialEpoch,
    pub(super) expires_at_ms: TimestampMs,
    pub(super) grants: Vec<Grant>,
    pub(super) bearer: Bearer,
}

impl IssuedCredential {
    /// The credential identity used for revocation.
    #[must_use]
    pub const fn id(&self) -> &CredentialId {
        &self.credential
    }

    /// The epoch the credential was issued at.
    #[must_use]
    pub const fn epoch(&self) -> CredentialEpoch {
        self.epoch
    }

    /// The text a client presents in `hello` to authenticate.
    #[must_use]
    pub fn bearer(&self) -> &str {
        &self.bearer.0
    }
}
