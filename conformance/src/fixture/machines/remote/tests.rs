//! One clock reading stamps both the remote child's operation id and its acceptance clock.

use std::path::PathBuf;
use std::time::Duration;

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::ops::{Context, OperationProducer, Operations, Request};
use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::json;

use super::{Core, stamped};
use crate::fixture::serve::identity;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Readings on, just past and just before a millisecond tick, oldest first; all lie in
/// the past, as a reading has by the time an id minted from the live clock follows it.
const READINGS: [Duration; 3] = [
    Duration::new(1_700_000_000, 123_000_000),
    Duration::new(1_700_000_000, 123_000_001),
    Duration::new(1_700_000_000, 124_999_999),
];

fn scratch() -> Result<PathBuf, std::io::Error> {
    let base = std::env::var_os("OVIS_FIXTURE_ROOT").map_or_else(std::env::temp_dir, PathBuf::from);
    let dir = base.join(format!("ovc-stamped-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

#[test]
fn an_operation_stamped_at_a_reading_is_accepted_at_that_reading() -> TestResult {
    let dir = scratch()?;
    let runtime = identity()?;
    let store = Store::open(&dir.join("state.db"), StoreOptions::default())?;
    let producer = OperationProducer::new(runtime.runtime_id());
    let core = Core::new(store, runtime, producer);
    for reading in READINGS {
        let (op, now) = stamped(reading)?;
        let context = Context::new(PrincipalId::parse("owner")?, runtime, now);
        let request = Request::new(op, "fixture.remote_child".to_owned(), json!({}))?;
        let accepted = Operations::new(&core, context).accept(&request);
        assert_eq!(
            accepted
                .map(|record| record.state)
                .map_err(|error| error.kind()),
            Ok(OpenEnum::Known(OpState::Accepted)),
            "reading {reading:?}"
        );
    }
    core.close()?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
