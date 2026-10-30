//! Linux/macOS native smoke: grid, resize, modes, start tokens, final bytes,
//! endpoint identity, holder survival and the latency and soak harnesses.

use std::path::Path;
use std::time::Duration;

use ovis_terminal_bench::harness::{LatencyProfile, SoakProfile, run_latency, run_soak};
use ovis_terminal_holder::protocol::{Exit, Message, Spawned, TerminalTarget};
use ovis_terminal_probes::platform::{
    Grid, Interruption, ProcessState, PtySession, StartTokenSource, StreamEnd, observe_process,
    probe_detach, probe_mode_round_trip, probe_unix_endpoint,
};
use ovis_terminal_vt::{PasteMode, ScreenBuffer};
use portable_pty::CommandBuilder;

use crate::{BOUND, ScratchDir, record, terminal, through_holder_frame};

/// Asks the shell in `session` for `stty size`; the marker is split in the
/// typed command so the terminal's echo of the command cannot match it.
fn reported_grid(session: &mut PtySession) -> Grid {
    session
        .write(b"printf 'GR''ID %s\\n' \"$(stty size)\"\n")
        .unwrap();
    session.read_until(b"GRID ", BOUND).unwrap();
    let line = session.read_until(b"\n", BOUND).unwrap();
    let text = String::from_utf8(line).unwrap();
    let mut numbers = text
        .split_whitespace()
        .map(|number| number.parse::<u16>().unwrap());
    let rows = numbers.next().unwrap();
    Grid::new(numbers.next().unwrap(), rows)
}

fn shell_session() -> PtySession {
    let mut command = CommandBuilder::new("sh");
    command.env("PS1", "");
    PtySession::spawn(command, Grid::new(80, 24)).unwrap()
}

fn grid_resize_token_and_final_bytes() {
    let mut session = shell_session();
    assert_eq!(reported_grid(&mut session), Grid::new(80, 24));
    session.resize(Grid::new(132, 40)).unwrap();
    assert_eq!(session.grid().unwrap(), Grid::new(132, 40));
    assert_eq!(reported_grid(&mut session), Grid::new(132, 40));
    let pid = session.pid().unwrap();
    let observed = observe_process(pid).unwrap();
    assert_eq!(observed.state, ProcessState::Live);
    assert_eq!(
        observe_process(pid).unwrap().start_token,
        observed.start_token
    );
    session
        .write(b"printf 'FINAL-''BYTES\\n'; exit 7\n")
        .unwrap();
    let exit = session.wait_exit(BOUND).unwrap();
    assert_eq!((exit.code, exit.signal.as_deref()), (7, None));
    let (tail, end) = session.drain_to_end(BOUND).unwrap();
    assert!(
        String::from_utf8_lossy(&tail).contains("FINAL-BYTES"),
        "final bytes lost: {tail:?}"
    );
    assert_eq!(end, StreamEnd::Eof);
    let final_offset = session.received().get();
    let gone = observe_process(pid).unwrap_err();
    assert_eq!(gone.kind(), "process_record_unreadable");
    let (terminal_id, terminal_incarnation, process_incarnation) = terminal("probe-shell");
    let spawned = Message::Spawned(Spawned {
        terminal_id: terminal_id.clone(),
        terminal_incarnation,
        process_incarnation,
        pid: pid.get(),
        start_token: observed.start_token.clone(),
    });
    let target = TerminalTarget {
        terminal_id,
        terminal_incarnation,
        process_incarnation: Some(process_incarnation),
    };
    let exited = Message::Exit(Exit {
        target,
        code: Some(7),
        signal: None,
        exit_unknown: None,
        io_failure: None,
        final_offset,
    });
    assert_eq!(through_holder_frame(&spawned), spawned);
    assert_eq!(through_holder_frame(&exited), exited);
    session.finish(BOUND).unwrap();
    record("start_token_source", &StartTokenSource::current().unwrap());
    record("process", &observed);
    record("holder_frames", &[spawned, exited]);
    record(
        "process_after_exit",
        &serde_json::json!({ "kind": gone.kind() }),
    );
}

fn modes_round_trip() {
    let modes = probe_mode_round_trip(Grid::new(80, 24), BOUND).unwrap();
    assert_eq!(
        (modes.set.paste, modes.set.buffer),
        (PasteMode::Bracketed, ScreenBuffer::Alternate)
    );
    assert_eq!(
        (modes.reset.paste, modes.reset.buffer),
        (PasteMode::Plain, ScreenBuffer::Primary)
    );
    record("modes", &modes);
}

fn endpoint_identity() {
    let scratch = ScratchDir::new("endpoint");
    let endpoint = probe_unix_endpoint(&scratch.0).unwrap();
    assert_eq!(
        endpoint.first_rejected_path_bytes,
        endpoint.max_path_bytes.saturating_add(1)
    );
    assert!(endpoint.max_path_bytes >= 100, "{endpoint:?}");
    assert_eq!(endpoint.identity.owner_uid, endpoint.directory_owner_uid);
    assert_eq!(endpoint.identity.mode & 0o077, 0);
    assert!(endpoint.exchanged.get() > 0);
    record("endpoint", &endpoint);
    record(
        "endpoint_inode_reused",
        &(endpoint.rebound_identity.inode == endpoint.identity.inode),
    );
}

fn holder_survives(fixture: &Path) {
    for interruption in [Interruption::CoreDeath, Interruption::Logout] {
        let report = probe_detach(fixture, interruption, BOUND).unwrap();
        assert!(report.launcher_exit.signal.is_some(), "{report:?}");
        for process in [&report.holder, &report.program] {
            assert_eq!(process.after.start_token, process.before.start_token);
            assert_eq!(process.after.state, ProcessState::Live);
        }
        record("detach", &report);
    }
}

fn harnesses() {
    let latency = run_latency(&LatencyProfile::LOCAL).unwrap();
    assert_eq!(latency.latency.samples.get(), 16 * 20);
    assert!(latency.received >= latency.written, "{latency:?}");
    assert!(latency.resident.harness.get() > 0 && latency.resident.programs.get() > 0);
    record("latency", &latency);
    let soak = run_soak(&SoakProfile::new(
        SoakProfile::CAPACITY_TERMINALS,
        Duration::from_secs(3),
    ))
    .unwrap();
    assert!(soak.rounds.get() > 0);
    assert_eq!(
        soak.latency.samples.get(),
        soak.rounds.get().saturating_mul(50)
    );
    assert_eq!(soak.programs_with_unchanged_tokens, 50);
    assert!(soak.resident_peak.total() >= soak.resident_first.total());
    record("soak", &soak);
}

/// Runs every Unix happy-path probe.
///
/// # Panics
///
/// When the host behaves other than the probe asserts.
pub fn run() {
    grid_resize_token_and_final_bytes();
    modes_round_trip();
    endpoint_identity();
    holder_survives(Path::new(env!("CARGO_BIN_EXE_ovis-terminal-probe-holder")));
    harnesses();
}
