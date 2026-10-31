//! Permissions a grant confers, the roles that bundle them and the actions they cover.

use std::fmt;

use serde::{Deserialize, Serialize};

/// One explicitly granted capability; no permission implies another.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Permission {
    /// Read state, screens and history; subscribe to events; attach as an observer.
    Observe,
    /// Send ordinary input, resize, launch, change layout and write documents.
    Interact,
    /// Send raw bytes, including stream DATA frames, that bypass prompt protection.
    RawInput,
    /// Displace the current controller of a terminal.
    Takeover,
    /// Answer a versioned prompt.
    PromptAnswer,
    /// Publish agent status and prompt reports.
    Report,
    /// Administer the machine: lifecycle, services, updates and credentials.
    Admin,
    /// Invoke plugin actions.
    Plugin,
}

impl Permission {
    const ALL: [Self; 8] = [
        Self::Observe,
        Self::Interact,
        Self::RawInput,
        Self::Takeover,
        Self::PromptAnswer,
        Self::Report,
        Self::Admin,
        Self::Plugin,
    ];

    /// The wire token of the permission.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Observe => "observe",
            Self::Interact => "interact",
            Self::RawInput => "raw_input",
            Self::Takeover => "takeover",
            Self::PromptAnswer => "prompt_answer",
            Self::Report => "report",
            Self::Admin => "admin",
            Self::Plugin => "plugin",
        }
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A named bundle of permissions for issuing common credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Role {
    /// Read-only access.
    Observer,
    /// Observation plus ordinary input; no takeover, prompt answers or raw input.
    Interactor,
    /// Every permission; held by the runtime owner's bootstrap credential.
    Owner,
}

impl Role {
    /// The permissions the role bundles.
    #[must_use]
    pub const fn permissions(self) -> &'static [Permission] {
        match self {
            Self::Observer => &[Permission::Observe],
            Self::Interactor => &[Permission::Observe, Permission::Interact],
            Self::Owner => &Permission::ALL,
        }
    }
}

/// An operation class a request or stream frame performs; each needs one permission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Action {
    /// Read state, a screen, history or a document.
    Read,
    /// Subscribe to events or attach an observing stream.
    Subscribe,
    /// Send text, keys or paste through the ordered input path.
    Input,
    /// Send raw bytes or a client DATA frame on a stream.
    RawInput,
    /// Change terminal geometry.
    Resize,
    /// Launch a program or create a terminal.
    Launch,
    /// Change workspaces, tabs, panes or splits.
    LayoutChange,
    /// Write a shared document.
    DocumentWrite,
    /// Take over the controller lease of a terminal.
    Takeover,
    /// Answer a prompt.
    PromptAnswer,
    /// Publish a status or prompt report.
    Report,
    /// Stop, restart, install, update or reconfigure the runtime or a machine.
    Administer,
    /// Invoke a plugin action.
    PluginInvoke,
    /// Issue a credential.
    IssueCredential,
    /// Revoke a credential.
    RevokeCredential,
}

impl Action {
    /// The permission a grant must confer for this action.
    #[must_use]
    pub const fn required(self) -> Permission {
        match self {
            Self::Read | Self::Subscribe => Permission::Observe,
            Self::Input
            | Self::Resize
            | Self::Launch
            | Self::LayoutChange
            | Self::DocumentWrite => Permission::Interact,
            Self::RawInput => Permission::RawInput,
            Self::Takeover => Permission::Takeover,
            Self::PromptAnswer => Permission::PromptAnswer,
            Self::Report => Permission::Report,
            Self::Administer | Self::IssueCredential | Self::RevokeCredential => Permission::Admin,
            Self::PluginInvoke => Permission::Plugin,
        }
    }

    /// The stable token of the action used in errors and audit records.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Subscribe => "subscribe",
            Self::Input => "input",
            Self::RawInput => "raw_input",
            Self::Resize => "resize",
            Self::Launch => "launch",
            Self::LayoutChange => "layout_change",
            Self::DocumentWrite => "document_write",
            Self::Takeover => "takeover",
            Self::PromptAnswer => "prompt_answer",
            Self::Report => "report",
            Self::Administer => "administer",
            Self::PluginInvoke => "plugin_invoke",
            Self::IssueCredential => "issue_credential",
            Self::RevokeCredential => "revoke_credential",
        }
    }
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::{Permission, Role};

    #[test]
    fn permission_tokens_match_their_wire_encoding() -> Result<(), serde_json::Error> {
        for permission in Permission::ALL {
            let encoded = serde_json::to_value(permission)?;
            assert_eq!(encoded, serde_json::Value::from(permission.as_str()));
        }
        Ok(())
    }

    #[test]
    fn only_the_owner_role_bundles_takeover_prompt_answer_and_raw_input() {
        let guarded = [
            Permission::Takeover,
            Permission::PromptAnswer,
            Permission::RawInput,
        ];
        let holders = [Role::Observer, Role::Interactor, Role::Owner].map(|role| {
            guarded
                .iter()
                .all(|permission| role.permissions().contains(permission))
        });

        assert_eq!(holders, [false, false, true]);
    }
}
