//! Crash recovery at start-up: an unplanned boot that finds programs recorded as running
//! proves what survived against the recorded operating-system boot, the holder group and
//! each PID's start token, journals every finding and never relaunches anything. Every
//! boot ends by stamping the operating-system boot identity it serves under, so the next
//! unplanned boot can tell a core crash from a host reboot.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::recovery::{GroupId, Recorded, RecoveryPlan, read_os_boot, recover};
use ovis_terminal_core::registry::{RuntimeIdentity, TerminalState};
use ovis_terminal_core::restart::{Boot, Control};
use ovis_terminal_core::terminals::{LifecycleEvent, Producer, Record};
use ovis_terminal_holder::protocol::ProcessStartToken;
use ovis_terminal_protocol::domain::recovery::CrashRecovery;
use ovis_terminal_protocol::schema::ids::{RuntimeLifetimeId, ServerBootId};
use ovis_terminal_store::{Store, StoreOptions};
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};

use super::AssemblyError;

/// The holder group of the server's in-process holder: one session per build.
const GROUP_SESSION: &str = "serve";

/// Scrollback of a screen rebuilt without a checkpoint.
const HISTORY_LINES: u32 = 10_000;

fn group() -> GroupId {
    GroupId::shared(GROUP_SESSION, env!("CARGO_PKG_VERSION"))
}

/// The program `record` names as running, with the evidence recovery checks it against.
fn recorded(record: &Record) -> Option<Recorded> {
    let process = record.lifecycle.process()?;
    let target = record.holder_target.clone()?;
    let handle = process.handle();
    let pid = u32::try_from(handle.pid().get()).ok()?;
    let token = ProcessStartToken::new(handle.start_token().as_str().to_owned());
    Some(Recorded::new(target, (pid, token), group()))
}

/// The programs the previous boot recorded as running, read before this boot adopts the
/// holder and settles them.
pub(super) fn inherited(
    state: &Path,
    identity: RuntimeIdentity,
) -> Result<Vec<Recorded>, AssemblyError> {
    let path = state.join("control").join("terminals");
    if !path.exists() {
        return Ok(Vec::new());
    }
    let store = Store::open(&path, StoreOptions::default())
        .map_err(AssemblyError::start("open the recorded terminals"))?;
    let log: Coordinator<LifecycleEvent, Producer> =
        Coordinator::new(store, identity, Producer::new(identity.runtime_id()));
    let snapshot = log
        .read_snapshot()
        .map_err(AssemblyError::start("read the recorded terminals"));
    log.close()
        .map_err(AssemblyError::start("close the recorded terminals"))?;
    Ok(snapshot?
        .sections
        .terminals
        .iter()
        .filter(|record| record.lifecycle.state() == TerminalState::Running)
        .filter_map(recorded)
        .collect())
}

fn plan(state: &Path, control: &Control) -> Result<RecoveryPlan, AssemblyError> {
    let identity = control
        .identity()
        .map_err(AssemblyError::start("read the boot identity"))?;
    let vt = VtConfig::new(
        Geometry::new(80, 24).map_err(AssemblyError::start("size a rebuilt screen"))?,
        HistoryLimit::new(HISTORY_LINES),
    );
    Ok(RecoveryPlan::new(
        state.to_path_buf(),
        (
            RuntimeLifetimeId::new(identity.runtime_id().to_string()),
            ServerBootId::new(identity.server_boot_id().to_string()),
        ),
        vt,
    ))
}

/// Recovers an unplanned boot that inherited running programs and stamps this boot,
/// off the async runtime because the stores block; `None` when there was nothing to
/// recover.
pub(super) async fn recover_boot(
    state: &Path,
    control: &Arc<Control>,
    (boot, inherited): (&Boot, Vec<Recorded>),
) -> Result<Option<CrashRecovery>, AssemblyError> {
    let (state, control) = (state.to_path_buf(), Arc::clone(control));
    let inherited = if matches!(boot, Boot::Unplanned) {
        inherited
    } else {
        Vec::new()
    };
    tokio::task::spawn_blocking(move || recover_blocking(&state, &control, &inherited))
        .await
        .map_err(AssemblyError::start("recover the previous boot"))?
}

fn recover_blocking(
    state: &Path,
    control: &Control,
    inherited: &[Recorded],
) -> Result<Option<CrashRecovery>, AssemblyError> {
    std::fs::create_dir_all(state.join("recovery"))
        .map_err(AssemblyError::start("create the recovery state"))?;
    let plan = plan(state, control)?;
    if !inherited.is_empty() {
        let recovered = recover(&plan, &[], inherited)
            .map_err(AssemblyError::start("recover the previous boot"))?;
        return Ok(Some(recovered.report));
    }
    if let Ok(current) = read_os_boot() {
        plan.stamp()
            .record(&current)
            .map_err(AssemblyError::start("stamp the operating-system boot"))?;
    }
    Ok(None)
}
