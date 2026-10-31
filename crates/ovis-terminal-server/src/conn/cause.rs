//! Orders competing terminal causes so teardown cannot rewrite an earlier outcome.

use std::sync::OnceLock;

use tokio::task::JoinSet;

use super::ConnectionError;
use super::budget::{Budget, Overflow};
use crate::peer_gone;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalCause {
    Connection,
    Overflow(Overflow),
}

/// The first terminal cause selected by the connection, set exactly once.
#[derive(Debug, Default)]
pub(super) struct FirstTerminalCause {
    first: OnceLock<TerminalCause>,
}

impl FirstTerminalCause {
    /// Records a driver or writer terminal path when it wins the ordering.
    pub(super) fn record_connection(&self) -> bool {
        self.first.set(TerminalCause::Connection).is_ok()
    }

    /// Records overflow when it wins the ordering.
    pub(super) fn record_overflow(&self, overflow: Overflow) -> bool {
        self.first.set(TerminalCause::Overflow(overflow)).is_ok()
    }

    /// Resolves the connection result against an earlier overflow, if any.
    pub(super) fn resolve<T>(&self, connection: T, overflow: impl FnOnce(Overflow) -> T) -> T {
        match self.first.get() {
            Some(TerminalCause::Overflow(hit)) => overflow(*hit),
            Some(TerminalCause::Connection) | None => connection,
        }
    }
}

/// Joins the writer and resolves the connection's first terminal cause.
pub(super) async fn resolve_connection(
    budget: &Budget,
    writers: &mut JoinSet<Result<(), ConnectionError>>,
    connection: Result<(), ConnectionError>,
) -> Result<(), ConnectionError> {
    let written = match writers.join_next().await {
        Some(Ok(Err(ConnectionError::Io(error)))) if peer_gone(&error) => Ok(()),
        Some(Ok(written)) => written,
        Some(Err(error)) => Err(ConnectionError::Io(std::io::Error::other(error))),
        None => Ok(()),
    };
    budget.resolve_terminal(connection.and(written), |hit| {
        Err(ConnectionError::Backpressure {
            limit: hit.limit,
            queued: hit.queued,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{FirstTerminalCause, Overflow};

    const OVERFLOW: Overflow = Overflow {
        limit: 8,
        queued: 0,
    };

    #[test]
    fn overflow_remains_first_after_connection_teardown_starts() {
        let cause = FirstTerminalCause::default();
        assert!(cause.record_overflow(OVERFLOW));
        assert!(!cause.record_connection());
        assert_eq!(cause.resolve(None, Some), Some(OVERFLOW));
    }

    #[test]
    fn connection_end_remains_first_after_a_later_overflow() {
        let cause = FirstTerminalCause::default();
        assert!(cause.record_connection());
        assert!(!cause.record_overflow(OVERFLOW));
        assert_eq!(cause.resolve(None, Some), None);
    }
}
