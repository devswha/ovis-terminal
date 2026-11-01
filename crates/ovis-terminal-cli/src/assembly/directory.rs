//! Explicit target resolution over the terminals this server spawned; there is no focus
//! and no fallback, and an incarnation that moved resolves to the current one only.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use ovis_terminal_core::registry::{
    MachineId, TargetResolutionError, TerminalGeneration, TerminalId, TerminalState,
};
use ovis_terminal_core::terminals::Record;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_server::dispatch::lanes::agent_status::{ResolvedTerminal, TerminalDirectory};

#[derive(Debug, Clone)]
struct Entry {
    generation: TerminalGeneration,
    name: Option<String>,
    live: bool,
}

/// Spawned terminals by identity, with their current incarnation and live name.
#[derive(Debug)]
pub(super) struct Directory {
    machine: MachineId,
    entries: Mutex<BTreeMap<TerminalId, Entry>>,
}

impl Directory {
    pub(super) const fn new(machine: MachineId) -> Self {
        Self {
            machine,
            entries: Mutex::new(BTreeMap::new()),
        }
    }

    fn entries(&self) -> MutexGuard<'_, BTreeMap<TerminalId, Entry>> {
        self.entries.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records `terminal` live at `generation` under `name`.
    pub(super) fn spawned(
        &self,
        (terminal, generation): (TerminalId, TerminalGeneration),
        name: Option<String>,
    ) {
        let entry = Entry {
            generation,
            name,
            live: true,
        };
        self.entries().insert(terminal, entry);
    }

    /// Registers the terminals earlier boots recorded, so their prompts, status and
    /// history stay addressable; only a recorded running program keeps its name taken.
    pub(super) fn recorded(&self, records: &[Record]) {
        let mut entries = self.entries();
        for record in records {
            entries.entry(record.id()).or_insert_with(|| Entry {
                generation: record.lifecycle.generation(),
                name: record
                    .launch
                    .name
                    .as_ref()
                    .map(|name| name.as_str().to_owned()),
                live: record.lifecycle.state() == TerminalState::Running,
            });
        }
    }

    /// The program of `terminal` ended; its name is free again.
    pub(super) fn exited(&self, terminal: TerminalId) {
        if let Some(entry) = self.entries().get_mut(&terminal) {
            entry.live = false;
        }
    }

    fn by_name(&self, name: &str) -> Result<TerminalId, TargetResolutionError> {
        let entries = self.entries();
        let mut named = entries
            .iter()
            .filter(|(_, entry)| entry.live && entry.name.as_deref() == Some(name))
            .map(|(id, _)| *id);
        let first = named.next().ok_or(TargetResolutionError::NotFound)?;
        drop(named);
        drop(entries);
        Ok(first)
    }

    /// `t7`, `name`, `machine/t7` or `machine/name`; another machine is not found here.
    fn by_address(&self, address: &str) -> Result<TerminalId, TargetResolutionError> {
        let local = match address.split_once('/') {
            Some((machine, local)) if machine == self.machine.to_string() => local,
            Some(_) => return Err(TargetResolutionError::NotFound),
            None => address,
        };
        let numbered = local
            .strip_prefix('t')
            .and_then(|digits| digits.parse::<u64>().ok());
        numbered.map_or_else(
            || self.by_name(local),
            |number| TerminalId::try_from(number).map_err(|_| TargetResolutionError::NotFound),
        )
    }
}

impl TerminalDirectory for Directory {
    fn resolve(&self, target: &Target) -> Result<ResolvedTerminal, TargetResolutionError> {
        let terminal = match target {
            Target::TerminalId(id) => id.as_str().parse()?,
            Target::Address(address) => self.by_address(address.as_str())?,
            Target::Name(name) => self.by_name(name.as_str())?,
            _ => return Err(TargetResolutionError::NeedsTarget),
        };
        let generation = self
            .entries()
            .get(&terminal)
            .map(|entry| entry.generation)
            .ok_or(TargetResolutionError::NotFound)?;
        Ok(ResolvedTerminal::new(self.machine, terminal, generation))
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_core::registry::{MachineId, TerminalGeneration, TerminalId};
    use ovis_terminal_protocol::schema::common::Target;
    use ovis_terminal_server::dispatch::lanes::agent_status::TerminalDirectory as _;
    use proptest::prelude::*;

    use super::Directory;

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-000000000031";

    fn target(json: &serde_json::Value) -> Result<Target, serde_json::Error> {
        serde_json::from_value(json.clone())
    }

    fn directory(
        id: u64,
        name: &str,
    ) -> Result<(Directory, TerminalId), Box<dyn std::error::Error>> {
        let directory = Directory::new(MACHINE.parse()?);
        let terminal = TerminalId::try_from(id)?;
        directory.spawned(
            (terminal, TerminalGeneration::new(1)?),
            Some(name.to_owned()),
        );
        Ok((directory, terminal))
    }

    fn resolved(directory: &Directory, json: &serde_json::Value) -> Option<TerminalId> {
        let target = target(json).ok()?;
        directory
            .resolve(&target)
            .ok()
            .map(|resolved| resolved.terminal)
    }

    #[test]
    fn every_explicit_form_names_the_terminal_and_nothing_else_resolves() -> TestResult {
        let (directory, terminal) = directory(7, "build")?;
        let machine: MachineId = MACHINE.parse()?;
        let forms = [
            serde_json::json!({"address": "t7"}),
            serde_json::json!({"address": format!("{machine}/t7")}),
            serde_json::json!({"address": format!("{machine}/build")}),
            serde_json::json!({"terminal_id": "terminal:7"}),
            serde_json::json!({"name": "build"}),
        ];
        for form in &forms {
            assert_eq!(resolved(&directory, form), Some(terminal), "{form}");
        }
        for stale in [
            serde_json::json!({"address": "t8"}),
            serde_json::json!({"address": "machine:0191f1a2-0000-7000-8000-000000000032/t7"}),
            serde_json::json!({"name": "other"}),
        ] {
            assert_eq!(resolved(&directory, &stale), None, "{stale}");
        }
        directory.exited(terminal);
        assert_eq!(
            resolved(&directory, &serde_json::json!({"name": "build"})),
            None
        );
        assert_eq!(
            resolved(&directory, &serde_json::json!({"address": "t7"})),
            Some(terminal)
        );
        Ok(())
    }

    #[test]
    fn the_largest_identity_round_trips_through_its_address() -> TestResult {
        let (directory, terminal) = directory(u64::MAX, "max")?;
        let address = serde_json::json!({"address": format!("t{}", u64::MAX)});
        assert_eq!(resolved(&directory, &address), Some(terminal));
        assert_eq!(terminal.to_string(), "terminal:18446744073709551615");
        Ok(())
    }

    proptest! {
        #[test]
        fn an_address_resolves_to_exactly_its_own_identity(id in 1_u64.., other in 1_u64..) {
            let (directory, terminal) = directory(id, "named").map_err(|error| TestCaseError::fail(error.to_string()))?;
            prop_assert_eq!(resolved(&directory, &serde_json::json!({"address": format!("t{id}")})), Some(terminal));
            let expected = (other == id).then_some(terminal);
            prop_assert_eq!(resolved(&directory, &serde_json::json!({"address": format!("t{other}")})), expected);
        }
    }
}
