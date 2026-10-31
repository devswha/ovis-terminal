//! Cancellation shared by every wait one request armed.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use super::entry::WatchId;
use super::hub::Shared;

#[derive(Debug, Default)]
struct Members {
    cancelled: bool,
    armed: Vec<(Weak<Shared>, WatchId)>,
}

/// Cancels every wait armed under it, resolving each as `cancelled` and detaching it.
///
/// A wait armed after cancellation resolves as `cancelled` without being armed.
#[derive(Debug, Clone, Default)]
pub struct CancelScope {
    members: Arc<Mutex<Members>>,
}

impl CancelScope {
    /// A scope nothing has cancelled yet.
    pub fn new() -> Self {
        Self::default()
    }

    /// Recovers a poisoned scope: each update is one assignment or push.
    fn lock(&self) -> MutexGuard<'_, Members> {
        self.members.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Whether the scope was cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// Cancels every wait armed under the scope and every wait armed later.
    pub fn cancel(&self) {
        let armed = {
            let mut members = self.lock();
            members.cancelled = true;
            std::mem::take(&mut members.armed)
        };
        for (hub, id) in armed {
            if let Some(hub) = hub.upgrade() {
                hub.cancel(id);
            }
        }
    }

    /// Records an armed wait; a scope cancelled meanwhile cancels it at once.
    pub(super) fn enlist(&self, hub: &Arc<Shared>, id: WatchId) {
        let mut members = self.lock();
        if members.cancelled {
            drop(members);
            hub.cancel(id);
            return;
        }
        members.armed.retain(|(hub, _)| hub.strong_count() > 0);
        members.armed.push((Arc::downgrade(hub), id));
    }
}
