//! Agent lanes: status, prompts (recovered from the log), session binding, reported
//! conversation records and the agent tree.

use std::sync::Arc;

use ovis_terminal_agent::conversation::binding::{BindingService, Stores};
use ovis_terminal_agent::conversation::records::RecordsService;
use ovis_terminal_agent::prompts::PromptService;
use ovis_terminal_agent::status::StatusService;
use ovis_terminal_agent::tree::TreeService;
use ovis_terminal_server::dispatch::lanes::agent_status::{
    AgentStatusLane, ClearStatus, GetAgent, GetStatus, RegisterAgent, ReportStatus,
};
use ovis_terminal_server::dispatch::lanes::agent_tree::{AgentTreeLane, ListChildren};
use ovis_terminal_server::dispatch::lanes::conversation_records::{
    AppendRecords, ConversationRecordsLane, Records,
};
use ovis_terminal_server::dispatch::lanes::prompts::{
    AnswerPrompt, ClosePrompt, GetPrompt, ListPrompts, OpenPrompt, PromptLane,
};
use ovis_terminal_server::dispatch::lanes::session_binding::{
    Conversation, SessionBindingLane, UpdateSession,
};

use super::{Dispatch, DispatchLog, Hosting, stage};
use crate::assembly::AssemblyError;

fn status_routes(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    status: &Arc<StatusService>,
) -> Result<(), AssemblyError> {
    let agents = AgentStatusLane::new(Arc::clone(&hosting.directory), Arc::clone(status));
    dispatch
        .register(RegisterAgent::new(&agents))
        .map_err(stage("register agent.register"))?;
    dispatch
        .register(GetAgent::new(&agents))
        .map_err(stage("register agent.get"))?;
    dispatch
        .register(ReportStatus::new(&agents))
        .map_err(stage("register status.report"))?;
    dispatch
        .register(GetStatus::new(&agents))
        .map_err(stage("register status.get"))?;
    dispatch
        .register(ClearStatus::new(&agents))
        .map_err(stage("register status.clear"))
}

fn prompt_routes(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    prompts: &Arc<PromptService>,
) -> Result<(), AssemblyError> {
    let machine = hosting.identity.machine_id();
    let lane = PromptLane::new(Arc::clone(&hosting.directory), Arc::clone(prompts), machine);
    dispatch
        .register(OpenPrompt::new(&lane))
        .map_err(stage("register prompt.open"))?;
    dispatch
        .register(AnswerPrompt::new(&lane))
        .map_err(stage("register prompt.answer"))?;
    dispatch
        .register(ClosePrompt::new(&lane))
        .map_err(stage("register prompt.close"))?;
    dispatch
        .register_read(GetPrompt::new(&lane))
        .map_err(stage("register prompt.get"))?;
    dispatch
        .register_read(ListPrompts::new(&lane))
        .map_err(stage("register prompt.list"))
}

/// Registers the agent lanes over one status service, with open prompts recovered.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
    log: &DispatchLog,
) -> Result<(), AssemblyError> {
    let runtime = hosting.identity.runtime_id();
    let status = Arc::clone(&hosting.status);
    status_routes(dispatch, hosting, &status)?;
    let prompts = Arc::new(PromptService::new(runtime, Arc::clone(&status)));
    log.read_view(|snapshot| prompts.recover(snapshot))
        .map_err(stage("recover open prompts"))?;
    prompt_routes(dispatch, hosting, &prompts)?;
    let tree = Arc::new(TreeService::new(
        hosting.identity,
        (Arc::clone(&status), Arc::clone(&prompts)),
    ));
    let children = AgentTreeLane::new(Arc::clone(&hosting.directory), tree);
    dispatch
        .register_read(ListChildren::new(&children))
        .map_err(stage("register agent.children"))?;
    let records = Arc::new(RecordsService::new(runtime, Arc::clone(&status)));
    let reported = ConversationRecordsLane::new(Arc::clone(&hosting.directory), records);
    dispatch
        .register(AppendRecords::new(&reported))
        .map_err(stage("register agent.append_records"))?;
    dispatch
        .register_read(Records::new(&reported))
        .map_err(stage("register agent.records"))?;
    let stores = Stores::new(&[]).map_err(stage("authorize session stores"))?;
    let binding = Arc::new(BindingService::new(runtime, status, stores));
    let sessions = SessionBindingLane::new(Arc::clone(&hosting.directory), binding);
    dispatch
        .register(UpdateSession::new(&sessions))
        .map_err(stage("register agent.update_session"))?;
    dispatch
        .register(Conversation::new(&sessions))
        .map_err(stage("register agent.conversation"))
}
