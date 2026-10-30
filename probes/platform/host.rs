//! Facts about the host the probe binary runs on.

use serde::Serialize;

/// Byte order of native integers.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ByteOrder {
    /// Least significant byte first.
    Little,
    /// Most significant byte first.
    Big,
}

impl ByteOrder {
    /// The byte order observed by decoding a native integer at run time.
    #[must_use]
    pub const fn measured() -> Self {
        if u16::from_ne_bytes([1, 0]) == 1 {
            Self::Little
        } else {
            Self::Big
        }
    }

    /// The byte order the binary was compiled for.
    #[must_use]
    pub const fn compiled() -> Self {
        if cfg!(target_endian = "little") {
            Self::Little
        } else {
            Self::Big
        }
    }
}

/// Operating system, architecture and byte order of the running host.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[non_exhaustive]
pub struct HostFacts {
    /// Operating system (`linux`, `macos`, `windows`, ...).
    pub os: &'static str,
    /// OS family (`unix` or `windows`).
    pub family: &'static str,
    /// CPU architecture (`x86_64`, `aarch64`, ...).
    pub arch: &'static str,
    /// Byte order measured at run time.
    pub byte_order: ByteOrder,
}

impl HostFacts {
    /// Facts about the current host.
    #[must_use]
    pub const fn current() -> Self {
        Self {
            os: std::env::consts::OS,
            family: std::env::consts::FAMILY,
            arch: std::env::consts::ARCH,
            byte_order: ByteOrder::measured(),
        }
    }
}
