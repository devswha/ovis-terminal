//! Machines keyed by proven identity, with names and aliases that resolve once.

use std::collections::{BTreeMap, BTreeSet};

use ovis_terminal_protocol::domain::machine_identity::MachineNames;
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::machine::MachineRecord;
use ovis_terminal_protocol::schema::scalar::Revision;

use super::{IdentityError, Name, Trusted};

/// Work bound to one machine identity; renames never change where it goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding(MachineId);

impl Binding {
    /// The bound machine.
    #[must_use]
    pub const fn machine_id(&self) -> &MachineId {
        &self.0
    }
}

/// A trusted machine together with the live link that reached it.
#[derive(Debug)]
pub struct Connected<L> {
    trusted: Trusted,
    link: L,
}

impl<L> Connected<L> {
    /// Pairs a trusted handshake with its link.
    #[must_use]
    pub const fn new(trusted: Trusted, link: L) -> Self {
        Self { trusted, link }
    }
}

/// A name held for an add in progress; return it with [`Machines::release`] or consume it
/// with [`Machines::insert`].
#[derive(Debug, PartialEq, Eq)]
pub struct NameHold(Name);

#[derive(Debug)]
struct Entry<L> {
    trusted: Trusted,
    name: Name,
    aliases: Vec<Name>,
    revision: u64,
    link: L,
}

impl<L> Entry<L> {
    fn record(&self) -> MachineRecord {
        let names = MachineNames::new(
            self.name.as_machine_name().clone(),
            self.aliases
                .iter()
                .map(|alias| alias.as_machine_name().clone())
                .collect(),
        );
        MachineRecord::connected(self.trusted.hello(), names, Revision::new(self.revision))
    }
}

/// Every added machine of one runtime and the live link `L` that reaches it.
#[derive(Debug)]
pub struct Machines<L> {
    entries: BTreeMap<MachineId, Entry<L>>,
    names: BTreeMap<Name, MachineId>,
    held: BTreeSet<Name>,
}

impl<L> Default for Machines<L> {
    fn default() -> Self {
        Self {
            entries: BTreeMap::new(),
            names: BTreeMap::new(),
            held: BTreeSet::new(),
        }
    }
}

impl<L> Machines<L> {
    /// Holds `name` for an add that has not proven its machine yet.
    ///
    /// # Errors
    ///
    /// [`IdentityError::DuplicateName`] when a machine, an alias or another add has it.
    pub fn hold(&mut self, name: Name) -> Result<NameHold, IdentityError> {
        if self.names.contains_key(&name) || !self.held.insert(name.clone()) {
            return Err(IdentityError::DuplicateName {
                name: name.as_machine_name().clone(),
            });
        }
        Ok(NameHold(name))
    }

    /// Returns a held name unused.
    pub fn release(&mut self, hold: NameHold) {
        let NameHold(name) = hold;
        self.held.remove(&name);
    }

    /// Fails if `trusted` already names a recorded machine, before any effect.
    ///
    /// # Errors
    ///
    /// [`IdentityError::MachineExists`] for a machine already added.
    pub fn ensure_new(&self, trusted: &Trusted) -> Result<(), IdentityError> {
        if self.entries.contains_key(trusted.machine_id()) {
            return Err(IdentityError::MachineExists {
                machine: trusted.machine_id().clone(),
            });
        }
        Ok(())
    }

    /// Records a connected machine under its held name.
    ///
    /// # Errors
    ///
    /// [`IdentityError::MachineExists`] when the machine was added meanwhile; the name is
    /// released either way.
    pub fn insert(
        &mut self,
        hold: NameHold,
        connected: Connected<L>,
    ) -> Result<MachineRecord, IdentityError> {
        let NameHold(name) = hold;
        let Connected { trusted, link } = connected;
        self.held.remove(&name);
        self.ensure_new(&trusted)?;
        let id = trusted.machine_id().clone();
        self.names.insert(name.clone(), id.clone());
        let entry = Entry {
            trusted,
            name,
            aliases: Vec::new(),
            revision: 1,
            link,
        };
        let record = entry.record();
        self.entries.insert(id, entry);
        Ok(record)
    }

    /// Resolves an identity, name or alias once; the binding never follows later renames.
    ///
    /// # Errors
    ///
    /// [`IdentityError::NotFound`] when nothing answers to `reference`.
    pub fn bind(&self, reference: &MachineRef) -> Result<Binding, IdentityError> {
        let id = match reference {
            MachineRef::MachineId(id) => self.entries.contains_key(id).then(|| id.clone()),
            MachineRef::Name(name) => Name::try_from(name)
                .ok()
                .and_then(|name| self.names.get(&name).cloned()),
            _ => None,
        };
        id.map(Binding).ok_or(IdentityError::NotFound)
    }

    /// The record of a bound machine.
    ///
    /// # Errors
    ///
    /// [`IdentityError::NotFound`] when the machine is no longer recorded.
    pub fn get(&self, binding: &Binding) -> Result<MachineRecord, IdentityError> {
        self.entry(binding).map(Entry::record)
    }

    /// The live link of a bound machine.
    ///
    /// # Errors
    ///
    /// [`IdentityError::NotFound`] when the machine is no longer recorded.
    pub fn link(&self, binding: &Binding) -> Result<&L, IdentityError> {
        self.entry(binding).map(|entry| &entry.link)
    }

    /// Fails unless `name` is free for the bound machine: unused, or already its own name
    /// or alias.
    ///
    /// # Errors
    ///
    /// [`IdentityError::NotFound`] and [`IdentityError::DuplicateName`].
    pub fn ensure_renamable(&self, binding: &Binding, name: &Name) -> Result<(), IdentityError> {
        self.entry(binding)?;
        let owner = self.names.get(name);
        if self.held.contains(name) || owner.is_some_and(|owner| owner != binding.machine_id()) {
            return Err(IdentityError::DuplicateName {
                name: name.as_machine_name().clone(),
            });
        }
        Ok(())
    }

    /// Renames a bound machine; the previous name stays an alias of the same machine.
    ///
    /// # Errors
    ///
    /// The errors of [`Machines::ensure_renamable`].
    pub fn rename(
        &mut self,
        binding: &Binding,
        name: Name,
    ) -> Result<MachineRecord, IdentityError> {
        self.ensure_renamable(binding, &name)?;
        let entry = self
            .entries
            .get_mut(binding.machine_id())
            .ok_or(IdentityError::NotFound)?;
        if entry.name != name {
            entry.aliases.retain(|alias| *alias != name);
            let previous = std::mem::replace(&mut entry.name, name.clone());
            entry.aliases.push(previous);
            entry.revision = entry.revision.saturating_add(1);
            self.names.insert(name, binding.machine_id().clone());
        }
        Ok(entry.record())
    }

    fn entry(&self, binding: &Binding) -> Result<&Entry<L>, IdentityError> {
        self.entries
            .get(binding.machine_id())
            .ok_or(IdentityError::NotFound)
    }
}

#[cfg(test)]
#[path = "registry/tests.rs"]
mod tests;
