//! Bridge sessions: at most one per program incarnation, opened with its process token.
//!
//! The registry is consulted and changed under one lock acquisition per request, so a
//! session is never resolved, released and then used.

use std::collections::{BTreeMap, VecDeque};
use std::fmt;
use std::num::NonZeroU8;
use std::sync::{Arc, Mutex, MutexGuard};

use ovis_terminal_core::registry::TerminalId;
use ovis_terminal_protocol::domain::prompt_bridge::{
    BridgeAttachment, DeliveryState, ExtensionEvent,
};
use ovis_terminal_protocol::schema::ids::{ProcessToken, PromptId, TerminalId as WireTerminal};
use ovis_terminal_protocol::schema::scalar::{Count, EventSeq};
use sha2::{Digest as _, Sha256};

use super::BridgeError;
use crate::prompts::PromptService;
use crate::status::{ProcessOwner, StatusService};

/// Most settled deliveries one program keeps readable; older ones are forgotten.
const SETTLED_LIMIT: usize = 64;

type TokenDigest = [u8; 32];

/// Capability of one bridge session; only its digest is kept by the bridge.
#[derive(Clone, PartialEq, Eq)]
pub struct BridgeToken(String);

impl BridgeToken {
    fn mint() -> Self {
        let (high, low) = (uuid::Uuid::now_v7(), uuid::Uuid::now_v7());
        Self(format!("bridge:{}{}", high.simple(), low.simple()))
    }

    fn digest(&self) -> TokenDigest {
        Sha256::digest(self.0.as_bytes()).into()
    }
}

impl fmt::Debug for BridgeToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BridgeToken(<redacted>)")
    }
}

/// A session opened for a program: its capability and what it may hold.
#[derive(Debug)]
#[non_exhaustive]
pub struct Attached {
    /// The session's capability.
    pub token: BridgeToken,
    /// The program and queue bound of the session.
    pub attachment: BridgeAttachment,
}

/// Delivery facts of one program incarnation.
#[derive(Debug)]
pub(super) struct Program {
    pub owner: ProcessOwner,
    pub session: Option<TokenDigest>,
    /// Answers up to this closing sequence were handed out and are never handed out again.
    pub watermark: Option<EventSeq>,
    pub in_flight: Vec<ExtensionEvent>,
    pub settled: VecDeque<(PromptId, DeliveryState)>,
}

impl Program {
    const fn new(owner: ProcessOwner) -> Self {
        Self {
            owner,
            session: None,
            watermark: None,
            in_flight: Vec::new(),
            settled: VecDeque::new(),
        }
    }

    pub(super) fn settle(&mut self, prompt: PromptId, state: DeliveryState) {
        self.settled.push_back((prompt, state));
        while self.settled.len() > SETTLED_LIMIT {
            self.settled.pop_front();
        }
    }

    /// Ends the session; what it held unacknowledged has an unknown outcome.
    fn end_session(&mut self) {
        self.session = None;
        for event in std::mem::take(&mut self.in_flight) {
            self.settle(event.prompt_id, DeliveryState::OutcomeUnknown);
        }
    }
}

pub(super) type Programs = BTreeMap<TerminalId, Program>;

/// Delivers extension answers of every program of one runtime to their bridge sessions.
#[derive(Debug)]
pub struct PromptBridge {
    pub(super) status: Arc<StatusService>,
    pub(super) prompts: Arc<PromptService>,
    pub(super) limit: NonZeroU8,
    programs: Mutex<Programs>,
}

impl PromptBridge {
    /// Sessions authenticated by `status`'s process tokens, reading `prompts`, each
    /// holding at most `limit` delivered, unacknowledged events.
    #[must_use]
    pub const fn new(
        status: Arc<StatusService>,
        prompts: Arc<PromptService>,
        limit: NonZeroU8,
    ) -> Self {
        Self {
            status,
            prompts,
            limit,
            programs: Mutex::new(BTreeMap::new()),
        }
    }

    pub(super) fn lock(&self) -> Result<MutexGuard<'_, Programs>, BridgeError> {
        self.programs.lock().map_err(|_| BridgeError::Poisoned)
    }

    pub(super) fn queue_limit(&self) -> Count {
        Count::new(u64::from(self.limit.get()))
    }

    /// Opens the session of the program holding `token`, ending its previous one.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` for a token that names no current program.
    pub fn attach(&self, token: &ProcessToken) -> Result<Attached, BridgeError> {
        let owner = self
            .status
            .tokens()
            .owner(token)
            .map_err(BridgeError::Owner)?
            .ok_or(BridgeError::Forbidden)?;
        let minted = BridgeToken::mint();
        let mut programs = self.lock()?;
        let program = programs
            .entry(owner.terminal)
            .or_insert_with(|| Program::new(owner));
        if program.owner != owner {
            *program = Program::new(owner);
        }
        program.end_session();
        program.session = Some(minted.digest());
        drop(programs);
        let wire = WireTerminal::new(owner.terminal.to_string());
        Ok(Attached {
            token: minted,
            attachment: BridgeAttachment::new((wire, owner.process), self.queue_limit()),
        })
    }

    /// Ends the session of `token`; its unacknowledged events are never handed out again.
    ///
    /// # Errors
    ///
    /// Returns `forbidden` for a token that names no current session.
    pub fn detach(&self, token: &BridgeToken) -> Result<(), BridgeError> {
        let mut programs = self.lock()?;
        self.session(&mut programs, token)?.end_session();
        drop(programs);
        Ok(())
    }

    /// The program whose current session `token` is, while that program still runs.
    pub(super) fn session<'p>(
        &self,
        programs: &'p mut Programs,
        token: &BridgeToken,
    ) -> Result<&'p mut Program, BridgeError> {
        let key = token.digest();
        let program = programs
            .values_mut()
            .find(|program| program.session == Some(key))
            .ok_or(BridgeError::Forbidden)?;
        let current = self
            .status
            .tokens()
            .current(program.owner.terminal)
            .map_err(BridgeError::Owner)?;
        if current != Some(program.owner) {
            program.end_session();
            return Err(BridgeError::Forbidden);
        }
        Ok(program)
    }
}
