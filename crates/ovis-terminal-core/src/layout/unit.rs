//! Operation-ledger steps around one planned layout commit unit.

use ovis_terminal_protocol::schema::methods::server::OpState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_store::Snapshot;
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::LayoutError;
use super::plan::Planned;
use super::records::{Change, LayoutEvent};
use crate::events::{EventData, EventError, SnapshotProducer};
use crate::ops::{
    Completion, OperationError, OperationEvent, OperationSnapshot, Operations, Request,
    RunningOperation,
};

pub(super) enum Begun {
    Replayed(Box<Change>),
    Started(RunningOperation),
}

/// Accepts a new operation, or returns the committed result of an identical retry.
pub(super) fn begin<D, P>(
    ops: &Operations<'_, D, P>,
    request: &Request,
) -> Result<Begun, LayoutError>
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let accepted = ops.accept(request)?;
    match (&accepted.state, &accepted.result) {
        (OpenEnum::Known(OpState::Accepted), _) => Ok(Begun::Started(ops.start(request.op_id())?)),
        (OpenEnum::Known(OpState::Committed), Some(result)) => Ok(Begun::Replayed(Box::new(
            serde_json::from_value(result.clone())?,
        ))),
        _ => Err(OperationError::Settled(Box::new(accepted)).into()),
    }
}

/// Commits the planned resources while the operation stays running.
pub(super) fn record<D, P>(
    ops: &Operations<'_, D, P>,
    permit: &RunningOperation,
    plan: impl FnOnce(&Snapshot) -> Result<Planned, LayoutError>,
) -> Result<Change, LayoutError>
where
    D: EventData + From<OperationEvent> + From<LayoutEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let mut outcome = None;
    ops.record_unit(permit, |snapshot| {
        Ok(match plan(snapshot) {
            Ok(planned) => {
                outcome = Some(Ok(planned.change.clone()));
                planned.pending()
            }
            Err(error) => {
                outcome = Some(Err(error));
                Completion::pending()
            }
        })
    })?;
    outcome.unwrap_or(Err(LayoutError::Event(EventError::InvalidRecord)))
}

/// Commits the result, or records the typed rejection as the operation's failure.
pub(super) fn settle<D, P>(
    ops: &Operations<'_, D, P>,
    permit: RunningOperation,
    plan: impl FnOnce(&Snapshot) -> Result<Planned, LayoutError>,
) -> Result<Change, LayoutError>
where
    D: EventData + From<OperationEvent> + From<LayoutEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    let mut outcome = None;
    ops.commit_unit(permit, |snapshot| match plan(snapshot) {
        Ok(planned) => {
            outcome = Some(Ok(planned.change.clone()));
            planned.completion()
        }
        Err(error) => {
            let data = error.data();
            outcome = Some(Err(error));
            Ok(Completion::failed(data))
        }
    })?;
    outcome.unwrap_or(Err(LayoutError::Event(EventError::InvalidRecord)))
}

/// Records `error` as the running operation's outcome and returns it.
pub(super) fn fail<D, P>(
    ops: &Operations<'_, D, P>,
    permit: RunningOperation,
    error: LayoutError,
) -> LayoutError
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned,
    P: SnapshotProducer,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    match ops.commit_unit(permit, |_| Ok(Completion::failed(error.data()))) {
        Ok(_) => error,
        Err(settle) => settle.into(),
    }
}
