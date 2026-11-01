//! The method, params type and flag shape of every agent-domain verb.

use ovis_terminal_protocol::domain::conversation_records::{
    AgentRecordsParams, AppendRecordsParams,
};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::methods::agent::{
    AgentConversationParams, AgentHistoryParams, AgentListParams, AgentNoteParams,
    AgentRegisterParams, AgentReleaseParams, AgentTargetParams, AgentUpdateSessionParams,
    AgentWaitChildrenParams,
};
use ovis_terminal_protocol::schema::methods::status::{
    PromptAnswerParams, PromptCloseParams, PromptGetParams, PromptListParams, PromptOpenParams,
    StatusClearParams, StatusReportParams, StatusTargetParams, TurnWaitParams,
};
use ovis_terminal_protocol::schema::methods::template::TemplateGetParams;

use crate::commands::{Check, TargetUse, VerbSpec, check};

const fn spec(method: &'static str, check: Check, (token, target): (bool, TargetUse)) -> VerbSpec {
    VerbSpec {
        method,
        check,
        token,
        target,
    }
}

/// Every typed verb of the agent domain, in catalog order.
pub(crate) const VERBS: [VerbSpec; 24] = [
    spec(
        "agent.register",
        check::<AgentRegisterParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "agent.get",
        check::<AgentTargetParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.list",
        check::<AgentListParams>,
        (false, TargetUse::Absent),
    ),
    spec(
        "agent.update_session",
        check::<AgentUpdateSessionParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "agent.release",
        check::<AgentReleaseParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "agent.children",
        check::<AgentTargetParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.note",
        check::<AgentNoteParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.wait_children",
        check::<AgentWaitChildrenParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.conversation",
        check::<AgentConversationParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.append_records",
        check::<AppendRecordsParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "agent.records",
        check::<AgentRecordsParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "agent.history",
        check::<AgentHistoryParams>,
        (false, TargetUse::Absent),
    ),
    spec(
        "status.get",
        check::<StatusTargetParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "status.report",
        check::<StatusReportParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "status.clear",
        check::<StatusClearParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "prompt.list",
        check::<PromptListParams>,
        (false, TargetUse::Optional),
    ),
    spec(
        "prompt.get",
        check::<PromptGetParams>,
        (false, TargetUse::Absent),
    ),
    spec(
        "prompt.open",
        check::<PromptOpenParams>,
        (true, TargetUse::Required),
    ),
    spec(
        "prompt.answer",
        check::<PromptAnswerParams>,
        (false, TargetUse::Absent),
    ),
    spec(
        "prompt.close",
        check::<PromptCloseParams>,
        (true, TargetUse::Absent),
    ),
    spec(
        "turn.get",
        check::<StatusTargetParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "turn.wait",
        check::<TurnWaitParams>,
        (false, TargetUse::Required),
    ),
    spec(
        "template.list",
        check::<NoParams>,
        (false, TargetUse::Absent),
    ),
    spec(
        "template.get",
        check::<TemplateGetParams>,
        (false, TargetUse::Absent),
    ),
];
