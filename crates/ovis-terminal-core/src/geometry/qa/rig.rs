//! Real authority, leases, durable coordinator, screen and geometry controller on one terminal.

use std::path::PathBuf;

use ovis_terminal_core::events::{Coordinator, Cursor, EventError, Filter, SnapshotProducer};
use ovis_terminal_core::geometry::{
    Dispatch, Fence, GeometryController, GeometryEpoch, GeometryError, GeometryEvent, Outcome,
    Requested, Settled,
};
use ovis_terminal_core::leases::{Acquire, ControlSlot, TerminalLeases};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, LeaseId, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
    TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, Resize, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::errors::OpId;
use ovis_terminal_protocol::schema::methods::attach::AttachResizeParams;
use ovis_terminal_store::{Snapshot, Store, StoreOptions};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};
use serde_json::json;
use uuid::Uuid;

use super::TestResult;
use super::fixture::{Client, Fixture, NOW, WRITER};

/// Geometry events carry no snapshot section of their own.
#[derive(Debug)]
#[non_exhaustive]
pub struct NoSections;

impl SnapshotProducer for NoSections {
    type Sections = ();

    fn capture(&self, _snapshot: &Snapshot, _filter: &Filter) -> Result<(), EventError> {
        Ok(())
    }
}

/// A temporary store root removed when the rig is dropped.
#[derive(Debug)]
pub struct Root(PathBuf);

impl Drop for Root {
    fn drop(&mut self) {
        let removed = std::fs::remove_dir_all(&self.0);
        assert!(removed.is_ok(), "geometry store cleanup: {removed:?}");
    }
}

/// The fixture terminal as the holder names it.
pub fn holder_target(process: u64) -> TerminalTarget {
    TerminalTarget {
        terminal_id: HolderTerminalId::new("terminal:9".into()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(process)),
    }
}

/// A screen of `target` at `cols` x `rows`.
///
/// # Errors
///
/// Fails for a grid the emulator rejects.
pub fn screen(target: TerminalTarget, cols: u16, rows: u16) -> TestResult<Screen> {
    let vt = VtConfig::new(Geometry::new(cols, rows)?, HistoryLimit::new(200));
    Ok(Screen::new(
        target,
        ScreenConfig::new(vt, ScreenBounds::DEFAULT),
    )?)
}

/// A fresh client-minted operation id.
pub fn op() -> OpId {
    OpId::new(Uuid::now_v7().to_string())
}

/// `attach.resize` params as a client sends them on the wire.
///
/// # Errors
///
/// Fails when the request does not decode.
pub fn params(op: &OpId, lease: LeaseId, size: (u32, u32)) -> TestResult<AttachResizeParams> {
    Ok(serde_json::from_value(json!({
        "op_id": op.as_str(),
        "target": {"terminal_id": "terminal:9"},
        "lease_id": lease.to_string(),
        "geometry": {"cols": size.0, "rows": size.1},
    }))?)
}

/// The operator holds interact and geometry; events commit through a real coordinator.
#[derive(Debug)]
pub struct Rig {
    /// Authority and lease identities.
    pub fixture: Fixture,
    /// Lease table of the terminal.
    pub leases: TerminalLeases,
    /// Holder of both control slots.
    pub operator: Client,
    /// The operator's interact lease.
    pub input: LeaseId,
    /// The operator's geometry lease.
    pub grid: LeaseId,
    /// The terminal's screen.
    pub screen: Screen,
    /// Unit under test.
    pub geometry: GeometryController,
    /// Durable commit path.
    pub core: Coordinator<GeometryEvent, NoSections>,
    /// Removed after `core` closed its store.
    _root: Root,
}

impl Rig {
    /// The operator leases geometry and input on `screen`, ready at epoch 0.
    ///
    /// # Errors
    ///
    /// Fails when any setup step is refused.
    pub fn new(screen: Screen) -> TestResult<Self> {
        let mut fixture = Fixture::new()?;
        let mut leases = fixture.leases()?;
        let operator = fixture.client("operator", WRITER)?;
        let input = fixture.interact(&mut leases, &operator)?;
        let grid = fixture.fresh()?;
        leases.acquire(operator.caller(), Acquire::new(ControlSlot::Geometry, grid))?;
        let terminal = TerminalId::try_from(9)?;
        let geometry = GeometryController::new(terminal, screen.clone(), GeometryEpoch::new(0))?;
        let root = Root(std::env::temp_dir().join(format!("ovis-geometry-{}", Uuid::now_v7())));
        let identity = RuntimeIdentity::new(
            HostIdentity::new(
                MachineId::new(Uuid::from_u128(26))?,
                HostBootId::try_from(1)?,
            ),
            RuntimeId::try_from(1)?,
            ServerBootId::try_from(1)?,
        );
        let core = Coordinator::new(
            Store::open(&root.0, StoreOptions::default())?,
            identity,
            NoSections,
        );
        Ok(Self {
            fixture,
            leases,
            operator,
            input,
            grid,
            screen,
            geometry,
            core,
            _root: root,
        })
    }

    /// The operator asks for `size` under its geometry lease.
    ///
    /// # Errors
    ///
    /// Fails when the lease table refuses; the controller's answer is returned as is.
    pub fn resize(
        &mut self,
        op: &OpId,
        size: (u32, u32),
    ) -> TestResult<Result<Requested, GeometryError>> {
        let admitted = self
            .leases
            .authorize_resize(self.operator.caller(), self.grid)?;
        Ok(self
            .geometry
            .request(admitted, &params(op, self.grid, size)?))
    }

    /// Commits the change event, then applies the grid; returns the holder message.
    ///
    /// # Errors
    ///
    /// Fails when the commit or the controller refuses.
    pub fn start(&mut self, dispatch: Dispatch) -> TestResult<(Resize, Cursor)> {
        let (unit, committing) = dispatch.into_commit(store_op());
        let receipt = self.core.commit(unit)?;
        let resize = self.geometry.apply(&committing, receipt)?;
        Ok((resize, receipt))
    }

    /// The holder acknowledged `epoch`.
    ///
    /// # Errors
    ///
    /// Returns the controller's refusal.
    pub fn acknowledged(&mut self, epoch: GeometryEpoch) -> Result<Settled, GeometryError> {
        self.geometry
            .acknowledged(epoch, Fence::new(&self.leases, NOW))
    }

    /// Commits a completed transition's events.
    ///
    /// # Errors
    ///
    /// Fails when it has no events or the commit fails.
    pub fn publish(&self, settled: &Settled) -> TestResult<Cursor> {
        let unit = settled.commit(store_op()).ok_or("no events to commit")?;
        Ok(self.core.commit(unit)?)
    }
}

/// The dispatch of a request that must have started a transition.
///
/// # Errors
///
/// Fails for any other outcome.
pub fn dispatched(requested: Requested) -> TestResult<Dispatch> {
    let Outcome::Dispatched(dispatch) = requested.outcome else {
        return Err(format!("expected a dispatch, got {:?}", requested.outcome).into());
    };
    Ok(dispatch)
}

/// A fresh store operation id for one commit.
pub fn store_op() -> ovis_terminal_store::OpId {
    ovis_terminal_store::OpId::new(Uuid::now_v7().to_string())
}
