//! Bounds of one terminal actor and the session budgets its terminals share.

use std::sync::Arc;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{ParseBudget, PendingCap, Vt};

use super::budget::Budget;
use super::error::BudgetScope;
use super::event::{OutputEvent, TerminalEvent};
use super::history::HistoryStore;

/// Control requests queued across the session before another is refused.
const SESSION_CONTROL_SLOTS: u64 = 1024;

/// Queue bounds and parse budget of every terminal actor of one runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorConfig {
    pub(super) parse: ParseBudget,
    pub(super) pending: PendingCap,
    pub(super) controls: usize,
    pub(super) admits: usize,
    pub(super) output_chunks: usize,
    pub(super) chunk_bytes: usize,
    pub(super) events: usize,
    pub(super) output_events: usize,
    pub(super) terminal_input: u64,
    pub(super) history_jobs: usize,
}

impl ActorConfig {
    /// The advertised bounds: the per-terminal input queue from `limits`, 16 reserved
    /// control slots, one admission slot per in-flight request, 64 queued output chunks of
    /// at most one stream chunk, 1 MiB of output waiting to be parsed, 1024 retained
    /// control events, 64 retained output events and 64 queued history jobs.
    #[must_use]
    pub fn advertised(limits: &Limits) -> Self {
        Self {
            parse: ParseBudget::DEFAULT,
            pending: PendingCap::DEFAULT,
            controls: 16,
            admits: usize::try_from(limits.max_in_flight_requests.get()).unwrap_or(usize::MAX),
            output_chunks: 64,
            chunk_bytes: usize::try_from(limits.stream_chunk_bytes.get()).unwrap_or(16 * 1024),
            events: 1024,
            output_events: 64,
            terminal_input: limits.input_queue_bytes.get(),
            history_jobs: 64,
        }
    }

    /// Replaces the bytes one parse turn may feed.
    #[must_use]
    pub const fn with_parse_budget(self, parse: ParseBudget) -> Self {
        Self { parse, ..self }
    }

    /// Bytes of event retention one terminal reserves against the session event budget:
    /// every output event slot may hold one full chunk, plus the control event slots.
    pub(super) fn retention_bytes(&self) -> u64 {
        let output = self.output_events.saturating_mul(self.chunk_bytes);
        let control = self.events.saturating_mul(
            std::mem::size_of::<TerminalEvent>().max(std::mem::size_of::<OutputEvent>()),
        );
        u64::try_from(output.saturating_add(control)).unwrap_or(u64::MAX)
    }
}

/// The session-wide budgets every terminal of a runtime reserves against.
#[derive(Debug, Clone)]
pub struct SessionBudgets {
    pub(super) input: Arc<Budget>,
    pub(super) control: Arc<Budget>,
    pub(super) output: Arc<Budget>,
    pub(super) events: Arc<Budget>,
}

impl SessionBudgets {
    /// The advertised session budgets: accepted input, output backlog and event queue
    /// bytes from `limits`, and 1024 queued control requests.
    #[must_use]
    pub fn advertised(limits: &Limits) -> Self {
        Self {
            input: Budget::new(BudgetScope::SessionInput, limits.session_input_bytes.get()),
            control: Budget::new(BudgetScope::SessionControl, SESSION_CONTROL_SLOTS),
            output: Budget::new(
                BudgetScope::SessionOutput,
                limits.session_backlog_bytes.get(),
            ),
            events: Budget::new(
                BudgetScope::SessionEvents,
                limits.session_event_queue_bytes.get(),
            ),
        }
    }

    /// Replaces the budget of `budget`'s scope; a terminal-scoped budget changes nothing.
    #[must_use]
    pub fn with(self, budget: Arc<Budget>) -> Self {
        match budget.scope() {
            BudgetScope::SessionInput => Self {
                input: budget,
                ..self
            },
            BudgetScope::SessionControl => Self {
                control: budget,
                ..self
            },
            BudgetScope::SessionOutput => Self {
                output: budget,
                ..self
            },
            BudgetScope::SessionEvents => Self {
                events: budget,
                ..self
            },
            BudgetScope::TerminalInput => self,
        }
    }
}

/// The actor bounds plus the session budgets every terminal reserves against.
#[derive(Debug, Clone)]
pub struct TerminalSetup {
    pub(super) config: ActorConfig,
    pub(super) session: SessionBudgets,
}

impl TerminalSetup {
    /// Terminals bounded by `config`, sharing `session`.
    #[must_use]
    pub const fn new(config: ActorConfig, session: SessionBudgets) -> Self {
        Self { config, session }
    }

    /// Terminals bounded by `config`, sharing fresh advertised session budgets.
    #[must_use]
    pub fn advertised(config: ActorConfig, limits: &Limits) -> Self {
        Self::new(config, SessionBudgets::advertised(limits))
    }
}

/// What one terminal's actor owns: its screen and, optionally, its disk history.
#[derive(Debug)]
pub struct Engine {
    pub(super) vt: Vt,
    pub(super) history: Option<HistoryStore>,
}

impl Engine {
    /// A screen without disk history.
    #[must_use]
    pub const fn new(vt: Vt) -> Self {
        Self { vt, history: None }
    }

    /// Records every accepted output chunk in `history` before it is parsed.
    #[must_use]
    pub fn with_history(self, history: HistoryStore) -> Self {
        Self {
            history: Some(history),
            ..self
        }
    }
}
