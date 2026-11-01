//! Where updates come from and go to, what a build must serve, and step bounds.

use std::path::PathBuf;
use std::time::Duration;

use ovis_terminal_protocol::domain::update::{
    ActiveBuild, BuildTarget, HolderGeneration, ReleaseVersion,
};
use ovis_terminal_protocol::schema::scalar::ProtocolGeneration;

use super::UpdateError;
use super::digest::hash_file;
use super::store::{Selected, Selection};

/// The build the runtime started with; it seeds the selection of a new install root.
#[derive(Debug, Clone)]
pub struct CurrentBuild {
    version: ReleaseVersion,
    binary: PathBuf,
}

impl CurrentBuild {
    /// Version `version` running from `binary`.
    #[must_use]
    pub const fn new(version: ReleaseVersion, binary: PathBuf) -> Self {
        Self { version, binary }
    }
}

/// What a build must serve to replace this runtime.
#[derive(Debug, Clone)]
pub struct Requirement {
    pub(super) target: BuildTarget,
    pub(super) protocol: ProtocolGeneration,
    pub(super) holder: HolderGeneration,
}

impl Requirement {
    /// A build for this host that serves `protocol` clients and adopts `holder` holders.
    #[must_use]
    pub fn new(protocol: ProtocolGeneration, holder: HolderGeneration) -> Self {
        Self {
            target: BuildTarget::host(),
            protocol,
            holder,
        }
    }
}

/// Size and time bounds of one update step.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub(super) archive_bytes: u64,
    pub(super) probe_deadline: Duration,
}

impl Limits {
    /// Archives up to 512 MiB; a readiness probe gets ten seconds.
    pub const DEFAULT: Self = Self::new(512 * 1024 * 1024, Duration::from_secs(10));

    /// Archives up to `archive_bytes`; a readiness probe gets `probe_deadline`.
    #[must_use]
    pub const fn new(archive_bytes: u64, probe_deadline: Duration) -> Self {
        Self {
            archive_bytes,
            probe_deadline,
        }
    }
}

/// Where updates are staged from and installed to, and what they must serve.
#[derive(Debug, Clone)]
pub struct Config {
    pub(super) install_root: PathBuf,
    pub(super) releases: PathBuf,
    current: CurrentBuild,
    pub(super) requirement: Requirement,
    pub(super) limits: Limits,
}

impl Config {
    /// Installs below `install_root` from archives below `releases`.
    #[must_use]
    pub fn new(
        (install_root, releases): (PathBuf, PathBuf),
        current: CurrentBuild,
        requirement: Requirement,
    ) -> Self {
        Self {
            install_root,
            releases,
            current,
            requirement,
            limits: Limits::DEFAULT,
        }
    }

    /// The same configuration with other bounds.
    #[must_use]
    pub const fn with_limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }
}

/// The selection of a new install root: the build the runtime started with.
pub(super) fn seed(config: &Config) -> Result<Selection, UpdateError> {
    let current = &config.current;
    let digest = hash_file(&current.binary, config.limits.archive_bytes)?;
    let build = ActiveBuild::new(
        current.version.clone(),
        config.requirement.target.clone(),
        digest,
    );
    Ok(Selection {
        active: Selected {
            build,
            binary: current.binary.clone(),
            update_id: None,
        },
        previous: None,
    })
}
