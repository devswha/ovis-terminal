//! Machines reach their actors: an added machine's authenticated link becomes its actor's
//! line, and a lost machine comes back over a fresh authenticated link.

use std::sync::Arc;
use std::time::Duration;

use ovis_terminal_agent::status::AgentStatusEvent;
use ovis_terminal_authz::Session;
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineAddParams;
use ovis_terminal_remote::actors::{ActorError, Line};
use ovis_terminal_remote::identity::{IdentityError, Trust, fresh_challenge};
use ovis_terminal_remote::orphans::{OrphanError, Replayed};
use ovis_terminal_remote::transport::{Handshake, Link, TransportError, TransportSpec};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::dispatch::lanes::machine_actor::MachineActorLane;
use crate::dispatch::lanes::machine_identity::{MachineAdd, MachineLane};
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// How long an added machine's actor may take to report the line it was handed.
const ATTACH_BOUND: Duration = Duration::from_secs(10);

/// `machine.add` whose proven machine gets an actor driving the link the add opened.
#[derive(Debug)]
pub struct MachineAddAttached<L, D, P> {
    add: MachineAdd,
    identity: Arc<MachineLane>,
    actors: Arc<MachineActorLane<L, D, P>>,
    line: fn(Link) -> L,
}

impl<L, D, P> MachineAddAttached<L, D, P> {
    /// `add` over `identity`, handing each added link to `actors` as `line(link)`.
    #[must_use]
    pub fn new(
        (add, identity): (MachineAdd, &Arc<MachineLane>),
        actors: &Arc<MachineActorLane<L, D, P>>,
        line: fn(Link) -> L,
    ) -> Self {
        Self {
            add,
            identity: Arc::clone(identity),
            actors: Arc::clone(actors),
            line,
        }
    }
}

fn actor_failure(error: ActorError) -> DispatchError {
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

impl<L, D, P> Method<D, P> for MachineAddAttached<L, D, P>
where
    L: Line + std::fmt::Debug,
    D: EventData
        + From<OperationEvent>
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + std::fmt::Debug
        + 'static,
    P: SnapshotProducer + Send + Sync + std::fmt::Debug + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "machine.add";
    type Params = MachineAddParams;

    fn resolve(&self, params: &Self::Params, session: &Session) -> Result<Access, DispatchError> {
        Method::<D, P>::resolve(&self.add, params, session)
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let expected = params.expected_machine_id.clone();
        if let Some(machine) = &expected
            && self.actors.actors().health(machine).is_ok()
        {
            return Err(actor_failure(ActorError::Exists {
                machine: machine.clone(),
            }));
        }
        let completed = Method::<D, P>::run(&self.add, params, execution).await?;
        let Some(machine) = expected else {
            return Ok(completed);
        };
        let (record, link) = self
            .identity
            .take_link(&MachineRef::MachineId(machine.clone()))?;
        if let Some(link) = link {
            self.actors.actors().add(record).map_err(actor_failure)?;
            let handed = ((self.line)(link), ATTACH_BOUND);
            attach_applied(&self.actors, &machine, handed)
                .await
                .map_err(|error| DispatchError::Domain {
                    kind: error.kind(),
                    hint: error.hint(),
                    source: Box::new(error),
                })?;
        }
        Ok(completed)
    }
}

/// Why a lost machine could not be reached again.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReconnectError {
    /// The transport did not start or its handshake failed.
    #[error("the transport to the machine failed")]
    Transport(#[source] TransportError),
    /// The runtime reached is not the trusted machine or could not prove its boot.
    #[error("the machine did not prove the trusted identity")]
    Identity(#[source] IdentityError),
    /// The machine has no actor, or replaying its orphan notices failed.
    #[error("the machine's actor did not take the line")]
    Actor(#[source] OrphanError),
}

impl ReconnectError {
    /// Stable machine token.
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Transport(error) => error.kind(),
            Self::Identity(error) => error.kind(),
            Self::Actor(error) => error.kind(),
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub fn hint(&self) -> &'static str {
        match self {
            Self::Transport(error) => error.hint(),
            Self::Identity(error) => error.hint(),
            Self::Actor(error) => error.hint(),
        }
    }
}

/// Reaches a lost machine again over `spec`, proving `trust`'s machine before attaching.
///
/// The runtime there must answer a fresh challenge within `bound`; the link then goes to
/// that machine's actor as `to_line(link)`, which replays the orphan notices it has not
/// acknowledged.
///
/// # Errors
///
/// [`ReconnectError`]; nothing is attached unless the identity is proven.
pub async fn reconnect<L, D, P>(
    lane: &MachineActorLane<L, D, P>,
    (spec, trust): (&TransportSpec, &Trust),
    (bound, to_line): (Duration, fn(Link) -> L),
) -> Result<Replayed, ReconnectError>
where
    L: Line,
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    let challenge = fresh_challenge();
    let mut link = Link::spawn(spec).map_err(ReconnectError::Transport)?;
    let answer = link
        .handshake(trust.credential(), Handshake::new(challenge, bound))
        .await
        .map_err(ReconnectError::Transport)?;
    let trusted = trust
        .admit(answer, challenge)
        .map_err(ReconnectError::Identity)?;
    attach_applied(lane, trusted.machine_id(), (to_line(link), bound))
        .await
        .map_err(ReconnectError::Actor)
}

/// Hands `line` to `machine`'s actor and returns only once the actor reports the new line
/// (its line epoch advanced) within `bound`; the lane's attach alone only queues it.
async fn attach_applied<L, D, P>(
    lane: &MachineActorLane<L, D, P>,
    machine: &MachineId,
    (handed, bound): (L, Duration),
) -> Result<Replayed, OrphanError>
where
    L: Line,
    D: EventData
        + From<AgentStatusEvent>
        + Clone
        + Serialize
        + DeserializeOwned
        + Send
        + Sync
        + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
{
    let mut health = lane.actors().watch_health(machine)?;
    let before = health.borrow().line_epoch.get();
    let replayed = lane.attach(machine, handed).await?;
    let taken = health.wait_for(|health| health.line_epoch.get() > before);
    match tokio::time::timeout(bound, taken).await {
        Ok(Ok(_)) => Ok(replayed),
        Ok(Err(_)) | Err(_) => Err(OrphanError::Actor(ActorError::Stopped {
            machine: machine.clone(),
        })),
    }
}
