//! The receipt boundary error, its stable kinds and recovery hints.

use ovis_terminal_protocol::errors::OpId;

use super::AgentStage;

/// Why a producer report did not change a receipt; the receipt is unchanged.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReceiptError {
    /// No retained receipt has the operation identity.
    #[error("no retained receipt belongs to operation {op_id:?}")]
    Unknown {
        /// Operation the report named.
        op_id: OpId,
    },
    /// The entry was admitted without a producer able to report this stage.
    #[error("stage {stage:?} has no declared producer for this entry")]
    Unsupported {
        /// Stage the report named.
        stage: AgentStage,
    },
    /// The report claims a stage the delivery evidence does not allow yet.
    #[error("stage {stage:?} cannot precede the PTY write of the entry")]
    BeforeDelivery {
        /// Stage the report named.
        stage: AgentStage,
    },
}

impl ReceiptError {
    /// The stable machine-readable kind clients branch on.
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => "not_found",
            Self::Unsupported { .. } | Self::BeforeDelivery { .. } => "invalid_record",
        }
    }

    /// Recovery guidance published with the kind.
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Unknown { .. } => "report against the op_id of an admitted input entry",
            Self::Unsupported { .. } => "declare the producer before the input is admitted",
            Self::BeforeDelivery { .. } => "report acceptance after the receipt shows pty_written",
        }
    }
}
