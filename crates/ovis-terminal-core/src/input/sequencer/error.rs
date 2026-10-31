//! The sequencer boundary error, its stable kinds and recovery hints.

use super::QueueLimits;
use crate::input::encoding::EncodingError;
use crate::input::receipts::InputSeq;
use crate::leases::LeaseError;
use crate::registry::ProcessGeneration;

/// Why input was not admitted or holder evidence was not applied; nothing was queued.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SequencerError {
    /// The binding was built for something other than one terminal incarnation.
    #[error("input sequencing needs a terminal target")]
    NotTerminal,
    /// The request resolved an identity the terminal no longer has.
    #[error("the request names a {field} the terminal no longer has")]
    StaleTarget {
        /// Identity that differs: machine, runtime, pane, terminal or terminal incarnation.
        field: &'static str,
    },
    /// The caller expected another process incarnation.
    #[error("expected process incarnation {} but the terminal runs {}", expected.get(), current.get())]
    ProcessReplaced {
        /// Incarnation the caller expected.
        expected: ProcessGeneration,
        /// Incarnation running now.
        current: ProcessGeneration,
    },
    /// The runtime has no program stream at a known grid yet.
    #[error("the terminal has no program stream at a known grid yet")]
    NotReady,
    /// The holder link was lost; nothing is admitted until the holder is re-adopted.
    #[error("the terminal lost its holder link")]
    Recovering,
    /// The program exited.
    #[error("the program has exited")]
    Finished,
    /// The lease or write grant check refused the writer.
    #[error("the writer may not write to this terminal now")]
    Lease(#[source] LeaseError),
    /// The input could not be encoded, or raw input was not granted.
    #[error("the input has no admissible encoding")]
    Encoding(#[source] EncodingError),
    /// The entry does not fit the bounded queue.
    #[error(
        "{requested} bytes do not fit beside {queued_bytes} bytes in {queued_entries} queued entries"
    )]
    Overflow {
        /// Bytes already queued.
        queued_bytes: usize,
        /// Entries already queued.
        queued_entries: usize,
        /// Bytes requested.
        requested: usize,
        /// Bounds in force.
        limits: QueueLimits,
    },
    /// An input, write or fence counter cannot advance.
    #[error("an input counter of this terminal incarnation is exhausted")]
    Exhausted,
    /// The barrier names a position no entry was admitted at yet.
    #[error("no entry was admitted at input sequence {}", requested.get())]
    SeqAhead {
        /// Position asked for.
        requested: InputSeq,
    },
    /// A holder acknowledgement matches no write this sequencer sent.
    #[error("holder acknowledgement for write {sequence} matches no outstanding write")]
    UnknownAck {
        /// Holder write sequence acknowledged.
        sequence: u64,
    },
    /// A holder acknowledgement names another terminal, process or grant.
    #[error("holder acknowledgement for write {sequence} names another target or grant")]
    ForeignAck {
        /// Holder write sequence acknowledged.
        sequence: u64,
    },
}

impl SequencerError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotTerminal => "invalid_params",
            Self::StaleTarget { .. } | Self::ForeignAck { .. } => "stale_target",
            Self::ProcessReplaced { .. } => "process_replaced",
            Self::NotReady => "not_ready",
            Self::Recovering => "recovering",
            Self::Finished => "finished",
            Self::Lease(source) => source.kind(),
            Self::Encoding(source) => source.kind(),
            Self::Overflow { .. } => "input_overflow",
            Self::Exhausted => "limit_exceeded",
            Self::SeqAhead { .. } => "seq_ahead",
            Self::UnknownAck { .. } => "stale_seq",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::NotTerminal => "target exactly one terminal",
            Self::StaleTarget { .. } => "resolve the target again and resend",
            Self::ProcessReplaced { .. } => {
                "read the terminal and decide whether to send to the new process"
            }
            Self::NotReady => "wait for terminal.started before sending",
            Self::Recovering => {
                "wait for the terminal to be re-adopted; earlier input is not resent"
            }
            Self::Finished => "relaunch the terminal to send more input",
            Self::Lease(source) => source.hint(),
            Self::Encoding(source) => source.hint(),
            Self::Overflow { .. } => "wait for queued input to be written or send less",
            Self::Exhausted => "recover the terminal to start a new incarnation",
            Self::SeqAhead { .. } => "name a queued_seq from an admission receipt",
            Self::UnknownAck { .. } | Self::ForeignAck { .. } => {
                "re-adopt the holder; acknowledgements of other writes are ignored"
            }
        }
    }
}
