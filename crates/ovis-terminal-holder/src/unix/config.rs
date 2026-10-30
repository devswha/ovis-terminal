//! Holder identity, per-terminal bounds and the environment for wire spawns.

use std::ffi::OsString;
use std::path::PathBuf;

use crate::protocol::HolderDescriptor;

/// Per-terminal and per-core-link memory bounds.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct HolderLimits {
    /// Raw output bytes retained per terminal for adoption replay.
    pub tail_bytes: usize,
    /// Input bytes that may wait for the PTY per terminal before `input_overflow`.
    pub input_queue_bytes: usize,
    /// Undelivered message cost toward one core before that link is dropped.
    pub link_bytes: usize,
}

impl HolderLimits {
    /// One MiB retained tail, 16 MiB input queue and 8 MiB undelivered link backlog.
    pub const DEFAULT: Self = Self::new(1 << 20, 16 << 20, 8 << 20);

    /// Creates explicit bounds.
    #[must_use]
    pub const fn new(tail_bytes: usize, input_queue_bytes: usize, link_bytes: usize) -> Self {
        Self {
            tail_bytes,
            input_queue_bytes,
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

/// Holder identity, bounds and the environment used for wire spawns.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct HolderConfig {
    /// Build and process-lifetime identity reported in every welcome.
    pub descriptor: HolderDescriptor,
    /// Memory bounds.
    pub limits: HolderLimits,
    /// Environment for v1 wire `Spawn` messages, which carry only argv and geometry.
    pub wire_environment: ProcessEnvironment,
}

impl HolderConfig {
    /// Creates a holder configuration.
    #[must_use]
    pub const fn new(
        descriptor: HolderDescriptor,
        limits: HolderLimits,
        wire_environment: ProcessEnvironment,
    ) -> Self {
        Self {
            descriptor,
            limits,
            wire_environment,
        }
    }
}
