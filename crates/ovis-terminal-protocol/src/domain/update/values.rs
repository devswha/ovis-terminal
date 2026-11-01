//! Validated update values: a release digest, build target and version, and the holder
//! wire generation a build adopts. Each is parsed once at the boundary.

use std::borrow::Cow;
use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::rpc::DecimalU64;

/// A value outside its update grammar.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum UpdateValueError {
    /// The digest is not 64 lowercase hexadecimal characters.
    #[error("the SHA-256 digest is not 64 lowercase hexadecimal characters")]
    Sha256,
    /// The target is not `<os>-<arch>`.
    #[error("the build target is not `<os>-<arch>` in lowercase letters, digits and `_`")]
    Target,
    /// The version could name a path or exceeds its bound.
    #[error("the release version is not 1 to 64 letters, digits, `.`, `+` or `-`")]
    Version,
}

impl UpdateValueError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(self) -> &'static str {
        match self {
            Self::Sha256 => "invalid_sha256",
            Self::Target => "invalid_target",
            Self::Version => "invalid_version",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(self) -> &'static str {
        match self {
            Self::Sha256 => "pass the release checksum as 64 lowercase hex digits",
            Self::Target => "pass the target named in the release manifest, such as linux-x86_64",
            Self::Version => "pass the release version exactly as the manifest names it",
        }
    }
}

/// Declares a validated string value with its grammar and schema pattern.
macro_rules! checked_string {
    ($(#[$meta:meta])* $name:ident, $check:expr, $error:ident, $pattern:literal) => {
        $(#[$meta])*
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(try_from = "String", into = "String")]
        pub struct $name(String);

        impl $name {
            /// Borrows the validated text.
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl TryFrom<String> for $name {
            type Error = UpdateValueError;

            fn try_from(value: String) -> Result<Self, Self::Error> {
                let check: fn(&str) -> bool = $check;
                if check(&value) {
                    Ok(Self(value))
                } else {
                    Err(UpdateValueError::$error)
                }
            }
        }

        impl std::str::FromStr for $name {
            type Err = UpdateValueError;

            fn from_str(value: &str) -> Result<Self, Self::Err> {
                Self::try_from(value.to_owned())
            }
        }

        impl From<$name> for String {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }

        impl JsonSchema for $name {
            fn schema_name() -> Cow<'static, str> {
                Cow::Borrowed(stringify!($name))
            }

            fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
                json_schema!({ "type": "string", "pattern": $pattern })
            }
        }
    };
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

fn is_target_part(part: &str) -> bool {
    (1..=32).contains(&part.len())
        && part
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

fn is_target(value: &str) -> bool {
    value
        .split_once('-')
        .is_some_and(|(os, arch)| is_target_part(os) && is_target_part(arch))
}

fn is_version(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= 64
        && bytes
            .next()
            .is_some_and(|first| first.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'+' | b'-'))
}

checked_string!(
    /// SHA-256 content digest as 64 lowercase hexadecimal characters.
    Sha256Digest,
    is_sha256,
    Sha256,
    "^[0-9a-f]{64}$"
);
checked_string!(
    /// Operating system and architecture a build runs on, such as `linux-x86_64`.
    BuildTarget,
    is_target,
    Target,
    "^[a-z0-9_]{1,32}-[a-z0-9_]{1,32}$"
);
checked_string!(
    /// Release version; never a path, so it can name a directory in the install root.
    ReleaseVersion,
    is_version,
    Version,
    "^[0-9A-Za-z][0-9A-Za-z.+-]{0,63}$"
);

impl BuildTarget {
    /// The target this process was built for.
    #[must_use]
    pub fn host() -> Self {
        Self(format!(
            "{}-{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        ))
    }
}

/// Holder wire generation, carried as a canonical decimal string.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct HolderGeneration(DecimalU64);

impl HolderGeneration {
    /// Wraps a native generation.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(DecimalU64::new(value))
    }

    /// The native generation.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

impl JsonSchema for HolderGeneration {
    fn schema_name() -> Cow<'static, str> {
        Cow::Borrowed("HolderGeneration")
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "pattern": "^(0|[1-9][0-9]{0,19})$",
            "description": "Holder wire generation carried as a canonical decimal string.",
        })
    }
}
