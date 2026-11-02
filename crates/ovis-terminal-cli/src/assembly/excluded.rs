//! Published methods this runtime explicitly does not serve. The released baseline keeps
//! them in the OpenRPC document, so removing them would break `openrpc compat`. Hello does
//! not advertise them, and a call answers the typed `not_implemented` refusal. Its hint
//! names the follow-up that would serve the method, never `method_not_found`.

use ovis_terminal_protocol::errors::BoundaryError;

use super::error::refusal;

/// One published method the assembled daemon refuses as not implemented.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum Excluded {
    /// `agent.note`: no child-to-parent note store exists.
    #[error("agent.note is published but not served by this runtime")]
    AgentNote,
    /// `agent.release`: the status service cannot unbind a program.
    #[error("agent.release is published but not served by this runtime")]
    AgentRelease,
    /// `agent.wait_children`: no per-parent news cursor exists.
    #[error("agent.wait_children is published but not served by this runtime")]
    AgentWaitChildren,
    /// `doc.list`: the store has no listing by resource kind.
    #[error("doc.list is published but not served by this runtime")]
    DocList,
    /// `machine.exec`: the remote lane has no bounded exec.
    #[error("machine.exec is published but not served by this runtime")]
    MachineExec,
    /// `machine.file`: the remote lane has no scoped file transfer.
    #[error("machine.file is published but not served by this runtime")]
    MachineFile,
    /// `machine.login`: the remote lane has no interactive login.
    #[error("machine.login is published but not served by this runtime")]
    MachineLogin,
    /// `machine.rm`: the remote lane cannot detach a machine and tombstone its terminals.
    #[error("machine.rm is published but not served by this runtime")]
    MachineRm,
    /// `service.install`: the daemon does not register itself with a service manager.
    #[error("service.install is published but not served by this runtime")]
    ServiceInstall,
    /// `service.remove`: the daemon does not remove a service registration.
    #[error("service.remove is published but not served by this runtime")]
    ServiceRemove,
}

impl Excluded {
    /// Every excluded method, in catalog name order.
    pub(super) const ALL: [Self; 10] = [
        Self::AgentNote,
        Self::AgentRelease,
        Self::AgentWaitChildren,
        Self::DocList,
        Self::MachineExec,
        Self::MachineFile,
        Self::MachineLogin,
        Self::MachineRm,
        Self::ServiceInstall,
        Self::ServiceRemove,
    ];

    /// The published method name.
    pub(super) const fn method(self) -> &'static str {
        match self {
            Self::AgentNote => "agent.note",
            Self::AgentRelease => "agent.release",
            Self::AgentWaitChildren => "agent.wait_children",
            Self::DocList => "doc.list",
            Self::MachineExec => "machine.exec",
            Self::MachineFile => "machine.file",
            Self::MachineLogin => "machine.login",
            Self::MachineRm => "machine.rm",
            Self::ServiceInstall => "service.install",
            Self::ServiceRemove => "service.remove",
        }
    }

    /// The excluded method named `method`, if it is one.
    pub(super) fn of(method: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|excluded| excluded.method() == method)
    }

    /// The stable error kind every excluded method answers.
    pub(super) const fn kind(self) -> &'static str {
        match self {
            Self::AgentNote
            | Self::AgentRelease
            | Self::AgentWaitChildren
            | Self::DocList
            | Self::MachineExec
            | Self::MachineFile
            | Self::MachineLogin
            | Self::MachineRm
            | Self::ServiceInstall
            | Self::ServiceRemove => "not_implemented",
        }
    }

    /// The follow-up that would serve the method, and what to use meanwhile.
    pub(super) const fn hint(self) -> &'static str {
        match self {
            Self::AgentNote => {
                "follow-up: agent notes lane (child-to-parent note store); use prompt.open and agent.children meanwhile"
            }
            Self::AgentRelease => {
                "follow-up: binding release in the agent status service; a binding ends when its program exits"
            }
            Self::AgentWaitChildren => {
                "follow-up: agent notes lane (per-parent news cursor); use agent.children with events.subscribe meanwhile"
            }
            Self::DocList => {
                "follow-up: store listing by resource kind; read known names with doc.get meanwhile"
            }
            Self::MachineExec => {
                "follow-up: bounded remote exec in the remote lane; launch with terminal.run on the machine meanwhile"
            }
            Self::MachineFile => "follow-up: scoped remote file transfer in the remote lane",
            Self::MachineLogin => {
                "follow-up: interactive remote login in the remote lane; add a machine with machine.add meanwhile"
            }
            Self::MachineRm => {
                "follow-up: machine detach with terminal tombstones in the remote lane; machine.list shows attached machines"
            }
            Self::ServiceInstall => {
                "follow-up: daemon-side service registration; run `ovis-terminal service install` locally meanwhile"
            }
            Self::ServiceRemove => {
                "follow-up: daemon-side service removal; run `ovis-terminal service uninstall` locally meanwhile"
            }
        }
    }

    /// The typed refusal a call of this method answers.
    pub(super) fn refusal(self) -> BoundaryError {
        refusal(self.kind(), self.hint())
    }
}

#[cfg(test)]
mod tests {
    use super::Excluded;

    #[test]
    fn every_excluded_method_resolves_to_itself_and_answers_not_implemented() {
        for excluded in Excluded::ALL {
            assert_eq!(Excluded::of(excluded.method()), Some(excluded));
            let error = excluded.refusal();
            assert_eq!(error.kind(), "not_implemented", "{}", excluded.method());
            assert_eq!(error.data().hint, excluded.hint(), "{}", excluded.method());
            assert!(
                excluded.hint().starts_with("follow-up: "),
                "{}",
                excluded.method()
            );
        }
    }

    #[test]
    fn served_and_unknown_methods_are_not_excluded() {
        for method in [
            "terminal.history",
            "events.ack",
            "agent.list",
            "server.start",
            "nope",
        ] {
            assert_eq!(Excluded::of(method), None, "{method}");
        }
    }

    #[test]
    fn the_excluded_set_is_sorted_and_distinct() {
        let names: Vec<&str> = Excluded::ALL
            .iter()
            .map(|excluded| excluded.method())
            .collect();
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(names, sorted);
    }
}
