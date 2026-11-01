//! The runtime's side of a planned restart: attach streams close `server_restart` and input
//! readiness is revoked before the old listeners close; afterwards the runtime reports the
//! successor identity, geometry events name its boot, new streams and the census report
//! into the successor's health, and attaches resume.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::health::Health;
use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_protocol::errors::BoundaryError;
use ovis_terminal_store::OpId;

use super::actor::Runtime;
use super::boot::{NoSections, store};
use super::error::internal;

impl Runtime {
    /// Closes every attach stream `server_restart` and revokes input readiness under `op`.
    pub(super) fn restarting(&self, op: &OpId) -> Result<(), BoundaryError> {
        for live in self.live.values() {
            live.hub
                .restarting(op)
                .map_err(internal("close attach streams for a restart"))?;
        }
        Ok(())
    }

    /// Serves under `identity`: geometry events name its boot, streams opened from now on
    /// and the census report into `health`, and attaches resume.
    pub(super) fn recovered(
        &mut self,
        (identity, state, health): (RuntimeIdentity, &Path, Arc<Health>),
    ) -> Result<(), BoundaryError> {
        self.identity = identity;
        let store = store(state, "geometry.db").map_err(internal("reopen the geometry log"))?;
        let reopened = Coordinator::new(store, identity, NoSections);
        std::mem::replace(&mut self.geometry_log, reopened)
            .close()
            .map_err(internal("close the geometry log of the old boot"))?;
        self.setup = self.setup.clone().reporting_to(Arc::clone(&health));
        self.health = health;
        self.census = None;
        self.publish_census();
        for live in self.live.values() {
            live.hub
                .recovered()
                .map_err(internal("resume attaches after a restart"))?;
        }
        Ok(())
    }
}
