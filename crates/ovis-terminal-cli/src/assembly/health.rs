//! Read-only health of each boot: the recovery facts start-up found, the latency of every
//! settled call, the backlog every attach stream reports, and the holder and live-terminal
//! census the runtime publishes whenever it changes.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_core::health::{Census, Health, RecoveryLedger};
use ovis_terminal_core::recovery::Incidents;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::domain::recovery::CrashRecovery;
use ovis_terminal_protocol::schema::ids::ServerBootId;

use super::AssemblyError;
use super::actor::{Runtime, now};

/// Holder processes one server lifetime owns.
const HOLDERS: u32 = 1;

/// The health of `identity`'s boot over the incident journal below `state`, with the
/// crash report start-up recovery produced, if any.
pub(super) fn open(
    state: &Path,
    identity: RuntimeIdentity,
    report: Option<CrashRecovery>,
) -> Result<Arc<Health>, AssemblyError> {
    let journal = Incidents::new(&state.join("recovery"));
    let boot = ServerBootId::new(identity.server_boot_id().to_string());
    let at = now().map_err(AssemblyError::start("read the clock"))?;
    let ledger = RecoveryLedger::open(journal, (boot, at), report)
        .map_err(AssemblyError::start("open the recovery ledger"))?;
    Ok(Arc::new(Health::new(identity, ledger)))
}

impl Runtime {
    /// Publishes the holder and live-terminal counts when they changed.
    pub(super) fn publish_census(&mut self) {
        let live = self.live.values().filter(|live| !live.ended).count();
        let census = Census::new(HOLDERS, u32::try_from(live).unwrap_or(u32::MAX));
        if self.census != Some(census) {
            self.health.publish_census(census);
            self.census = Some(census);
        }
    }
}
