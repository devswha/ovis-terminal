feat(conpty): own windows ptys through safe conpty boundaries (plan todo 53)

Closes GAP-048, GAP-049, GAP-051, GAP-062, GAP-121 (holder side). Delivers IDEAL-034, IDEAL-042, IDEAL-043, IDEAL-044, IDEAL-046, IDEAL-093 (holder side). Enforces EXCL-017, EXCL-019, EXCL-026 and does not reproduce V-017, V-068, V-4076, V-4077.

## What changed

- `crates/ovis-terminal-holder/src/windows/` (new): ConPTY terminals.
  - `model/` is platform-neutral and builds on every host. It also builds under `cfg(test)` and with the new `ffi-model` feature, for Miri.
    - `OutputLog`: bounded raw output addressed by absolute `OutputOffset`. Evicted ranges are flagged as a `gap`. The exit is readable only by a reader that has consumed everything up to `final_offset`, so the exit cannot overtake output (IDEAL-043, GAP-062).
    - `InputQueue`: bounded in both bytes and entries. A rejected request is refused before any of its bytes are accepted (`input_overflow`, `process_exited`), and every admitted sequence is either written or reported unwritten (IDEAL-044).
    - `Teardown`: the pseudoconsole is closed exactly once, only after the program exits, while the reader keeps draining. The exit is published exactly once, after both the program exit and end-of-output are observed. This is the fix for the todo-12 finding `conpty_eof_after_exit: timeout`: ConPTY never reaches EOF until the pseudoconsole is closed.
    - `Ended::exit_message`: maps to the holder wire `Exit`. The u32 Windows exit code is carried bit-for-bit; a failed wait becomes `exit_unknown` and a read failure becomes `io_failure`, never a default.
  - `cfg(windows)` FFI boundary:
    - `job.rs`: RAII job object created with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`. `kill` uses `TerminateJobObject` on the whole tree.
    - `process.rs`: an owned duplicate of the child's process handle, so the PID cannot be reused while held. Its `start_token` is `windows-created:<FILETIME>`, read with `GetProcessTimes` through that handle.
    - Every `unsafe` block is a single operation with its own `SAFETY:` comment (`undocumented_unsafe_blocks` and `multiple_unsafe_ops_per_block` are denied).
  - `terminal.rs` / `workers.rs`: `ConptyTerminal::spawn(&ConptyLaunch, TerminalLimits)` starts the program through the pinned portable-pty 0.9.0 ConPTY backend with explicit argv, cwd and env (env cleared), and assigns it to its job. Three named threads run, and all are joined on `close()`/`Drop` (no detached loops, V-4076):
    - the waiter starts first, so an exit can always close the console;
    - the reader always drains output;
    - the writer empties the input queue.
  - `resize` and `kill` act on the caller's thread and never queue behind input. Output bytes pass through untouched, including ConPTY's `CSI ? 9001 h` win32-input request; the core's VT engine owns modes and query replies. The holder parses no VT and does no screen mirroring (EXCL-019).
- `crates/ovis-terminal-protocol/src/domain/conpty.rs` (new): `ConptyGrid` and `ConptyLaunch::admit(&TerminalRunParams)`.
  - The grid is checked first, as 1..=32767 per dimension. portable-pty casts `u16` to the signed `COORD` with `as i16`, so 32768 and above would wrap; such values are refused, not clamped.
  - Then argv (non-empty, no NUL), cwd (present, non-empty, no NUL) and env (Windows environment-block rules) are checked.
  - `ConptyLaunchError` is a thiserror enum with `kind()` (`invalid_geometry` / `invalid_params`) and `hint()`.
- `crates/ovis-terminal-server/src/dispatch/lanes/conpty.rs` (new):
  - `admit` maps refusals to `DispatchError::Domain`, carrying the domain kind and hint, before any allocation.
  - `spawn` (Windows only) maps `ConptyError` to its kind; a program that could not start is `spawn_failed`.
- `conformance/scenarios/platform/conpty.json`: declared-only descriptor, listed in `declared.json` with owner 53 and the blocking todos. See DEFERRED VERIFICATION below.
- `.github/workflows/probe-stress-macos.yml`: my `conpty` option and a `windows-2025` job appended at the end. The job runs the native ConPTY and lane tests 10 times.

## Seams

- New public API:
  - `ovis_terminal_holder::windows`: `model::*` always; `ConptyTerminal`, `ConptyError`, `TerminalLimits`, `InputProgress` and `KILL_EXIT_CODE` on Windows.
  - `ovis_terminal_protocol::domain::conpty`.
  - `ovis_terminal_server::dispatch::lanes::conpty::{admit, spawn}`.
- New holder feature `ffi-model` and a `cfg(windows)` dependency `windows-sys 0.61`. That is the version already in Cargo.lock; the only lock change is the holder's dependency list.
- Slot lines added in `holder/src/lib.rs`, `protocol/src/domain.rs` and `server/src/dispatch/lanes.rs`. `holder/src/windows.rs` is the module root of the `src/windows/` write scope.
- No landed API changed. Holder wire generation stays 2, and no message shape changed.
- `#[cfg(unix)] pub mod unix_service` is untouched.

## Before

`absent`: no comparable baseline capability; this is not an executed failing test. Gap baseline: GAP-048, GAP-049, GAP-051, GAP-062, GAP-121. Todo 12's Windows CI smoke recorded the gaps this closes on the holder side: `conpty_output_before_exit` held output behind an unanswered `ESC[6n`, and `conpty_eof_after_exit` was `{"Err":"timeout"}` (docs/evidence/task-12/ci-probe-records.log).

## After

Host for the local runs: `Linux x86_64`. Native Windows results come from the CI `test (windows-latest)` job on `windows-2025` and from the `conpty` stress run linked below.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal          # exit 0
$ cargo test -p ovis-terminal-holder --lib windows::                     # 7 passed (model; Linux)
$ cargo test -p ovis-terminal-protocol --lib conpty                      # 4 passed
$ cargo test -p ovis-terminal-server --lib conpty                        # 2 passed (Linux); +1 Windows-only
$ PROPTEST_CASES=16 MIRIFLAGS=-Zmiri-disable-isolation cargo +nightly miri test -p ovis-terminal-holder --no-default-features --features ffi-model --lib windows::model
  test result: ok. 7 passed; 0 failed ... MIRI_EXIT 0                     # Miri checks the pure model only; it does not execute ConPTY
$ RUSTFLAGS=-Dwarnings cargo clippy -p ovis-terminal-holder -p ovis-terminal-protocol -p ovis-terminal-server --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings   # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence all-all.json
  {"result":"passed","totals":{"assertions":"530","cases":"22","declared_only":"28","failed":"0","passed":"22","skipped":"0"}}   # exit 0
```

Native Windows tests, run by ci.yml `test (windows-latest)` through nextest and by the `conpty` stress job (`crates/ovis-terminal-holder/src/windows/terminal_tests.rs`). The test driver plays the core: it feeds output into `ovis_terminal_vt::Vt` and writes the engine's query replies back as input, which is how ConPTY's start-up `ESC[6n` gets answered.

- `utf8_output_and_win32_input_mode_precede_an_exit_carrying_the_final_offset`: `cmd.exe /d /c "echo OVIS-é-53& exit /b 7"`. Asserts:
  - the UTF-8 marker bytes are present;
  - `ProgramExit::Code(7)` and `OutputEnd::Eof`;
  - `final_offset` equals the number of bytes read;
  - the stream contains `ESC[?9001h`, and `vt.modes().keyboard.win32_input` is true.
- `a_resize_and_a_keypress_reach_the_program_through_separate_paths`: `pause >NUL & mode con & exit /b 3`. Resizes to 132x40, then sends a keypress as input. Asserts the `mode con` output contains `132` and `40`, and exit code 3.
- `kill_ends_a_program_that_never_reads_input_and_the_job_is_left_empty`: `ping -n 600` never reads stdin. Asserts:
  - a request one byte over the byte bound is `input_overflow`;
  - up to 64 queued 256 KiB inputs do not delay `kill`;
  - the exit is `Code(137)` (`KILL_EXIT_CODE`) within 30 s;
  - the job reports 0 active processes;
  - later input is `process_exited`.
- `a_missing_program_or_directory_is_spawn_failed_with_nothing_left_running`: a missing exe and a missing cwd are each `spawn_failed` with a source.
- `closing_a_terminal_leaves_no_handle_open`: re-executes the test binary in isolation. After a warm-up cycle, three spawn/close cycles leave `GetProcessHandleCount` unchanged.

## DEFERRED VERIFICATION (runner lands in Todo 21)

The Todo-21 runner is landed, but `platform/conpty` cannot be executable yet:

- no served binary routes `terminal.run` to the ConPTY lane: the named-pipe Windows service host is todo 54, and the served `terminal.run` assembly is todo 31;
- the runner's fixture binds only a Unix socket;
- a windows-only case would be `skipped` on every runner the suite runs on.

It is therefore declared (`declared.json`, owner 53), and the plan's commands were recorded verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/conpty --isolated --case happy --evidence "$ATTEMPT_DIR/task-53-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"platform/conpty is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"platform/conpty","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
exit 1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/conpty --isolated --case failure --inject '{"method":"terminal.run","params":{"rows":0,"cols":80}}' --evidence "$ATTEMPT_DIR/task-53-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"platform/conpty is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},"result":"failed",...}
exit 1
```

Until the route exists, these behaviours are proven by the lane tests instead:

- "invalid_geometry before ConPTY allocation": `lanes::conpty::tests::a_zero_row_launch_is_invalid_geometry_on_the_wire_before_anything_else` and `domain::conpty::tests::geometry_is_refused_before_any_other_defect`. Admission is pure and runs before `ConptyTerminal::spawn`.
- "broken child launch reports spawn_failed": `lanes::conpty::tests::a_program_that_cannot_start_is_spawn_failed` (Windows CI).

## QA

- Happy and failure behaviour runs natively on Windows CI (above). The scenario runs are recorded as DEFERRED.
- `bash scripts/check.sh`: `733 tests run: 733 passed (1 slow), 1 skipped`, exit 0 (check-summary.log).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0 (windows-precheck.log).

## Adversarial

- Malformed input:
  - rows 0, cols 32768 (would wrap a signed COORD), empty argv, NUL in an argument, `A=B` and empty env names, NUL in an env value, missing or empty cwd, and an unresolved template are each refused with their typed cause;
  - proptest over all u16x2 grids;
  - a missing program or directory is `spawn_failed`.
- Hung command: a program that never reads stdin is killed through its job while input is queued, and the exit arrives within the bound.
- Misleading success:
  - `kill()` only requests termination;
  - the exit (code 137, after `final_offset`) and 0 active job processes are the evidence (EXCL-026);
  - a failed wait or read is reported as `exit_unknown` / `io_failure`, never as success.
- Ordering: proptests show that over any event interleaving the console closes once after the program exit and the exit is published once after both teardown events. Over arbitrary chunking and retention, reads return the retained stream in order and the exit comes last.

## Contract checklist

1. Every u64 on the wire stays a decimal string. `Exit.final_offset` round-trips at u64::MAX as `"18446744073709551615"` (`model::tests`), and any u32 exit code survives the holder frame (proptest). No new JSON counters.
2. `ConptyLaunchError`, `ConptyError`, `OutputError` and `InputRejected` are thiserror enums with `kind()`/`hint()` and `#[source]`, and Display does not repeat the kind (asserted).
3. Newtypes: `OutputOffset`, `InputSequence`, `ConptyGrid`; process identity is `ProcessStartToken`.
4. Tests assert behaviour (ordering, bounds, typed kinds), with proptest for the model and grids.
5. No `#[allow]`/`#[expect]` and no TODOs; the hygiene checker reports 0 violations.
6. Holder wire generation is unchanged at 2.

## Risks

- A grandchild the program creates between `CreateProcessW` and `AssignProcessToJobObject` escapes the job: portable-pty does not expose `CREATE_SUSPENDED`.
- Holder separation from the SSH job is the service host's contract (todo 54). Per-terminal jobs nest inside whatever job the holder runs in.
- The Windows `ConptyTerminal` is a per-terminal ownership API. Adoption, `Welcome` replay, write-grant fences and the named-pipe link are todo 54; they are not implemented here.
- `ExitStatus::exit_code` from portable-pty is used for the status; `TerminateJobObject` sets it to 137.
- The `cmd.exe` echo of `é` relies on ConPTY emitting UTF-8, which is its documented output encoding.
