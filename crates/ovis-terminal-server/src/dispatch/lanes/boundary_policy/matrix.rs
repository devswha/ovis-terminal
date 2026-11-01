//! The mutation-role matrix: every published mutation and the action it performs.
//!
//! A test generates the mutation set from the OpenRPC document and requires this table to
//! name exactly those methods, so a new mutation cannot ship without a role. Each action
//! is the one its domain lane authorizes; the gate only refuses a session whose
//! credential confers that action's permission nowhere.

use ovis_terminal_authz::Action;

/// Each published mutation and the action it performs.
pub const MUTATION_ROLES: &[(&str, Action)] = &[
    ("agent.append_records", Action::Report),
    ("agent.note", Action::Report),
    ("agent.register", Action::Report),
    ("agent.release", Action::Report),
    ("agent.update_session", Action::Report),
    ("attach.focus", Action::Resize),
    ("attach.open", Action::Subscribe),
    ("attach.release_geometry", Action::Resize),
    ("attach.resize", Action::Resize),
    ("attach.takeover", Action::Takeover),
    ("credential.revoke", Action::RevokeCredential),
    ("credential.rotate", Action::IssueCredential),
    ("doc.set", Action::DocumentWrite),
    ("input.interrupt", Action::Input),
    ("input.keys", Action::Input),
    ("input.paste", Action::Input),
    ("input.raw", Action::RawInput),
    ("input.send", Action::Input),
    ("layout.edit", Action::LayoutChange),
    ("machine.add", Action::Administer),
    ("machine.exec", Action::Administer),
    ("machine.file", Action::Administer),
    ("machine.login", Action::Administer),
    ("machine.rename", Action::Administer),
    ("machine.rm", Action::Administer),
    ("op.cancel", Action::CancelOperation),
    ("plugin.invoke", Action::PluginInvoke),
    ("prompt.answer", Action::PromptAnswer),
    ("prompt.close", Action::Report),
    ("prompt.open", Action::Report),
    ("selection.set", Action::LayoutChange),
    ("server.restart", Action::Administer),
    ("server.start", Action::Administer),
    ("server.stop", Action::Administer),
    ("service.install", Action::Administer),
    ("service.remove", Action::Administer),
    ("status.clear", Action::Report),
    ("status.report", Action::Report),
    ("terminal.keep", Action::Input),
    ("terminal.kill", Action::Input),
    ("terminal.label", Action::LayoutChange),
    ("terminal.recover", Action::Launch),
    ("terminal.rename", Action::LayoutChange),
    ("terminal.rm", Action::LayoutChange),
    ("terminal.run", Action::Launch),
    ("update.activate", Action::Administer),
    ("update.rollback", Action::Administer),
    ("update.stage", Action::Administer),
    ("update.verify", Action::Administer),
    ("worktree.create", Action::LayoutChange),
    ("worktree.open", Action::LayoutChange),
    ("worktree.remove", Action::Administer),
];

/// The action of mutation `method`; `None` for a query, a control call or an unknown name.
#[must_use]
pub fn required_action(method: &str) -> Option<Action> {
    MUTATION_ROLES
        .binary_search_by(|(name, _)| (*name).cmp(method))
        .ok()
        .and_then(|index| MUTATION_ROLES.get(index))
        .map(|(_, action)| *action)
}
