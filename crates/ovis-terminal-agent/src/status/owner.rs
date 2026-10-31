//! Process-token ownership: a token names exactly one terminal's current program.

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::schema::ids::ProcessToken;
use ovis_terminal_protocol::schema::scalar::Incarnation;
use sha2::{Digest as _, Sha256};

use super::StatusError;

/// The program a token was handed to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct ProcessOwner {
    /// Terminal running the program.
    pub terminal: TerminalId,
    /// Program incarnation inside that terminal.
    pub process: Incarnation,
}

impl ProcessOwner {
    /// Names one program incarnation of one terminal.
    #[must_use]
    pub const fn new(terminal: TerminalId, process: Incarnation) -> Self {
        Self { terminal, process }
    }
}

type TokenDigest = [u8; 32];

#[derive(Debug, Default)]
struct Bindings {
    owners: BTreeMap<TokenDigest, ProcessOwner>,
    current: BTreeMap<TerminalId, TokenDigest>,
}

/// Tokens handed to spawned programs; only digests are kept.
///
/// Binding a new program to a terminal retires the previous program's token, so a
/// report from a replaced incarnation is never attributed to its successor.
#[derive(Debug, Default)]
pub struct ProcessTokens {
    bindings: Mutex<Bindings>,
}

fn digest(token: &ProcessToken) -> TokenDigest {
    Sha256::digest(token.as_str().as_bytes()).into()
}

impl ProcessTokens {
    fn lock(&self) -> Result<MutexGuard<'_, Bindings>, StatusError> {
        self.bindings.lock().map_err(|_| StatusError::Poisoned)
    }

    /// Binds the token handed to `owner` at spawn, retiring the terminal's previous token.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the registry lock is poisoned.
    pub fn bind(&self, token: &ProcessToken, owner: ProcessOwner) -> Result<(), StatusError> {
        let key = digest(token);
        let mut bindings = self.lock()?;
        if let Some(previous) = bindings.current.insert(owner.terminal, key) {
            bindings.owners.remove(&previous);
        }
        bindings.owners.insert(key, owner);
        drop(bindings);
        Ok(())
    }

    /// The program `token` belongs to, if it is still current.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the registry lock is poisoned.
    pub fn owner(&self, token: &ProcessToken) -> Result<Option<ProcessOwner>, StatusError> {
        Ok(self.lock()?.owners.get(&digest(token)).copied())
    }

    /// The current program of `terminal`, if one was bound.
    ///
    /// # Errors
    ///
    /// Returns `internal` when the registry lock is poisoned.
    pub fn current(&self, terminal: TerminalId) -> Result<Option<ProcessOwner>, StatusError> {
        let bindings = self.lock()?;
        Ok(bindings
            .current
            .get(&terminal)
            .and_then(|key| bindings.owners.get(key))
            .copied())
    }

    /// The owner of `token` when it is the current program of `terminal`.
    ///
    /// # Errors
    ///
    /// Returns `not_owner` for an unknown, retired or foreign token.
    pub fn authorize(
        &self,
        token: &ProcessToken,
        terminal: TerminalId,
    ) -> Result<ProcessOwner, StatusError> {
        match self.owner(token)? {
            Some(owner) if owner.terminal == terminal => Ok(owner),
            Some(_) | None => Err(StatusError::NotOwner),
        }
    }
}
