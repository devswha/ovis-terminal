//! Adopting one holder group after a crash, over its own socket.

use std::collections::VecDeque;
use std::time::Duration;

use ovis_terminal_holder::protocol::{
    CoreBuildId, HOLDER_PROTOCOL_GENERATION, Hello, Message, PreviousCoreState, Tail, TailRequest,
    TerminalTarget, Welcome,
};
use ovis_terminal_holder::unix::CoreConnection;

use super::RecoveryError;
use super::groups::HolderGroup;

fn link(
    stage: &'static str,
) -> impl FnOnce(ovis_terminal_holder::unix::CoreConnectionError) -> RecoveryError {
    move |source| RecoveryError::HolderLink {
        stage,
        source: Box::new(source),
    }
}

/// A holder group adopted by the recovering core, with every message it sent that the
/// recovery step itself did not consume, kept in order.
#[derive(Debug)]
pub struct Adopted {
    connection: CoreConnection,
    welcome: Welcome,
    pending: VecDeque<Message>,
    bound: Duration,
}

impl Adopted {
    /// Connects to `group` and adopts it as the successor of a crashed core; each
    /// receive waits at most `bound`.
    ///
    /// # Errors
    ///
    /// [`RecoveryError::HolderLink`] when the socket does not answer, and
    /// [`RecoveryError::HolderProtocol`] when the first reply is not a welcome.
    pub fn adopt(
        group: &HolderGroup,
        core_build: CoreBuildId,
        bound: Duration,
    ) -> Result<Self, RecoveryError> {
        let mut connection = CoreConnection::connect(&group.socket).map_err(link("connect"))?;
        connection
            .send(&Message::Hello(Hello {
                generation: HOLDER_PROTOCOL_GENERATION,
                core_build,
                previous: PreviousCoreState::Crash,
            }))
            .map_err(link("send the hello"))?;
        let Message::Welcome(welcome) =
            connection.recv(bound).map_err(link("await the welcome"))?
        else {
            return Err(RecoveryError::HolderProtocol {
                expected: "a welcome",
            });
        };
        Ok(Self {
            connection,
            welcome,
            pending: VecDeque::new(),
            bound,
        })
    }

    /// The adoption snapshot.
    #[must_use]
    pub const fn welcome(&self) -> &Welcome {
        &self.welcome
    }

    /// Asks for the retained output of `target` from `from`; messages that arrive before
    /// the answer are kept for [`Adopted::receive`].
    ///
    /// # Errors
    ///
    /// A link failure, or [`RecoveryError::HolderProtocol`] for a tail of another target.
    pub fn tail(&mut self, target: &TerminalTarget, from: u64) -> Result<Tail, RecoveryError> {
        self.connection
            .send(&Message::TailRequest(TailRequest {
                target: target.clone(),
                from_offset: from,
            }))
            .map_err(link("request the retained tail"))?;
        loop {
            match self
                .connection
                .recv(self.bound)
                .map_err(link("await the retained tail"))?
            {
                Message::Tail(tail) if tail.target == *target => return Ok(tail),
                Message::Tail(_) => {
                    return Err(RecoveryError::HolderProtocol {
                        expected: "the tail of the requested terminal",
                    });
                }
                other => self.pending.push_back(other),
            }
        }
    }

    /// The next holder message: kept ones first, then live ones.
    ///
    /// # Errors
    ///
    /// A link failure or a receive that outlasted the bound.
    pub fn receive(&mut self) -> Result<Message, RecoveryError> {
        match self.pending.pop_front() {
            Some(message) => Ok(message),
            None => self
                .connection
                .recv(self.bound)
                .map_err(link("receive holder output")),
        }
    }
}
