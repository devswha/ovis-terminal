//! Registration of the template launch lane, the fixture templates, and the wait hub
//! and status feed of each launched terminal.

use std::num::NonZeroUsize;
use std::path::Path;
use std::sync::Arc;

use ovis_terminal_authz::{Authority, ResolvedTarget, TargetResource};
use ovis_terminal_core::events::Coordinator;
use ovis_terminal_core::input::receipts::InputSeq;
use ovis_terminal_core::leases::TerminalLeases;
use ovis_terminal_core::ops::OperationProducer;
use ovis_terminal_core::registry::{
    MachineId, RegistryIdentities, RuntimeIdentity, TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::{StatusFact, TurnPosition, Waits};
use ovis_terminal_holder::protocol::{
    ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation, TerminalTarget,
};
use ovis_terminal_protocol::schema::records::StatusSnapshot;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_server::dispatch::DispatchError;
use ovis_terminal_server::dispatch::lanes::template_launch::{
    GetFirstMessage, GetTemplate, ListTemplates, RunTerminal, TemplateLaunchLane,
};
use ovis_terminal_services::templates::TemplateCatalog;
use ovis_terminal_vt::{Geometry, HistoryLimit, VtConfig};
use serde_json::json;

use super::agents::AgentParts;
use super::handler::{Dispatch, Event};
use super::launches::RecordingLauncher;
use super::serve::at;
use crate::error::RunnerError;

type Registry = RegistryIdentities;

/// `build@1` and `build@2` start different argv; `silent@1` never hears readiness.
fn templates() -> [(&'static str, serde_json::Value); 3] {
    let build = |version: &str, target: &str| {
        json!({
            "name": "build", "version": version, "description": format!("build agent {version}"),
            "argv": ["/usr/bin/env", "build-agent", "--target", target],
            "env": {"ROLE": "build"},
            "readiness": {"by": "declared", "states": ["idle"]}
        })
    };
    let silent = json!({
        "name": "silent", "version": "1", "description": "an agent that never declares readiness",
        "argv": ["/usr/bin/env", "silent-agent"], "ready_deadline_ms": "400"
    });
    [
        ("build-1.json", build("1", "debug")),
        ("build-2.json", build("2", "release")),
        ("silent-1.json", silent),
    ]
}

/// Registers the template launch methods over `root/templates` and the recording launcher.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    (runtime, root): (RuntimeIdentity, &Path),
    (authority, core, agents): (
        &Arc<Authority<Registry>>,
        &Coordinator<Event, OperationProducer>,
        AgentParts,
    ),
) -> Result<(), RunnerError> {
    let dir = root.join("templates");
    std::fs::create_dir(&dir).map_err(at("create the template directory"))?;
    for (file, definition) in templates() {
        std::fs::write(dir.join(file), definition.to_string()).map_err(at("write a template"))?;
    }
    let events = core
        .subscribe(None)
        .map_err(at("subscribe to status events"))?
        .delivery;
    let launcher = RecordingLauncher::new(
        (&root.join("effects"), root),
        (Arc::clone(authority), runtime),
        (agents, events),
    );
    let lane = TemplateLaunchLane::new(
        (TemplateCatalog::new(dir), launcher),
        Arc::clone(authority),
        runtime.machine_id(),
    );
    dispatch
        .register(RunTerminal::new(&lane))
        .map_err(at("register terminal.run"))?;
    dispatch
        .register(GetFirstMessage::new(&lane))
        .map_err(at("register terminal.first_message"))?;
    dispatch
        .register_read(ListTemplates::new(&lane))
        .map_err(at("register template.list"))?;
    dispatch
        .register_read(GetTemplate::new(&lane))
        .map_err(at("register template.get"))?;
    Ok(())
}

/// A wait hub and an empty lease table for launched `terminal`, process 1.
pub(crate) fn hub(
    terminal: TerminalId,
    (authority, machine): (&Arc<Authority<Registry>>, MachineId),
) -> Result<(Waits, TerminalLeases), DispatchError> {
    let closed = |_| DispatchError::Closed;
    let target = TerminalTarget {
        terminal_id: HolderTerminalId::new(terminal.to_string()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(1)),
    };
    let slice = NonZeroUsize::new(4096).ok_or(DispatchError::Closed)?;
    let bounds = ScreenBounds::DEFAULT
        .with_work_budget(slice)
        .with_checkpoint_interval(slice);
    let geometry = Geometry::new(80, 24).map_err(closed)?;
    let config = ScreenConfig::new(VtConfig::new(geometry, HistoryLimit::new(10)), bounds);
    let screen = Screen::new(target, config).map_err(|_| DispatchError::Closed)?;
    let resource = TargetResource::Terminal {
        placement: None,
        terminal,
        incarnation: TerminalGeneration::new(1).map_err(|_| DispatchError::Closed)?,
    };
    let leases = TerminalLeases::new(
        Arc::clone(authority),
        ResolvedTarget::new(machine, resource),
    )
    .map_err(|_| DispatchError::Closed)?;
    Ok((Waits::new(screen), leases))
}

/// Feeds the root report of a committed status to `waits`; other snapshots are ignored.
pub(crate) fn feed(waits: &Waits, snapshot: &StatusSnapshot) {
    let Some(root) = &snapshot.root else {
        return;
    };
    let OpenEnum::Known(state) = root.report.state else {
        return;
    };
    let fact = StatusFact::new(
        root.report_seq,
        state,
        TurnPosition::new(root.turn, InputSeq::new(0)),
    );
    // A report the hub refuses (stale or after exit) cannot satisfy readiness either.
    drop(waits.report(fact));
}

/// A live fixture terminal already carries the requested name.
#[derive(Debug, thiserror::Error)]
#[error("a live fixture terminal already carries this name")]
pub(crate) struct DuplicateName;

/// The `duplicate_name` refusal of the fixture registry.
pub(crate) fn duplicate_name() -> DispatchError {
    DispatchError::Domain {
        kind: "duplicate_name",
        hint: "choose a name no live terminal carries",
        source: Box::new(DuplicateName),
    }
}
