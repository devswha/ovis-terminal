## Before

Absent: GAP-038, GAP-062 and GAP-063 have no comparable new-runtime capability
claimed as a legacy execution (todo 6 Before line). For context only, the
interrupt trap this holder must not reproduce (EXCL-026) is recorded at
`docs/evidence/baseline/smoke/runtime-observations.md:8` (RT-HC-03: `hc interrupt`
reported `"outcome":"stopped"` while `sleep 30` kept running).

## After

`ovis-terminal-holder` now owns Unix PTYs through pinned `portable-pty` 0.9.0
(Cargo.lock).

- `unix::Holder` is one holder for every terminal of a session and build.
  - Each launch carries its own argv (argv[0] names the executable, resolved
    through the launch's `PATH`), working directory, complete environment
    (nothing inherited from the holder) and grid. This holds both over the wire
    `Spawn` and through the Rust `LaunchRequest`.
  - A spawn failure is reported separately as `SpawnFailed` with a typed
    `reason`, a `hint`, and the OS `os_error` when one exists. For example, a
    missing executable path gives `executable_unavailable` / ENOENT, and a
    missing cwd gives `cwd_unavailable` / ENOENT.
- Each terminal has:
  - an always-draining pump into a bounded 1 MiB raw tail;
  - a per-terminal input queue bounded by both bytes (16 MiB) and entries
    (4096), with write-boundary fencing;
  - resize and signal paths that never wait behind input.
- Input that would exceed either bound is rejected before any byte is queued.
  The core receives a typed `InputRejected{reason: input_overflow, hint,
  queued_bytes, queued_entries, limit_bytes, limit_entries}` over the socket,
  and the rejected sequence is not consumed. `InputWritten` reports actual
  kernel-accepted counts, including partial writes.
- `Exit` is published only after the pump reaches the PTY hang-up, so
  `final_offset` covers every output byte. A PTY hang-up (EOF, EIO, POLLHUP) is
  the normal end. Any other read, poll, write or record failure is a typed
  `TerminalIoError` that keeps its `#[source]` (available through
  `Holder::io_errors`) and reaches the core as `Exit.io_failure{operation,
  os_error}`. A failed `wait()`, or a lost waiter or pump, becomes
  `Exit.exit_unknown{reason: wait_failed|waiter_lost, os_error}` with no code
  or signal, never a null exit.
- `unix::HolderService` serves one adopting core at a time over a Unix socket.
  Programs survive core disconnects, and every thread is joined on shutdown.
  `unix::spawn_detached` starts a holder in its own session, and
  `unix::CoreConnection` is the core side of the socket.

### Dependency seam: holder protocol generation bumped to 2 (todo 5)

Because the wire contract below changed, `HOLDER_PROTOCOL_GENERATION` is now
**2**. Every frame header carries the generation, so a generation-1 core is
refused at its first frame (its Hello) with the typed `unsupported_generation`,
before any state-changing message. It is never accepted and then broken mid-session
by a "missing field `cwd`" error. `generation_one_core_is_refused_at_hello_before_any_spawn`
replays the recorded generation-1 Hello and Spawn bytes over a real connection.
It asserts that the codec refuses the old Spawn by generation (not by payload
shape), that the core gets no Welcome, that `last_failure_kind` is
`unsupported_generation`, and that nothing was spawned. Pinned generations in
todo-5 fixtures/tests and in the `part-a/task-5` and `part-a/task-6` scenario
JSON were updated to the constant/"2".

### Dependency seam: holder protocol (todo 5) edited

Fixing B1 and B3 required changing todo 5's frozen message payloads, as the
orchestrator directed. The changes are additive in shape but alter the v1 wire
contract:

- `Spawn` gains required `cwd` and `env`.
- `SpawnFailed` gains `reason` (`SpawnFailureReason`), `hint` and `os_error`.
- A new `InputRejected` message (`MessageKind` tag 16) carries `InputRejection`.
- `Exit` gains optional `exit_unknown` and `io_failure`
  (`PtyIoFailure{operation, os_error, poll_revents}`).
- The new types live in `protocol/messages/outcome.rs`. The todo-5 property
  fixtures, counter-string test (the new u64 fields are decimal strings) and
  `Exit` literals in tests were updated to match.
- `HolderConfig` no longer carries a holder-wide wire environment; every launch
  is explicit.

### Forced cross-lane edit after merging integration (todo 12)

Todo 12's probe files construct holder messages directly. After merging
`origin/integration`, only the new required fields were added at their
construction sites: `probes/fixtures/probe_holder.rs`,
`probes/platform/tests/{platform_probe,unix_happy,windows_smoke}.rs`. No probe
behavior or assertion changed. The holder manifest is the union of both sides:
integration's `portable-pty = "=0.9.0"`, probe targets and machete ignores, plus
this lane's unix-only `libc`. `clippy.toml` is the union of both sides (identical
lists). `Cargo.lock` = integration's lock re-locked; `cargo generate-lockfile
--offline` reproduces it byte-for-byte and `cargo metadata --locked` exits 0.

### Out-of-scope edit (orchestrator-granted)

`clippy.toml` `allowed-duplicate-crates` (adds `bitflags`, `syn`, `thiserror`
and `thiserror-impl`) and `Cargo.lock` are changed. Both are forced by the
portable-pty 0.9.0 pin and were verified by todo 12's verifier with
`cargo tree -d`. `libc = "0.2"` is declared in the holder manifest.

Commands run on Linux 7.0.0-38-generic x86_64, rustc 1.99.1:

```text
$ cargo test -p ovis-terminal-holder --test unix happy -- --exact --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --test unix failure -- --exact --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out
exit: 0

$ cargo nextest run -p ovis-terminal-holder --status-level all --final-status-level all --no-tests=fail
Summary [   5.412s] 34 tests run: 34 passed, 0 skipped
exit: 0

$ cargo clippy -p ovis-terminal-holder --all-targets -- -D warnings
exit: 0

$ bash scripts/check.sh
Summary 215 tests run: 215 passed, 1 skipped   (merged with integration bc73b93)   (the skip is the pre-existing ignored store child-process fixture)
check.sh: all steps passed
exit: 0
```

Stability: the `happy`/`failure` pair ran 64 times under 16-way parallel load
(0 failures), and the full holder suite ran 10 consecutive times (0 failures).

Logs: `docs/evidence/task-6/{qa-happy,qa-failure,nextest,clippy,check,conformance}.log`.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The requested real-binary scenario was attempted. The conformance runner is
still the Todo 21 scaffold and registers no scenarios, so this is a recorded
deferred result, not a passing CLI/socket conformance claim (plan line 275).
`conformance/scenarios/part-a/task-6.json` now encodes the typed wire records
above.

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-6 --isolated --case happy --evidence "$ATTEMPT_DIR/task-6-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit: 2
```

The socket evidence below comes from the holder crate's own tests, which drive a
real Unix socket with the real frame codec; it is not the `ovis-terminal` binary.

## QA

`happy` (real holder socket, real `/bin/sh` child on a real PTY):

- The child prints `\303`, the first byte of `é`, then waits on `read gate`.
- Core 1 disconnects. Core 2 re-adopts, and `Welcome` shows the same `pid` and
  `start_token` (also checked independently with `read_start_token`),
  `output_offset 1` and no exit.
- `Input "go\n"` produces contiguous `Output{offset 1, [0xA9, 0x0D, 0x0A]}`, then
  `Exit{code 7, final_offset 4}`.
- `Tail{0..4}` decodes as `"é\r\n"`, and shutdown joins every thread.

`failure` (real holder socket only; this replaces the earlier in-process check):

- argv `["/nonexistent/ovis-fixture"]` yields
  `SpawnFailed{kind: invalid_request, reason: executable_unavailable, os_error: ENOENT}`
  with a non-empty hint.
- A raw-mode child that never reads stdin receives twenty 1 MiB `Input`
  messages through `CoreConnection`. This is the reviewer's repro. The core
  receives `InputRejected{reason: input_overflow, sequence <= 18, limit_bytes
  16 MiB, limit_entries 4096}`, with a non-empty hint and `queued_bytes + 1 MiB`
  above the limit. Later sequences are also rejected, not written.
- `Resize` and `Signal(kill)` sent over the same socket still settle:
  `Exit{signal 9, exit_unknown null, io_failure null}`.
- Every admitted entry is acknowledged once, entry 1 is a partial write
  (`0 < written < 1 MiB`), and total written bytes stay below the admitted bytes.

## Adversarial

- B1, typed wire overflow: the `failure` test above.
- B2, unbounded metadata: `empty_requests_are_bounded_by_the_entry_limit` is the
  reviewer's standalone regression. With a byte budget of 1 already full,
  100,000 empty requests admit exactly 1023, and the next empty request returns
  `Full{bytes 1, entries 1024}`.
  `admission_matches_a_byte_and_entry_bounded_fenced_model` is a proptest over
  random (length, fence, count) sequences against a reference model of both
  bounds plus fence revocation.
- B3, per-launch launch config:
  `each_launch_gets_its_own_cwd_and_environment_over_the_socket`. Two terminals
  launched over the socket with different `cwd` and `OVIS_PROBE` each print
  exactly their own canonical directory and value. A missing cwd yields
  `SpawnFailed{reason: cwd_unavailable, os_error: ENOENT}`.
- F2 (round 2), poll error flags: `wait()` treats POLLHUP as the only normal
  hang-up. POLLNVAL, or POLLERR without POLLHUP, becomes the typed
  `TerminalIoError::PollFlags{revents}` (kind `pty_poll_flagged`), which reaches
  `Exit.io_failure{operation: poll, poll_revents}` through the same path as read
  faults. Poll syscall failures keep their `#[source]`. The reviewer's probe is
  permanent as `poll_error_without_hangup_is_a_typed_failure_not_a_hangup`: a
  pipe with its reader dropped gives POLLOUT|POLLERR and no POLLHUP, and the test
  asserts the typed failure with the POLLERR bit. POLLNVAL shares the branch but
  is not separately fault-injected.
- B4, typed I/O failures:
  - `unexpected_read_failure_is_typed_not_a_hangup`: a real EISDIR read failure
    is `pty_read_failed` with its source and `PtyIoFailure{read, EISDIR}`, not
    EOF.
  - `unexpected_write_failure_is_returned_with_the_written_prefix`: a real
    EBADF write gives `pty_write_failed` with `written 0`.
  - `failed_wait_and_read_are_typed_exit_evidence_not_a_null_exit`: an ECHILD
    wait plus an EISDIR read produce `Exit{code null, signal null,
    exit_unknown{wait_failed, ECHILD}, io_failure{read, EISDIR}}`, and both
    retained errors keep `source()`.
- Hung commands, cancel/resume, repeated interruptions, malformed frames, the
  EXCL-026 interrupt check and shutdown joins: the earlier tests still pass
  (fence revocation behind a blocked write, re-adoption x3, bad magic,
  pre-Hello, generation 999, interrupt delivery is not termination evidence,
  stop and drain).

## Contract checklist

1. u64 as decimal strings: all holder wire counters, including the new
   `InputRejected` sequence and bounds, use the canonical decimal-string serde.
   The counter-string test raises them above 2^53 / to `u64::MAX` and asserts
   exact decoding, and `maximum_counters_round_trip_exactly_through_the_socket`
   does the same over the real socket.
2. Boundary errors: `LaunchError` (now with `ExecutableUnavailable`, `reason()`
   and `os_error()`), `HolderError` (`InputOverflow` with bytes, entries and
   limits), the new `TerminalIoError`, `SignalError`, `StartTokenError`,
   `RecvError`, `CoreConnectionError`, `ConnectionError` and `ServeError` all
   derive thiserror and have `kind()`/`hint()` and `#[source]`/`#[from]`. No
   failure becomes EOF or a null exit.
3. Identities: `ProcessId` and the protocol newtypes; `Grid` and `QueueLimits`
   are validated value types.
4. Behavior tests: tests assert transitions, ordering and typed kinds. Two
   proptests compare against reference models (the retained tail; the two-bound
   fenced queue), and the minimized regression seed is committed under the new
   test path.
5. No `#[allow]` and no TODO. The only `#[expect]` attributes are the per-struct
   `clippy::exhaustive_structs` ones on the new frozen protocol structs, which
   follow todo 5's convention. The 9 `unsafe` blocks are single FFI calls, each
   with a `SAFETY:` comment.
6. Evidence: this body states what each test asserts. Real-binary conformance
   is DEFERRED-BY-PLAN, with the exit-2 attempt recorded above.

## Risks

- The holder wire changed (seam above). Todo 16/assembly consumers must send
  `cwd`/`env` in `Spawn` and handle `InputRejected`, `SpawnFailed.reason` and
  `Exit.exit_unknown`/`io_failure`.
- After an unexpected read failure, the holder keeps the program running (no
  policy). The failure is recorded immediately and reported on `Exit`; the core
  may stop the program with `Signal`.
- macOS start token: `ps -o lstart=` has one-second resolution; todo 55 owns
  cross-OS process evidence.
- Exit completes at PTY EOF, so a background descendant holding the PTY delays
  `Exit` by design.
- Miri cannot run the FFI calls; the evidence is native Linux tests plus the
  macOS and Windows CI jobs.
