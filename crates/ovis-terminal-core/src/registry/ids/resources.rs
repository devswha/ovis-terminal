//! Opaque family-prefixed resource identifiers.

use std::fmt;
use std::num::{IntErrorKind, NonZeroU64};
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::IdentityError;

/// A typed, durably allocatable resource identity.
pub trait DurableId: Copy + fmt::Display + Eq + Serialize {
    /// The persisted resource family used to allocate this identity.
    const FAMILY: &'static str;

    /// Builds the typed identity from its nonzero family-local sequence.
    fn from_sequence(value: NonZeroU64) -> Self;
}

macro_rules! resource_id {
    ($name:ident, $family:literal) => {
        #[doc = concat!("Opaque ", $family, " resource identity.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Creates this identity from a known nonzero sequence.
            pub const fn new(value: NonZeroU64) -> Self {
                Self(value)
            }

            /// The numeric identity value, distinct from every other ID type.
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }

        impl TryFrom<u64> for $name {
            type Error = IdentityError;

            fn try_from(value: u64) -> Result<Self, Self::Error> {
                NonZeroU64::new(value).map(Self).ok_or(IdentityError::Zero)
            }
        }

        impl FromStr for $name {
            type Err = IdentityError;

            fn from_str(raw: &str) -> Result<Self, Self::Err> {
                let (family, sequence) =
                    raw.split_once(':').ok_or(IdentityError::InvalidIdentity)?;
                if family != $family {
                    return Err(IdentityError::WrongFamily {
                        expected: $family,
                        actual: family.to_owned(),
                    });
                }
                let value = u64::from_str(sequence).map_err(|error| {
                    if error.kind() == &IntErrorKind::PosOverflow {
                        IdentityError::Overflow
                    } else {
                        IdentityError::InvalidIdentity
                    }
                })?;
                let id = Self::try_from(value)?;
                if id.to_string() != raw {
                    return Err(IdentityError::InvalidIdentity);
                }
                Ok(id)
            }
        }

        impl DurableId for $name {
            const FAMILY: &'static str = $family;

            fn from_sequence(value: NonZeroU64) -> Self {
                Self(value)
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(formatter, "{}:{}", $family, self.0)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let raw = String::deserialize(deserializer)?;
                raw.parse().map_err(D::Error::custom)
            }
        }
    };
}

resource_id!(RuntimeId, "runtime");
resource_id!(ServerBootId, "boot");
resource_id!(HostBootId, "host-boot");
resource_id!(WorkspaceId, "workspace");
resource_id!(TabId, "tab");
resource_id!(PaneId, "pane");
resource_id!(SplitId, "split");
resource_id!(TerminalId, "terminal");
resource_id!(ProcessId, "process");
resource_id!(StreamId, "stream");
resource_id!(LeaseId, "lease");
