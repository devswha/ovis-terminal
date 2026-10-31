//! Bounds of one terminal actor and the session budgets its terminals share.

use std::sync::Arc;

use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_vt::{ParseBudget, PendingCap};

use super::budget::ByteBudget;
use super::error::BudgetScope;

/// Queue bounds and parse budget of every terminal actor of one runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActorConfig {
    pub(super) parse: ParseBudget,
    pub(super) pending: PendingCap,
    pub(super) controls: usize,
    pub(super) admits: usize,
    pub(super) output_chunks: usize,
    pub(super) events: usize,
    pub(super) output_events: usize,
    pub(super) terminal_input: u64,
}

impl ActorConfig {
    /// The advertised bounds: the per-terminal input queue from `limits`, 16 reserved
    /// control slots, one admission slot per in-flight request, 64 queued output chunks,
    /// 1 MiB of output waiting to be parsed, 1024 retained control events and 256
    /// retained output events.
    #[must_use]
    pub fn advertised(limits: &Limits) -> Self {
        Self {
            parse: ParseBudget::DEFAULT,
            pending: PendingCap::DEFAULT,
            controls: 16,
            admits: usize::try_from(limits.max_in_flight_requests.get()).unwrap_or(usize::MAX),
            output_chunks: 64,
            events: 1024,
            output_events: 256,
            terminal_input: limits.input_queue_bytes.get(),
        }
    }

    /// Replaces the bytes one parse turn may feed.
    #[must_use]
    pub const fn with_parse_budget(self, parse: ParseBudget) -> Self {
        Self { parse, ..self }
    }

    /// Replaces the bytes of one terminal's accepted input queue.
    #[must_use]
    pub const fn with_terminal_input(self, terminal_input: u64) -> Self {
        Self {
            terminal_input,
            ..self
        }
    }

    /// Replaces the events a subscriber may fall behind before it is told of the loss.
    #[must_use]
    pub const fn with_events(self, events: usize) -> Self {
        Self { events, ..self }
    }
}

/// The actor bounds plus the session-wide input budget every terminal reserves against.
#[derive(Debug, Clone)]
pub struct TerminalSetup {
    pub(super) config: ActorConfig,
    pub(super) session: Arc<ByteBudget>,
}

impl TerminalSetup {
    /// Terminals bounded by `config`, sharing `session` for accepted input.
    #[must_use]
    pub const fn new(config: ActorConfig, session: Arc<ByteBudget>) -> Self {
        Self { config, session }
    }

    /// Terminals bounded by `config`, sharing a fresh session input budget from `limits`.
    #[must_use]
    pub fn advertised(config: ActorConfig, limits: &Limits) -> Self {
        Self::new(
            config,
            ByteBudget::new(BudgetScope::SessionInput, limits.session_input_bytes.get()),
        )
    }

    /// The session input budget.
    #[must_use]
    pub const fn session(&self) -> &Arc<ByteBudget> {
        &self.session
    }
}
