//! `template.list` and `template.get` over T35's landed lane, as unrecorded reads of
//! `<state>/templates`. Template launches are not served here: `terminal.run` launches
//! argv programs only, so neither `RunTerminal` nor `terminal.first_message` (which only
//! ever answers a message a template launch queued) is registered. The lane is generic
//! over its launcher; the daemon's launcher refuses every step, and no registered method
//! reaches it.

use std::sync::Arc;

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::domain::template_launch::LaunchedTerminal;
use ovis_terminal_protocol::schema::scalar::EventSeq;
use ovis_terminal_server::dispatch::DispatchError;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    Boundary, GetTemplate, Launch, Launcher, ListTemplates, Reservation, TemplateLaunchLane,
};
use ovis_terminal_services::templates::TemplateCatalog;

use super::{Dispatch, Hosting, stage};
use crate::assembly::AssemblyError;

/// Why the daemon's template launcher refused.
#[derive(Debug, thiserror::Error)]
enum LaunchRefused {
    /// The daemon launches argv programs only.
    #[error("this daemon does not launch templates")]
    TemplatesNotLaunched,
}

impl LaunchRefused {
    const fn kind(&self) -> &'static str {
        match self {
            Self::TemplatesNotLaunched => "unsupported_field",
        }
    }

    const fn hint(&self) -> &'static str {
        match self {
            Self::TemplatesNotLaunched => "launch the template's argv with terminal.run",
        }
    }
}

fn refused() -> DispatchError {
    let error = LaunchRefused::TemplatesNotLaunched;
    DispatchError::Domain {
        kind: error.kind(),
        hint: error.hint(),
        source: Box::new(error),
    }
}

/// Refuses every launch step.
#[derive(Debug)]
struct NoTemplateLaunches;

impl Launcher for NoTemplateLaunches {
    fn reserve(&self, _: Launch<'_>) -> Result<Reservation, DispatchError> {
        Err(refused())
    }

    fn start(&self, _: &Reservation, _: Launch<'_>) -> Result<LaunchedTerminal, DispatchError> {
        Err(refused())
    }

    fn boundary(&self, _: TerminalId) -> Result<Boundary, DispatchError> {
        Err(refused())
    }

    fn write(&self, _: TerminalId, _: &[u8]) -> Result<EventSeq, DispatchError> {
        Err(refused())
    }
}

/// Registers `template.list` and `template.get`.
pub(super) fn register(
    dispatch: &mut Dispatch,
    hosting: &Hosting<'_>,
) -> Result<(), AssemblyError> {
    let dir = hosting.state.join("templates");
    std::fs::create_dir_all(&dir).map_err(stage("create the template directory"))?;
    let lane = TemplateLaunchLane::new(
        (TemplateCatalog::new(dir), NoTemplateLaunches),
        Arc::clone(&hosting.authority),
        hosting.identity.machine_id(),
    );
    dispatch
        .register_read(ListTemplates::new(&lane))
        .map_err(stage("register template.list"))?;
    dispatch
        .register_read(GetTemplate::new(&lane))
        .map_err(stage("register template.get"))
}
