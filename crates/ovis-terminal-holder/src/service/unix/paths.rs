//! Where one session's holder service keeps its socket, record and definition.
//!
//! Resolution reads only the explicit [`ServiceEnvironment`] values and never touches
//! the filesystem; there is no `/tmp` fallback, so isolated roots stay isolated.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::schema::ids::ServiceLabel;

use super::ServiceError;

const MAX_SESSION_BYTES: usize = 32;
/// Longest socket path accepted: macOS `sun_path` holds 104 bytes including the NUL.
const MAX_SOCKET_PATH: usize = 103;

/// An environment variable service resolution reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ServiceVar {
    /// `XDG_RUNTIME_DIR`: preferred socket and record directory.
    RuntimeDir,
    /// `XDG_STATE_HOME`: fallback socket and record directory.
    StateDir,
    /// `XDG_CONFIG_HOME`: systemd user unit directory base.
    ConfigDir,
    /// `HOME`: last fallback, and the launchd agent directory base.
    Home,
}

impl ServiceVar {
    /// Every variable, in resolution order.
    pub const ALL: [Self; 4] = [
        Self::RuntimeDir,
        Self::StateDir,
        Self::ConfigDir,
        Self::Home,
    ];

    /// The variable name.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::RuntimeDir => "XDG_RUNTIME_DIR",
            Self::StateDir => "XDG_STATE_HOME",
            Self::ConfigDir => "XDG_CONFIG_HOME",
            Self::Home => "HOME",
        }
    }
}

/// The absolute values of every [`ServiceVar`]; relative or empty values count as unset.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServiceEnvironment {
    values: std::collections::BTreeMap<ServiceVar, PathBuf>,
}

impl ServiceEnvironment {
    /// Reads every [`ServiceVar`] from the process environment.
    #[must_use]
    pub fn from_process() -> Self {
        ServiceVar::ALL
            .into_iter()
            .filter_map(|var| std::env::var_os(var.name()).map(|value| (var, value)))
            .fold(Self::default(), |env, (var, value)| env.with(var, value))
    }

    /// Sets `var` when `value` is an absolute path.
    #[must_use]
    pub fn with(mut self, var: ServiceVar, value: impl Into<OsString>) -> Self {
        let path = PathBuf::from(value.into());
        if path.is_absolute() {
            self.values.insert(var, path);
        } else {
            self.values.remove(&var);
        }
        self
    }

    /// The set variables, for writing them into a service definition.
    pub fn vars(&self) -> impl Iterator<Item = (ServiceVar, &Path)> {
        self.values.iter().map(|(var, path)| (*var, path.as_path()))
    }

    fn get(&self, var: ServiceVar) -> Option<&Path> {
        self.values.get(&var).map(PathBuf::as_path)
    }
}

/// A session name and the private labels derived from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceSession(String);

impl ServiceSession {
    /// Parses a session name.
    ///
    /// # Errors
    ///
    /// [`ServiceError::InvalidSession`] unless it is 1 to 32 ASCII letters, digits, `-` or `_`.
    pub fn parse(text: &str) -> Result<Self, ServiceError> {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        if text.is_empty() || text.len() > MAX_SESSION_BYTES || !text.bytes().all(allowed) {
            return Err(ServiceError::InvalidSession);
        }
        Ok(Self(text.to_owned()))
    }

    /// The session name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The label every manager and record uses for this session.
    #[must_use]
    pub fn label(&self) -> ServiceLabel {
        ServiceLabel::new(format!("ovis-terminal-holder-{}", self.0))
    }

    /// The launchd job label.
    #[must_use]
    pub fn launchd_label(&self) -> String {
        format!("dev.ovis-terminal.holder.{}", self.0)
    }
}

/// Every path one session's holder service uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServicePaths {
    session: ServiceSession,
    environment: ServiceEnvironment,
    directory: PathBuf,
}

impl ServicePaths {
    /// Resolves the paths of `session` from `environment`.
    ///
    /// # Errors
    ///
    /// [`ServiceError::NoRuntimeDirectory`] or [`ServiceError::SocketPathTooLong`].
    pub fn resolve(
        environment: ServiceEnvironment,
        session: ServiceSession,
    ) -> Result<Self, ServiceError> {
        let base = environment
            .get(ServiceVar::RuntimeDir)
            .or_else(|| environment.get(ServiceVar::StateDir))
            .map(Path::to_path_buf)
            .or_else(|| {
                environment
                    .get(ServiceVar::Home)
                    .map(|home| home.join(".local").join("state"))
            })
            .ok_or(ServiceError::NoRuntimeDirectory)?;
        let paths = Self {
            session,
            environment,
            directory: base.join("ovis-terminal"),
        };
        let length = paths.socket().as_os_str().len();
        if length > MAX_SOCKET_PATH {
            return Err(ServiceError::SocketPathTooLong {
                length,
                limit: MAX_SOCKET_PATH,
            });
        }
        Ok(paths)
    }

    /// The session.
    #[must_use]
    pub const fn session(&self) -> &ServiceSession {
        &self.session
    }

    /// The environment the paths were resolved from.
    #[must_use]
    pub const fn environment(&self) -> &ServiceEnvironment {
        &self.environment
    }

    /// The owner-only directory holding the socket and record.
    #[must_use]
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// The holder socket.
    #[must_use]
    pub fn socket(&self) -> PathBuf {
        self.directory
            .join(format!("holder-{}.sock", self.session.as_str()))
    }

    /// The start lock file; it is never removed.
    #[must_use]
    pub fn lock(&self) -> PathBuf {
        self.directory
            .join(format!("holder-{}.lock", self.session.as_str()))
    }

    /// The discovery record.
    #[must_use]
    pub fn record(&self) -> PathBuf {
        self.directory
            .join(format!("holder-{}.json", self.session.as_str()))
    }

    /// The systemd user unit, when a config or home directory is set.
    #[must_use]
    pub fn systemd_unit(&self) -> Option<PathBuf> {
        let config = self
            .environment
            .get(ServiceVar::ConfigDir)
            .map(Path::to_path_buf);
        let config = config.or_else(|| {
            self.environment
                .get(ServiceVar::Home)
                .map(|home| home.join(".config"))
        })?;
        Some(
            config
                .join("systemd")
                .join("user")
                .join(self.systemd_unit_name()),
        )
    }

    /// The systemd unit name.
    #[must_use]
    pub fn systemd_unit_name(&self) -> String {
        format!("{}.service", self.session.label().as_str())
    }

    /// The launchd agent property list, when a home directory is set.
    #[must_use]
    pub fn launchd_plist(&self) -> Option<PathBuf> {
        let home = self.environment.get(ServiceVar::Home)?;
        let file = format!("{}.plist", self.session.launchd_label());
        Some(home.join("Library").join("LaunchAgents").join(file))
    }
}

#[cfg(test)]
mod tests {
    use super::{ServiceEnvironment, ServicePaths, ServiceSession, ServiceVar};

    fn session(name: &str) -> ServiceSession {
        ServiceSession::parse(name).unwrap()
    }

    #[test]
    fn paths_follow_the_explicit_directories_in_order_and_never_tmp() {
        let home = ServiceEnvironment::default().with(ServiceVar::Home, "/h");
        let paths = ServicePaths::resolve(home.clone(), session("s1")).unwrap();
        assert_eq!(
            paths.socket(),
            std::path::Path::new("/h/.local/state/ovis-terminal/holder-s1.sock")
        );
        assert_eq!(
            paths.systemd_unit().unwrap(),
            std::path::Path::new("/h/.config/systemd/user/ovis-terminal-holder-s1.service")
        );
        let runtime = home
            .with(ServiceVar::RuntimeDir, "/r")
            .with(ServiceVar::ConfigDir, "/c");
        let paths = ServicePaths::resolve(runtime, session("s1")).unwrap();
        assert_eq!(
            paths.record(),
            std::path::Path::new("/r/ovis-terminal/holder-s1.json")
        );
        assert_eq!(
            paths.systemd_unit().unwrap(),
            std::path::Path::new("/c/systemd/user/ovis-terminal-holder-s1.service")
        );
        let relative = ServiceEnvironment::default().with(ServiceVar::RuntimeDir, "run");
        let refused = ServicePaths::resolve(relative, session("s1"))
            .map(drop)
            .map_err(|e| e.kind());
        assert_eq!(refused, Err("no_runtime_directory"));
    }

    #[test]
    fn session_names_are_closed_and_long_socket_paths_are_refused() {
        for bad in ["", "a b", "../x", &"x".repeat(33)] {
            assert_eq!(
                ServiceSession::parse(bad).map(drop).map_err(|e| e.kind()),
                Err("invalid_session")
            );
        }
        let deep = ServiceEnvironment::default()
            .with(ServiceVar::RuntimeDir, format!("/{}", "d".repeat(90)));
        let refused = ServicePaths::resolve(deep, session("s"))
            .map(drop)
            .map_err(|e| e.kind());
        assert_eq!(refused, Err("socket_path_too_long"));
    }
}
