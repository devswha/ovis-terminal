//! Native ConPTY tests: real pseudoconsoles, real `cmd.exe`, real job objects.
//!
//! The driver plays the core: it feeds output into the VT engine and writes the
//! engine's query replies back as input, which is how ConPTY's start-up cursor query is
//! answered in production.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use ovis_terminal_protocol::domain::conpty::{ConptyGrid, ConptyLaunch};
use ovis_terminal_vt::{Geometry, HistoryLimit, QueryOutcome, Vt, VtConfig};

use super::model::{Ended, InputLimits, OutputEnd, OutputOffset, OutputRead, ProgramExit};
use super::process::open_handle_count;
use super::{ConptyTerminal, KILL_EXIT_CODE, TerminalLimits};

const BOUND: Duration = Duration::from_secs(30);
const HANDLE_PROBE: &str = "OVIS_CONPTY_HANDLE_PROBE";

fn system_root() -> PathBuf {
    std::env::var_os("SystemRoot").map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
}

fn cmd(script: &str) -> ConptyLaunch {
    let root = system_root();
    let shell = root.join(r"System32\cmd.exe").display().to_string();
    let env = BTreeMap::from([
        ("SystemRoot".to_owned(), root.display().to_string()),
        (
            "PATH".to_owned(),
            root.join("System32").display().to_string(),
        ),
        ("ComSpec".to_owned(), shell.clone()),
    ]);
    let argv = vec![shell, "/d".to_owned(), "/c".to_owned(), script.to_owned()];
    let grid = ConptyGrid::new(80, 24).expect("grid");
    ConptyLaunch::new(argv, std::env::temp_dir(), grid)
        .and_then(|launch| launch.with_env(env))
        .expect("launch")
}

struct Driver {
    terminal: ConptyTerminal,
    vt: Vt,
    at: OutputOffset,
    bytes: Vec<u8>,
    win32_seen: bool,
}

impl Driver {
    fn spawn(launch: &ConptyLaunch, limits: TerminalLimits) -> Self {
        let geometry = Geometry::new(80, 24).expect("geometry");
        Self {
            terminal: ConptyTerminal::spawn(launch, limits).expect("spawn"),
            vt: Vt::new(VtConfig::new(geometry, HistoryLimit::new(50))),
            at: OutputOffset::ZERO,
            bytes: Vec::new(),
            win32_seen: false,
        }
    }

    fn until(&mut self, stop: impl Fn(&[u8]) -> bool) -> Option<Ended> {
        let deadline = Instant::now().checked_add(BOUND).expect("deadline");
        while !stop(&self.bytes) {
            let left = deadline.saturating_duration_since(Instant::now());
            assert!(
                !left.is_zero(),
                "no progress within {BOUND:?}; read {:?}",
                self.bytes.escape_ascii().to_string()
            );
            match self.terminal.read(self.at, left).expect("read") {
                OutputRead::Bytes { from, bytes, gap } => {
                    assert!(!gap && from == self.at, "output is contiguous");
                    let length = u64::try_from(bytes.len()).expect("len");
                    self.at = OutputOffset::new(self.at.get().checked_add(length).expect("offset"));
                    for query in self.vt.feed(&bytes).queries {
                        if let QueryOutcome::Reply(reply) = query.outcome {
                            self.terminal.input(reply).expect("query reply admitted");
                        }
                    }
                    self.win32_seen |= self.vt.modes().keyboard.win32_input;
                    self.bytes.extend(bytes);
                }
                OutputRead::Pending => {}
                OutputRead::Ended(ended) => return Some(ended),
            }
        }
        None
    }

    fn finish(mut self) -> (Ended, Vec<u8>, bool) {
        let ended = self.until(|_| false).expect("the exit arrives");
        self.terminal.close().expect("workers joined");
        (ended, self.bytes, self.win32_seen)
    }
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

#[test]
fn utf8_output_and_win32_input_mode_precede_an_exit_carrying_the_final_offset() {
    let driver = Driver::spawn(
        &cmd("echo OVIS-\u{e9}-53& exit /b 7"),
        TerminalLimits::DEFAULT,
    );
    let (ended, bytes, win32_seen) = driver.finish();
    assert!(
        contains(&bytes, "OVIS-\u{e9}-53".as_bytes()),
        "{:?}",
        bytes.escape_ascii().to_string()
    );
    assert_eq!(ended.exit, ProgramExit::Code(7));
    assert_eq!(ended.output, OutputEnd::Eof);
    assert_eq!(
        ended.final_offset.get(),
        u64::try_from(bytes.len()).expect("len")
    );
    assert!(
        contains(&bytes, b"\x1b[?9001h"),
        "ConPTY's win32-input request reaches the core"
    );
    assert!(
        win32_seen,
        "the VT engine reports mode 9001 while the program runs"
    );
}

#[test]
fn a_resize_and_a_keypress_reach_the_program_through_separate_paths() {
    let mut driver = Driver::spawn(
        &cmd("pause >NUL & mode con & exit /b 3"),
        TerminalLimits::DEFAULT,
    );
    let started = driver.until(|bytes| contains(bytes, b"\x1b]0;"));
    assert!(
        started.is_none(),
        "cmd sets its title once the cursor handshake is answered"
    );
    driver
        .terminal
        .resize(ConptyGrid::new(132, 40).expect("grid"))
        .expect("resize");
    driver
        .terminal
        .input(b"x".to_vec())
        .expect("keypress admitted");
    let (ended, bytes, _) = driver.finish();
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("132") && text.contains("40"),
        "mode con reports the new grid: {text}"
    );
    assert_eq!(ended.exit, ProgramExit::Code(3));
}

#[test]
fn kill_ends_a_program_that_never_reads_input_and_the_job_is_left_empty() {
    let limits = TerminalLimits {
        input: InputLimits::new(1 << 20, 64),
        ..TerminalLimits::DEFAULT
    };
    let mut driver = Driver::spawn(&cmd("ping -n 600 127.0.0.1 >NUL"), limits);
    let oversized = driver
        .terminal
        .input(vec![b'y'; (1 << 20) + 1])
        .expect_err("over the byte bound");
    assert_eq!(oversized.kind(), "input_overflow");
    let mut admitted = 0_u32;
    while admitted < 64 && driver.terminal.input(vec![b'y'; 256 << 10]).is_ok() {
        admitted = admitted.saturating_add(1);
    }
    let requested = Instant::now();
    driver.terminal.kill().expect("kill requested");
    let ended = driver.until(|_| false).expect("exit evidence");
    assert!(
        requested.elapsed() < BOUND,
        "kill did not wait behind {admitted} queued inputs"
    );
    assert_eq!(ended.exit, ProgramExit::Code(KILL_EXIT_CODE));
    assert_eq!(
        driver.terminal.active_processes().expect("job accounting"),
        0
    );
    assert_eq!(
        driver
            .terminal
            .input(Vec::new())
            .map_err(|error| error.kind()),
        Err("process_exited")
    );
    driver.terminal.close().expect("workers joined");
}

#[test]
fn a_missing_program_or_directory_is_spawn_failed_with_nothing_left_running() {
    let grid = ConptyGrid::new(80, 24).expect("grid");
    let missing = ConptyLaunch::new(
        vec![r"C:\ovis-missing\absent.exe".to_owned()],
        std::env::temp_dir(),
        grid,
    )
    .expect("launch");
    let error =
        ConptyTerminal::spawn(&missing, TerminalLimits::DEFAULT).expect_err("no such program");
    assert_eq!(
        (error.kind(), std::error::Error::source(&error).is_some()),
        ("spawn_failed", true)
    );
    let nowhere = ConptyLaunch::new(
        vec!["cmd.exe".to_owned()],
        PathBuf::from(r"C:\ovis-missing-dir"),
        grid,
    )
    .expect("launch");
    let error =
        ConptyTerminal::spawn(&nowhere, TerminalLimits::DEFAULT).expect_err("no such directory");
    assert_eq!(error.kind(), "spawn_failed");
}

fn handle_cycle() {
    let mut driver = Driver::spawn(&cmd("echo cycle& exit /b 0"), TerminalLimits::DEFAULT);
    assert!(driver.until(|_| false).is_some(), "the cycle program exits");
    driver.terminal.close().expect("workers joined");
}

/// Runs in a fresh copy of this test binary, so no concurrent test opens handles.
#[test]
fn closing_a_terminal_leaves_no_handle_open() {
    if std::env::var_os(HANDLE_PROBE).is_some() {
        handle_cycle();
        let before = open_handle_count().expect("count");
        for _ in 0..3 {
            handle_cycle();
        }
        let after = open_handle_count().expect("count");
        println!("handle-probe before={before} after={after}");
        assert_eq!(
            after, before,
            "handles leaked across three spawn/close cycles"
        );
        return;
    }
    let name = "windows::terminal_tests::closing_a_terminal_leaves_no_handle_open";
    let output = std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args([name, "--exact", "--nocapture", "--test-threads=1"])
        .env(HANDLE_PROBE, "1")
        .output()
        .expect("probe process");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success() && stdout.contains("handle-probe before="),
        "status {:?}\nstdout {stdout}\nstderr {}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}
