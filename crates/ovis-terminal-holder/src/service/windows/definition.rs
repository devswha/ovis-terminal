//! What a Windows holder service is installed as: its command line and ownership marker.
//!
//! The command line is quoted by the rules `CommandLineToArgvW` parses, so the service
//! control manager starts exactly the arguments written here. The description carries a
//! marker naming the label and the owner's SID; administration acts only on a service
//! whose marker it wrote, never on a same-named service someone else installed.

use std::path::Path;

use ovis_terminal_protocol::domain::windows_service::HolderPlacement;

use super::{HostError, HostPaths};

/// One holder service as the service control manager stores it.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct ServiceDefinition {
    /// Service name, equal to the session label.
    pub name: String,
    /// Display name shown in service tools.
    pub display_name: String,
    /// The full command line the manager runs.
    pub command_line: String,
    /// Description holding the ownership marker.
    pub description: String,
}

impl ServiceDefinition {
    /// The definition that runs `program` as the holder of `paths` for `owner_sid`.
    ///
    /// # Errors
    ///
    /// [`HostError::UnsupportedPath`] when `program` is not UTF-8.
    pub fn new(
        paths: &HostPaths,
        program: &Path,
        owner: (&str, HolderPlacement),
    ) -> Result<Self, HostError> {
        let (owner_sid, placement) = owner;
        let program = program.to_str().ok_or_else(|| HostError::UnsupportedPath {
            path: program.to_path_buf(),
        })?;
        let state = paths
            .state_dir()
            .to_str()
            .ok_or_else(|| HostError::UnsupportedPath {
                path: paths.state_dir().to_path_buf(),
            })?;
        let arguments = [
            program,
            "service",
            "run",
            "--session",
            paths.session().as_str(),
            "--state-dir",
            state,
            "--placement",
            placement.token(),
        ];
        let label = paths.session().label();
        Ok(Self {
            name: label.as_str().to_owned(),
            display_name: format!("ovis-terminal holder ({})", paths.session().as_str()),
            command_line: arguments.map(quote).join(" "),
            description: marker(label.as_str(), owner_sid),
        })
    }
}

/// The description marker of a service this runtime installed for `owner_sid`.
#[must_use]
pub fn marker(label: &str, owner_sid: &str) -> String {
    format!("ovis-terminal holder service; label={label}; owner={owner_sid}")
}

/// Quotes one argument so `CommandLineToArgvW` returns it unchanged.
#[must_use]
pub fn quote(argument: &str) -> String {
    if !argument.is_empty() && !argument.contains([' ', '\t', '\n', '"']) {
        return argument.to_owned();
    }
    let mut quoted = String::from('"');
    let mut backslashes = 0_usize;
    for character in argument.chars() {
        match character {
            '\\' => backslashes = backslashes.saturating_add(1),
            '"' => {
                quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2).saturating_add(1)));
                quoted.push('"');
                backslashes = 0;
            }
            other => {
                quoted.push_str(&"\\".repeat(backslashes));
                quoted.push(other);
                backslashes = 0;
            }
        }
    }
    quoted.push_str(&"\\".repeat(backslashes.saturating_mul(2)));
    quoted.push('"');
    quoted
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use ovis_terminal_protocol::domain::windows_service::HolderPlacement;
    use proptest::prelude::*;

    use super::{ServiceDefinition, marker, quote};
    use crate::service::windows::{HostPaths, HostSession};

    /// The argument splitting `CommandLineToArgvW` documents, for arguments after the first.
    fn split(line: &str) -> Vec<String> {
        let mut arguments = Vec::new();
        let mut chars = line.chars().peekable();
        loop {
            while chars.next_if(|c| *c == ' ' || *c == '\t').is_some() {}
            if chars.peek().is_none() {
                return arguments;
            }
            let (mut current, mut quoted, mut backslashes) = (String::new(), false, 0_usize);
            while let Some(c) = chars.next() {
                match c {
                    '\\' => backslashes = backslashes.saturating_add(1),
                    '"' => {
                        current.push_str(&"\\".repeat(backslashes / 2));
                        if backslashes % 2 == 1 || (quoted && chars.next_if_eq(&'"').is_some()) {
                            current.push('"');
                        } else {
                            quoted = !quoted;
                        }
                        backslashes = 0;
                    }
                    ' ' | '\t' if !quoted => break,
                    other => {
                        current.push_str(&"\\".repeat(backslashes));
                        backslashes = 0;
                        current.push(other);
                    }
                }
            }
            current.push_str(&"\\".repeat(backslashes));
            arguments.push(current);
        }
    }

    proptest! {
        #[test]
        fn every_quoted_argument_splits_back_unchanged(args in proptest::collection::vec("[a-z \\\\\"\t]{0,12}", 1..6)) {
            let line = args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ");
            prop_assert_eq!(split(&line), args);
        }
    }

    #[test]
    fn the_definition_runs_this_program_as_the_session_holder_and_names_its_owner() {
        let state = std::env::temp_dir().join("state dir");
        let paths = HostPaths::new(HostSession::parse("ci").unwrap(), state.clone()).unwrap();
        let program = PathBuf::from("/opt/ovis terminal/ovis-terminal");
        let owner = ("S-1-5-21-4", HolderPlacement::Dedicated);
        let definition = ServiceDefinition::new(&paths, &program, owner).unwrap();
        assert_eq!(definition.name, "ovis-terminal-holder-ci");
        assert_eq!(
            split(&definition.command_line),
            [
                program.to_str().unwrap(),
                "service",
                "run",
                "--session",
                "ci",
                "--state-dir",
                state.to_str().unwrap(),
                "--placement",
                "dedicated"
            ]
        );
        assert_eq!(
            definition.description,
            marker("ovis-terminal-holder-ci", "S-1-5-21-4")
        );
        assert_ne!(
            definition.description,
            marker("ovis-terminal-holder-ci", "S-1-5-21-5")
        );
    }
}
