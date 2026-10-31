//! Waits armed in the current boot, so a restart can end each with an explicit outcome.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use ovis_terminal_protocol::domain::restart::InterruptedWait;

use crate::waits::CancelScope;

#[derive(Debug, Default)]
struct Table {
    next: u64,
    armed: BTreeMap<u64, (InterruptedWait, CancelScope)>,
}

/// The waits of one control boot.
#[derive(Debug, Clone, Default)]
pub struct ArmedWaits {
    table: Arc<Mutex<Table>>,
}

/// One armed wait; dropping it, once its ticket resolved, removes it from the boot.
#[derive(Debug)]
#[must_use = "dropping the guard forgets the wait before it resolves"]
pub struct ArmedWait {
    table: Arc<Mutex<Table>>,
    id: u64,
    scope: CancelScope,
}

impl ArmedWait {
    /// The scope to arm the wait under; a restart cancels it.
    pub const fn scope(&self) -> &CancelScope {
        &self.scope
    }
}

impl Drop for ArmedWait {
    fn drop(&mut self) {
        lock(&self.table).armed.remove(&self.id);
    }
}

/// Each update is one insert or removal, so a poisoned table is still consistent.
fn lock(table: &Mutex<Table>) -> MutexGuard<'_, Table> {
    table.lock().unwrap_or_else(PoisonError::into_inner)
}

impl ArmedWaits {
    /// No wait armed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers a wait that resumes at `wait.resume_cursor`; arm it under the guard's scope.
    pub fn arm(&self, wait: InterruptedWait) -> ArmedWait {
        let scope = CancelScope::new();
        let mut table = lock(&self.table);
        let id = table.next;
        table.next = table.next.wrapping_add(1);
        table.armed.insert(id, (wait, scope.clone()));
        drop(table);
        ArmedWait {
            table: Arc::clone(&self.table),
            id,
            scope,
        }
    }

    /// Cancels every armed wait, so each resolves `cancelled`, and names them.
    pub fn interrupt(&self) -> Vec<InterruptedWait> {
        let armed = std::mem::take(&mut lock(&self.table).armed);
        armed
            .into_values()
            .map(|(wait, scope)| {
                scope.cancel();
                wait
            })
            .collect()
    }

    /// Waits armed now.
    pub fn len(&self) -> usize {
        lock(&self.table).armed.len()
    }

    /// Whether no wait is armed.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::errors::OpId;
    use ovis_terminal_protocol::schema::ids::TerminalId;
    use ovis_terminal_protocol::schema::scalar::ByteOffset;

    use super::*;

    fn wait(op: &str) -> InterruptedWait {
        InterruptedWait::new(
            OpId::new(op.to_owned()),
            TerminalId::new("terminal:4".to_owned()),
            ByteOffset::new(3),
        )
    }

    #[test]
    fn interrupting_cancels_every_armed_scope_and_a_resolved_wait_is_not_reported() {
        let waits = ArmedWaits::new();
        let resolved = waits.arm(wait("resolved"));
        let pending = waits.arm(wait("pending"));
        drop(resolved);
        assert_eq!(waits.len(), 1);
        assert_eq!(waits.interrupt(), vec![wait("pending")]);
        assert!(pending.scope().is_cancelled());
        assert!(waits.is_empty());
        drop(pending);
        assert_eq!(waits.interrupt(), Vec::new());
    }
}
