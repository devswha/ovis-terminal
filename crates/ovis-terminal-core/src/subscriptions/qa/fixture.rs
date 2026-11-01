//! Real store, coordinator and subscription hub for the subscription scenarios.

use std::collections::BTreeMap;
use std::error::Error;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ovis_terminal_core::events::{
    Commit, Coordinator, Cursor, Event, EventData, EventError, Filter, Limits, ResourceId,
    SnapshotProducer,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId, TerminalId,
};
use ovis_terminal_core::subscriptions::{
    Feed, LossRecord, QueueLimits, Request, Selector, Start, Started, Subscriptions,
};
use ovis_terminal_store::{
    CasWrite, OpId, Payload, ResourceKey, ResourceLookup, Snapshot, Store, StoreLimits,
    StoreOptions, Version,
};
use serde::ser::Error as _;
use serde::{Deserialize, Serialize, Serializer};
use uuid::Uuid;

/// Outcome of an assertion-bearing case.
pub type TestResult = Result<(), Box<dyn Error>>;
/// Terminal values by terminal number, as a snapshot or a rebuilt view.
pub type State = BTreeMap<u64, String>;
/// The hub under test.
pub type Hub = Subscriptions<Data, Producer>;
/// One consumer handle.
pub type Sub = Feed<Data, Producer>;

/// Terminals the producer captures.
pub const TERMINALS: u64 = 3;
/// Upper bound of every wait.
pub const BOUND: Duration = Duration::from_secs(5);

/// Committed event data of the fixture producer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Data {
    /// A terminal value changed.
    Set {
        /// Terminal number.
        terminal: u64,
        /// New value.
        value: String,
    },
    /// A durable subscription loss marker.
    Lost(LossRecord),
    /// An event whose fan-out holds the worker until [`World::hold_worker`] releases it.
    Gate(Gated),
}

type Armed = (SyncSender<()>, Receiver<()>);

/// Gates armed by [`World::hold_worker`], keyed by the gate event that springs each one,
/// so worlds sharing one test process never take or replace each other's gate.
static GATES: Mutex<BTreeMap<u32, Armed>> = Mutex::new(BTreeMap::new());
static NEXT_GATE: AtomicU32 = AtomicU32::new(0);

/// Payload of a gate event: the key of the gate it springs.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[non_exhaustive]
pub struct Gated(pub u32);

impl Serialize for Gated {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if std::thread::current().name() == Some("ovis-subscriptions") {
            let armed = GATES
                .lock()
                .ok()
                .and_then(|mut gates| gates.remove(&self.0));
            if let Some((entered, release)) = armed {
                entered.send(()).map_err(S::Error::custom)?;
                release.recv().map_err(S::Error::custom)?;
            }
        }
        serializer.serialize_u32(self.0)
    }
}

impl EventData for Data {
    fn event_type(&self) -> &'static str {
        match self {
            Self::Set { .. } => "terminal.updated",
            Self::Lost(_) => "subscription.lost",
            Self::Gate(_) => "test.gate",
        }
    }
}

impl From<LossRecord> for Data {
    fn from(record: LossRecord) -> Self {
        Self::Lost(record)
    }
}

/// Captures terminal values from the store.
#[derive(Debug)]
#[non_exhaustive]
pub struct Producer;

fn key(terminal: u64) -> ResourceKey {
    ResourceKey::new("terminal", format!("terminal:{terminal}"))
}

impl SnapshotProducer for Producer {
    type Sections = State;

    fn capture(&self, snapshot: &Snapshot, _filter: &Filter) -> Result<State, EventError> {
        let mut state = State::new();
        for terminal in 1..=TERMINALS {
            match snapshot.resource(&key(terminal))? {
                ResourceLookup::Live { value, .. } => {
                    state.insert(terminal, serde_json::from_slice(value.as_bytes())?);
                }
                ResourceLookup::Absent | ResourceLookup::Tombstone { .. } => {}
                _ => return Err(EventError::SnapshotUnavailable),
            }
        }
        Ok(state)
    }
}

/// The fixture runtime identity with the given runtime lifetime.
///
/// # Errors
///
/// Returns an identity construction failure.
pub fn identity(runtime: u64) -> Result<RuntimeIdentity, Box<dyn Error>> {
    Ok(RuntimeIdentity::new(
        HostIdentity::new(
            MachineId::new(Uuid::from_u128(7))?,
            HostBootId::try_from(1)?,
        ),
        RuntimeId::try_from(runtime)?,
        ServerBootId::try_from(1)?,
    ))
}

/// Bounds a world is opened with; each test narrows only what it exercises.
#[derive(Debug)]
#[non_exhaustive]
pub struct Bounds {
    /// Subscription queue bounds.
    pub queue: QueueLimits,
    /// Store retention bounds.
    pub store: StoreLimits,
    /// Coordinator delivery bounds.
    pub core: Limits,
}

impl Bounds {
    /// The advertised v1 bounds.
    ///
    /// # Errors
    ///
    /// Returns an invalid advertised bound.
    pub fn v1() -> Result<Self, Box<dyn Error>> {
        Ok(Self {
            queue: QueueLimits::v1()?,
            store: StoreLimits::default(),
            core: Limits::default(),
        })
    }
}

/// A real SQLite store, commit coordinator and subscription hub in a private directory.
#[derive(Debug)]
#[non_exhaustive]
pub struct World {
    dir: PathBuf,
    /// Commit coordinator.
    pub core: Arc<Coordinator<Data, Producer>>,
    /// Subscription hub.
    pub hub: Hub,
    versions: Mutex<BTreeMap<u64, u64>>,
}

impl World {
    /// Opens a fresh world.
    ///
    /// # Errors
    ///
    /// Returns a store, coordinator or hub start failure.
    pub fn open(bounds: Bounds) -> Result<Self, Box<dyn Error>> {
        let dir = std::env::temp_dir().join(format!("ovis-subscriptions-{}", Uuid::now_v7()));
        let store = Store::open(&dir, StoreOptions::default().with_limits(bounds.store))?;
        let core =
            Arc::new(Coordinator::new(store, identity(1)?, Producer).with_limits(bounds.core)?);
        let hub = Subscriptions::start(Arc::clone(&core), identity(1)?, bounds.queue)?;
        Ok(Self {
            dir,
            core,
            hub,
            versions: Mutex::new(BTreeMap::new()),
        })
    }

    /// Commits one terminal value; tests commit from one writer at a time.
    ///
    /// # Errors
    ///
    /// Returns a typed commit failure.
    pub fn set(&self, terminal: u64, value: &str) -> Result<Cursor, Box<dyn Error>> {
        let version = *self
            .versions
            .lock()
            .map_err(|_| "versions poisoned")?
            .get(&terminal)
            .unwrap_or(&0);
        let write = CasWrite::new(
            key(terminal),
            Version::new(version),
            Some(Payload::from(serde_json::to_vec(value)?)),
        );
        let data = Data::Set {
            terminal,
            value: value.into(),
        };
        let affected = vec![ResourceId::Terminal(TerminalId::try_from(terminal)?)];
        let commit =
            Commit::new(OpId::new(Uuid::now_v7().to_string()), data, affected).write(write);
        let cursor = self.core.commit(commit)?;
        self.versions
            .lock()
            .map_err(|_| "versions poisoned")?
            .insert(terminal, version.saturating_add(1));
        Ok(cursor)
    }

    /// Holds the fan-out worker inside the fan-out of a gate event of its own until the
    /// returned sender releases it or is dropped; returns once the worker is held.
    ///
    /// # Errors
    ///
    /// Returns a typed commit failure, or the bound passing before the worker is held.
    pub fn hold_worker(&self) -> Result<SyncSender<()>, Box<dyn Error>> {
        let key = NEXT_GATE.fetch_add(1, Ordering::Relaxed);
        let (entered, reached) = sync_channel(1);
        let (release, released) = sync_channel(1);
        GATES
            .lock()
            .map_err(|_| "gates poisoned")?
            .insert(key, (entered, released));
        let affected = vec![ResourceId::Terminal(TerminalId::try_from(1)?)];
        let op = OpId::new(Uuid::now_v7().to_string());
        self.core
            .commit(Commit::new(op, Data::Gate(Gated(key)), affected))?;
        reached.recv_timeout(BOUND)?;
        Ok(release)
    }

    /// Installs one subscription.
    ///
    /// # Errors
    ///
    /// Returns the typed subscription failure.
    pub fn subscribe(
        &self,
        start: Start,
        selector: Selector,
    ) -> Result<Started<Data, Producer>, Box<dyn Error>> {
        Ok(self.hub.subscribe(Request::new(start, selector))?)
    }

    /// The state a brand-new snapshot subscription reports, released immediately.
    ///
    /// # Errors
    ///
    /// Returns the typed subscription failure.
    pub fn fresh(&self) -> Result<(u64, State), Box<dyn Error>> {
        let started = self.subscribe(Start::Snapshot, Selector::all())?;
        let snapshot = started
            .snapshot
            .ok_or("snapshot start carried no snapshot")?;
        Ok((snapshot.cursor.seq.get(), snapshot.sections))
    }

    /// Shuts the hub down, closes the store and removes the directory.
    ///
    /// # Errors
    ///
    /// Returns a shutdown, close or cleanup failure.
    pub fn finish(self) -> TestResult {
        let Self { dir, core, hub, .. } = self;
        hub.shutdown()?;
        Arc::try_unwrap(core)
            .map_err(|_| "coordinator still shared")?
            .close()?;
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}

/// A terminal identity.
///
/// # Errors
///
/// Returns an identity failure for zero.
pub fn terminal(id: u64) -> Result<TerminalId, Box<dyn Error>> {
    Ok(TerminalId::try_from(id)?)
}

/// Applies committed `terminal.updated` events to a state reconstructed from a snapshot.
pub fn apply(state: &mut State, events: &[Event<Data>]) {
    for event in events {
        if let Data::Set { terminal, value } = &event.data {
            state.insert(*terminal, value.clone());
        }
    }
}
