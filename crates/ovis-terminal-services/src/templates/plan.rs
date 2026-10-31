//! The launch a template or a direct argument vector resolves to.

use std::collections::BTreeMap;

use ovis_terminal_protocol::domain::template_launch::{DEFAULT_READY_DEADLINE, ReadinessContract};
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::DurationMs;

use super::Template;

/// A first message to queue under the launch until the program declares readiness.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PlannedMessage {
    /// Literal text, submitted once.
    pub text: String,
    /// How long readiness may take before the message expires.
    pub deadline: DurationMs,
}

/// One launch: literal argv and environment, readiness states and the first message.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct LaunchPlan {
    /// Executable followed by literal arguments; never passed through a shell.
    pub argv: Vec<String>,
    /// Environment: template entries, then the launch's own additions.
    pub env: BTreeMap<String, String>,
    /// Declared states that release the first message.
    pub readiness: Vec<AgentState>,
    /// The first message, when the launch or template has one.
    pub first_message: Option<PlannedMessage>,
}

impl LaunchPlan {
    /// A direct program: the argument vector exactly as given, `idle` readiness.
    #[must_use]
    pub fn direct(argv: &[String], params: &TerminalRunParams) -> Self {
        Self {
            argv: argv.to_vec(),
            env: params.env.clone().unwrap_or_default(),
            readiness: ReadinessContract::idle().states(),
            first_message: params.first_message.as_ref().map(|first| PlannedMessage {
                text: first.text.clone(),
                deadline: first.deadline_ms.unwrap_or(DEFAULT_READY_DEADLINE),
            }),
        }
    }
}

impl Template {
    /// Resolves the template for one launch.
    ///
    /// The argument vector is the template's, unchanged. The first message is the
    /// template body, then `task`, then the launch's own first-message text, each kept
    /// literal; the launch's deadline wins over the template's.
    #[must_use]
    pub fn plan(&self, params: &TerminalRunParams, task: Option<&str>) -> LaunchPlan {
        let mut env = self.env.clone();
        env.extend(params.env.clone().unwrap_or_default());
        let first = params.first_message.as_ref();
        let parts: Vec<&str> = [
            self.body.as_deref(),
            task,
            first.map(|first| first.text.as_str()),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect();
        let first_message = (!parts.is_empty()).then(|| PlannedMessage {
            text: parts.join("\n\n"),
            deadline: first
                .and_then(|first| first.deadline_ms)
                .unwrap_or(self.ready_deadline),
        });
        LaunchPlan {
            argv: self.argv.clone(),
            env,
            readiness: self.readiness.states(),
            first_message,
        }
    }
}
