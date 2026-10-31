//! Retained receipts of one terminal and the only transitions they allow.

use std::collections::VecDeque;

use ovis_terminal_protocol::errors::OpId;

use super::{AgentStage, Delivery, InputSeq, Receipt, ReceiptError, Stage, StageReport};

/// Settled receipts kept for lookup; queued receipts are bounded by the input queue.
const RETAINED: usize = 1024;

/// Receipts in admission order.
#[derive(Debug, Default)]
pub(in crate::input) struct ReceiptBook {
    receipts: VecDeque<Receipt>,
}

const fn advances(from: Delivery, to: Delivery) -> bool {
    match from {
        Delivery::Queued => !matches!(to, Delivery::Queued),
        // Late holder evidence resolves an unknown outcome; nothing else moves a settled one.
        Delivery::OutcomeUnknown => matches!(
            to,
            Delivery::PtyWritten { .. } | Delivery::Partial { .. } | Delivery::Fenced { .. }
        ),
        Delivery::PtyWritten { .. } | Delivery::Partial { .. } | Delivery::Fenced { .. } => false,
    }
}

impl ReceiptBook {
    pub(in crate::input) fn insert(&mut self, receipt: Receipt) {
        self.receipts.push_back(receipt);
        if self.receipts.len() > RETAINED
            && let Some(oldest) = self
                .receipts
                .iter()
                .position(|kept| kept.delivery != Delivery::Queued)
        {
            self.receipts.remove(oldest);
        }
    }

    pub(in crate::input) fn by_op(&self, op_id: &OpId) -> Option<&Receipt> {
        self.receipts.iter().find(|receipt| receipt.op_id == *op_id)
    }

    pub(in crate::input) fn through(&self, seq: InputSeq) -> impl Iterator<Item = &Receipt> {
        self.receipts
            .iter()
            .filter(move |receipt| receipt.queued_seq <= seq)
    }

    /// Moves a receipt forward; returns the updated receipt only when it changed.
    pub(in crate::input) fn settle(
        &mut self,
        seq: InputSeq,
        delivery: Delivery,
    ) -> Option<Receipt> {
        let receipt = self
            .receipts
            .iter_mut()
            .find(|receipt| receipt.queued_seq == seq)
            .filter(|receipt| advances(receipt.delivery, delivery))?;
        receipt.delivery = delivery;
        Some(receipt.clone())
    }

    pub(in crate::input) fn report(
        &mut self,
        report: &StageReport,
    ) -> Result<Receipt, ReceiptError> {
        let receipt = self
            .receipts
            .iter_mut()
            .find(|receipt| receipt.op_id == report.op_id)
            .ok_or_else(|| ReceiptError::Unknown {
                op_id: report.op_id.clone(),
            })?;
        let written = matches!(
            receipt.delivery,
            Delivery::PtyWritten { .. } | Delivery::Partial { .. }
        );
        let stage = report.stage;
        if !written && stage == AgentStage::Accepted {
            return Err(ReceiptError::BeforeDelivery { stage });
        }
        let slot = receipt.stage_mut(stage);
        match *slot {
            Stage::Unknown => return Err(ReceiptError::Unsupported { stage }),
            Stage::Pending => *slot = Stage::Reported { at_ms: report.at },
            Stage::Reported { .. } => {}
        }
        Ok(receipt.clone())
    }
}
