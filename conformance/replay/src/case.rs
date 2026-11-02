//! The closed replay case format `ovis-replay/1`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::expect::Expect;

/// Marker of a replay case file.
pub(crate) const FORMAT: &str = "ovis-replay/1";

/// Identity of a replay case, also its file stem below `cases/`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct CaseId(String);

impl CaseId {
    /// The identity as written in the fixture.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A client connection a case names; each label is one persistent connection.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[non_exhaustive]
pub struct ConnLabel(String);

impl ConnLabel {
    /// The label as written in the fixture.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The server a case runs against, each in a fresh isolated root.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Server {
    /// `ovis-terminal serve`: the binary under test.
    Assembled,
    /// The conformance protocol fixture: the real server crates with a fixture agent.
    Protocol,
}

/// Whether a case proves an accepted journey or a refusal that changes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CaseKind {
    /// Runs in the runner's `happy` case.
    Happy,
    /// Runs in the runner's `failure` case; its `inject` steps answer `--inject`.
    Failure,
}

/// The plan's adversarial classes; together the cases must represent all nine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Adversarial {
    /// Typos, unknown fields and wrong types.
    MalformedInput,
    /// Stale revisions, leases and cursors.
    StaleState,
    /// A refusal that must not look like, or leave traces of, success.
    MisleadingSuccess,
    /// The same operation resent after an interruption.
    RepeatedInterruptions,
    /// A held or cancelled action resumed after its boundary.
    CancelResume,
    /// A wait whose condition never happens ends at its bound.
    HungCommands,
    /// Instruction- or prompt-like bytes stay data.
    PromptInjection,
    /// Order and outcome never depend on timing.
    FlakyTests,
    /// State starts from a fresh isolated root, never a leftover one.
    DirtyWorktree,
}

impl Adversarial {
    /// Every class, in declaration order.
    pub const ALL: [Self; 9] = [
        Self::MalformedInput,
        Self::StaleState,
        Self::MisleadingSuccess,
        Self::RepeatedInterruptions,
        Self::CancelResume,
        Self::HungCommands,
        Self::PromptInjection,
        Self::FlakyTests,
        Self::DirtyWorktree,
    ];
}

/// One replay case.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Case {
    /// Always `ovis-replay/1`.
    pub format: String,
    /// Case identity.
    pub id: CaseId,
    /// Server the case runs against.
    pub server: Server,
    /// Accepted journey or refusal.
    pub kind: CaseKind,
    /// IDEAL rows the case proves.
    pub ideals: Vec<String>,
    /// GAP rows the case closes.
    pub gaps: Vec<String>,
    /// EXCL rows whose negative the case enforces.
    pub excludes: Vec<String>,
    /// Adversarial classes the case represents.
    pub adversarial: Vec<Adversarial>,
    /// Steps in order.
    pub steps: Vec<Step>,
}

impl Case {
    /// Whether `request` (`{"method", "params"}`) is one of this case's inject steps.
    #[must_use]
    pub fn plans(&self, request: &Value) -> bool {
        self.steps.iter().any(|step| step.injects(request))
    }
}

/// One step; exactly one key per step object.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
#[non_exhaustive]
pub enum Step {
    /// A published method, sent as written.
    Call(Call),
    /// A herdr-era call Ovis makes today, translated by the legacy map before sending.
    Legacy(Call),
    /// Await one notification on a connection.
    Event(EventStep),
}

impl Step {
    /// Whether this is an inject step.
    pub(crate) const fn is_inject(&self) -> bool {
        match self {
            Self::Call(call) | Self::Legacy(call) => call.inject,
            Self::Event(_) => false,
        }
    }

    /// Whether this is an inject step planning exactly `request`.
    pub(crate) fn injects(&self, request: &Value) -> bool {
        match self {
            Self::Call(call) | Self::Legacy(call) => {
                call.inject
                    && request.get("method") == Some(&Value::String(call.method.clone()))
                    && request.get("params") == Some(&call.params)
            }
            Self::Event(_) => false,
        }
    }
}

/// A request and the reply it must get.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Call {
    /// Connection that sends it.
    pub conn: ConnLabel,
    /// Step identity, also the JSON-RPC id where the transport keeps ids.
    pub id: String,
    /// Method as the consumer writes it.
    pub method: String,
    /// Params as the consumer writes them, before binding.
    pub params: Value,
    /// Whether `--inject` may select this step; unselected inject steps are skipped.
    #[serde(default)]
    pub inject: bool,
    /// Checks over the reply.
    pub expect: Expect,
}

/// A notification a connection must receive.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct EventStep {
    /// Connection that receives it.
    pub conn: ConnLabel,
    /// Notification method, such as `event`.
    pub method: String,
    /// Checks over `{"method", "params"}`.
    pub expect: Expect,
}
