//! Where an endpoint listens: transport kind, connectable address and where it came from.
//!
//! Resolution reads only the values in [`EndpointEnvironment`]; it never touches the
//! filesystem, so describing an endpoint cannot create, start or repair anything.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::limits::TransportKind;
use serde::Serialize;

use super::EndpointError;

const DEFAULT_SESSION: &str = "default";
const MAX_SESSION_BYTES: usize = 32;
/// Longest Unix socket path accepted: macOS `sun_path` holds 104 bytes including the NUL.
pub const MAX_UNIX_SOCKET_PATH: usize = 103;
/// Namespace prefix of every local Windows named pipe.
pub const PIPE_PREFIX: &str = r"\\.\pipe\";

/// An environment variable endpoint resolution reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum EnvVar {
    /// `OVIS_TERMINAL_SOCKET`: a complete socket path or pipe name, on every platform.
    SocketOverride,
    /// `OVIS_TERMINAL_SESSION`: the session whose endpoint is resolved.
    Session,
    /// `XDG_RUNTIME_DIR`: preferred Unix socket directory.
    RuntimeDir,
    /// `XDG_STATE_HOME`: Unix fallback directory.
    StateDir,
    /// `HOME`: last Unix fallback, as `$HOME/.local/state`.
    Home,
    /// `USERNAME`: the Windows account that names the pipe.
    User,
}

impl EnvVar {
    const ALL: [Self; 6] = [
        Self::SocketOverride,
        Self::Session,
        Self::RuntimeDir,
        Self::StateDir,
        Self::Home,
        Self::User,
    ];

    /// The variable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::SocketOverride => "OVIS_TERMINAL_SOCKET",
            Self::Session => "OVIS_TERMINAL_SESSION",
            Self::RuntimeDir => "XDG_RUNTIME_DIR",
            Self::StateDir => "XDG_STATE_HOME",
            Self::Home => "HOME",
            Self::User => "USERNAME",
        }
    }
}

/// The non-empty values of every [`EnvVar`]; nothing else is consulted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EndpointEnvironment {
    values: std::collections::BTreeMap<EnvVar, OsString>,
}

impl EndpointEnvironment {
    /// Reads every [`EnvVar`] from the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        EnvVar::ALL
            .into_iter()
            .filter_map(|var| std::env::var_os(var.name()).map(|value| (var, value)))
            .fold(Self::default(), |env, (var, value)| env.with(var, value))
    }

    /// Sets `var`; an empty value counts as unset.
    #[must_use]
    pub fn with(mut self, var: EnvVar, value: impl Into<OsString>) -> Self {
        let value = value.into();
        if value.is_empty() {
            self.values.remove(&var);
        } else {
            self.values.insert(var, value);
        }
        self
    }

    fn get(&self, var: EnvVar) -> Option<&OsString> {
        self.values.get(&var)
    }

    fn absolute_dir(&self, var: EnvVar) -> Option<PathBuf> {
        self.get(var)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    }
}

/// The operating-system family whose endpoint form is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Platform {
    /// Unix domain sockets.
    Unix,
    /// Windows named pipes.
    Windows,
}

impl Platform {
    /// The platform this build runs on.
    pub const CURRENT: Self = if cfg!(windows) {
        Self::Windows
    } else {
        Self::Unix
    };
}

/// A session name: 1 to 32 ASCII letters, digits, `-` or `_`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SessionName(String);

impl SessionName {
    /// Parses a session name.
    ///
    /// # Errors
    ///
    /// [`EndpointError::InvalidSession`] for any other text.
    pub fn parse(text: &str) -> Result<Self, EndpointError> {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        if text.is_empty() || text.len() > MAX_SESSION_BYTES || !text.bytes().all(allowed) {
            return Err(EndpointError::InvalidSession);
        }
        Ok(Self(text.to_owned()))
    }

    /// Borrows the session name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Whether the address came from the override variable or the platform default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AddressSource {
    /// `OVIS_TERMINAL_SOCKET` named the address.
    EnvOverride,
    /// The platform default for the session.
    Default,
}

/// A connectable address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(untagged)]
#[non_exhaustive]
pub enum EndpointAddress {
    /// An absolute Unix socket path.
    Unix(PathBuf),
    /// A local named pipe such as `\\.\pipe\ovis-terminal-alice-default`.
    Pipe(String),
}

/// A resolved endpoint: what a client connects to and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EndpointDescriptor {
    kind: TransportKind,
    address: EndpointAddress,
    session: SessionName,
    source: AddressSource,
    env_override: &'static str,
}

impl EndpointDescriptor {
    /// Resolves the endpoint of `platform` from `env`.
    ///
    /// # Errors
    ///
    /// [`EndpointError::InvalidSession`], [`EndpointError::InvalidOverride`],
    /// [`EndpointError::NoRuntimeDirectory`], [`EndpointError::InvalidUser`] or
    /// [`EndpointError::SocketPathTooLong`] when no safe address can be named.
    pub fn resolve(env: &EndpointEnvironment, platform: Platform) -> Result<Self, EndpointError> {
        let session = env.get(EnvVar::Session).map_or_else(
            || SessionName::parse(DEFAULT_SESSION),
            |value| SessionName::parse(value.to_str().ok_or(EndpointError::InvalidSession)?),
        )?;
        let overridden = env.get(EnvVar::SocketOverride);
        let address = match (platform, overridden) {
            (Platform::Unix, Some(value)) => unix_override(value)?,
            (Platform::Windows, Some(value)) => pipe_override(value)?,
            (Platform::Unix, None) => unix_default(env, &session)?,
            (Platform::Windows, None) => pipe_default(env, &session)?,
        };
        let kind = match address {
            EndpointAddress::Unix(_) => TransportKind::Unix,
            EndpointAddress::Pipe(_) => TransportKind::Pipe,
        };
        let source = overridden.map_or(AddressSource::Default, |_| AddressSource::EnvOverride);
        Ok(Self {
            kind,
            address,
            session,
            source,
            env_override: EnvVar::SocketOverride.name(),
        })
    }

    /// The transport the address speaks.
    #[must_use]
    pub const fn kind(&self) -> TransportKind {
        self.kind
    }

    /// The connectable address.
    #[must_use]
    pub const fn address(&self) -> &EndpointAddress {
        &self.address
    }

    /// The session the endpoint serves.
    #[must_use]
    pub const fn session(&self) -> &SessionName {
        &self.session
    }

    /// Whether the override variable chose the address.
    #[must_use]
    pub const fn source(&self) -> AddressSource {
        self.source
    }

    /// The socket path, when the address is a Unix socket.
    #[must_use]
    pub fn unix_path(&self) -> Option<&Path> {
        match &self.address {
            EndpointAddress::Unix(path) => Some(path),
            EndpointAddress::Pipe(_) => None,
        }
    }
}

fn checked_unix(path: PathBuf) -> Result<EndpointAddress, EndpointError> {
    let length = path.as_os_str().len();
    if length > MAX_UNIX_SOCKET_PATH {
        return Err(EndpointError::SocketPathTooLong {
            length,
            limit: MAX_UNIX_SOCKET_PATH,
        });
    }
    Ok(EndpointAddress::Unix(path))
}

fn unix_override(value: &OsString) -> Result<EndpointAddress, EndpointError> {
    let path = PathBuf::from(value);
    if !path.is_absolute() || path.parent().is_none() {
        return Err(EndpointError::InvalidOverride);
    }
    checked_unix(path)
}

fn unix_default(
    env: &EndpointEnvironment,
    session: &SessionName,
) -> Result<EndpointAddress, EndpointError> {
    let base = env
        .absolute_dir(EnvVar::RuntimeDir)
        .or_else(|| env.absolute_dir(EnvVar::StateDir))
        .or_else(|| {
            env.absolute_dir(EnvVar::Home)
                .map(|home| home.join(".local").join("state"))
        })
        .ok_or(EndpointError::NoRuntimeDirectory)?;
    let file = format!("{}.sock", session.as_str());
    checked_unix(base.join("ovis-terminal").join(file))
}

fn pipe_override(value: &OsString) -> Result<EndpointAddress, EndpointError> {
    let text = value.to_str().ok_or(EndpointError::InvalidOverride)?;
    let name = text
        .strip_prefix(PIPE_PREFIX)
        .ok_or(EndpointError::InvalidOverride)?;
    if name.is_empty() || name.contains('\\') {
        return Err(EndpointError::InvalidOverride);
    }
    Ok(EndpointAddress::Pipe(text.to_owned()))
}

fn pipe_default(
    env: &EndpointEnvironment,
    session: &SessionName,
) -> Result<EndpointAddress, EndpointError> {
    let user = env
        .get(EnvVar::User)
        .and_then(|user| user.to_str())
        .filter(|user| !user.contains(['\\', '/']))
        .ok_or(EndpointError::InvalidUser)?;
    let name = format!("{PIPE_PREFIX}ovis-terminal-{user}-{}", session.as_str());
    Ok(EndpointAddress::Pipe(name))
}
