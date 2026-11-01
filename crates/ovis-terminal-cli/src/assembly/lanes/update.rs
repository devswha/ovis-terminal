//! `update.*` (plan todo 47): staging from `<state>/updates/releases`, verification through
//! the readiness probe, and activation or rollback, which restart control onto the selected
//! build; the serve loop then hands every listener over as it does after `server.restart`.

use std::sync::Arc;

use ovis_terminal_holder::protocol::HOLDER_PROTOCOL_GENERATION;
use ovis_terminal_protocol::domain::update::{HolderGeneration, ReleaseVersion};
use ovis_terminal_protocol::schema::PROTOCOL_GENERATION;
use ovis_terminal_server::dispatch::lanes::update::UpdateLane;
use ovis_terminal_server::dispatch::lanes::update::control::ControlRestart;
use ovis_terminal_services::update::{Config, CurrentBuild, Requirement, Updates};

use super::{Dispatch, Hosting, stage};
use crate::assembly::AssemblyError;

/// Registers the four update mutations and `update.query` over the install root below
/// the state directory, seeded with the running build.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
) -> Result<(), AssemblyError> {
    let root = hosting.state.join("updates");
    let releases = root.join("releases");
    std::fs::create_dir_all(&releases).map_err(stage("create the release source"))?;
    let version: ReleaseVersion = crate::VERSION
        .parse()
        .map_err(stage("parse this build's version"))?;
    let binary = std::env::current_exe().map_err(stage("locate this build"))?;
    let holder = HolderGeneration::new(u64::from(HOLDER_PROTOCOL_GENERATION));
    let config = Config::new(
        (root.join("install"), releases),
        CurrentBuild::new(version, binary),
        Requirement::new(PROTOCOL_GENERATION, holder),
    );
    let restart = Arc::new(ControlRestart::new(Arc::clone(&hosting.control)));
    let service = Updates::open(config, restart).map_err(stage("open the update install root"))?;
    UpdateLane::new(Arc::new(service), hosting.identity.machine_id())
        .register(dispatch)
        .map_err(stage("register update.*"))
}
