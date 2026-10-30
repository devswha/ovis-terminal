//! Nonzero generations serialized as canonical decimal strings.

use std::num::NonZeroU64;
use std::str::FromStr;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::IdentityError;

macro_rules! generation {
    ($name:ident) => {
        #[doc = concat!("A nonzero ", stringify!($name), " value serialized as a decimal string.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(NonZeroU64);

        impl $name {
            /// Creates a positive generation.
            ///
            /// # Errors
            ///
            /// Returns `invalid_identity` when the generation is zero.
            pub fn new(value: u64) -> Result<Self, IdentityError> {
                NonZeroU64::new(value).map(Self).ok_or(IdentityError::Zero)
            }

            /// The generation value.
            pub const fn get(self) -> u64 {
                self.0.get()
            }

            /// Advances the generation without wrapping.
            ///
            /// # Errors
            ///
            /// Returns `counter_overflow` when the generation is exhausted.
            pub fn checked_next(self) -> Result<Self, IdentityError> {
                let next = self
                    .0
                    .get()
                    .checked_add(1)
                    .ok_or(IdentityError::CounterOverflow)?;
                Self::new(next)
            }
        }

        impl Serialize for $name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(&self.0.to_string())
            }
        }

        impl<'de> Deserialize<'de> for $name {
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
                let generation = Self::new(value).map_err(D::Error::custom)?;
                if generation.0.to_string() != raw {
                    return Err(D::Error::custom(IdentityError::InvalidIdentity));
                }
                Ok(generation)
            }
        }
    };
}

generation!(TerminalGeneration);
generation!(ProcessGeneration);
generation!(StreamGeneration);
generation!(LeaseGeneration);
generation!(MachineGeneration);
generation!(RuntimeGeneration);
generation!(ServerBootGeneration);
generation!(HostBootGeneration);
generation!(WorkspaceGeneration);
generation!(TabGeneration);
generation!(PaneGeneration);
generation!(SplitGeneration);
