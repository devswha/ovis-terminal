use std::sync::Arc;

use ovis_terminal_authz::{Authority, AuthorityConfig, OwnerSpec, PrincipalId, RuntimeBinding};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_services::plugins::Plugins;
use ovis_terminal_store::{Store, StoreOptions};

use super::ServiceLanes;
use crate::conn::RequestHandler as _;
use crate::dispatch::Dispatcher;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;
type Dispatch = Dispatcher<OperationEvent, OperationProducer>;

fn dispatcher(root: &std::path::Path) -> TestResult<(Dispatch, RuntimeIdentity)> {
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(
            "machine:0191f1a2-0000-7000-8000-000000000052".parse()?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(52)?,
        ServerBootId::try_from(1)?,
    );
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, _) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let store = Store::open(&root.join("state.db"), StoreOptions::default())?;
    let core = Arc::new(Coordinator::new(
        store,
        runtime,
        OperationProducer::new(runtime.runtime_id()),
    ));
    Ok((Dispatch::new(Arc::new(authority), runtime, core), runtime))
}

#[test]
fn configured_services_register_exactly_their_methods_and_a_second_time_is_refused() -> TestResult {
    let root = std::env::temp_dir().join(format!("ovt52-services-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let (mut dispatch, runtime) = dispatcher(&root)?;
    let plugins = Arc::new(Plugins::new(root.join("plugins")));
    let before = dispatch.methods();

    ServiceLanes::new(runtime.machine_id()).register(&mut dispatch)?;
    let unchanged = dispatch.methods();
    ServiceLanes::new(runtime.machine_id())
        .with_plugins(Arc::clone(&plugins))
        .register(&mut dispatch)?;
    let mut added: Vec<&str> = dispatch
        .methods()
        .into_iter()
        .filter(|method| !before.contains(method))
        .collect();
    added.sort_unstable();
    let again = ServiceLanes::new(runtime.machine_id())
        .with_plugins(plugins)
        .register(&mut dispatch);

    assert_eq!(unchanged, before);
    assert_eq!(added, ["plugin.invoke", "plugin.list", "plugin.run_get"]);
    assert!(again.is_err(), "a duplicate registration must be refused");
    drop(dispatch);
    std::fs::remove_dir_all(&root)?;
    Ok(())
}
