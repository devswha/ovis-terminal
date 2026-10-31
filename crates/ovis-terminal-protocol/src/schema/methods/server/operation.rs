//! Additive operation inspection and cancellation facts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::OpRecord;
use crate::rpc::DecimalU64;
use crate::schema::common::Expect;
use crate::schema::ids::TerminalId;
use crate::schema::scalar::{ByteCount, ByteOffset, Incarnation, OpenEnum};

macro_rules! evidence_counter {
    ($name:ident) => {
        #[doc = concat!("Holder ", stringify!($name), " as a canonical decimal string.")]
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
        #[serde(transparent)]
        pub struct $name(DecimalU64);
        impl $name {
            /// Tags the holder counter with its semantic domain.
            pub const fn new(value: u64) -> Self {
                Self(DecimalU64::new(value))
            }
            /// Reads the holder counter.
            pub const fn get(self) -> u64 {
                self.0.get()
            }
        }
    };
}

evidence_counter!(WriteSequence);
evidence_counter!(GrantEpoch);
evidence_counter!(FenceEpoch);
evidence_counter!(AckEpoch);

/// Write identity persisted before handing input to the holder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WriteIntent {
    /// Ordered input entry.
    pub sequence: WriteSequence,
    /// Granted authority epoch.
    pub grant_epoch: GrantEpoch,
    /// Revocation fence at admission.
    pub fence_epoch: FenceEpoch,
    /// Exact number of bytes admitted for this input.
    pub requested_bytes: ByteCount,
}

impl WriteIntent {
    /// Binds one sequenced input to its authority and revocation fence.
    pub const fn new(
        sequence: WriteSequence,
        grant_epoch: GrantEpoch,
        fence_epoch: FenceEpoch,
    ) -> Self {
        Self {
            sequence,
            grant_epoch,
            fence_epoch,
            requested_bytes: ByteCount::new(0),
        }
    }

    /// Adds the byte count whose full delivery the receipt must prove.
    #[must_use]
    pub const fn with_bytes(mut self, requested_bytes: ByteCount) -> Self {
        self.requested_bytes = requested_bytes;
        self
    }
}

/// Receipt of a holder write, not proof of program acceptance or termination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct WriteEvidence {
    /// Bytes actually written, including a partial prefix.
    pub written_bytes: ByteCount,
    /// Output cursor reported at the holder's acknowledgement.
    pub output_offset: ByteOffset,
    /// Input entry and authority that the holder acknowledged.
    pub intent: WriteIntent,
    /// Holder acknowledgement epoch.
    pub ack_epoch: AckEpoch,
}

impl WriteEvidence {
    /// Preserves holder delivery quantities without conflating offset and length.
    pub const fn new(amount: (ByteCount, ByteOffset), proof: (WriteIntent, AckEpoch)) -> Self {
        Self {
            written_bytes: amount.0,
            output_offset: amount.1,
            intent: proof.0,
            ack_epoch: proof.1,
        }
    }
}

/// Holder exit evidence, including explicitly uncertain or incomplete outcomes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct ProcessExitEvidence {
    /// Observed exit code, absent for signals or unavailable status.
    pub code: Option<i32>,
    /// Observed signal, absent for exit codes or unavailable status.
    pub signal: Option<i32>,
    /// True only when the holder observed a process exit status.
    pub observed: bool,
    /// PTY failure means the output may be incomplete.
    pub io_failure: bool,
    /// Last retained output offset.
    pub final_offset: ByteOffset,
}

impl ProcessExitEvidence {
    /// Records exit fields without converting an unknown exit to success.
    pub const fn new(status: (Option<i32>, Option<i32>), final_offset: ByteOffset) -> Self {
        Self {
            code: status.0,
            signal: status.1,
            observed: status.0.is_some() || status.1.is_some(),
            io_failure: false,
            final_offset,
        }
    }
}

/// Durable facts supporting an operation outcome; reads never clear them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OperationEvidence {
    /// Terminal identity attached to holder evidence.
    pub terminal_id: Option<TerminalId>,
    /// Terminal and process incarnations the evidence names.
    pub incarnations: Option<Expect>,
    /// Input identity persisted before external effects.
    pub intent: Option<WriteIntent>,
    /// Actual delivery, independent of process exit.
    pub write: Option<WriteEvidence>,
    /// Actual process exit status or its explicit absence.
    pub exit: Option<ProcessExitEvidence>,
    /// Cancellation was requested after work could have begun.
    pub cancellation_requested: bool,
}

impl OperationEvidence {
    /// Binds holder evidence to a terminal and its exact incarnations.
    pub const fn for_process(
        terminal_id: TerminalId,
        incarnations: (Incarnation, Option<Incarnation>),
    ) -> Self {
        Self {
            terminal_id: Some(terminal_id),
            incarnations: Some(Expect {
                terminal_incarnation: Some(incarnations.0),
                process_incarnation: incarnations.1,
                status_seq: None,
            }),
            intent: None,
            write: None,
            exit: None,
            cancellation_requested: false,
        }
    }
}
/// What cancellation can truthfully guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum CancelOutcome {
    /// No effect was permitted.
    CancelledBeforeCommit,
    /// The committed outcome cannot be undone.
    AlreadyCommitted,
    /// The retained record is terminal and was not changed.
    AlreadyFinished,
    /// Work may have begun; inspect the operation rather than replaying it.
    Pending,
}

/// Cancellation adds a typed outcome while preserving every released `OpRecord` field.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[non_exhaustive]
pub struct OpCancelResult {
    /// Full retained record, flattened for released-client compatibility.
    #[serde(flatten)]
    pub operation: OpRecord,
    /// Cancellation guarantee, independent of the operation's execution state.
    pub outcome: OpenEnum<CancelOutcome>,
}

impl OpCancelResult {
    /// Joins the cancellation guarantee to its supporting durable record.
    pub const fn new(operation: OpRecord, outcome: CancelOutcome) -> Self {
        Self {
            operation,
            outcome: OpenEnum::Known(outcome),
        }
    }
}
