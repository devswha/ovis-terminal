//! Why one core connection to a Windows holder ended with a failure.

use std::io;

use thiserror::Error;

use crate::protocol::{AdoptionError, FrameError, MessageKind, TerminalId};
use crate::windows::ConptyError;

/// A core connection failure. The holder wire has no error reply, so the connection
/// ends with this typed reason and the core re-adopts; terminals keep running.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LinkError {
    /// Reading or writing the pipe failed.
    #[error("the holder pipe failed")]
    Io(#[source] io::Error),
    /// The peer sent nothing before the admission bound.
    #[error("the peer sent nothing within {bound_ms} ms")]
    PeerSilent {
        /// Bound waited.
        bound_ms: u64,
    },
    /// The peer is not the account that owns the holder.
    #[error("the peer is not the holder's owner")]
    PeerRejected,
    /// A frame could not be decoded.
    #[error("a holder frame could not be decoded")]
    Frame(#[from] FrameError),
    /// The first message was not `Hello`.
    #[error("the first message was {kind:?}, not hello")]
    HelloRequired {
        /// What arrived instead.
        kind: MessageKind,
    },
    /// A message only a holder sends arrived from the core.
    #[error("{kind:?} is not a core request")]
    UnexpectedMessage {
        /// The message kind.
        kind: MessageKind,
    },
    /// Generation negotiation or adoption failed.
    #[error("adoption failed")]
    Adoption(#[from] AdoptionError),
    /// No terminal has the identity.
    #[error("no terminal {terminal_id:?} is held")]
    UnknownTerminal {
        /// The identity named.
        terminal_id: TerminalId,
    },
    /// The terminal exists with another incarnation or process.
    #[error("terminal {terminal_id:?} is held with another incarnation")]
    StaleTarget {
        /// The identity named.
        terminal_id: TerminalId,
    },
    /// A request named a terminal that already exists.
    #[error("terminal {terminal_id:?} already exists")]
    DuplicateTerminal {
        /// The identity named.
        terminal_id: TerminalId,
    },
    /// A terminal operation failed.
    #[error("the terminal operation failed")]
    Terminal(#[from] ConptyError),
    /// `Forget` came before the exit, or acknowledged less than the final offset.
    #[error("the terminal cannot be forgotten yet")]
    NotForgettable {
        /// The identity named.
        terminal_id: TerminalId,
    },
    /// `Drain(Complete)` came while terminals remain.
    #[error("drain completion was sent while terminals remain")]
    DrainIncomplete,
    /// A per-holder counter cannot advance.
    #[error("a holder counter is exhausted")]
    CounterExhausted,
    /// The interrupt could not be queued behind the pending input.
    #[error("the interrupt did not fit the input queue")]
    InterruptRefused,
    /// Output pump threads panicked before they could be joined.
    #[error("{count} output pump(s) panicked")]
    PumpPanicked {
        /// Panicked pumps.
        count: usize,
    },
}

impl LinkError {
    /// Stable machine token.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Io(_) => "holder_io",
            Self::PeerSilent { .. } => "peer_silent",
            Self::PeerRejected => "peer_rejected",
            Self::Frame(_) => "invalid_frame",
            Self::HelloRequired { .. } => "hello_required",
            Self::UnexpectedMessage { .. } => "unexpected_message",
            Self::Adoption(source) => source.kind(),
            Self::UnknownTerminal { .. } => "not_found",
            Self::StaleTarget { .. } => "stale_target",
            Self::DuplicateTerminal { .. } => "duplicate_terminal",
            Self::Terminal(source) => source.kind(),
            Self::NotForgettable { .. } => "exit_not_drained",
            Self::DrainIncomplete => "drain_incomplete",
            Self::CounterExhausted => "counter_exhausted",
            Self::PumpPanicked { .. } => "worker_panicked",
            Self::InterruptRefused => "input_overflow",
        }
    }

    /// Recovery guidance.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Io(_) => "reconnect and adopt the holder again",
            Self::PeerSilent { .. } => "send hello right after connecting",
            Self::PeerRejected => "connect as the account that owns the holder",
            Self::Frame(_) => "speak the holder frame protocol of this generation",
            Self::HelloRequired { .. } => "send hello first",
            Self::UnexpectedMessage { .. } => "send only core-to-holder messages",
            Self::Adoption(source) => source.hint(),
            Self::UnknownTerminal { .. } | Self::StaleTarget { .. } => {
                "re-adopt and use the identities the welcome lists"
            }
            Self::DuplicateTerminal { .. } => "allocate a new terminal identity",
            Self::Terminal(source) => source.hint(),
            Self::NotForgettable { .. } => "read output through the exit's final offset first",
            Self::DrainIncomplete => "wait until every terminal exited and was forgotten",
            Self::CounterExhausted => "restart the holder service",
            Self::PumpPanicked { .. } => "report the holder defect; its terminals ended",
            Self::InterruptRefused => "kill the program, or retry once its input drains",
        }
    }
}
