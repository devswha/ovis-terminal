//! Validated display names and OS process evidence.

use std::num::NonZeroU64;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::IdentityError;

/// A validated display name, independent of resource identity and layout.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ResourceName(String);

impl ResourceName {
    /// Parses a name that cannot be confused with an address or path.
    ///
    /// # Errors
    ///
    /// Returns `invalid_name` for empty, oversized, non-ASCII or reserved text.
    pub fn new(value: &str) -> Result<Self, IdentityError> {
        let valid = !value.is_empty()
            && value.len() <= 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'));
        if valid {
            Ok(Self(value.to_owned()))
        } else {
            Err(IdentityError::InvalidName)
        }
    }

    /// The display name text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ResourceName {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ResourceName> for String {
    fn from(value: ResourceName) -> Self {
        value.0
    }
}

/// The operating-system PID, separate from the runtime's process ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcessPid(NonZeroU64);

impl ProcessPid {
    /// Creates a valid positive process ID.
    ///
    /// # Errors
    ///
    /// Returns `invalid_identity` when the PID is zero.
    pub const fn new(value: NonZeroU64) -> Self {
        Self(value)
    }

    /// The process ID value.
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl TryFrom<u64> for ProcessPid {
    type Error = IdentityError;

    fn try_from(value: u64) -> Result<Self, Self::Error> {
        NonZeroU64::new(value).map(Self).ok_or(IdentityError::Zero)
    }
}

impl Serialize for ProcessPid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for ProcessPid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let raw = String::deserialize(deserializer)?;
        let value = u64::from_str(&raw).map_err(|error| {
            if error.kind() == &std::num::IntErrorKind::PosOverflow {
                D::Error::custom(IdentityError::Overflow)
            } else {
                D::Error::custom(IdentityError::InvalidIdentity)
            }
        })?;
        let pid = Self::try_from(value).map_err(D::Error::custom)?;
        if pid.0.to_string() != raw {
            return Err(D::Error::custom(IdentityError::InvalidIdentity));
        }
        Ok(pid)
    }
}

/// A validated OS-specific process start token that distinguishes PID reuse.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProcessStartToken(String);

impl ProcessStartToken {
    /// Creates a nonempty bounded start token.
    ///
    /// # Errors
    ///
    /// Returns `invalid_identity` for empty, oversized or control-containing tokens.
    pub fn new(value: &str) -> Result<Self, IdentityError> {
        if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
            return Err(IdentityError::InvalidIdentity);
        }
        Ok(Self(value.to_owned()))
    }

    /// The opaque start-token text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ProcessStartToken {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(&value)
    }
}

impl From<ProcessStartToken> for String {
    fn from(value: ProcessStartToken) -> Self {
        value.0
    }
}
