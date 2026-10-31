//! `service.describe` over Windows holder service registrations (plan todo 54).
//!
//! Describing reads the service control manager's view of the services this runtime
//! installed for its owner and the pipe each one serves; it never installs, starts or
//! repairs a service, and a same-named service someone else installed is not described.
//! The served binary that registers this lane is assembled by plan todo 31.

use ovis_terminal_protocol::domain::windows_service::PipeName;
use ovis_terminal_protocol::schema::ids::ServiceLabel;
use ovis_terminal_protocol::schema::methods::service::{ServiceDescribeResult, ServiceRecord};

/// The registrations of `installed`, each with the pipe its holder serves.
#[must_use]
pub fn describe(installed: &[(ServiceLabel, PipeName)]) -> ServiceDescribeResult {
    let services = installed
        .iter()
        .map(|(label, pipe)| ServiceRecord::pipe_registration(label.clone(), pipe))
        .collect();
    ServiceDescribeResult::new(services)
}

#[cfg(windows)]
pub use native::ServiceDescribe;

#[cfg(windows)]
mod native {
    use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
    use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
    use ovis_terminal_core::registry::MachineId;
    use ovis_terminal_holder::service::windows::{HostError, HostPaths, installation};
    use ovis_terminal_protocol::domain::windows_service::PipeName;
    use ovis_terminal_protocol::schema::common::NoParams;
    use ovis_terminal_protocol::schema::ids::ServiceLabel;

    use super::describe;
    use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

    /// Describes the Windows holder services of a fixed set of sessions for one owner.
    #[derive(Debug, Clone)]
    pub struct ServiceDescribe {
        machine: MachineId,
        owner_sid: String,
        sessions: Vec<HostPaths>,
    }

    impl ServiceDescribe {
        /// Describes `sessions` installed for `owner_sid` on `machine`.
        #[must_use]
        pub const fn new(machine: MachineId, owner_sid: String, sessions: Vec<HostPaths>) -> Self {
            Self {
                machine,
                owner_sid,
                sessions,
            }
        }
    }

    fn installed(
        sessions: &[HostPaths],
        owner_sid: &str,
    ) -> Result<Vec<(ServiceLabel, PipeName)>, HostError> {
        let mut found = Vec::new();
        for paths in sessions {
            match installation(paths, owner_sid) {
                Ok(Some(_)) => found.push((paths.session().label(), paths.pipe(owner_sid)?)),
                Ok(None) | Err(HostError::ForeignRegistration { .. }) => {}
                Err(error) => return Err(error),
            }
        }
        Ok(found)
    }

    impl Method<OperationEvent, OperationProducer> for ServiceDescribe {
        const NAME: &'static str = "service.describe";
        type Params = NoParams;

        fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
            Ok(Access::new(
                Action::Read,
                ResolvedTarget::new(self.machine, TargetResource::Machine),
            ))
        }

        async fn run(
            &self,
            _: Self::Params,
            execution: Execution<OperationEvent, OperationProducer>,
        ) -> Result<Completed, DispatchError> {
            let (sessions, owner) = (self.sessions.clone(), self.owner_sid.clone());
            execution
                .commit(move || {
                    installed(&sessions, &owner)
                        .map(|found| describe(&found))
                        .map_err(|error| std::io::Error::other(error).into())
                })
                .await
        }
    }

    #[cfg(test)]
    mod tests {
        use ovis_terminal_holder::service::windows::{HostPaths, HostSession, pipe};

        #[test]
        fn a_session_without_an_installed_service_is_not_described_and_nothing_is_created() {
            let state =
                std::env::temp_dir().join(format!("ovis-t54-describe-{}", std::process::id()));
            let session = HostSession::parse("t54-never-installed").unwrap();
            let paths = HostPaths::new(session, state.clone()).unwrap();
            let owner = pipe::current_user_sid().unwrap();
            assert_eq!(super::installed(&[paths], &owner).unwrap(), Vec::new());
            assert!(!state.exists(), "describing created the state directory");
        }
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::domain::windows_service::PipeName;
    use ovis_terminal_protocol::schema::ids::ServiceLabel;
    use serde_json::json;

    use super::describe;

    #[test]
    fn each_registration_names_the_pipe_its_holder_serves_and_nothing_is_invented() {
        assert_eq!(
            serde_json::to_value(describe(&[])).unwrap(),
            json!({"services": []})
        );
        let pipe = PipeName::parse(r"\\.\pipe\ovis-terminal-holder-S-1-5-21-1-ci").unwrap();
        let label = ServiceLabel::new("ovis-terminal-holder-ci".to_owned());
        let described = serde_json::to_value(describe(&[(label, pipe)])).unwrap();
        assert_eq!(
            described,
            json!({"services": [{"label": "ovis-terminal-holder-ci", "manager": "windows_service",
                "endpoint": {"transport": "pipe", "address": r"\\.\pipe\ovis-terminal-holder-S-1-5-21-1-ci", "server_boot_id": null},
                "missing_prerequisites": []}]})
        );
    }
}
