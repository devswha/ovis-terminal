//! Each machine's mirror cursor and the last-known state of its terminals.

use std::collections::BTreeMap;

use ovis_terminal_core::registry::RuntimeId;
use ovis_terminal_protocol::domain::machine_actor::{
    BoundaryKind, MirrorBoundary, MirrorSource, MirroredEvent, RemoteEvent, RemoteScan,
};
use ovis_terminal_protocol::schema::ids::{MachineId, RuntimeLifetimeId, TerminalId};
use ovis_terminal_protocol::schema::scalar::{EventSeq, Revision, TimestampMs};

use super::{MirrorError, Mirrored};

/// A remote terminal as last mirrored.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RemoteTerminalState {
    /// The terminal.
    pub terminal_id: TerminalId,
    /// Revision of the last event about it.
    pub revision: Revision,
    /// False while its machine's line is lost or its lifetime changed.
    pub available: bool,
}

#[derive(Debug, Default)]
struct Cursor {
    lifetime: Option<RuntimeLifetimeId>,
    last: Option<EventSeq>,
    terminals: BTreeMap<TerminalId, (Revision, bool)>,
}

impl Cursor {
    fn last_seen(&self, machine: &MachineId) -> Option<MirrorSource> {
        let (lifetime, seq) = (self.lifetime.clone()?, self.last?);
        Some(MirrorSource::new(machine.clone(), lifetime, seq))
    }

    fn unavailable(&mut self) {
        for (_, available) in self.terminals.values_mut() {
            *available = false;
        }
    }

    /// Starts `lifetime`, with a boundary when it replaces another one.
    fn enter(&mut self, (machine, lifetime): (&MachineId, &RuntimeLifetimeId)) -> Option<Mirrored> {
        if self.lifetime.as_ref() == Some(lifetime) {
            return None;
        }
        let previous = self.last_seen(machine);
        let had_lifetime = self.lifetime.is_some();
        self.lifetime = Some(lifetime.clone());
        self.last = None;
        if !had_lifetime {
            return None;
        }
        self.unavailable();
        let kind = (machine.clone(), BoundaryKind::LifetimeChanged);
        Some(Mirrored::boundary(MirrorBoundary::new(
            kind, previous, None,
        )))
    }
}

/// Mirror cursors of every machine.
#[derive(Debug, Default)]
pub struct Mirror {
    machines: BTreeMap<MachineId, Cursor>,
}

fn checked(lifetime: &RuntimeLifetimeId) -> Result<(), MirrorError> {
    lifetime
        .as_str()
        .parse::<RuntimeId>()
        .map(drop)
        .map_err(MirrorError::Identity)
}

impl Mirror {
    /// Records to commit, in order, for `event` received at `received` from `machine`.
    /// A remote sequence already mirrored yields nothing.
    ///
    /// # Errors
    ///
    /// [`MirrorError::Identity`] for a lifetime that is no runtime identity; nothing changes.
    pub fn accept(
        &mut self,
        (machine, received): (&MachineId, TimestampMs),
        event: RemoteEvent,
    ) -> Result<Vec<Mirrored>, MirrorError> {
        checked(&event.runtime_lifetime)?;
        let cursor = self.machines.entry(machine.clone()).or_default();
        let mut records: Vec<Mirrored> = cursor
            .enter((machine, &event.runtime_lifetime))
            .into_iter()
            .collect();
        if cursor.last.is_some_and(|last| event.seq <= last) {
            return Ok(records);
        }
        let expected = cursor.last.map(|last| last.get().saturating_add(1));
        if expected.is_some_and(|expected| event.seq.get() > expected) {
            let through = Some(EventSeq::new(event.seq.get().saturating_sub(1)));
            let kind = (machine.clone(), BoundaryKind::Gap);
            records.push(Mirrored::boundary(MirrorBoundary::new(
                kind,
                cursor.last_seen(machine),
                through,
            )));
        }
        if let Some(terminal) = &event.terminal {
            if terminal.removed {
                cursor.terminals.remove(&terminal.terminal_id);
            } else {
                cursor
                    .terminals
                    .insert(terminal.terminal_id.clone(), (terminal.revision, true));
            }
        }
        cursor.last = Some(event.seq);
        records.push(Mirrored::event(MirroredEvent::received(
            machine.clone(),
            event,
            received,
        )));
        Ok(records)
    }

    /// Records for a remote scan: a boundary when it examined sequences not yet mirrored.
    ///
    /// # Errors
    ///
    /// [`MirrorError::Identity`] for a lifetime that is no runtime identity; nothing changes.
    pub fn scan(
        &mut self,
        machine: &MachineId,
        scan: &RemoteScan,
    ) -> Result<Vec<Mirrored>, MirrorError> {
        checked(&scan.runtime_lifetime)?;
        let cursor = self.machines.entry(machine.clone()).or_default();
        let mut records: Vec<Mirrored> = cursor
            .enter((machine, &scan.runtime_lifetime))
            .into_iter()
            .collect();
        if cursor.last.is_none_or(|last| scan.scan_seq > last) {
            let kind = (machine.clone(), BoundaryKind::Scan);
            let boundary =
                MirrorBoundary::new(kind, cursor.last_seen(machine), Some(scan.scan_seq));
            records.push(Mirrored::boundary(boundary));
            cursor.last = Some(scan.scan_seq);
        }
        Ok(records)
    }

    /// The boundary for `machine`'s lost line; its terminals stay, unavailable.
    pub fn lost(&mut self, machine: &MachineId) -> Mirrored {
        let cursor = self.machines.entry(machine.clone()).or_default();
        cursor.unavailable();
        let kind = (machine.clone(), BoundaryKind::LineLost);
        Mirrored::boundary(MirrorBoundary::new(kind, cursor.last_seen(machine), None))
    }

    /// The boundary for a new line to `machine`; mirroring continues after it.
    pub fn resumed(&mut self, machine: &MachineId) -> Mirrored {
        let cursor = self.machines.entry(machine.clone()).or_default();
        let kind = (machine.clone(), BoundaryKind::Resumed);
        Mirrored::boundary(MirrorBoundary::new(kind, cursor.last_seen(machine), None))
    }

    /// Every terminal of `machine` mirrored and not removed, by identity.
    #[must_use]
    pub fn terminals(&self, machine: &MachineId) -> Vec<RemoteTerminalState> {
        self.machines.get(machine).map_or_else(Vec::new, |cursor| {
            cursor
                .terminals
                .iter()
                .map(|(terminal_id, (revision, available))| RemoteTerminalState {
                    terminal_id: terminal_id.clone(),
                    revision: *revision,
                    available: *available,
                })
                .collect()
        })
    }
}
