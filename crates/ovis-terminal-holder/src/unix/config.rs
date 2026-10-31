//! Holder identity, per-terminal bounds and the environment of a launch.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::protocol::HolderDescriptor;

/// Per-terminal input queue bounds; both apply to every request, empty or not.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct QueueLimits {
    /// Input bytes that may wait for the PTY.
    pub bytes: usize,
    /// Input requests that may wait for the PTY.
    pub entries: usize,
}

impl QueueLimits {
    /// Creates explicit queue bounds.
    #[must_use]
    pub const fn new(bytes: usize, entries: usize) -> Self {
        Self { bytes, entries }
    }
}

/// Per-terminal and per-core-link memory bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HolderLimits {
    /// Raw output bytes retained per terminal for adoption replay.
    pub tail_bytes: usize,
    /// Input that may wait for the PTY per terminal before `input_overflow`.
    pub input: QueueLimits,
    /// Undelivered message cost toward one core before that link is dropped.
    pub link_bytes: usize,
}

impl HolderLimits {
    /// One MiB retained tail, a 16 MiB / 4096-entry input queue and 8 MiB link backlog.
    pub const DEFAULT: Self = Self::new(1 << 20, QueueLimits::new(16 << 20, 4096), 8 << 20);

    /// Creates explicit bounds.
    #[must_use]
    pub const fn new(tail_bytes: usize, input: QueueLimits, link_bytes: usize) -> Self {
        Self {
            tail_bytes,
            input,
            link_bytes,
        }
    }
}

impl Default for HolderLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Working directory and complete environment of a spawned program.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct ProcessEnvironment {
    /// Directory the program starts in; it must exist.
    pub cwd: PathBuf,
    /// The program's entire environment; nothing is inherited from the holder.
    pub vars: Vec<(OsString, OsString)>,
}

impl ProcessEnvironment {
    /// Creates an explicit working directory and environment.
    #[must_use]
    pub const fn new(cwd: PathBuf, vars: Vec<(OsString, OsString)>) -> Self {
        Self { cwd, vars }
    }
}

/// Holder identity and bounds; every launch carries its own environment.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HolderConfig {
    /// Build and process-lifetime identity reported in every welcome.
    pub descriptor: HolderDescriptor,
    /// Memory bounds.
    pub limits: HolderLimits,
}

impl HolderConfig {
    /// Creates a holder configuration.
    #[must_use]
    pub const fn new(descriptor: HolderDescriptor, limits: HolderLimits) -> Self {
        Self { descriptor, limits }
    }
}
