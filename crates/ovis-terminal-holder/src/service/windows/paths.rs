//! Naming one session's Windows holder: service label, state directory, record and pipe.
//!
//! Resolution reads only the values it is given; it creates nothing.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::schema::ids::ServiceLabel;

use super::HostError;

const MAX_SESSION_BYTES: usize = 32;
const RECORD_FILE: &str = "holder-host.json";

/// A session name: 1 to 32 ASCII letters, digits, `-` or `_`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostSession(String);

impl HostSession {
    /// Parses a session name.
    ///
    /// # Errors
    ///
    /// [`HostError::InvalidSession`] for any other text.
    pub fn parse(text: &str) -> Result<Self, HostError> {
        let allowed = |byte: u8| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_';
        if text.is_empty() || text.len() > MAX_SESSION_BYTES || !text.bytes().all(allowed) {
            return Err(HostError::InvalidSession);
        }
        Ok(Self(text.to_owned()))
    }

    /// The session name.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The service name and label: `ovis-terminal-holder-<session>`.
    #[must_use]
    pub fn label(&self) -> ServiceLabel {
        ServiceLabel::new(format!("ovis-terminal-holder-{}", self.0))
    }
}

/// Where one session's holder keeps its record, and which pipe it serves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    session: HostSession,
    state: PathBuf,
}

impl HostPaths {
    /// Paths under an explicit absolute state directory.
    ///
    /// # Errors
    ///
    /// [`HostError::UnsupportedPath`] for a relative path, one that is not UTF-8, or one
    /// holding a quote, which a service command line cannot carry.
    pub fn new(session: HostSession, state: PathBuf) -> Result<Self, HostError> {
        let usable = state.is_absolute() && state.to_str().is_some_and(|text| !text.contains('"'));
        if !usable {
            return Err(HostError::UnsupportedPath { path: state });
        }
        Ok(Self { session, state })
    }

    /// `<LOCALAPPDATA>\ovis-terminal\<session>`, from the given `LOCALAPPDATA` value.
    ///
    /// # Errors
    ///
    /// [`HostError::NoStateDirectory`] when it is unset or empty, and the errors of
    /// [`Self::new`].
    pub fn resolve(
        session: HostSession,
        local_app_data: Option<OsString>,
    ) -> Result<Self, HostError> {
        let base = local_app_data
            .filter(|value| !value.is_empty())
            .ok_or(HostError::NoStateDirectory)?;
        let state = PathBuf::from(base)
            .join("ovis-terminal")
            .join(session.as_str());
        Self::new(session, state)
    }

    /// The session.
    #[must_use]
    pub const fn session(&self) -> &HostSession {
        &self.session
    }

    /// The state directory.
    #[must_use]
    pub fn state_dir(&self) -> &Path {
        &self.state
    }

    /// The host record the running holder publishes.
    #[must_use]
    pub fn record(&self) -> PathBuf {
        self.state.join(RECORD_FILE)
    }

    /// `\\.\pipe\ovis-terminal-holder-<owner sid>-<session>`: the owner's SID keeps the
    /// names of different accounts apart, so one account cannot claim another's name.
    ///
    /// # Errors
    ///
    /// [`HostError::Name`] when the SID makes the name unusable.
    pub fn pipe(&self, owner_sid: &str) -> Result<PipeName, HostError> {
        let name = format!(
            r"\\.\pipe\ovis-terminal-holder-{owner_sid}-{}",
            self.session.as_str()
        );
        PipeName::parse(&name).map_err(HostError::Name)
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::path::PathBuf;

    use super::{HostPaths, HostSession};

    #[test]
    fn sessions_name_one_label_and_malformed_names_are_refused() {
        let session = HostSession::parse("ci-54_a").unwrap();
        assert_eq!(session.label().as_str(), "ovis-terminal-holder-ci-54_a");
        for refused in ["", "a b", "../x", "a\\b", &"x".repeat(33)] {
            assert_eq!(
                HostSession::parse(refused).map_err(|e| e.kind()),
                Err("invalid_session")
            );
        }
    }

    #[test]
    fn state_comes_from_local_app_data_or_an_explicit_absolute_path_only() {
        let session = HostSession::parse("s").unwrap();
        let unset = HostPaths::resolve(session.clone(), None).map_err(|e| e.kind());
        assert_eq!(unset, Err("no_runtime_directory"));
        let empty = HostPaths::resolve(session.clone(), Some(OsString::new()));
        assert_eq!(empty.map_err(|e| e.kind()), Err("no_runtime_directory"));
        let relative = HostPaths::new(session.clone(), PathBuf::from("state"));
        assert_eq!(relative.map_err(|e| e.kind()), Err("unsupported_path"));
        let quoted = HostPaths::new(session.clone(), PathBuf::from("/a\"b"));
        assert_eq!(quoted.map_err(|e| e.kind()), Err("unsupported_path"));
        let root = std::env::temp_dir();
        let paths = HostPaths::resolve(session, Some(root.clone().into_os_string())).unwrap();
        assert_eq!(paths.state_dir(), root.join("ovis-terminal").join("s"));
        assert_eq!(paths.record(), paths.state_dir().join("holder-host.json"));
        assert!(
            !paths.state_dir().exists(),
            "resolving created the directory"
        );
    }

    #[test]
    fn the_pipe_is_named_for_its_owner_and_session() {
        let paths = HostPaths::new(HostSession::parse("s").unwrap(), std::env::temp_dir()).unwrap();
        assert_eq!(
            paths.pipe("S-1-5-21-9").unwrap().as_str(),
            r"\\.\pipe\ovis-terminal-holder-S-1-5-21-9-s"
        );
        assert_eq!(
            paths.pipe("a\\b").map_err(|e| e.kind()),
            Err("invalid_pipe_name")
        );
    }
}
