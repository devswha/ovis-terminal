## Before

Absent: GAP-038, GAP-062 and GAP-063 have no comparable new-runtime capability
claimed as a legacy execution (todo 6 Before line). For context only, the
interrupt trap this holder must not reproduce (EXCL-026) is recorded at
`docs/evidence/baseline/smoke/runtime-observations.md:8` (RT-HC-03: `hc interrupt`
reported `"outcome":"stopped"` while `sleep 30` kept running).

## After

`ovis-terminal-holder` now owns Unix PTYs through pinned `portable-pty` 0.9.0
(Cargo.lock). What landed:

- `unix::Holder` is one holder for every terminal of a session and build. It
  spawns from an explicit argv, cwd, complete environment (nothing inherited)
  and grid. Spawn failures are reported separately, as `SpawnFailed` plus typed
  `LaunchError` kinds.
- Each terminal has an always-draining pump into a bounded 1 MiB raw tail, a
  16 MiB per-terminal input queue with write-boundary fencing, and
  resize/signal paths that never wait behind input. `InputWritten` reports
  actual kernel-accepted byte counts, including partial writes.
- `Exit` is published only after the pump reaches PTY EOF, so `final_offset`
  covers every output byte. On macOS the holder keeps the slave open until the
  master is drained, because macOS discards unread output on the last slave
  close.
- `unix::HolderService` serves one adopting core at a time over a Unix socket.
  When a core disconnects, programs keep running and the next `Hello` gets a
  `Welcome` with the same PID and start token. Every thread is joined on
  shutdown: the accept loop is woken by a self-connect, avoiding V-215, and
  serial serving keeps at most one link-writer thread, avoiding V-220.
- `unix::spawn_detached` starts a holder process in its own session.
  `unix::CoreConnection` is the core side of the socket.
- `process::read_start_token` reads the start token from `/proc/<pid>/stat`
  plus the boot ID on Linux, and from `ps -o lstart=` on other Unix systems.
  `process::send_signal` delivers signals to a process or process group.
- Dedicated isolation is documented in `src/unix.rs` as an internal placement
  of the same holder. It adds no protocol or public schema field. Holders do
  no VT parsing, run no SQLite, infer no status and apply no policy.

Commands run on Linux 7.0.0-38-generic x86_64, rustc 1.99.1:

```text
$ cargo test -p ovis-terminal-holder --test unix happy -- --exact --nocapture
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --test unix failure -- --exact --nocapture
test failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out
exit: 0

$ cargo nextest run -p ovis-terminal-holder --status-level all --final-status-level all --no-tests=fail
Summary 27 tests run: 27 passed, 0 skipped
exit: 0

$ cargo clippy -p ovis-terminal-holder --all-targets -- -D warnings
exit: 0

$ bash scripts/check.sh
Summary 141 tests run: 141 passed, 1 skipped   (the skip is the pre-existing ignored store child-process fixture)
workspace-hygiene: 0 violation(s) in 168 file(s) checked
check.sh: all steps passed
exit: 0

$ cargo metadata --locked --format-version 1 >/dev/null
exit: 0
```

Stability: the `happy`/`failure` pair ran 160 times under 16-way parallel load
(0 failures), and the full holder suite ran 25 consecutive times (0 failures).

Logs: `docs/evidence/task-6/{qa-happy,qa-failure,nextest,clippy,check,conformance}.log`.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The requested real-binary scenario was attempted. The conformance runner is
still the Todo 21 scaffold and registers no scenarios, so this is a recorded
deferred result, not a passing CLI/socket conformance claim (plan line 275).
`conformance/scenarios/part-a/task-6.json` defines the happy and failure
transcripts for that runner.

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-6 --isolated --case happy --evidence "$ATTEMPT_DIR/task-6-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit: 2
```

The socket evidence below comes from the holder crate's own tests, which drive a
real Unix socket with the real frame codec; it is not the `ovis-terminal` binary.

## QA

`happy` (real holder socket, real `/bin/sh` child on a real PTY):

- The child runs `stty -echo; printf '\303'; read gate; printf '\251\n'; exit 7`.
  Core 1 receives `Spawned`, then `Output{offset 0, bytes [0xC3]}`, which is half
  of the UTF-8 encoding of `é`.
- Core 1 disconnects. Core 2 sends `Hello` and the `Welcome` record carries the
  same `pid` and `start_token`, `output_offset 1` and no exit.
  `read_start_token(pid)` read independently from the OS equals the original
  token.
- Core 2 sends `Input "go\n"`. It receives `Output{offset 1, [0xA9, 0x0D, 0x0A]}`
  and `InputWritten{written_bytes 3}`, then `Exit{code 7, signal null,
  final_offset 4}`. The test fails on any non-contiguous offset or on an `Exit`
  whose `final_offset` differs from the bytes received.
- `TailRequest{from 0}` returns `Tail{0..4, gap false}`, and the retained bytes
  decode as `"é\r\n"` across the disconnect. `Forget` then `Goodbye{shutdown}`
  stop the service, every thread is joined, and the holder is empty.

`failure` (in-process holder API plus its core link):

- argv `["/nonexistent/ovis-fixture"]` yields `HolderError` kind `spawn_failed`
  with source kind `executable_rejected`, plus
  `SpawnFailed{kind: invalid_request}` on the link. No terminal record is kept.
- A raw-mode child that never reads stdin is offered 1 MiB inputs until the
  holder returns kind `input_overflow` with `limit` 16 MiB, at send 18 or
  earlier.
- `resize` and `signal(kill)` still return `Ok` at that point. `Exit` arrives
  with signal 9. Every offered input is acknowledged exactly once. Entry 1
  reports a partial count (`0 < written < 1 MiB`), the overflowed entry reports
  0, and total written bytes stay below the queued bytes.

## Adversarial

- Hung commands (gated stdin reader): `failure` and
  `newer_fence_revokes_queued_input_behind_a_blocked_write`. A child that never
  reads leaves the writer blocked, yet resize, kill and fence revocation
  (`InputWritten{seq 2, written 0, fence 2}`) all complete.
- Cancel/resume (core socket disconnect): `happy` disconnects mid-character and
  resumes on a new connection without losing a byte.
- Repeated interruptions (holder re-adoption):
  `repeated_readoption_keeps_pid_and_start_token` re-adopts three times with the
  same PID and token each time. After a shutdown `Goodbye`, the start token
  reads `process_gone`.
- Malformed input: a bad-magic header, a request before `Hello`, and `Hello`
  generation 999 each end only that connection, with the queryable kinds
  `malformed_frame`, `hello_required` and `unsupported_generation`. Ownership
  and held terminals are unchanged.
- Misleading success (EXCL-026): `interrupt_delivery_is_not_termination_evidence`.
  An interrupt to a program that traps SIGINT returns delivered, and `Exit`
  (code 0) appears only after the program actually exits.
- Shutdown joins (V-215/V-220): `stop_wakes_accept_and_joins_while_a_core_is_connected`
  and the drain test (`Drain::Begin` refuses spawns with `unavailable`;
  `Drain::Complete` on an empty holder stops the service).

## Contract checklist

1. u64 as decimal strings: every holder wire counter is serialized through the
   todo-5 protocol types (canonical decimal strings). This lane only emits and
   accepts those types. `maximum_counters_round_trip_exactly_through_the_socket`
   sends `u64::MAX` sequence, grant and fence epochs over the real socket and
   asserts the acknowledgement decodes to exactly `u64::MAX`. The scenario JSON
   writes counters and offsets as strings.
2. Boundary errors: `LaunchError`, `HolderError`, `SignalError`,
   `StartTokenError`, `RecvError`, `CoreConnectionError`, `ConnectionError` and
   `ServeError` all derive `thiserror::Error` and have `kind()` and `hint()`.
   Inner errors use `#[source]`/`#[from]`. Display text never repeats the kind
   token.
3. Identities: `ProcessId` is new. Terminal, process and holder identities are
   the protocol newtypes. `Grid` rejects zero dimensions at construction.
   Offsets and sequences appear in the public API only inside protocol
   messages.
4. Behavior tests: tests assert state transitions, ordering and typed kinds.
   Proptests compare the retained tail with a stream model and the input queue
   with a byte-bounded fenced model. The minimized regression seed found during
   development is committed (it exposed stale-fence input being queued instead
   of revoked; fixed).
5. No `#[allow]`, no `#[expect]`, no TODO placeholders. The 9 `unsafe` blocks are
   single FFI calls (`kill`, `setsid`, `fcntl`, `poll`, `ioctl FIONREAD`,
   `BorrowedFd::borrow_raw`, plus `getsid` in a test), each with a `SAFETY:`
   comment.
6. Evidence: this body states what each test asserts. Real-binary conformance
   is DEFERRED-BY-PLAN, with the exit-2 attempt recorded above.

## Risks

- Protocol gaps (todo-5 scope, reported, not changed):
  - The v1 `Spawn` message has no cwd or environment. Wire spawns use the
    holder's configured `wire_environment`; explicit cwd/env is available only
    through the Rust `LaunchRequest`.
  - There is no typed input-rejection message. Over the wire, an overflow is a
    zero-byte `InputWritten`, and the typed `input_overflow` exists only in the
    Rust API.
  - There is no error reply. A rejected core request ends that connection with a
    queryable `last_failure_kind()`, and the core re-adopts.
- Root seams: `clippy.toml` `allowed-duplicate-crates` gains `bitflags`, `syn`,
  `thiserror` and `thiserror-impl`, because portable-pty 0.9 pulls their older
  majors. This follows the vt lane's precedent. `libc = "0.2"` is declared in
  the holder manifest (it is already in the lockfile) rather than in root
  `[workspace.dependencies]`, which belongs to todo 1.
- The macOS start token comes from `ps -o lstart=` (one-second resolution);
  plan todo 55 owns cross-OS process evidence. The macOS drain-before-close path
  and `getsid` behaviour were verified only by reasoning and will be exercised
  by the macOS CI job.
- Exit completes at PTY EOF. If a background descendant keeps the terminal
  open, `Exit` waits for it (by design, so `final_offset` is complete).
- Miri cannot execute the FFI calls; their evidence is native Linux tests here
  and the macOS CI run.
