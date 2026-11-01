//! The boot after a planned handover: the predecessor saved every screen at its head
//! and said goodbye; this boot adopts as its successor, proves each program by holder
//! record and operating-system start token, and rebuilds each screen from the saved
//! checkpoint plus the retained bytes after it.

use ovis_terminal_core::recovery::{BootVerdict, Fence, Retained, read_os_boot, restore};
use ovis_terminal_holder::process::{ProcessId, StartTokenError, read_start_token};
use ovis_terminal_holder::protocol::{HeldTerminal, PreviousCoreState};
use ovis_terminal_protocol::domain::recovery::{
    LossReason, Survival, TerminalRecovery, UnknownReason,
};
use ovis_terminal_protocol::schema::ids::TerminalId;
use ovis_terminal_protocol::schema::scalar::{Count, Incarnation};

use super::super::super::error::SoakError;
use super::super::super::wire::{BootMode, Ready, Recovery};
use super::super::links::Link;
use super::super::ops;
use super::super::screens::Screen;
use super::super::setup::{Entry, Setup};
use super::{Begun, adopt, held, identities, plan};

/// What the evidence proves about `entry` after a handover in boot `boot`.
fn survival(
    boot: BootVerdict,
    (link, now): (Option<&Link>, Option<&HeldTerminal>),
    entry: &Entry,
) -> Survival {
    match boot {
        BootVerdict::Same => {}
        BootVerdict::Rebooted => return Survival::RestoredMetadata {},
        _ => {
            return Survival::Unknown {
                reason: UnknownReason::BootEvidenceUnreadable,
            };
        }
    }
    if link.is_none() {
        return Survival::Unknown {
            reason: UnknownReason::HolderUnreachable,
        };
    }
    let Some(held) = now else {
        return Survival::Lost {
            reason: LossReason::NotHeld,
        };
    };
    let same = held.exit.is_none()
        && held.process_incarnation == entry.target.process_incarnation
        && held.pid == Some(entry.pid.get())
        && held.start_token.as_ref() == Some(&entry.start_token);
    if !same || entry.pid.get() <= 1 {
        let reason = if held.exit.is_some() {
            LossReason::Exited
        } else {
            LossReason::Replaced
        };
        return Survival::Lost { reason };
    }
    match read_start_token(ProcessId::new(entry.pid.get())) {
        Ok(token) if token == entry.start_token => Survival::Survived {},
        Ok(_) => Survival::Lost {
            reason: LossReason::Replaced,
        },
        Err(StartTokenError::ProcessGone { .. }) => Survival::Lost {
            reason: LossReason::Gone,
        },
        Err(_) => Survival::Unknown {
            reason: UnknownReason::ProcessEvidenceUnreadable,
        },
    }
}

pub(super) fn begin(setup: &Setup, boot: u64) -> Result<Begun, SoakError> {
    let entries = setup.recorded()?;
    let recovery = plan(setup, boot)?;
    let current = read_os_boot();
    let verdict = recovery.stamp().compare(current.as_ref())?;
    let (mut links, welcomes) = adopt(setup, PreviousCoreState::Handover);
    let mut screens = Vec::with_capacity(entries.len());
    let mut terminals = Vec::with_capacity(entries.len());
    for entry in &entries {
        let group = usize::try_from(entry.group.get()).unwrap_or(usize::MAX);
        let link = links.get_mut(group).and_then(Option::as_mut);
        let survived = survival(verdict, (link.as_deref(), held(&welcomes, entry)), entry);
        let wire = (
            TerminalId::new(entry.target.terminal_id.as_str().to_owned()),
            Incarnation::new(entry.target.terminal_incarnation.get()),
        );
        let (Survival::Survived {}, Some(link)) = (survived, link) else {
            terminals.push(TerminalRecovery::new(wire, survived, None));
            screens.push(None);
            continue;
        };
        let checkpoint = recovery.checkpoints().load(&entry.target)?;
        let from = checkpoint.as_ref().map_or(0, |saved| saved.offset().get());
        let tail = link.tail(&entry.target, from)?;
        let retained = Retained::new(tail.from_offset, tail.bytes);
        let restored = restore(
            (checkpoint.as_ref(), recovery.config),
            &retained,
            Fence::new(0, 0),
        )?;
        terminals.push(TerminalRecovery::new(
            wire,
            survived,
            Some(restored.screen.clone()),
        ));
        screens.push(Some(Screen::restored(entry.target.clone(), restored)));
    }
    if let Ok(current) = &current {
        recovery.stamp().record(current)?;
    }
    let ready = Ready {
        mode: BootMode::Handover,
        boot: Count::new(boot),
        terminals: identities(&entries, &welcomes),
        recovery: Some(Recovery::Handover { terminals }),
        interrupted: ops::interrupted(&setup.state)?,
    };
    Ok(Begun {
        links,
        screens,
        ready,
    })
}
