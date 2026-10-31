//! Parsed transport specifications and the exact command each one runs.

use ovis_terminal_protocol::schema::methods::machine::MachineTransport;
use tokio::process::Command;

use super::TransportError;

/// The fixed command the far side runs; it bridges stdio to the remote runtime's endpoint.
const REMOTE_COMMAND: [&str; 2] = ["ovis-terminal", "connect"];
const MAX_DESTINATION_BYTES: usize = 255;

/// A validated way to reach a machine.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TransportSpec {
    /// `ssh` without a terminal, prompts or escape character to a validated destination.
    Ssh(String),
    /// A program and its arguments, run without a shell.
    Argv(Vec<String>),
}

fn ssh_destination(text: &str) -> Result<String, TransportError> {
    let allowed = |byte: u8| byte.is_ascii_alphanumeric() || b"._-:@".contains(&byte);
    let user_and_host = text
        .split_once('@')
        .is_none_or(|(user, host)| !user.is_empty() && !host.is_empty() && !host.contains('@'));
    let valid = !text.is_empty()
        && text.len() <= MAX_DESTINATION_BYTES
        && !text.starts_with('-')
        && text.bytes().all(allowed)
        && user_and_host;
    if valid {
        Ok(text.to_owned())
    } else {
        Err(TransportError::InvalidSpec {
            reason: "an ssh destination is [user@]host of letters, digits, '.', '_', '-', ':'",
        })
    }
}

impl TransportSpec {
    /// Parses the transport of a `machine.add` request.
    ///
    /// # Errors
    ///
    /// [`TransportError::InvalidSpec`] for an unsafe SSH destination, an empty argument
    /// vector, an argument with a NUL byte, or a transport this build does not know.
    pub fn parse(transport: &MachineTransport) -> Result<Self, TransportError> {
        match transport {
            MachineTransport::Ssh(destination) => ssh_destination(destination).map(Self::Ssh),
            MachineTransport::Argv(argv) => {
                let program_named = argv.first().is_some_and(|program| !program.is_empty());
                if program_named && argv.iter().all(|arg| !arg.contains('\0')) {
                    Ok(Self::Argv(argv.clone()))
                } else {
                    Err(TransportError::InvalidSpec {
                        reason: "an argument vector names a program and has no NUL bytes",
                    })
                }
            }
            _ => Err(TransportError::InvalidSpec {
                reason: "this build supports ssh and argv transports",
            }),
        }
    }

    /// The program and arguments this transport runs, exactly as passed to the OS.
    #[must_use]
    pub fn argv(&self) -> Vec<String> {
        match self {
            Self::Ssh(destination) => [
                "ssh",
                "-T",
                "-e",
                "none",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "--",
                destination,
            ]
            .into_iter()
            .chain(REMOTE_COMMAND)
            .map(str::to_owned)
            .collect(),
            Self::Argv(argv) => argv.clone(),
        }
    }

    pub(super) fn command(&self) -> Option<Command> {
        let argv = self.argv();
        let (program, arguments) = argv.split_first()?;
        let mut command = Command::new(program);
        command.args(arguments);
        Some(command)
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::schema::methods::machine::MachineTransport;

    use super::TransportSpec;

    fn ssh(destination: &str) -> Result<Vec<String>, &'static str> {
        TransportSpec::parse(&MachineTransport::Ssh(destination.to_owned()))
            .map(|spec| spec.argv())
            .map_err(|error| error.kind())
    }

    #[test]
    fn ssh_runs_a_fixed_remote_command_without_prompts() {
        let argv = ssh("me@build-01.example").expect("valid destination");

        assert_eq!(
            argv,
            [
                "ssh",
                "-T",
                "-e",
                "none",
                "-o",
                "BatchMode=yes",
                "-o",
                "StrictHostKeyChecking=yes",
                "--",
                "me@build-01.example",
                "ovis-terminal",
                "connect",
            ]
        );
    }

    #[test]
    fn destinations_that_a_shell_or_ssh_would_reinterpret_are_refused() {
        let hostile = [
            "",
            "-oProxyCommand=id",
            "box;id",
            "box id",
            "$(id)",
            "`id`",
            "box\nid",
            "@box",
            "me@",
            "a@b@c",
            "box|id",
            "box&",
            "box>f",
            "b\u{e9}",
        ];
        for destination in hostile {
            assert_eq!(ssh(destination), Err("invalid_params"), "{destination:?}");
        }
    }

    #[test]
    fn argv_is_kept_verbatim_and_never_joined_into_a_shell_line() {
        let argv = vec![
            "relay".to_owned(),
            "--to".to_owned(),
            "a b;$(id)".to_owned(),
        ];
        let spec = TransportSpec::parse(&MachineTransport::Argv(argv.clone())).expect("argv");

        assert_eq!(spec.argv(), argv);
        for empty in [vec![], vec![String::new()], vec!["x\0y".to_owned()]] {
            let refused = TransportSpec::parse(&MachineTransport::Argv(empty));
            assert_eq!(refused.map_err(|error| error.kind()), Err("invalid_params"));
        }
    }
}
