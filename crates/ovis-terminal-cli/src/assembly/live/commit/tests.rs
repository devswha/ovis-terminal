//! A holder chunk reaches the output waits only after the attach hub committed it.

use std::sync::Arc;
use std::time::{Duration, Instant};

use ovis_terminal_authz::{
    Authority, AuthorityConfig, OwnerSpec, PrincipalId, ResolvedTarget, RuntimeBinding,
    TargetResource,
};
use ovis_terminal_core::leases::{TerminalLeases, Transition};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, MachineId, RuntimeId, RuntimeIdentity, ServerBootId,
    TerminalGeneration, TerminalId,
};
use ovis_terminal_core::screen::{Screen, ScreenBounds, ScreenConfig};
use ovis_terminal_core::waits::{CancelScope, OutputOutcome, OutputWait, Waits};
use ovis_terminal_holder::protocol::{
    Exit, Output, ProcessIncarnation, TerminalId as HolderTerminalId, TerminalIncarnation,
    TerminalTarget,
};
use ovis_terminal_protocol::schema::limits::Limits;
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};
use ovis_terminal_server::streams::{
    LeaseJournal, SessionBacklog, StreamError, StreamLimits, StreamSetup, TerminalStreams,
};
use ovis_terminal_store::OpId;
use ovis_terminal_vt::{Geometry, HistoryLimit, OutputOffset, VtConfig};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error + Send + Sync>>;

/// Lease transitions are dropped; no stream is opened here.
#[derive(Debug)]
struct Discard;

impl LeaseJournal for Discard {
    fn record(&self, _op: OpId, _transition: Transition) -> Result<Option<EventSeq>, StreamError> {
        Ok(None)
    }
}

fn target() -> TerminalTarget {
    TerminalTarget {
        terminal_id: HolderTerminalId::new("terminal:7".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        process_incarnation: Some(ProcessIncarnation::new(1)),
    }
}

fn screen() -> TestResult<Screen> {
    let vt = VtConfig::new(Geometry::new(40, 8)?, HistoryLimit::new(200));
    Ok(Screen::new(
        target(),
        ScreenConfig::new(vt, ScreenBounds::DEFAULT),
    )?)
}

/// The hub and the waits of one terminal, each with its own screen as the runtime builds them.
fn readers() -> TestResult<(Arc<TerminalStreams>, Waits)> {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000064".parse()?;
    let runtime = RuntimeIdentity::new(
        HostIdentity::new(machine, HostBootId::try_from(1)?),
        RuntimeId::try_from(7)?,
        ServerBootId::try_from(1)?,
    );
    let binding = RuntimeBinding::new(runtime.machine_id(), runtime.runtime_id());
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, _issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let resolved = ResolvedTarget::new(
        runtime.machine_id(),
        TargetResource::Terminal {
            placement: None,
            terminal: TerminalId::try_from(7)?,
            incarnation: TerminalGeneration::new(1)?,
        },
    );
    let leases = TerminalLeases::new(Arc::new(authority), resolved)?;
    let setup = StreamSetup::new(
        StreamLimits::advertised(&Limits::v1()),
        Arc::new(SessionBacklog::new(64 * 1024 * 1024)),
        Arc::new(Discard),
    );
    let hub = TerminalStreams::new(screen()?, leases, setup);
    Ok((hub, Waits::new(screen()?)))
}

fn chunk(offset: u64, bytes: &[u8]) -> Output {
    Output {
        target: target(),
        offset,
        bytes: bytes.to_vec(),
        hangup: None,
    }
}

const OPEN: &[u8] = b"OPEN\x1b[3\r";
const CLOSING: &[u8] = b"1mDONE\r\n";

fn offset(bytes: &[u8]) -> TestResult<u64> {
    Ok(u64::try_from(bytes.len())?)
}

/// The scenario's open CSI, then its closing bytes: once the wait for the closing bytes
/// matches, the hub paints the closed sequence exactly.
#[test]
fn a_matched_wait_finds_its_bytes_already_in_the_hub_paint() -> TestResult {
    let (hub, waits) = readers()?;
    let scope = CancelScope::new();
    let ticket = waits.output(OutputWait::new(OutputOffset::new(0), b"1mDONE")?, &scope)?;
    super::output((&hub, &waits), &chunk(0, OPEN), Duration::ZERO)?;
    assert!(!hub.screen().first_paint()?.fidelity.is_exact());
    super::output(
        (&hub, &waits),
        &chunk(offset(OPEN)?, CLOSING),
        Duration::ZERO,
    )?;
    let outcome = ticket.wait(Instant::now())?;
    assert!(
        matches!(outcome, OutputOutcome::Matched { .. }),
        "{outcome:?}"
    );
    let paint = hub.screen().first_paint()?;
    assert!(paint.fidelity.is_exact(), "{:?}", paint.fidelity);
    assert_eq!(paint.offset.get(), offset(OPEN)? + offset(CLOSING)?);
    Ok(())
}

/// The hub refuses a chunk after its terminal ended there (the kill path ends the hub
/// alone); the waits must not report bytes no attach stream or screen read will ever show.
#[test]
fn a_chunk_the_hub_refuses_resolves_no_wait() -> TestResult {
    let (hub, waits) = readers()?;
    hub.finish(&Exit {
        target: target(),
        code: Some(0),
        signal: None,
        exit_unknown: None,
        io_failure: None,
        final_offset: 0,
    })?;
    let scope = CancelScope::new();
    let ticket = waits.output(OutputWait::new(OutputOffset::new(0), b"DONE")?, &scope)?;
    let committed = super::output((&hub, &waits), &chunk(0, b"DONE\r\n"), Duration::ZERO);
    assert!(committed.is_err(), "the ended hub took the chunk");
    let outcome = ticket.wait(Instant::now())?;
    assert!(
        matches!(outcome, OutputOutcome::Timeout { .. }),
        "{outcome:?}"
    );
    assert_eq!(waits.cursor()?.get(), 0);
    Ok(())
}
