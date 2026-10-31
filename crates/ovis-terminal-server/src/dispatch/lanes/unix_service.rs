//! `service.describe` over Unix holder service registrations (plan todo 51).
//!
//! Describing reads only the definitions this runtime wrote and read-only manager
//! queries; it never installs, starts or repairs a service. The served binary that
//! registers this lane is assembled by plan todo 31.

use ovis_terminal_authz::{Action, ResolvedTarget, Session, TargetResource};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::MachineId;
use ovis_terminal_holder::service::unix::{ServiceError, ServicePaths, prerequisites, registered};
use ovis_terminal_protocol::schema::common::NoParams;
use ovis_terminal_protocol::schema::methods::service::{ServiceDescribeResult, ServiceRecord};

use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// Describes the holder service registrations of a fixed set of sessions.
#[derive(Debug, Clone)]
pub struct ServiceDescribe {
    machine: MachineId,
    sessions: Vec<ServicePaths>,
}

impl ServiceDescribe {
    /// Describes `sessions` on `machine`.
    #[must_use]
    pub const fn new(machine: MachineId, sessions: Vec<ServicePaths>) -> Self {
        Self { machine, sessions }
    }
}

/// Every registration among `sessions`, with its endpoint and unmet prerequisites.
///
/// # Errors
///
/// A definition file that cannot be read.
pub fn describe(sessions: &[ServicePaths]) -> Result<ServiceDescribeResult, ServiceError> {
    let mut services = Vec::new();
    for paths in sessions {
        let Some(manager) = registered(paths)? else {
            continue;
        };
        let address = paths.socket().display().to_string();
        let record = ServiceRecord::registration(paths.session().label(), manager, address);
        services.push(record.with_missing(&prerequisites(Some(manager))));
    }
    Ok(ServiceDescribeResult::new(services))
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
        let sessions = self.sessions.clone();
        execution
            .commit(move || {
                describe(&sessions).map_err(|error| std::io::Error::other(error).into())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_holder::service::unix::{
        ServiceEnvironment, ServicePaths, ServiceSession, ServiceVar,
    };

    use super::describe;

    fn paths(root: &std::path::Path, session: &str) -> ServicePaths {
        let environment = ServiceEnvironment::default()
            .with(ServiceVar::RuntimeDir, root.join("run"))
            .with(ServiceVar::Home, root.join("home"));
        ServicePaths::resolve(environment, ServiceSession::parse(session).unwrap()).unwrap()
    }

    #[test]
    fn only_definitions_this_runtime_wrote_are_described_and_describing_creates_nothing() {
        let root = std::path::PathBuf::from(format!("/tmp/ovs-describe-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let ours = paths(&root, "ours");
        let theirs = paths(&root, "theirs");
        let (native, location) = if cfg!(target_os = "macos") {
            ("launchd", ours.launchd_plist().unwrap())
        } else {
            ("systemd_user", ours.systemd_unit().unwrap())
        };
        std::fs::create_dir_all(location.parent().unwrap()).unwrap();
        std::fs::write(&location, "X-OvisTerminalLabel=ovis-terminal-holder-ours\n").unwrap();
        let foreign = location.with_file_name(
            location
                .file_name()
                .unwrap()
                .to_string_lossy()
                .replace("ours", "theirs"),
        );
        std::fs::write(&foreign, "not ours\n").unwrap();
        let described = serde_json::to_value(describe(&[ours.clone(), theirs]).unwrap()).unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        let expected = serde_json::json!({"services": [{
            "label": "ovis-terminal-holder-ours",
            "manager": native,
            "endpoint": {"transport": "unix", "address": ours.socket().display().to_string(), "server_boot_id": null},
            "missing_prerequisites": described.pointer("/services/0/missing_prerequisites").cloned().unwrap_or_default(),
        }]});
        assert_eq!(described, expected);
        assert!(
            !ours.directory().exists(),
            "describing created the runtime directory"
        );
    }

    #[test]
    fn the_lane_registers_once_against_the_published_catalog() {
        use std::sync::Arc;

        use ovis_terminal_authz::{
            Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding,
        };
        use ovis_terminal_core::events::Coordinator;
        use ovis_terminal_core::ops::OperationProducer;
        use ovis_terminal_core::registry::{
            HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
        };
        use ovis_terminal_protocol::schema::scalar::TimestampMs;
        use ovis_terminal_store::{Store, StoreOptions};

        use crate::conn::RequestHandler as _;
        use crate::dispatch::Dispatcher;

        let root = std::path::PathBuf::from(format!("/tmp/ovs-lane-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000051"
            .parse()
            .unwrap();
        let identity = RuntimeIdentity::new(
            HostIdentity::new(machine, HostBootId::try_from(1).unwrap()),
            RuntimeId::try_from(7).unwrap(),
            ServerBootId::try_from(1).unwrap(),
        );
        let binding = RuntimeBinding::new(identity.machine_id(), identity.runtime_id());
        let owner = OwnerSpec::new(
            PrincipalId::parse("owner").unwrap(),
            TimestampMs::new(u64::MAX),
        );
        let (authority, _) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
        let store = Store::open(&root.join("db"), StoreOptions::default()).unwrap();
        let core = Arc::new(Coordinator::new(
            store,
            identity,
            OperationProducer::new(identity.runtime_id()),
        ));
        let mut dispatch = Dispatcher::new(Arc::new(authority), identity, core);
        let lane = super::ServiceDescribe::new(machine, vec![paths(&root, "s")]);
        dispatch.register(lane.clone()).unwrap();
        assert!(dispatch.methods().contains(&"service.describe"));
        assert_eq!(
            dispatch.register(lane).map_err(|e| e.kind()),
            Err("method_not_found")
        );
        drop(dispatch);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
