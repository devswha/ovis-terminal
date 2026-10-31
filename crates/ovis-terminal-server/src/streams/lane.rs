//! The attach lane: terminal hubs, stream ownership and pump tasks.
//!
//! Stream identities are `UUIDv7` and never reused. An open stream belongs to the
//! principal that opened it; another principal is told `stream_not_found`. A closed
//! stream's identity is remembered, so a late acknowledgement for it is refused as
//! `invalid_credit` instead of reaching a newer stream.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_authz::PrincipalId;
use ovis_terminal_core::registry::{LeaseId, TerminalGeneration, TerminalId};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::ids::StreamId;
use serde_json::Value;
use tokio::task::JoinSet;

use super::credit::CreditRefusal;
use super::error::StreamError;
use super::hub::TerminalStreams;
use super::offset::StreamOffset;
use crate::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};

/// Closed stream identities remembered for refusing late acknowledgements.
const RETIRED: usize = 4096;
/// `attach.open` results kept for idempotent retries by operation id.
const RESULTS: usize = 1024;

#[derive(Debug, Default)]
struct Index {
    open: HashMap<StreamId, (PrincipalId, Arc<TerminalStreams>)>,
    retired: HashMap<StreamId, PrincipalId>,
    retired_order: VecDeque<StreamId>,
    results: HashMap<(PrincipalId, OpId), (Value, Value)>,
    results_order: VecDeque<(PrincipalId, OpId)>,
}

/// Attach streams of every terminal this runtime serves.
#[derive(Debug)]
pub struct AttachLane<T> {
    pub(super) terminals: T,
    hubs: Mutex<HashMap<TerminalId, (TerminalGeneration, Arc<TerminalStreams>)>>,
    index: Mutex<Index>,
    leases: AtomicU64,
    pub(super) tasks: Mutex<JoinSet<Result<(), StreamError>>>,
}

impl<T: TerminalDirectory> AttachLane<T> {
    /// Serves attaches to the terminals `terminals` resolves and [`AttachLane::install`] adds.
    pub fn new(terminals: T) -> Arc<Self> {
        Arc::new(Self {
            terminals,
            hubs: Mutex::new(HashMap::new()),
            index: Mutex::new(Index::default()),
            leases: AtomicU64::new(0),
            tasks: Mutex::new(JoinSet::new()),
        })
    }

    /// Serves `hub` as generation `generation` of `terminal`, replacing an older one.
    ///
    /// # Errors
    ///
    /// Returns [`StreamError::Poisoned`] when the registry lock is poisoned.
    pub fn install(
        &self,
        (terminal, generation): (TerminalId, TerminalGeneration),
        hub: Arc<TerminalStreams>,
    ) -> Result<(), StreamError> {
        self.hubs
            .lock()
            .map_err(|_| StreamError::Poisoned)?
            .insert(terminal, (generation, hub));
        Ok(())
    }

    pub(super) fn hub(
        &self,
        resolved: &ResolvedTerminal,
    ) -> Result<Arc<TerminalStreams>, StreamError> {
        let (generation, hub) = self
            .hubs
            .lock()
            .map_err(|_| StreamError::Poisoned)?
            .get(&resolved.terminal)
            .map(|(generation, hub)| (*generation, Arc::clone(hub)))
            .ok_or(StreamError::NotAttachable)?;
        if generation != resolved.generation {
            return Err(StreamError::StaleTarget);
        }
        Ok(hub)
    }

    fn index(&self) -> Result<MutexGuard<'_, Index>, StreamError> {
        self.index.lock().map_err(|_| StreamError::Poisoned)
    }

    pub(super) fn fresh_lease(&self) -> LeaseId {
        let next = self.leases.fetch_add(1, Ordering::Relaxed);
        LeaseId::new(NonZeroU64::MIN.saturating_add(next))
    }

    /// A previous result for the same request, or `op_reused` for different params.
    pub(super) fn prior(
        &self,
        key: &(PrincipalId, OpId),
        canonical: &Value,
    ) -> Result<Option<Value>, StreamError> {
        match self.index()?.results.get(key) {
            Some((params, result)) if params == canonical => Ok(Some(result.clone())),
            Some(_) => Err(StreamError::OpReused),
            None => Ok(None),
        }
    }

    pub(super) fn remember(
        &self,
        key: (PrincipalId, OpId),
        (canonical, result): (Value, Value),
    ) -> Result<(), StreamError> {
        let mut index = self.index()?;
        if index.results_order.len() >= RESULTS
            && let Some(oldest) = index.results_order.pop_front()
        {
            index.results.remove(&oldest);
        }
        index.results_order.push_back(key.clone());
        index.results.insert(key, (canonical, result));
        drop(index);
        Ok(())
    }

    pub(super) fn register(
        &self,
        stream: StreamId,
        owner: (PrincipalId, Arc<TerminalStreams>),
    ) -> Result<(), StreamError> {
        self.index()?.open.insert(stream, owner);
        Ok(())
    }

    /// Moves a closed stream to the retired set.
    pub(super) fn retire(&self, stream: &StreamId) -> Result<(), StreamError> {
        let mut index = self.index()?;
        let Some((principal, _)) = index.open.remove(stream) else {
            return Ok(());
        };
        if index.retired_order.len() >= RETIRED
            && let Some(oldest) = index.retired_order.pop_front()
        {
            index.retired.remove(&oldest);
        }
        index.retired_order.push_back(stream.clone());
        index.retired.insert(stream.clone(), principal);
        drop(index);
        Ok(())
    }

    /// The hub of an open stream `principal` owns.
    ///
    /// # Errors
    ///
    /// Returns `invalid_credit` (`stream_closed`) for the caller's closed stream and
    /// `stream_not_found` for an unknown or foreign one.
    pub fn owned(
        &self,
        principal: &PrincipalId,
        stream: &StreamId,
    ) -> Result<Arc<TerminalStreams>, StreamError> {
        let index = self.index()?;
        let open = index
            .open
            .get(stream)
            .filter(|(owner, _)| owner == principal)
            .map(|(_, hub)| Arc::clone(hub));
        let retired = index.retired.get(stream) == Some(principal);
        drop(index);
        match (open, retired) {
            (Some(hub), _) => Ok(hub),
            (None, true) => Err(StreamError::InvalidCredit(CreditRefusal::Closed)),
            (None, false) => Err(StreamError::StreamNotFound),
        }
    }

    /// Applies a cumulative acknowledgement from `principal` for `stream`; returns the
    /// credit now available. A refused acknowledgement grants nothing.
    ///
    /// # Errors
    ///
    /// Returns `invalid_credit` or `stream_not_found`.
    pub fn acknowledge(
        &self,
        principal: &PrincipalId,
        (stream, at): (&StreamId, StreamOffset),
    ) -> Result<u64, StreamError> {
        let hub = self.owned(principal, stream)?;
        match hub.acknowledge(stream, at) {
            Err(StreamError::StreamNotFound) => {
                Err(StreamError::InvalidCredit(CreditRefusal::Closed))
            }
            other => other,
        }
    }

    /// Waits for every delivery task to end; call it once the server stopped serving,
    /// so each task sees its connection close and revokes what that stream held.
    ///
    /// # Errors
    ///
    /// Returns the first delivery failure, or [`StreamError::Poisoned`].
    pub async fn shutdown(&self) -> Result<(), StreamError> {
        let mut tasks = std::mem::take(&mut *self.tasks.lock().map_err(|_| StreamError::Poisoned)?);
        let mut first = Ok(());
        while let Some(joined) = tasks.join_next().await {
            let outcome = joined
                .map_err(StreamError::from)
                .and_then(|delivered| delivered);
            if first.is_ok() {
                first = outcome;
            }
        }
        first
    }
}
