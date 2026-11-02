//! The layout feed over real layout and dispatch stores: a subscriber of the dispatch log
//! receives each committed layout change and selection once, with the client's op id, and a
//! record committed while no boot served is forwarded by the next one.

use std::path::PathBuf;
use std::sync::mpsc::TryRecvError;
use std::time::Duration;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Presentation, PrincipalId, RemoteTransport, RuntimeBinding, Session, UnixUid,
};
use ovis_terminal_core::events::{Coordinator, Delivery, EventError};
use ovis_terminal_core::layout::{Call, Edit, EditRequest, Layouts, Producer, SelectRequest};
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RegistryIdentities, RuntimeId, RuntimeIdentity,
    ServerBootId,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_store::{Store, StoreOptions};
use serde_json::{Value, json};
use uuid::Uuid;

use super::forward;
use crate::assembly::lanes::{DispatchLog, ServedEvent};
use crate::assembly::layout::{LayoutLog, wire};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// Bound on one awaited delivery; a dropped publish fails here instead of hanging.
const DELIVERY: Duration = Duration::from_secs(10);

struct Fixture {
    root: PathBuf,
    identity: RuntimeIdentity,
    authority: Authority<RegistryIdentities>,
    session: Session,
    layout: LayoutLog,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        match std::fs::remove_dir_all(&self.root) {
            Ok(()) | Err(_) => {}
        }
    }
}

impl Fixture {
    fn new() -> TestResult<Self> {
        let root = std::env::temp_dir().join(format!("ovt-p4-feed-{}", Uuid::now_v7()));
        std::fs::create_dir_all(&root)?;
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(Uuid::from_u128(4))?,
                HostBootId::try_from(1)?,
            ),
            RuntimeId::try_from(1)?,
            ServerBootId::try_from(1)?,
        );
        let (authority, owner) = Authority::<RegistryIdentities>::bootstrap(AuthorityConfig::new(
            RuntimeBinding::new(identity.machine_id(), identity.runtime_id()),
            OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX)),
        ))?;
        let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1)))
            .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
        let session = authority.authenticate(
            &admission,
            Presentation::new(owner.bearer(), TimestampMs::new(1)),
        )?;
        let store = Store::open(&root.join("layout.db"), StoreOptions::default())?;
        let layout = Coordinator::new(store, identity, Producer::new(identity.runtime_id()));
        Ok(Self {
            root,
            identity,
            authority,
            session,
            layout,
        })
    }

    /// A dispatch log on the fixture's dispatch store, as one boot opens it.
    fn boot(&self) -> TestResult<DispatchLog> {
        let store = Store::open(&self.root.join("dispatch.db"), StoreOptions::default())?;
        let producer = OperationProducer::new(self.identity.runtime_id());
        Ok(Coordinator::new(store, self.identity, producer))
    }

    fn layouts(&self) -> Layouts<'_, ovis_terminal_core::layout::LayoutEvent, Producer> {
        Layouts::new(&self.layout, &self.authority, self.identity)
    }

    /// Creates a workspace under `op`; returns its id as published.
    fn create(&self, op: &str) -> TestResult<String> {
        let edit = Edit::CreateWorkspace {
            name: "w".to_owned(),
            group_with: None,
        };
        let call = Call::new(&self.session, OpId::new(op.to_owned()));
        let change = self.layouts().edit(call, &EditRequest::new(edit))?;
        let created = change.created.iter().find_map(wire::resource);
        let id = created.and_then(|id| id.get("workspace_id").cloned());
        Ok(id
            .and_then(|id| id.as_str().map(str::to_owned))
            .ok_or("no workspace")?)
    }

    fn select(&self, op: &str, (client, workspace): (&str, &str)) -> TestResult {
        let request: SelectRequest = serde_json::from_value(json!({"client": client,
            "workspace": workspace, "tab": null, "pane": null, "expect_revision": null}))?;
        let call = Call::new(&self.session, OpId::new(op.to_owned()));
        self.layouts().select(call, &request)?;
        Ok(())
    }

    fn tree(&self) -> TestResult<Value> {
        Ok(wire::tree(&self.layouts().get(&self.session)?))
    }

    fn forward(&self, feed: &DispatchLog) -> TestResult<usize> {
        Ok(forward(&self.layout, feed, &self.tree()?)?)
    }
}

fn at<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    value.pointer(pointer)
}

fn op() -> String {
    Uuid::now_v7().to_string()
}

/// Subscribes to `feed` from its head, before anything is forwarded.
fn subscribe(feed: &DispatchLog) -> TestResult<Delivery<ServedEvent>> {
    Ok(feed.subscribe(None)?.delivery)
}

/// The next delivered event's type and data, within the bound.
fn next(delivery: &Delivery<ServedEvent>) -> TestResult<(String, Value)> {
    let event = delivery.recv_timeout(DELIVERY)?;
    let ServedEvent::Layout(published) = &event.data else {
        return Err(format!("not a layout event: {}", event.event_type).into());
    };
    Ok((event.event_type.clone(), published.payload()?))
}

fn nothing_more(delivery: &Delivery<ServedEvent>) -> TestResult {
    match delivery.try_recv() {
        Err(EventError::TryReceive(TryRecvError::Empty)) => Ok(()),
        Ok(event) => Err(format!("published again: {}", event.event_type).into()),
        Err(error) => Err(error.into()),
    }
}

#[test]
fn a_layout_edit_and_a_selection_reach_a_subscriber_with_their_op_ids() -> TestResult {
    let fixture = Fixture::new()?;
    let feed = fixture.boot()?;
    let delivery = subscribe(&feed)?;
    let (edit, select) = (op(), op());
    let workspace = fixture.create(&edit)?;
    assert_eq!(fixture.forward(&feed)?, 1);
    let (kind, data) = next(&delivery)?;
    assert_eq!(kind, "layout.changed");
    assert_eq!(at(&data, "/op_id"), Some(&json!(edit)));
    assert_eq!(
        at(&data, "/created/0"),
        Some(&json!({"workspace_id": workspace}))
    );
    assert_eq!(at(&data, "/revision"), at(&fixture.tree()?, "/revision"));
    assert_eq!(
        at(&data, "/workspaces/0/workspace_id"),
        Some(&json!(workspace))
    );
    assert!(
        at(&data, "/revision").is_some_and(Value::is_string),
        "revision is a decimal string"
    );

    fixture.select(&select, ("client:a", &workspace))?;
    assert_eq!(fixture.forward(&feed)?, 1);
    let (kind, data) = next(&delivery)?;
    assert_eq!(kind, "selection.changed");
    assert_eq!(at(&data, "/op_id"), Some(&json!(select)));
    assert_eq!(at(&data, "/client_id"), Some(&json!("client:a")));
    assert_eq!(at(&data, "/workspace_id"), Some(&json!(workspace)));
    assert_eq!(at(&data, "/cause"), Some(&json!("requested")));
    assert_eq!(at(&data, "/revision"), at(&fixture.tree()?, "/revision"));
    nothing_more(&delivery)?;
    feed.close()?;
    Ok(())
}

#[test]
fn a_record_is_published_once_and_a_later_boot_forwards_what_no_boot_served() -> TestResult {
    let fixture = Fixture::new()?;
    let first = fixture.boot()?;
    let delivery = subscribe(&first)?;
    let (served, between) = (op(), op());
    fixture.create(&served)?;
    assert_eq!(fixture.forward(&first)?, 1);
    assert_eq!(at(&next(&delivery)?.1, "/op_id"), Some(&json!(served)));
    assert_eq!(
        fixture.forward(&first)?,
        0,
        "a second forward published again"
    );
    nothing_more(&delivery)?;
    first.close()?;

    fixture.create(&between)?;
    let second = fixture.boot()?;
    let delivery = subscribe(&second)?;
    assert_eq!(fixture.forward(&second)?, 1);
    let (kind, data) = next(&delivery)?;
    assert_eq!(kind, "layout.changed");
    assert_eq!(at(&data, "/op_id"), Some(&json!(between)));
    nothing_more(&delivery)?;
    second.close()?;
    Ok(())
}
