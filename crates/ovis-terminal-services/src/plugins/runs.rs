//! Live run state shared by the supervising task and `plugin.run_get`.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use ovis_terminal_protocol::schema::ids::{PluginRunId, TerminalId};
use ovis_terminal_protocol::schema::methods::plugin::{PluginRunRecord, PluginRunState};
use ovis_terminal_protocol::schema::scalar::ByteOffset;

use super::PluginError;

/// Retained output bytes per run; a run record must fit one operation receipt.
pub(super) const OUTPUT_BYTES: usize = 4 << 10;
const RETAINED_RUNS: usize = 256;

/// The newest `cap` bytes of a stream and the offset of the oldest one kept.
#[derive(Debug)]
pub(super) struct Tail {
    cap: usize,
    bytes: VecDeque<u8>,
    total: u64,
}

impl Tail {
    pub(super) const fn new(cap: usize) -> Self {
        Self {
            cap,
            bytes: VecDeque::new(),
            total: 0,
        }
    }

    pub(super) fn push(&mut self, chunk: &[u8]) {
        self.total = self
            .total
            .saturating_add(u64::try_from(chunk.len()).unwrap_or(u64::MAX));
        self.bytes.extend(chunk);
        let excess = self.bytes.len().saturating_sub(self.cap);
        self.bytes.drain(..excess);
    }

    pub(super) fn floor(&self) -> ByteOffset {
        let kept = u64::try_from(self.bytes.len()).unwrap_or(u64::MAX);
        ByteOffset::new(self.total.saturating_sub(kept))
    }

    pub(super) fn retained(&self) -> Vec<u8> {
        self.bytes.iter().copied().collect()
    }
}

#[derive(Debug)]
struct Live {
    tail: Tail,
    state: PluginRunState,
    exit_code: Option<i32>,
    created: Vec<TerminalId>,
}

/// The supervising task's handle on one live run.
#[derive(Debug, Clone)]
pub(super) struct RunHandle {
    run_id: PluginRunId,
    live: Arc<Mutex<Live>>,
}

impl RunHandle {
    pub(super) fn append(&self, chunk: &[u8]) -> Result<(), PluginError> {
        self.live
            .lock()
            .map_err(|_| PluginError::Poisoned)?
            .tail
            .push(chunk);
        Ok(())
    }

    pub(super) fn settle(
        &self,
        state: PluginRunState,
        exit_code: Option<i32>,
    ) -> Result<(), PluginError> {
        let mut live = self.live.lock().map_err(|_| PluginError::Poisoned)?;
        live.state = state;
        live.exit_code = exit_code;
        drop(live);
        Ok(())
    }

    pub(super) fn record(&self) -> Result<PluginRunRecord, PluginError> {
        let live = self.live.lock().map_err(|_| PluginError::Poisoned)?;
        let output = base64::engine::general_purpose::STANDARD.encode(live.tail.retained());
        let record = PluginRunRecord::running(self.run_id.clone())
            .with_output(output, live.tail.floor())
            .with_created(live.created.clone());
        Ok(match live.state {
            PluginRunState::Running => record,
            state => record.settled(state, live.exit_code),
        })
    }

    fn is_running(&self) -> Result<bool, PluginError> {
        Ok(matches!(
            self.live.lock().map_err(|_| PluginError::Poisoned)?.state,
            PluginRunState::Running
        ))
    }
}

/// Runs of this runtime lifetime, oldest settled ones evicted first.
#[derive(Debug, Default)]
pub(super) struct Runs {
    table: Mutex<(BTreeMap<PluginRunId, RunHandle>, VecDeque<PluginRunId>)>,
}

impl Runs {
    pub(super) fn start(&self, run_id: PluginRunId) -> Result<RunHandle, PluginError> {
        let handle = RunHandle {
            run_id: run_id.clone(),
            live: Arc::new(Mutex::new(Live {
                tail: Tail::new(OUTPUT_BYTES),
                state: PluginRunState::Running,
                exit_code: None,
                created: Vec::new(),
            })),
        };
        let mut table = self.table.lock().map_err(|_| PluginError::Poisoned)?;
        let (runs, order) = &mut *table;
        if runs.contains_key(&run_id) {
            return Err(PluginError::RunExists);
        }
        if runs.len() >= RETAINED_RUNS {
            let mut settled = None;
            for (index, id) in order.iter().enumerate() {
                if let Some(run) = runs.get(id)
                    && !run.is_running()?
                {
                    settled = Some(index);
                    break;
                }
            }
            let evicted = settled
                .and_then(|index| order.remove(index))
                .ok_or(PluginError::RunCapacity)?;
            runs.remove(&evicted);
        }
        runs.insert(run_id.clone(), handle.clone());
        order.push_back(run_id);
        drop(table);
        Ok(handle)
    }

    pub(super) fn get(&self, run_id: &PluginRunId) -> Result<Option<RunHandle>, PluginError> {
        Ok(self
            .table
            .lock()
            .map_err(|_| PluginError::Poisoned)?
            .0
            .get(run_id)
            .cloned())
    }

    /// Links a terminal that runtime authorization registered for a still-running run.
    pub(super) fn link_created(
        &self,
        run_id: &PluginRunId,
        terminal: TerminalId,
    ) -> Result<(), PluginError> {
        let run = self.get(run_id)?.ok_or(PluginError::NotFound)?;
        let mut live = run.live.lock().map_err(|_| PluginError::Poisoned)?;
        if !matches!(live.state, PluginRunState::Running) {
            return Err(PluginError::NotFound);
        }
        live.created.push(terminal);
        drop(live);
        Ok(())
    }
}
