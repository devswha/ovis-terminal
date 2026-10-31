//! Globally unique stable machine identities.

use std::fmt;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

use super::IdentityError;

/// Globally unique stable identity for one installed machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MachineId(Uuid);

impl MachineId {
    /// Wraps a nonzero machine identity that has been durably reserved.
    ///
    /// # Errors
    ///
    /// Returns `invalid_identity` for the nil UUID.
    pub const fn new(value: Uuid) -> Result<Self, IdentityError> {
        if value.as_u128() == 0 {
            Err(IdentityError::Zero)
        } else {
            Ok(Self(value))
        }
    }

    /// The UUID value of this machine identity.
    pub const fn get(self) -> Uuid {
        self.0
    }
}

impl fmt::Display for MachineId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "machine:{}", self.0)
    }
}

impl FromStr for MachineId {
    type Err = IdentityError;

    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        let uuid = raw
            .strip_prefix("machine:")
            .ok_or_else(|| IdentityError::WrongFamily {
                expected: "machine",
                actual: raw
                    .split_once(':')
                    .map_or_else(|| raw.to_owned(), |(family, _)| family.to_owned()),
            })?;
        let value = Uuid::parse_str(uuid).map_err(|source| IdentityError::UuidParse { source })?;
        let id = Self::new(value)?;
        if id.to_string() != raw {
            return Err(IdentityError::InvalidIdentity);
        }
        Ok(id)
    }
}

impl Serialize for MachineId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MachineId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(D::Error::custom)
    }
}
