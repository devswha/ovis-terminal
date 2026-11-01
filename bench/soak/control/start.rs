//! How a control boot begins: a first boot spawns every program; a boot after SIGKILL
//! runs the core's crash recovery; a boot after a planned handover adopts and verifies.

mod handover;

use std::collections::BTreeMap;

use ovis_terminal_core::recovery::{Recovered, RecoveryPlan, read_os_boot, recover};
use ovis_terminal_holder::protocol::{
    HeldTerminal, PreviousCoreState, Spawn, TerminalIncarnation, TerminalTarget, Welcome,
};
use ovis_terminal_protocol::schema::ids::{ProcessPid, RuntimeLifetimeId, ServerBootId};
use ovis_terminal_protocol::schema::scalar::Count;

use super::super::error::SoakError;
use super::super::wire::{BootMode, Identity, Ready, Recovery};
use super::links::Link;
use super::ops;
use super::screens::Screen;
use super::setup::{Entry, Setup};

/// A control boot ready to enter its loop.
#[derive(Debug)]
pub(super) struct Begun {
    /// One link per holder group; `None` for a group that did not answer.
    pub(super) links: Vec<Option<Link>>,
    /// One screen per terminal; `None` for a program that did not survive.
    pub(super) screens: Vec<Option<Screen>>,
    pub(super) ready: Ready,
}

pub(super) fn plan(setup: &Setup, boot: u64) -> Result<RecoveryPlan, SoakError> {
    let identity = (
        RuntimeLifetimeId::new("runtime:soak".to_owned()),
        ServerBootId::new(format!("server-boot:soak-{boot}")),
    );
    Ok(RecoveryPlan::new(
        setup.state.clone(),
        identity,
        setup.config()?,
    ))
}

/// Adopts every group as the successor of `previous`; a group that does not answer
/// has no link and its terminals are not served by this boot.
fn adopt(setup: &Setup, previous: PreviousCoreState) -> (Vec<Option<Link>>, Vec<Option<Welcome>>) {
    setup
        .groups
        .iter()
        .map(|group| match Link::adopt(&group.socket, previous) {
            Ok((link, welcome)) => (Some(link), Some(welcome)),
            Err(_) => (None, None),
        })
        .unzip()
}

fn held<'w>(welcomes: &'w [Option<Welcome>], entry: &Entry) -> Option<&'w HeldTerminal> {
    let group = usize::try_from(entry.group.get()).ok()?;
    welcomes
        .get(group)?
        .as_ref()?
        .terminals
        .iter()
        .find(|held| held.terminal_id == entry.target.terminal_id)
}

/// The identities a boot reports: what the holder holds now, else what was recorded.
fn identities(entries: &[Entry], welcomes: &[Option<Welcome>]) -> Vec<Identity> {
    (0_u64..)
        .zip(entries)
        .map(|(index, entry)| {
            let now = held(welcomes, entry);
            Identity {
                index: Count::new(index),
                target: entry.target.clone(),
                group: entry.group,
                pid: now
                    .and_then(|held| held.pid)
                    .map_or(entry.pid, ProcessPid::new),
                start_token: now
                    .and_then(|held| held.start_token.clone())
                    .unwrap_or_else(|| entry.start_token.clone()),
                input_sequence: now.map(|held| Count::new(held.input_sequence)),
            }
        })
        .collect()
}

fn index_of(entries: &[Entry], target: &TerminalTarget) -> Option<usize> {
    entries
        .iter()
        .position(|entry| entry.target.terminal_id == target.terminal_id)
}

fn fresh(setup: &Setup, boot: u64) -> Result<Begun, SoakError> {
    let config = setup.config()?;
    let (mut links, welcomes) = adopt(setup, PreviousCoreState::Crash);
    let mut entries = Vec::with_capacity(setup.count());
    let mut screens = Vec::with_capacity(setup.count());
    for index in 0..setup.count() {
        let group = setup.group_of(index);
        let link = links
            .get_mut(group)
            .and_then(Option::as_mut)
            .ok_or(SoakError::Protocol {
                expected: "a holder that answers the first adoption",
            })?;
        let spawned = link.spawn(Spawn {
            terminal_id: Setup::terminal_id(index),
            terminal_incarnation: TerminalIncarnation::new(1),
            argv: setup.program.clone(),
            cwd: setup.state.clone(),
            env: BTreeMap::from([("PATH".to_owned(), "/usr/bin:/bin".to_owned())]),
            geometry: (80, 24),
        })?;
        let target = TerminalTarget {
            terminal_id: spawned.terminal_id,
            terminal_incarnation: spawned.terminal_incarnation,
            process_incarnation: Some(spawned.process_incarnation),
        };
        screens.push(Some(Screen::fresh(target.clone(), config)));
        entries.push(Entry {
            target,
            group: Count::new(u64::try_from(group).unwrap_or(u64::MAX)),
            pid: ProcessPid::new(spawned.pid),
            start_token: spawned.start_token,
        });
    }
    setup.save_recorded(&entries)?;
    plan(setup, boot)?.stamp().record(&read_os_boot()?)?;
    let ready = Ready {
        mode: BootMode::Fresh,
        boot: Count::new(boot),
        terminals: identities(&entries, &welcomes),
        recovery: None,
        interrupted: Vec::new(),
    };
    Ok(Begun {
        links,
        screens,
        ready,
    })
}

/// Crash recovery by the core, then this boot's own links, caught up from each
/// survivor's rebuilt head so no retained byte is skipped or fed twice.
fn crash(setup: &Setup, boot: u64) -> Result<Begun, SoakError> {
    let entries = setup.recorded()?;
    let recorded: Vec<_> = entries
        .iter()
        .map(|entry| setup.as_recorded(entry))
        .collect();
    let Recovered {
        report,
        screens: rebuilt,
        links: adopted,
        ..
    } = recover(&plan(setup, boot)?, &setup.holder_groups(), &recorded)?;
    drop(adopted);
    let (mut links, welcomes) = adopt(setup, PreviousCoreState::Crash);
    let mut screens: Vec<Option<Screen>> = entries.iter().map(|_| None).collect();
    for (target, restored) in rebuilt {
        let Some(index) = index_of(&entries, &target) else {
            continue;
        };
        let mut screen = Screen::restored(target, restored);
        let group = entries
            .get(index)
            .and_then(|entry| usize::try_from(entry.group.get()).ok());
        if let Some(link) = group.and_then(|group| links.get_mut(group)?.as_mut()) {
            let tail = link.tail(&screen.target, screen.head())?;
            screen.admit(tail.from_offset, &tail.bytes);
        }
        if let Some(slot) = screens.get_mut(index) {
            *slot = Some(screen);
        }
    }
    let ready = Ready {
        mode: BootMode::Crash,
        boot: Count::new(boot),
        terminals: identities(&entries, &welcomes),
        recovery: Some(Recovery::Crash(Box::new(report))),
        interrupted: ops::interrupted(&setup.state)?,
    };
    Ok(Begun {
        links,
        screens,
        ready,
    })
}

pub(super) fn begin(setup: &Setup, (mode, boot): (BootMode, u64)) -> Result<Begun, SoakError> {
    match mode {
        BootMode::Fresh => fresh(setup, boot),
        BootMode::Crash => crash(setup, boot),
        BootMode::Handover => handover::begin(setup, boot),
    }
}
