//! An in-memory launcher for the template launch lane tests: it records reservations,
//! lease tables and writes, and feeds declared readiness to each real wait hub.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use ovis_terminal_authz::{ResolvedTarget, TargetResource};
use ovis_terminal_core::input::encoding::ModeSnapshot;
use ovis_terminal_core::input::receipts::InputSeq;
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::registry::{
    ProcessGeneration, RegistryIdentities, TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::{StatusFact, TurnPosition, Waits};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::domain::template_launch::{LaunchedTerminal, started_process};
use ovis_terminal_protocol::schema::ids::{MachineId, TerminalAddress, TerminalId as WireId};
use ovis_terminal_protocol::schema::records::AgentState;
use ovis_terminal_protocol::schema::scalar::{
    EventSeq, Incarnation, ReportSeq, TimestampMs, TurnNumber,
};
use ovis_terminal_server::dispatch::DispatchError;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    Boundary, Launch, Launcher, Reservation,
};
use ovis_terminal_vt::{Geometry, HistoryLimit, Vt, VtConfig};

use super::support::runtime;

/// When the launched program declares readiness.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ready {
    /// Already while starting, before any wait could be observed from outside.
    AtStart,
    /// Only when the test reports it.
    Later,
}

#[derive(Debug)]
pub(crate) struct Memory {
    pub(crate) ready: Ready,
    pub(crate) hubs: Vec<(Waits, TerminalLeases)>,
    pub(crate) writes: Vec<Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct MemoryLauncher {
    pub(crate) state: Mutex<Memory>,
    pub(crate) authority: Arc<ovis_terminal_authz::Authority<RegistryIdentities>>,
}

pub(crate) fn idle(waits: &Waits) -> Result<(), DispatchError> {
    let fact = StatusFact::new(
        ReportSeq::new(1),
        AgentState::Idle,
        TurnPosition::new(TurnNumber::new(1), InputSeq::new(0)),
    );
    waits.report(fact).map_err(|_| DispatchError::Closed)
}

impl Launcher for MemoryLauncher {
    fn reserve(&self, _: Launch<'_>) -> Result<Reservation, DispatchError> {
        let closed = |_| DispatchError::Closed;
        let mut state = self.state.lock().map_err(|_| DispatchError::Closed)?;
        let id = u64::try_from(state.hubs.len())
            .map_err(closed)?
            .saturating_add(100);
        let terminal = TerminalId::try_from(id).map_err(|_| DispatchError::Closed)?;
        let target = TerminalTarget {
            terminal_id: HolderTerminalId::new(terminal.to_string()),
            terminal_incarnation: TerminalIncarnation::new(1),
            process_incarnation: Some(ProcessIncarnation::new(1)),
        };
        let slice = NonZeroUsize::new(4096).ok_or(DispatchError::Closed)?;
        let bounds = ScreenBounds::DEFAULT
            .with_work_budget(slice)
            .with_checkpoint_interval(slice);
        let geometry = Geometry::new(80, 24).map_err(|_| DispatchError::Closed)?;
        let config = ScreenConfig::new(VtConfig::new(geometry, HistoryLimit::new(10)), bounds);
        let waits = Waits::new(Screen::new(target, config).map_err(|_| DispatchError::Closed)?);
        let resource = TargetResource::Terminal {
            placement: None,
            terminal,
            incarnation: TerminalGeneration::new(1).map_err(|_| DispatchError::Closed)?,
        };
        let machine = runtime(1).map_err(|_| DispatchError::Closed)?.machine_id();
        let leases = TerminalLeases::new(
            Arc::clone(&self.authority),
            ResolvedTarget::new(machine, resource),
        )
        .map_err(|_| DispatchError::Closed)?;
        state.hubs.push((waits.clone(), leases));
        drop(state);
        let process = ProcessGeneration::new(1).map_err(|_| DispatchError::Closed)?;
        Ok(Reservation::new(terminal, process, waits))
    }

    fn start(
        &self,
        reservation: &Reservation,
        launch: Launch<'_>,
    ) -> Result<LaunchedTerminal, DispatchError> {
        let ready = self.state.lock().map_err(|_| DispatchError::Closed)?.ready;
        if ready == Ready::AtStart {
            idle(&reservation.waits)?;
        }
        let process = started_process(
            Incarnation::new(1),
            (launch.plan.argv.clone(), None),
            TimestampMs::new(0),
        );
        let identity = (
            WireId::new(reservation.terminal.to_string()),
            TerminalAddress::new("local/test".to_owned()),
            MachineId::new("machine:test".to_owned()),
        );
        Ok(LaunchedTerminal::new(
            identity,
            (process, EventSeq::new(0)),
            launch.params,
        ))
    }

    fn boundary(&self, terminal: TerminalId) -> Result<Boundary, DispatchError> {
        let state = self.state.lock().map_err(|_| DispatchError::Closed)?;
        let index = usize::try_from(terminal.get().saturating_sub(100))
            .map_err(|_| DispatchError::Closed)?;
        let view = state.hubs.get(index).ok_or(DispatchError::Closed)?.1.view();
        drop(state);
        let geometry = Geometry::new(80, 24).map_err(|_| DispatchError::Closed)?;
        let modes = ModeSnapshot::capture(&Vt::new(VtConfig::new(geometry, HistoryLimit::new(10))));
        let process = ProcessGeneration::new(1).map_err(|_| DispatchError::Closed)?;
        Ok(Boundary::new(view, modes, process))
    }

    fn write(&self, _: TerminalId, bytes: &[u8]) -> Result<EventSeq, DispatchError> {
        let mut state = self.state.lock().map_err(|_| DispatchError::Closed)?;
        state.writes.push(bytes.to_vec());
        let count = state.writes.len();
        drop(state);
        Ok(EventSeq::new(
            u64::try_from(count).map_err(|_| DispatchError::Closed)?,
        ))
    }
}
