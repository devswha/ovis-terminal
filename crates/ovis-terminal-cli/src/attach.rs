//! `ovis-terminal terminal attach`: a plain attach client, not multiplexer chrome.
//!
//! Interact mode needs a TTY on stdin and stdout, holds the interact lease, puts the outer
//! terminal in raw mode for exactly as long as it is attached, forwards typed bytes only
//! while the stream is input-ready, follows window resizes under its geometry lease, and
//! detaches on Ctrl-\. There is no prefix key and no client mode: every other byte is
//! forwarded unchanged. Observe mode needs no TTY, never reads stdin and never resizes,
//! focuses or takes a lease that controls the terminal. A held interact lease is reported
//! as `lease_conflict` with its owner and the lease change to retry after; only an
//! explicit `--takeover <epoch>` displaces it, and a stale epoch is `lease_epoch_mismatch`
//! naming the current lease. Every outcome, including a refusal, connection loss and
//! repeated interruption at any stage, leaves the outer terminal's settings and modes as
//! it found them.

mod client;
pub mod error;
mod input;
mod modes;
mod open;
mod screen;
mod session;
mod signals;
mod tty;

pub(crate) use error::AttachError;
use ovis_terminal_protocol::schema::common::Target;
use ovis_terminal_protocol::schema::ids::ClientId;
use ovis_terminal_protocol::schema::scalar::LeaseEpoch;
use ovis_terminal_server::listen::EndpointAddress;
use serde_json::Value;

/// One planned attach: everything was validated before a connection is opened.
#[derive(Debug, Clone)]
pub(crate) struct AttachPlan {
    pub(crate) target: Target,
    pub(crate) observe: bool,
    pub(crate) client: ClientId,
    /// The interact lease epoch an explicit takeover expects to displace.
    pub(crate) takeover: Option<LeaseEpoch>,
}

/// Connects, attaches and runs until detach, close or failure; the outer terminal is
/// restored before this returns. Returns the detach record.
pub(crate) async fn attach(
    plan: &AttachPlan,
    (address, bearer): (&EndpointAddress, Option<&str>),
) -> Result<Value, AttachError> {
    let EndpointAddress::Unix(path) = address else {
        return Err(AttachError::UnsupportedTransport);
    };
    let mut signals = signals::Signals::listen()?;
    let connecting = client::Client::connect(path, bearer);
    let connected = signals.during("connecting", connecting).await?;
    session::run(plan, connected, &mut signals).await
}
