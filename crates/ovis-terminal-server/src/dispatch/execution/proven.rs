//! Commits an effect that can also end in a proven failure.
//!
//! [`Execution::commit`] records every effect error as `outcome_unknown`. An effect that
//! knows its failure left nothing behind, such as a refused digest or an activation that
//! selected the previous build again, reports [`Unsettled::Proven`] instead, and the
//! ledger keeps that typed failure as the operation's outcome.

use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot, Operations};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::{Completed, Execution, now};
use crate::dispatch::DispatchError;

/// How an effect inside [`Execution::commit_proven`] ended without a result.
#[derive(Debug)]
#[non_exhaustive]
pub enum Unsettled {
    /// The failure's outcome is known; it is retained as the operation's failure.
    Proven(DispatchError),
    /// Effects may have begun; the operation is recorded `outcome_unknown`.
    Uncertain(DispatchError),
}

impl<D, P> Execution<D, P>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    /// Like [`Execution::commit`], but a proven failure settles the operation `failed`.
    ///
    /// # Errors
    ///
    /// The effect's failure, or authority, cancellation, encoding or persistence failures.
    pub async fn commit_proven<F, R>(self, effect: F) -> Result<Completed, DispatchError>
    where
        F: FnOnce() -> Result<R, Unsettled> + Send + 'static,
        R: Serialize + Send + 'static,
    {
        tokio::task::spawn_blocking(move || {
            let operations = Operations::new(&self.state.core, self.context.clone().at(now()?));
            let permission = self.permit(&operations)?;
            let encoded = effect().and_then(|result| {
                serde_json::to_value(result).map_err(|error| Unsettled::Uncertain(error.into()))
            });
            match encoded {
                Ok(result) => {
                    operations.commit(permission, result.clone())?;
                    Ok(Completed(result))
                }
                Err(Unsettled::Proven(error)) => {
                    operations.fail_running(permission, error.boundary().data().clone())?;
                    Err(error)
                }
                Err(Unsettled::Uncertain(error)) => {
                    operations.outcome_unknown(permission)?;
                    Err(error)
                }
            }
        })
        .await?
    }
}
