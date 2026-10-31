//! Bounded admission to Tokio's blocking pool for short synchronous jobs.
//!
//! Long-lived loops never run here: pumps and the database writer own dedicated threads.

use std::sync::Arc;

use tokio::sync::Semaphore;
use tokio::task::JoinHandle;

use super::error::SchedulerError;

/// At most `limit` short blocking jobs at once; one more is refused, never queued.
#[derive(Debug, Clone)]
pub struct BlockingGate {
    slots: Arc<Semaphore>,
    limit: usize,
}

impl BlockingGate {
    /// A gate of `limit` slots.
    #[must_use]
    pub fn new(limit: usize) -> Self {
        Self {
            slots: Arc::new(Semaphore::new(limit)),
            limit,
        }
    }

    /// Runs `job` on the blocking pool while it holds a slot.
    ///
    /// # Errors
    ///
    /// Returns `backpressure` when every slot is busy; `job` is dropped unrun then.
    pub fn submit<F, R>(&self, job: F) -> Result<JoinHandle<R>, SchedulerError>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        let slot = Arc::clone(&self.slots)
            .try_acquire_owned()
            .map_err(|_| SchedulerError::BlockingSaturated { limit: self.limit })?;
        Ok(tokio::task::spawn_blocking(move || {
            let result = job();
            drop(slot);
            result
        }))
    }

    /// Slots free now.
    #[must_use]
    pub fn available(&self) -> usize {
        self.slots.available_permits()
    }
}
