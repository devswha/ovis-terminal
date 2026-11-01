//! Every machine's actor, reachable without one machine's state guarding another's.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use ovis_terminal_protocol::domain::machine_actor::MachineHealth;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineRecord;
use serde_json::Value;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;

use super::actor::{Actor, Call, Command, Inbound, Reply};
use super::health::Health;
use super::{ActorError, Line};

/// How long past its deadline a caller waits for the actor to report the expiry.
const EXPIRY_GRACE: Duration = Duration::from_secs(1);

/// Bounds shared by every machine's actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct Limits {
    /// Calls one machine's queue holds before refusing more.
    pub queue: usize,
    /// Longest deadline a call may ask for.
    pub max_deadline: Duration,
    /// Unconsumed remote notifications held for the local consumer.
    pub inbound: usize,
}

impl Limits {
    /// `queue` calls per machine, deadlines up to `max_deadline`, `inbound` notifications.
    #[must_use]
    pub const fn new(queue: usize, max_deadline: Duration, inbound: usize) -> Self {
        Self {
            queue,
            max_deadline,
            inbound,
        }
    }
}

/// One request to a machine and how long its answer may take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    method: String,
    params: Value,
    deadline: Duration,
}

impl Request {
    /// `method` with `params`, answered within `deadline`.
    #[must_use]
    pub fn new(method: &str, params: Value, deadline: Duration) -> Self {
        Self {
            method: method.to_owned(),
            params,
            deadline,
        }
    }
}

#[derive(Debug)]
struct Entry<L> {
    record: MachineRecord,
    commands: mpsc::Sender<Command<L>>,
    health: watch::Receiver<MachineHealth>,
    task: JoinHandle<()>,
}

/// The actors of every machine; a lock is held only to find a machine, never across a call.
#[derive(Debug)]
pub struct Actors<L> {
    machines: Mutex<BTreeMap<MachineId, Entry<L>>>,
    inbound: mpsc::Sender<Inbound>,
    limits: Limits,
}

impl<L> std::fmt::Debug for Command<L> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Call(_) => "Call",
            Self::Attach(_) => "Attach",
        })
    }
}

impl<L: Line> Actors<L> {
    /// No machines yet; remote notifications arrive on the returned receiver.
    #[must_use]
    pub fn new(limits: Limits) -> (Self, mpsc::Receiver<Inbound>) {
        let (inbound, notifications) = mpsc::channel(limits.inbound.max(1));
        let actors = Self {
            machines: Mutex::new(BTreeMap::new()),
            inbound,
            limits,
        };
        (actors, notifications)
    }

    fn machines(&self) -> MutexGuard<'_, BTreeMap<MachineId, Entry<L>>> {
        self.machines.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Starts the actor of the machine `record` describes, without a line yet. Must be
    /// called inside a Tokio runtime.
    ///
    /// # Errors
    ///
    /// [`ActorError::Exists`] when the machine already has one.
    pub fn add(&self, record: MachineRecord) -> Result<(), ActorError> {
        let mut machines = self.machines();
        let machine = record.machine_id.clone();
        if machines.contains_key(&machine) {
            return Err(ActorError::Exists { machine });
        }
        let (health, receiver) = Health::new(MachineHealth::detached(machine.clone()));
        let (commands, mailbox) = mpsc::channel(self.limits.queue.max(1));
        let actor = Actor::new((machine.clone(), health), mailbox, self.inbound.clone());
        let task = tokio::spawn(actor.run());
        let entry = Entry {
            record,
            commands,
            health: receiver,
            task,
        };
        machines.insert(machine, entry);
        drop(machines);
        Ok(())
    }

    fn sender(&self, machine: &MachineId) -> Result<mpsc::Sender<Command<L>>, ActorError> {
        self.machines()
            .get(machine)
            .map(|entry| entry.commands.clone())
            .ok_or_else(|| ActorError::NotFound {
                machine: machine.clone(),
            })
    }

    /// Hands a new line to the machine's actor, replacing a lost one.
    ///
    /// # Errors
    ///
    /// [`ActorError::NotFound`] or [`ActorError::Stopped`].
    pub async fn attach(&self, machine: &MachineId, line: L) -> Result<(), ActorError> {
        let sender = self.sender(machine)?;
        sender
            .send(Command::Attach(line))
            .await
            .map_err(|_| ActorError::Stopped {
                machine: machine.clone(),
            })
    }

    /// Sends `request` to `machine` and waits at most its deadline for the answer.
    ///
    /// # Errors
    ///
    /// Every [`ActorError`] but `Exists`; a full queue fails at once.
    pub async fn call(&self, machine: &MachineId, request: Request) -> Result<Reply, ActorError> {
        let Request {
            method,
            params,
            deadline,
        } = request;
        if deadline.is_zero() || deadline > self.limits.max_deadline {
            let max = self.limits.max_deadline;
            return Err(ActorError::DeadlineOutOfRange {
                requested: deadline,
                max,
            });
        }
        let sender = self.sender(machine)?;
        let (reply, answer) = oneshot::channel();
        let call = Call {
            method,
            params,
            deadline,
            reply,
        };
        let stopped = || ActorError::Stopped {
            machine: machine.clone(),
        };
        sender
            .try_send(Command::Call(call))
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ActorError::QueueFull {
                    machine: machine.clone(),
                    capacity: self.limits.queue.max(1),
                },
                mpsc::error::TrySendError::Closed(_) => stopped(),
            })?;
        match tokio::time::timeout(deadline.saturating_add(EXPIRY_GRACE), answer).await {
            Ok(Ok(outcome)) => outcome,
            Ok(Err(_)) => Err(stopped()),
            Err(_) => Err(ActorError::DeadlineExceeded {
                machine: machine.clone(),
                deadline,
            }),
        }
    }

    /// The machine's current failure state, read without messaging its actor.
    ///
    /// # Errors
    ///
    /// [`ActorError::NotFound`].
    pub fn health(&self, machine: &MachineId) -> Result<MachineHealth, ActorError> {
        self.machines()
            .get(machine)
            .map(|entry| entry.health.borrow().clone())
            .ok_or_else(|| ActorError::NotFound {
                machine: machine.clone(),
            })
    }

    /// A receiver of every change to the machine's failure state, starting at the current one.
    ///
    /// # Errors
    ///
    /// [`ActorError::NotFound`].
    pub fn watch_health(
        &self,
        machine: &MachineId,
    ) -> Result<watch::Receiver<MachineHealth>, ActorError> {
        self.machines()
            .get(machine)
            .map(|entry| entry.health.clone())
            .ok_or_else(|| ActorError::NotFound {
                machine: machine.clone(),
            })
    }

    /// Every machine with its line state, unreachable ones included, by identity.
    #[must_use]
    pub fn list(&self) -> Vec<MachineRecord> {
        self.machines()
            .values()
            .map(|entry| entry.record.observed(&entry.health.borrow()))
            .collect()
    }

    /// Every machine served.
    #[must_use]
    pub fn machine_ids(&self) -> Vec<MachineId> {
        self.machines().keys().cloned().collect()
    }
}

impl<L> Drop for Actors<L> {
    fn drop(&mut self) {
        let machines = self
            .machines
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner);
        for entry in machines.values() {
            entry.task.abort();
        }
    }
}
