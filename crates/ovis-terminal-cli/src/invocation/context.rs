//! What an invocation reads from its environment, read once at start-up.

use std::ffi::{OsStr, OsString};
use std::fmt;

use ovis_terminal_server::listen::EndpointEnvironment;

use super::error::CliError;

/// Variable that selects JSON output when set to `1`.
pub const JSON_ENV: &str = "OVIS_JSON";
/// Variable holding the bearer credential presented in `hello`.
pub const CREDENTIAL_ENV: &str = "OVIS_TERMINAL_CREDENTIAL";
/// Variable the runtime sets for a program it spawned: the token that proves the program
/// owns its terminal when it reports status, prompts or records.
pub const PROCESS_TOKEN_ENV: &str = "OVIS_TERMINAL_PROCESS_TOKEN";

/// A bearer credential or process token; its text never appears in Debug output.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential(String);

impl Credential {
    /// Wraps bearer text.
    #[must_use]
    pub const fn new(bearer: String) -> Self {
        Self(bearer)
    }

    pub(super) fn bearer(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Credential(<redacted>)")
    }
}

/// The environment of one invocation.
#[derive(Debug, Clone, Default)]
pub struct Context {
    endpoint: EndpointEnvironment,
    credential: Option<Credential>,
    process_token: Option<Credential>,
    json_switch: Option<OsString>,
}

impl Context {
    /// Reads the endpoint variables, the credential and the JSON switch of this process.
    #[must_use]
    pub fn from_process() -> Self {
        Self {
            endpoint: EndpointEnvironment::from_process(),
            credential: std::env::var(CREDENTIAL_ENV)
                .ok()
                .filter(|bearer| !bearer.is_empty())
                .map(Credential::new),
            process_token: std::env::var(PROCESS_TOKEN_ENV)
                .ok()
                .filter(|token| !token.is_empty())
                .map(Credential::new),
            json_switch: std::env::var_os(JSON_ENV),
        }
    }

    pub(super) const fn endpoint(&self) -> &EndpointEnvironment {
        &self.endpoint
    }

    pub(super) const fn credential(&self) -> Option<&Credential> {
        self.credential.as_ref()
    }

    /// The process token this invocation inherited from the terminal it runs in.
    pub(super) fn process_token(&self) -> Option<&str> {
        self.process_token.as_ref().map(Credential::bearer)
    }
}

/// How results and errors are written; never chosen by whether stdout is a terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OutputMode {
    Json,
    Text,
}

impl OutputMode {
    /// `--json` before any `--` separator, or `OVIS_JSON=1`, selects JSON; `OVIS_JSON=0`
    /// or an unset or empty variable leaves text.
    ///
    /// The raw arguments are scanned so that a command line clap rejects still gets
    /// its error in the requested form.
    pub(super) fn select(args: &[OsString], context: &Context) -> Result<Self, CliError> {
        let flagged = args
            .iter()
            .take_while(|arg| arg.as_os_str() != OsStr::new("--"))
            .any(|arg| arg.as_os_str() == OsStr::new("--json"));
        let switched = match context.json_switch.as_deref().map(OsStr::to_str) {
            None | Some(Some("" | "0")) => false,
            Some(Some("1")) => true,
            Some(_) => {
                return Err(CliError::JsonSwitch {
                    mode: if flagged { Self::Json } else { Self::Text },
                });
            }
        };
        Ok(if flagged || switched {
            Self::Json
        } else {
            Self::Text
        })
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;

    use super::{Context, Credential, OutputMode};

    fn context(switch: Option<&str>) -> Context {
        Context {
            json_switch: switch.map(OsString::from),
            ..Context::default()
        }
    }

    fn args(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[test]
    fn json_comes_from_the_flag_or_the_switch_and_never_from_the_terminal() {
        let select = |values: &[&str], switch| OutputMode::select(&args(values), &context(switch));
        assert_eq!(select(&["schema"], None).ok(), Some(OutputMode::Text));
        assert_eq!(
            select(&["--json", "schema"], None).ok(),
            Some(OutputMode::Json)
        );
        assert_eq!(
            select(&["schema", "--json"], Some("0")).ok(),
            Some(OutputMode::Json)
        );
        assert_eq!(select(&["schema"], Some("1")).ok(), Some(OutputMode::Json));
        assert_eq!(select(&["schema"], Some("")).ok(), Some(OutputMode::Text));
        assert_eq!(
            select(&["call", "x", "--", "--json"], None).ok(),
            Some(OutputMode::Text)
        );
    }

    #[test]
    fn an_unrecognized_switch_value_is_refused_in_the_requested_form() {
        let refused = OutputMode::select(&args(&["--json"]), &context(Some("yes")));
        assert!(matches!(
            refused,
            Err(crate::invocation::error::CliError::JsonSwitch {
                mode: OutputMode::Json
            })
        ));
    }

    #[test]
    fn a_credential_never_prints_its_bearer_text() {
        let credential = Credential::new("secret-bearer".to_owned());
        assert!(!format!("{credential:?}").contains("secret-bearer"));
    }
}
