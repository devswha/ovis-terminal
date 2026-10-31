//! The operation's cancellation request as seen by Git invocations.

use tokio::sync::watch;

/// Resolves once the owning operation asks to stop; a dropped sender counts as a request.
#[derive(Debug, Clone)]
pub struct Cancel {
    requested: Option<watch::Receiver<bool>>,
}

impl Cancel {
    /// Follows a cancellation flag that becomes `true` when the operation is cancelled.
    #[must_use]
    pub const fn new(requested: watch::Receiver<bool>) -> Self {
        Self {
            requested: Some(requested),
        }
    }

    /// Never requested; used to inspect what a cancelled command already committed.
    pub(super) const fn never() -> Self {
        Self { requested: None }
    }

    pub(super) async fn requested(&mut self) {
        match &mut self.requested {
            // A closed channel means the owner is gone, which also ends the work.
            Some(requested) => match requested.wait_for(|requested| *requested).await {
                Ok(_) | Err(_) => {}
            },
            None => std::future::pending().await,
        }
    }
}
