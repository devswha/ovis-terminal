//! `machine.add`: connect, authenticate and trust before anything is recorded.

use std::sync::{Arc, PoisonError};

use ovis_terminal_authz::{Action, Session};
use ovis_terminal_core::events::{EventData, SnapshotProducer};
use ovis_terminal_core::ops::{OperationEvent, OperationSnapshot};
use ovis_terminal_protocol::schema::methods::machine::{MachineAddParams, MachineRecord};
use ovis_terminal_remote::identity::{
    Connected, Credential, Name, NameHold, Trust, Trusted, fresh_challenge,
};
use ovis_terminal_remote::transport::{Handshake, Link, TransportSpec};
use serde::Serialize;
use serde::de::DeserializeOwned;

use super::MachineLane;
use crate::dispatch::{Access, Completed, DispatchError, Execution, Method};

/// A name held while an add runs; dropped unused, it is released.
struct Held {
    lane: Arc<MachineLane>,
    hold: Option<NameHold>,
}

impl Held {
    fn new(lane: &Arc<MachineLane>, name: Name) -> Result<Self, DispatchError> {
        let hold = lane.machines()?.hold(name)?;
        Ok(Self {
            lane: Arc::clone(lane),
            hold: Some(hold),
        })
    }

    fn record(mut self, connected: Connected<Link>) -> Result<MachineRecord, DispatchError> {
        let hold = self.hold.take().ok_or(DispatchError::Registration)?;
        Ok(self.lane.machines()?.insert(hold, connected)?)
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        if let Some(hold) = self.hold.take() {
            let mut machines = self
                .lane
                .machines
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            machines.release(hold);
        }
    }
}

/// `machine.add`: the machine is recorded only after its runtime proved the expected
/// identity over a fresh challenge with the presented credential.
#[derive(Debug)]
pub struct MachineAdd(Arc<MachineLane>);

/// Where an add connects and whom it trusts there.
struct Target {
    spec: TransportSpec,
    trust: Trust,
}

impl MachineAdd {
    /// Adds machines to `lane`.
    #[must_use]
    pub fn new(lane: &Arc<MachineLane>) -> Self {
        Self(Arc::clone(lane))
    }

    /// Connects and verifies; `None` when the caller cancelled first.
    async fn connect<D, P>(
        &self,
        target: Target,
        execution: &Execution<D, P>,
    ) -> Result<Option<(Trusted, Link)>, DispatchError>
    where
        D: Send,
        P: Send + Sync,
    {
        let Target { spec, trust } = target;
        let challenge = fresh_challenge();
        let mut link = Link::spawn(&spec)?;
        let handshake = Handshake::new(challenge, self.0.handshake_bound);
        let hello = tokio::select! {
            hello = link.handshake(trust.credential(), handshake) => hello?,
            () = execution.cancelled() => return Ok(None),
        };
        let trusted = trust.admit(hello, challenge)?;
        self.0.machines()?.ensure_new(&trusted)?;
        Ok(Some((trusted, link)))
    }
}

impl<D, P> Method<D, P> for MachineAdd
where
    D: EventData + From<OperationEvent> + Clone + Serialize + DeserializeOwned + Send + 'static,
    P: SnapshotProducer + Send + Sync + 'static,
    P::Sections: AsRef<OperationSnapshot> + DeserializeOwned,
{
    const NAME: &'static str = "machine.add";
    type Params = MachineAddParams;

    fn resolve(&self, _: &Self::Params, _: &Session) -> Result<Access, DispatchError> {
        Ok(self.0.access(Action::Administer))
    }

    async fn run(
        &self,
        params: Self::Params,
        execution: Execution<D, P>,
    ) -> Result<Completed, DispatchError> {
        let name = Name::try_from(&params.name)?;
        let trust = Trust::required(
            params.expected_machine_id,
            Credential::new(params.credential),
        )?;
        let spec = TransportSpec::parse(&params.transport)?;
        let held = Held::new(&self.0, name)?;
        let connected = self.connect(Target { spec, trust }, &execution).await?;
        // A cancelled add commits nothing: the commit sees the cancellation before the effect.
        execution
            .commit(move || {
                let (trusted, link) = connected.ok_or(DispatchError::Closed)?;
                held.record(Connected::new(trusted, link))
            })
            .await
    }
}
