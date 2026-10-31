//! The rechecks a first message passes at the moment it is written.

use ovis_terminal_authz::{AccessRequest, Action, Authority, Decision};
use ovis_terminal_protocol::domain::template_launch::FirstMessageReason;
use ovis_terminal_protocol::schema::methods::input::Submit;
use ovis_terminal_protocol::schema::scalar::TimestampMs;

use crate::input::encoding::{Encoded, InputRequest, ModeSnapshot, encode};
use crate::leases::LeaseView;
use crate::registry::{ProcessGeneration, RegistryIdentities};

type Registry = RegistryIdentities;

/// The terminal and authority state a first message is checked against when written.
///
/// `decision` is the requesting principal's input decision taken when the launch was
/// admitted; `process` pairs the incarnation the message was queued for with the one
/// running at the write.
#[derive(Debug, Clone, Copy)]
#[non_exhaustive]
pub struct WriteBoundary<'a> {
    authority: &'a Authority<Registry>,
    decision: &'a Decision<Registry>,
    at: TimestampMs,
    leases: &'a LeaseView,
    process: (ProcessGeneration, ProcessGeneration),
    modes: &'a ModeSnapshot,
}

/// A terminal's leases, the incarnation pair (queued for, running now) and its modes.
pub type TerminalState<'a> = (
    &'a LeaseView,
    (ProcessGeneration, ProcessGeneration),
    &'a ModeSnapshot,
);

impl<'a> WriteBoundary<'a> {
    /// The boundary of a write at `at` by the holder of `decision` under `authority`.
    #[must_use]
    pub const fn new(
        (authority, decision): (&'a Authority<Registry>, &'a Decision<Registry>),
        at: TimestampMs,
        (leases, process, modes): TerminalState<'a>,
    ) -> Self {
        Self {
            authority,
            decision,
            at,
            leases,
            process,
            modes,
        }
    }

    /// Rechecks authority, lease and incarnation, then encodes `text` and its Enter as
    /// one submission for the current modes.
    ///
    /// # Errors
    ///
    /// `authority_lapsed` when the credential was revoked or expired, `lease_held` while
    /// an interact lease exists, `process_replaced` when another incarnation runs, and
    /// `encoding_refused` when the modes cannot carry the text literally.
    pub fn admit(&self, text: &str) -> Result<Encoded, FirstMessageReason> {
        let commit = AccessRequest::new(Action::Input, self.decision.target().clone(), self.at);
        self.authority
            .recheck(self.decision, &commit)
            .map_err(|_| FirstMessageReason::AuthorityLapsed)?;
        if self.leases.interact.is_some() {
            return Err(FirstMessageReason::LeaseHeld);
        }
        let (bound, current) = self.process;
        if bound != current {
            return Err(FirstMessageReason::ProcessReplaced);
        }
        encode(
            self.modes,
            InputRequest::Text {
                text,
                submit: &Submit::Enter,
            },
        )
        .map_err(|_| FirstMessageReason::EncodingRefused)
    }
}
