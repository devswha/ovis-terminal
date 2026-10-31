//! Per-stream output credit from cumulative acknowledgements.
//!
//! A viewer may have at most `window` delivered bytes unacknowledged. An
//! acknowledgement names the cumulative filtered-stream offset consumed; one beyond
//! the bytes actually sent, or behind an earlier acknowledgement, is refused and grants
//! nothing.

use thiserror::Error;

use super::offset::StreamOffset;

/// Why an acknowledgement granted no credit.
#[derive(Debug, Error, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum CreditRefusal {
    /// The acknowledgement names bytes the stream never sent.
    #[error("acknowledged offset {} is beyond the {} bytes sent", acknowledged.get(), sent.get())]
    Beyond {
        /// Offset the client acknowledged.
        acknowledged: StreamOffset,
        /// Bytes sent on the stream.
        sent: StreamOffset,
    },
    /// The acknowledgement moves backwards.
    #[error("acknowledged offset {} is behind the earlier {}", acknowledged.get(), previous.get())]
    Regressed {
        /// Offset the client acknowledged.
        acknowledged: StreamOffset,
        /// Offset acknowledged before.
        previous: StreamOffset,
    },
    /// The stream already closed; its identity never grants credit again.
    #[error("the stream has closed")]
    Closed,
}

impl CreditRefusal {
    /// The stable machine-readable reason.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Beyond { .. } => "beyond_sent",
            Self::Regressed { .. } => "regressed",
            Self::Closed => "stream_closed",
        }
    }

    /// Recovery guidance for the reason.
    #[must_use]
    pub const fn hint(&self) -> &'static str {
        match self {
            Self::Beyond { .. } => "acknowledge at most the offset after the last byte received",
            Self::Regressed { .. } => "acknowledge cumulative offsets that never decrease",
            Self::Closed => "open a new stream; a closed stream's credit is never revived",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Credit {
    window: u64,
    sent: StreamOffset,
    acked: StreamOffset,
}

impl Credit {
    pub(super) const fn new(window: u64) -> Self {
        Self {
            window,
            sent: StreamOffset::ZERO,
            acked: StreamOffset::ZERO,
        }
    }

    /// Bytes that may still be sent before the next acknowledgement.
    pub(super) const fn available(&self) -> u64 {
        self.window.saturating_sub(self.sent.since(self.acked))
    }

    pub(super) const fn sent(&self) -> StreamOffset {
        self.sent
    }

    pub(super) fn record(&mut self, len: usize) {
        self.sent = self.sent.advanced(len);
    }

    /// Accepts a cumulative acknowledgement and returns the credit now available.
    pub(super) fn acknowledge(&mut self, at: StreamOffset) -> Result<u64, CreditRefusal> {
        if at > self.sent {
            return Err(CreditRefusal::Beyond {
                acknowledged: at,
                sent: self.sent,
            });
        }
        if at < self.acked {
            return Err(CreditRefusal::Regressed {
                acknowledged: at,
                previous: self.acked,
            });
        }
        self.acked = at;
        Ok(self.available())
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Credit, CreditRefusal, StreamOffset};

    #[test]
    fn an_acknowledgement_beyond_sent_bytes_grants_nothing() {
        let mut credit = Credit::new(8);
        credit.record(8);
        assert_eq!(credit.available(), 0);
        let refused = credit.acknowledge(StreamOffset::new(9));
        assert_eq!(
            refused.map_err(|refusal| refusal.kind()),
            Err("beyond_sent")
        );
        assert_eq!(
            credit.available(),
            0,
            "a refused acknowledgement grants no credit"
        );
        assert_eq!(credit.acknowledge(StreamOffset::new(8)), Ok(8));
        assert!(matches!(
            credit.acknowledge(StreamOffset::new(4)),
            Err(CreditRefusal::Regressed { .. })
        ));
    }

    proptest! {
        #[test]
        fn outstanding_bytes_never_exceed_the_window(
            window in 1u64..10_000,
            steps in proptest::collection::vec((0usize..5_000, 0u64..20_000), 1..40),
        ) {
            let mut credit = Credit::new(window);
            for (send, ack) in steps {
                let allowed = usize::try_from(credit.available()).unwrap_or(usize::MAX);
                credit.record(send.min(allowed));
                let before = credit.available();
                if credit.acknowledge(StreamOffset::new(ack)).is_err() {
                    prop_assert_eq!(credit.available(), before);
                }
                prop_assert!(credit.sent().since(credit.acked) <= window);
            }
        }
    }
}
