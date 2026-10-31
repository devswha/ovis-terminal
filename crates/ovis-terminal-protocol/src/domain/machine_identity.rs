//! Machine identity on the wire: the fresh boot challenge, the proof that answers it and
//! the record of an authenticated machine.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::schema::ids::{MachineId, MachineName, RuntimeLifetimeId, ServerBootId};
use crate::schema::methods::machine::{MachineRecord, MachineState};
use crate::schema::methods::server::HelloResult;
use crate::schema::scalar::{OpenEnum, Revision};

const HEX32_LEN: usize = 64;

/// Text that is not 64 lowercase hexadecimal digits.
#[derive(Debug, Error)]
#[error("expected 64 lowercase hexadecimal digits")]
#[non_exhaustive]
pub struct Hex32Error;

impl Hex32Error {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        "invalid_params"
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        "send 32 bytes as 64 lowercase hexadecimal digits"
    }
}

/// 32 bytes carried as 64 lowercase hexadecimal digits.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
struct Hex32([u8; 32]);

impl Hex32 {
    fn parse(text: &str) -> Result<Self, Hex32Error> {
        let digit = |byte: u8| {
            let lowercase = byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte);
            char::from(byte)
                .to_digit(16)
                .filter(|_| lowercase)
                .and_then(|value| u8::try_from(value).ok())
                .ok_or(Hex32Error)
        };
        let (pairs, rest) = text.as_bytes().as_chunks::<2>();
        if text.len() != HEX32_LEN || !rest.is_empty() {
            return Err(Hex32Error);
        }
        let mut bytes = [0_u8; 32];
        for (slot, [high, low]) in bytes.iter_mut().zip(pairs) {
            *slot = (digit(*high)? << 4) | digit(*low)?;
        }
        Ok(Self(bytes))
    }
}

impl fmt::Display for Hex32 {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// Declares a 32-byte value carried as lowercase hexadecimal text.
macro_rules! hex32_value {
    ($(#[$meta:meta])* $name:ident, $description:literal) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(Hex32);

        impl $name {
            /// Wraps raw bytes.
            #[must_use]
            pub const fn from_bytes(bytes: [u8; 32]) -> Self {
                Self(Hex32(bytes))
            }

            /// The raw bytes.
            #[must_use]
            pub const fn to_bytes(self) -> [u8; 32] {
                self.0.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = Hex32Error;

            fn try_from(text: String) -> Result<Self, Self::Error> {
                Hex32::parse(&text).map(Self)
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0.to_string()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}({})", stringify!($name), self.0)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({
                    "type": "string",
                    "pattern": "^[0-9a-f]{64}$",
                    "description": $description,
                })
            }
        }
    };
}

hex32_value!(
    /// A fresh random value a caller sends in `hello`; the proof must echo it.
    BootChallenge,
    "Fresh random challenge, 32 bytes as lowercase hexadecimal."
);
hex32_value!(
    /// HMAC-SHA256 of a boot statement keyed by the presented credential.
    ProofMac,
    "Keyed proof of credential possession, 32 bytes as lowercase hexadecimal."
);

/// What a runtime vouches for when it answers a boot challenge.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BootStatement {
    /// The caller's challenge, echoed.
    pub challenge: BootChallenge,
    /// Stable machine identity.
    pub machine_id: MachineId,
    /// Holder-supervisor lifetime.
    pub runtime_lifetime: RuntimeLifetimeId,
    /// Control-server boot identity at the moment of the answer.
    pub server_boot_id: ServerBootId,
}

impl BootStatement {
    /// The statement of the runtime that formed `hello`, answering `challenge`.
    #[must_use]
    pub fn answering(challenge: BootChallenge, hello: &HelloResult) -> Self {
        Self {
            challenge,
            machine_id: hello.machine_id.clone(),
            runtime_lifetime: hello.runtime_lifetime.clone(),
            server_boot_id: hello.server_boot_id.clone(),
        }
    }

    /// Whether this statement names the identity `hello` reports.
    #[must_use]
    pub fn describes(&self, hello: &HelloResult) -> bool {
        self.machine_id == hello.machine_id
            && self.runtime_lifetime == hello.runtime_lifetime
            && self.server_boot_id == hello.server_boot_id
    }

    /// The unambiguous bytes the proof is computed over: a version tag, then every field
    /// prefixed by its 8-byte big-endian length.
    #[must_use]
    pub fn signed_bytes(&self) -> Vec<u8> {
        let challenge = String::from(self.challenge);
        let fields = [
            challenge.as_str(),
            self.machine_id.as_str(),
            self.runtime_lifetime.as_str(),
            self.server_boot_id.as_str(),
        ];
        let mut bytes = b"ovis-terminal/boot-proof/1".to_vec();
        for field in fields {
            let length = u64::try_from(field.len()).unwrap_or(u64::MAX);
            bytes.extend_from_slice(&length.to_be_bytes());
            bytes.extend_from_slice(field.as_bytes());
        }
        bytes
    }
}

/// A runtime's answer to a boot challenge, keyed by the credential of the connection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct BootProof {
    /// The vouched statement.
    pub statement: BootStatement,
    /// HMAC-SHA256 of the statement's signed bytes keyed by the presented bearer.
    pub mac: ProofMac,
}

impl BootProof {
    /// Binds `statement` to its keyed proof.
    #[must_use]
    pub const fn new(statement: BootStatement, mac: ProofMac) -> Self {
        Self { statement, mac }
    }
}

/// A machine's current name and the former names that still resolve to it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct MachineNames {
    /// Current name.
    pub name: MachineName,
    /// Former names, oldest first.
    pub aliases: Vec<MachineName>,
}

impl MachineNames {
    /// The names of one machine.
    #[must_use]
    pub const fn new(name: MachineName, aliases: Vec<MachineName>) -> Self {
        Self { name, aliases }
    }
}

impl MachineRecord {
    /// The record of a machine whose runtime authenticated with `hello`.
    #[must_use]
    pub fn connected(hello: &HelloResult, names: MachineNames, revision: Revision) -> Self {
        Self {
            machine_id: hello.machine_id.clone(),
            name: names.name,
            aliases: names.aliases,
            state: OpenEnum::Known(MachineState::Connected),
            remote_lifetime: Some(hello.runtime_lifetime.clone()),
            remote_boot_id: Some(hello.server_boot_id.clone()),
            last_seen_seq: None,
            version: Some(hello.version.clone()),
            revision,
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{BootChallenge, ProofMac};

    proptest! {
        #[test]
        fn hex_values_round_trip_every_byte_string(bytes in any::<[u8; 32]>()) {
            let challenge = BootChallenge::from_bytes(bytes);
            let text = serde_json::to_string(&challenge)?;
            prop_assert_eq!(text.len(), 66);
            prop_assert_eq!(serde_json::from_str::<BootChallenge>(&text)?, challenge);
            let mac: ProofMac = serde_json::from_str(&text)?;
            prop_assert_eq!(mac.to_bytes(), bytes);
        }
    }

    #[test]
    fn non_canonical_hex_is_rejected() {
        let upper = format!("\"{}\"", "AB".repeat(32));
        let short = format!("\"{}\"", "ab".repeat(31));
        let long = format!("\"{}\"", "ab".repeat(33));
        let wide = format!("\"{}é\"", "a".repeat(62));
        for text in [upper, short, long, wide, "1".to_owned()] {
            assert!(
                serde_json::from_str::<BootChallenge>(&text).is_err(),
                "{text}"
            );
        }
    }
}
