//! Stopping a running holder through its own protocol and enumerating what ended.

use std::path::Path;
use std::time::{Duration, Instant};

use ovis_terminal_protocol::domain::unix_service::{
    BootProof, DiscoveryRecord, EndedTerminal, Observed, StopReceipt,
};
use ovis_terminal_protocol::schema::ids::{ProcessPid, StartToken, TerminalId};
use ovis_terminal_protocol::schema::scalar::Incarnation;

use super::run::instance;
use super::{ServiceError, ServicePaths, observe, record};
use crate::process::{ProcessId, read_start_token};
use crate::protocol::{
    CoreBuildId, Goodbye, GoodbyeReason, HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello, Message,
    PreviousCoreState,
};
use crate::unix::{CoreConnection, CoreConnectionError};

/// Bound on each holder exchange and on the holder withdrawing after `Goodbye`.
const EXCHANGE_BOUND: Duration = Duration::from_secs(5);
const SETTLE_POLL: Duration = Duration::from_millis(10);

fn ended(held: &HeldTerminal) -> EndedTerminal {
    let terminal = EndedTerminal::new(
        TerminalId::new(held.terminal_id.as_str().to_owned()),
        Incarnation::new(held.terminal_incarnation.get()),
    );
    match (held.process_incarnation, held.pid, &held.start_token) {
        (Some(incarnation), Some(pid), Some(token)) => terminal.with_process(
            Incarnation::new(incarnation.get()),
            BootProof::new(
                ProcessPid::new(pid),
                StartToken::new(token.as_str().to_owned()),
            ),
        ),
        _ => terminal,
    }
}

fn running(paths: &ServicePaths) -> Result<DiscoveryRecord, ServiceError> {
    match observe(paths)? {
        Observed::Running { record } => Ok(record),
        Observed::Stopped => Err(ServiceError::NotRunning),
        Observed::StaleEndpoint { reason, .. } => Err(ServiceError::Stale { reason }),
        _ => Err(ServiceError::Unreachable),
    }
}

const fn connection(error: CoreConnectionError) -> ServiceError {
    ServiceError::Connection(error)
}

/// Whether the holder named by `proof` has withdrawn its record or exited.
fn settled(paths: &ServicePaths, proof: &BootProof) -> Result<bool, ServiceError> {
    let named = record::read(&paths.record())?.is_some_and(|found| found.boot_proof == *proof);
    let alive = read_start_token(ProcessId::new(proof.pid.get()))
        .is_ok_and(|token| token.as_str() == proof.start_token.as_str());
    Ok(!(named && alive))
}

/// Stops the running holder of `paths`: adopt, enumerate, `Goodbye`, await withdrawal.
///
/// # Errors
///
/// [`ServiceError::NotRunning`], [`ServiceError::Stale`] or [`ServiceError::Unreachable`]
/// when no live holder answers; [`ServiceError::IdentityMismatch`] when the answering
/// holder is not the recorded one; [`ServiceError::NotReady`] when it never withdraws.
pub fn stop(paths: &ServicePaths) -> Result<StopReceipt, ServiceError> {
    let found = running(paths)?;
    let mut link = CoreConnection::connect(Path::new(&found.address)).map_err(connection)?;
    link.send(&Message::Hello(Hello {
        generation: HOLDER_PROTOCOL_GENERATION,
        core_build: CoreBuildId::new(format!("ovis-terminal/{}", env!("CARGO_PKG_VERSION"))),
        previous: PreviousCoreState::Handover,
    }))
    .map_err(connection)?;
    let Message::Welcome(welcome) = link.recv(EXCHANGE_BOUND).map_err(connection)? else {
        return Err(ServiceError::IdentityMismatch);
    };
    if welcome.instance.as_str() != instance(&found.boot_proof) {
        return Err(ServiceError::IdentityMismatch);
    }
    let terminals = welcome.terminals.iter().map(ended).collect();
    link.send(&Message::Goodbye(Goodbye {
        reason: GoodbyeReason::Shutdown,
    }))
    .map_err(connection)?;
    loop {
        match link.recv(EXCHANGE_BOUND) {
            Ok(_) => {}
            Err(CoreConnectionError::Closed) => break,
            Err(other) => return Err(connection(other)),
        }
    }
    let deadline = Instant::now().checked_add(EXCHANGE_BOUND);
    while !settled(paths, &found.boot_proof)? {
        if deadline.is_none_or(|deadline| Instant::now() >= deadline) {
            return Err(ServiceError::NotReady {
                bound_ms: u64::try_from(EXCHANGE_BOUND.as_millis()).unwrap_or(u64::MAX),
            });
        }
        std::thread::sleep(SETTLE_POLL);
    }
    Ok(StopReceipt::new(found.label, found.boot_proof, terminals))
}
