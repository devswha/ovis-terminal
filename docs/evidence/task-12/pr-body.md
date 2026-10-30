# test(platform-probe): establish native platform probes and performance harnesses

Plan todo 12 (wave 3). Closes GAP-048, GAP-049, GAP-051 at the probe tier; delivers the probe/harness slice of IDEAL-027, IDEAL-034, IDEAL-051, IDEAL-071, IDEAL-093. End-to-end acceptance through the `ovis-terminal` socket waits for the holder (Todo 6), attach lanes and the Todo 21 runner.

What lands:

- **`portable-pty` pinned to `=0.9.0`** (workspace `Cargo.toml`) after a native Linux spike ([portable-pty-spike.log](portable-pty-spike.log)). The probes below run the same checks on every CI OS.
- **`probes/platform/`** (`ovis_terminal_probes::platform`): `PtySession` (real PTY via portable-pty, bounded reader/waiter threads, marker reads, resize, kill, exit, joined `finish`), `HostFacts`/`ByteOrder`, `observe_process` (PID + start token + live/zombie; Linux `/proc/<pid>/stat` starttime + boot id, macOS `ps -o stat=,lstart=`, elsewhere typed `unsupported`), `resident_memory`, `probe_mode_round_trip` (program-set modes recovered by `ovis-terminal-vt`), `probe_stalled_writer`, `probe_detach`, `probe_unix_endpoint` / `probe_broken_unix_endpoint`, Windows `probe_named_pipe` / `probe_broken_named_pipe`. One `thiserror` boundary enum `ProbeError` with `kind()`/`hint()`/`#[source]`, and `spawn_failure_kind()` mapping onto the frozen holder `SpawnFailureKind`.
- **`probes/fixtures/probe_holder.rs`**: a holder fixture (bin `ovis-terminal-probe-holder` of the holder crate). `launch` starts `serve` in its own process group from a login terminal, `serve` owns a PTY running `cat`, and `openpty` answers an injected descriptor exhaustion with a real holder-protocol `SpawnFailed` frame.
- **`bench/harness/`** (`ovis_terminal_bench::harness`, bin `pty-harness`): `run_latency` (local profile: 16 PTYs, 1 MiB/s aggregate paced input, 100 ms verdict) and `run_soak` (50-PTY capacity, memory sampled each second, start tokens compared at the end). A fixed-memory 10 µs-bucket histogram keeps the exact max, so a long soak cannot grow without bound.
- **Test target** `cargo test -p ovis-terminal-holder --test platform_probe {happy,failure}` (source `probes/platform/tests/`), and **`.github/workflows/platform-probes.yml`** (Linux/macOS/Windows; the bootstrap `ci.yml` is untouched). `conformance/scenarios/part-a/task-12.json` uses the shell runner.

## Before

absent. The todo's Before line reads: "absent (GAP-048, GAP-049, GAP-051; no comparable new-runtime capability is claimed as a legacy execution)". No legacy transcript is cited because no legacy probe exists. The nearest legacy fact is herdr's missing Windows attach (GAP-048 evidence in `docs/design/gap-list.md:68`), which is research evidence, not a smoke transcript.

## After

All commands ran on **Linux x86_64**, rustc/cargo 1.99.1 (isolated local toolchain), worktree base `ea6fd5c` (origin/integration, unchanged at push time).

| Command | Exit | Result |
|---|---|---|
| `bash scripts/check.sh` (final head, [check-fix2.log](check-fix2.log)) | 0 | 131 run, 131 passed, 1 skipped; hygiene 0 violations in 173 files. First head: | fmt, workspace clippy `-D warnings`, nextest **131 run, 131 passed, 1 skipped** (the skip is pre-existing), doctests, hygiene 0 violations in 173 files, manifests (16), json (13) ([check.log](check.log)) |
| `cargo nextest run -p ovis-terminal-holder --locked` | 0 | 12 run, 12 passed, 0 skipped ([acceptance-nextest.log](acceptance-nextest.log)) |
| `cargo clippy -p ovis-terminal-holder --all-targets --locked -- -D warnings` | 0 | [acceptance-clippy.log](acceptance-clippy.log) |
| `cargo test --locked -p ovis-terminal-holder --test platform_probe happy -- --exact --nocapture` | 0 | `test happy ... ok` ([qa-happy.log](qa-happy.log)) |
| `cargo test --locked -p ovis-terminal-holder --test platform_probe failure -- --exact --nocapture` | 0 | `test failure ... ok` ([qa-failure.log](qa-failure.log)) |
| `cargo run --locked --release -p ovis-terminal-bench --bin pty-harness -- latency` | 0 | [harness-latency.log](harness-latency.log) |
| `cargo run --locked --release -p ovis-terminal-bench --bin pty-harness -- soak 50 30` | 0 | [harness-soak.log](harness-soak.log) |
| `cargo clippy -p ovis-terminal-probes --all-targets --target x86_64-pc-windows-msvc -- -D warnings`, and the same for `-p ovis-terminal-holder --bin ovis-terminal-probe-holder --lib` | 0 | Windows cross-check without linking. The holder *test* target and the bench bin cannot be cross-checked locally (bundled SQLite needs the MSVC C toolchain), so Windows execution proof comes from CI only. |

Unit tests added: `probes` `measure::tests::{u64_max_round_trips_exactly_as_a_string, numeric_and_non_canonical_forms_are_rejected, every_u64_round_trips (proptest)}`; `bench` `histogram::tests::{percentiles_follow_nearest_rank_and_max_stays_exact, merging_equals_recording_into_one}`.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The plan's real-binary command was run as written. Verbatim from [conformance-attempt.log](conformance-attempt.log):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-12 --isolated --case happy --evidence "$ATTEMPT_DIR/task-12-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit=2
$ uname -sm
Linux x86_64
```

Conformance execution is deferred by plan (plan line 275) for part-A foundational todos until Todo 21 lands. The binary has no PTY/holder socket surface yet. [task-12-ovis-terminal-v1.json](task-12-ovis-terminal-v1.json) is assembled by this lane, not by the runner. It holds the exit-2 attempt, every parsed `PROBE` record from both QA runs and both release harness reports.

## QA

### happy (Linux x86_64, parsed `PROBE` records from [qa-happy.log](qa-happy.log))

- **Host:** `{"os":"linux","family":"unix","arch":"x86_64","byte_order":"little"}`. The test asserts that the run-time measured byte order equals the compile-time `target_endian` and that os/arch equal `std::env::consts`.
- **Grid and resize:** the shell's `stty size` reports `24 80` at spawn. After `resize(132x40)` the backend reports 132x40 and `stty size` inside the program reports `40 132`.
- **Process token:** `{"pid":"…","start_token":"linux:<boot_id>:<starttime>","state":"live"}`, read twice with an equal token. After exit, `observe_process` fails with `process_record_unreadable`, so no stale "live" is reported.
- **Final bytes before exit:** the program prints `FINAL-BYTES` then `exit 7`. The test asserts exit `code 7, signal null`, `FINAL-BYTES` in the drained tail and stream end `eof`. Holder frames `Spawned{pid,start_token}` and `Exit{code:7, final_offset:"146"}` round-trip through `encode_message`/`decode_message` unchanged; u64 fields are decimal strings on the wire.
- **Mode round trip:** `ESC[?2004h ESC[?1049h` then the resets travel through the real PTY into `ovis-terminal-vt`, giving `set {bracketed, alternate}` then `reset {plain, primary}`.
- **Socket endpoint:** longest accepted Unix socket path 107 bytes, first rejected 108. Socket owner uid equals the directory owner, mode `0o600` (asserted `mode & 0o077 == 0`), 15 bytes exchanged unchanged. Whether a rebound socket gets a new inode is recorded, not asserted, because inode reuse is filesystem-dependent (`false` on this run).
- **Holder survival, two interruptions:** `core_death` (launcher SIGKILLed, exit `signal:"Killed"`) and `logout` (launcher SIGHUPed, exit `signal:"Hangup"`). Both the holder and its PTY program keep identical PID + start token and state `live` after the launcher's exit was collected. The detached processes are then SIGKILLed by the probe's reaper.
- **Harness (debug build inside the test):** latency 16 PTYs × 20 samples = 320 samples. The test asserts the sample count, `received >= written` and nonzero harness/program RSS. Soak: 50 PTYs for 3 s; the test asserts `samples == rounds × 50`, `programs_with_unchanged_tokens == 50` and peak ≥ first.
- **Release harness bin:**
  - latency: p50 4.27 ms, p95 5.26 ms, p99 5.80 ms, max 7.23 ms, `within_verdict: true`; 1,310,720 bytes written, 1,333,976 received; harness RSS 5.2 MB, 16 `cat` programs 123.7 MB in total.
  - soak (50 × 30 s): 816 rounds, 40,800 samples, p50 0.46 ms, p99 6.11 ms, max 94.68 ms; harness RSS 5.11 → 5.24 MB; all 50 tokens unchanged.

### failure (Linux x86_64, [qa-failure.log](qa-failure.log))

- **Hung command (no-read child):** `sh -c 'trap "" HUP; …; exec sleep 600'`. The writer blocked after 8,192 of 1,048,576 bytes. portable-pty's own `kill()` (SIGHUP on Unix) did **not** end the program (`backend_kill_ended_program:false`, asserted). SIGKILL did (`signal:"Killed"`, asserted), 1.0 s after the first kill (mostly the bounded SIGHUP wait). The blocked writer was **still blocked** after the program died (`writer:"still_blocked"`, recorded).
- **Missing executable:** `PtySession::spawn` fails with `kind() == "spawn_failed"` and `spawn_failure_kind() == Unavailable`. The holder `SpawnFailed{kind: unavailable}` frame round-trips. No process record is produced.
- **PTY creation refused:** under `ulimit -n 4` the fixture's `openpty` fails with `EMFILE` (`Too many open files`) and writes one holder frame, which decodes to `SpawnFailed{kind:"unavailable"}` (asserted).
- **Missing cwd:** requested `/nonexistent/ovis-probe-cwd`; portable-pty 0.9.0 started the program in `$HOME` (`/home/devswha`). The test asserts the program never reports the missing directory as its own.
- **Broken endpoint:** connecting to a removed socket gives `endpoint_failed` with io kind `NotFound`; writing to a closed peer gives `endpoint_failed` with io kind `BrokenPipe` (asserted, `ConnectionReset` also accepted for macOS).

The first failure run wrote 4 KiB chunks without newlines. On Linux, canonical mode accepted all 1 MiB without blocking, so no write blocked and the input past 4095 bytes per line was never delivered ([failure-run1-canonical-discard.log](failure-run1-canonical-discard.log)). The probe now writes whole lines; that silent discard is listed under Risks.

## Platform findings

Records below come from the PR's own CI runs of `platform-probes.yml`, copied from the job logs (`PROBE` lines) into [ci-probe-records.log](ci-probe-records.log).

### Windows (windows-2025, x86_64): blocks Todos 53/54

- **`conpty_output_before_exit`**: ConPTY withholds all program output until the terminal answers its cursor-position query. Exact record from run 41127802737 (head 4b5295e):

  ```json
  {"blocks":"todo-53/54","capability":"conpty_output_before_exit","detail":{"after_cursor_reply":"marker_arrived","cursor_query_seen":true,"error_kind":"timeout","received_bytes":"79","unconsumed":"\\x1b[6n","waited_ms":"10000"}}
  ```

  The only bytes ConPTY sent within 10 s were `ESC [ 6 n` (DSR). After the probe wrote `ESC [ 1 ; 1 R`, the marker arrived, exit code 7 was collected and the holder `Exit` frame round-tripped (`conpty_exit`). **Implication for Todos 53/54:** the Windows holder must answer ConPTY's startup DSR `ESC[6n` itself, from the core's VT state or with a holder-side cursor reply, before any output or attach can flow. Otherwise every ConPTY terminal stalls with no output. This is not permission to mirror.
- **`conpty_eof_after_exit`**: after the program exited, the output stream did not reach EOF within 2 s (`{"Err":"timeout"}` in the earlier run). The holder must decide when output after exit is complete without waiting for EOF from ConPTY (final-drain-before-exit, IDEAL-043).
- **`conpty_backend_kill`**: in run 41127802737, `PtySession::kill()` (portable-pty's cloned `ChildKiller` on Windows) failed with `Io { operation: Kill, source: Os { code: 6, message: "The handle is invalid." } }` on a running `ping`. The holder cannot rely on portable-pty's cloned killer handle on Windows and needs its own process/job handle for termination. From this push the smoke records it as a finding instead of panicking.
- **Start tokens**: `observe_process` returns typed `unsupported` on Windows (IDEAL-071 explicit unsupported field); `CreationTime` belongs to Todo 53.

How findings behave: the Windows smoke records each missing capability as one `PROBE finding` line (typed kind, hint, message and OS source chain) plus a GitHub `::error::` annotation, and the test still exits 0 because the finding *is* the measurement. `OVIS_PROBE_REQUIRE_PASS=1` turns every finding into a test failure; Todos 53/54 should run the smoke that way to prove the gap closed. On Windows the probe tests are `#[cfg_attr(windows, ignore = ...)]` so the generic `ci.yml` `test (windows-latest)` job skips them, and `platform-probes.yml` runs them with `--include-ignored`. On Linux/macOS they are not ignored, so the plan's literal QA commands still run them.

### macOS (macos-15 runner, aarch64), native: passes, no findings

From run 41127802737:

- Start token `macos-lstart:Fri Oct 30 21:02:02 2026` (source `macos_ps_lstart`).
- Modes `set {bracketed, alternate}` / `reset {plain, primary}`.
- Socket path limit 103 (first rejected 104), mode `0o600`.
- Detach: `core_death` (`Killed: 9`) and `logout` (`Hangup: 1`), with holder and program tokens unchanged and `live`.
- Latency: 16 PTYs p50 5.80 ms, p99 17.50 ms, max 17.76 ms, `within_verdict: true`.
- Soak (50 PTYs × 3 s): 505 rounds, p99 0.20 ms, max 7.45 ms, 50/50 tokens unchanged.
- Failure side:
  - Stall: `accepted_before_kill: "0"`, SIGHUP ignored, SIGKILL `Killed: 9`; unlike Linux, the blocked writer was **released** with `input/output error`.
  - `openpty` refused under `ulimit -n 4` (`EMFILE`) gives `SpawnFailed{unavailable}`.
  - Missing cwd fell back to `/Users/runner`.
  - Broken endpoint: `NotFound` / `BrokenPipe`.

### Linux (ubuntu-24.04 runner, x86_64), native: passes, no findings

Same shape as the local run. Socket path limit 107; `endpoint_inode_reused: true` on the runner's filesystem (a rebound socket got the same inode, so `(dev, ino)` alone cannot detect socket replacement there). Stall writer still blocked after SIGKILL. Latency p99 3.78 ms; soak 50/50 tokens unchanged.

## Adversarial

| Class | Observable |
|---|---|
| hung commands: no-read child | Stall probe: blocked writer, SIGHUP ignored, SIGKILL collects the exit within the bound; the writer thread stays blocked (measured, reported to Todo 6). |
| repeated interruptions: core death / logout | `probe_detach` runs both: launcher SIGKILL and launcher SIGHUP. Holder and program PID + start token unchanged and `live` after each (asserted twice). |
| misleading success: actual platform process evidence | Survival is decided from the OS process table (`/proc` / `ps`) including zombie state, never from the fixture's own claim. A dead PID reports `process_record_unreadable`. The missing-cwd case asserts the backend's substitute directory is not reported as the requested one. Spawn and openpty failures become typed `spawn_failed`/`unavailable`, never a process record. |
| Not reproduced: EXCL-019, EXCL-024, V-RT-HC-01, V-220 | No probe injects scroll input to read history (EXCL-019). No write waits for a status change; round trips wait only for their own echoed marker (EXCL-024/V-RT-HC-01). Every probe thread is bounded and joined by `finish`/`thread::scope`; the only unjoined thread is the deliberately blocked stall writer, reported as `still_blocked` (V-220). |

## Contract checklist

1. **u64 as decimal strings:** `ByteCount`, `Micros`, `SampleCount`, `Pid` and `SocketIdentity.{device,inode}` serialize through `platform/decimal.rs` as canonical decimal strings. A JSON number, sign, padding, leading zero, empty string or overflow is rejected. Tests: `u64_max_round_trips_exactly_as_a_string` (`"18446744073709551615"`), `numeric_and_non_canonical_forms_are_rejected` and proptest `every_u64_round_trips`. These records are emitted, not accepted on an RPC boundary, so mapping a rejection to `invalid_params` belongs to the server lanes.
2. **Errors:** `ProbeError`, `HarnessError`, and the bins' `FixtureError`/`CliError` are `#[derive(thiserror::Error, Debug)]` with `kind()`/`hint()` returning `&'static str` and `#[source]` on wrapped errors. Display text never repeats the kind token. There are no hand-written `Error` impls.
3. **Newtypes:** `Pid`, `ByteCount`, `Micros`, `SampleCount`, `Grid`, `SocketIdentity`; start tokens use the holder's `ProcessStartToken`. `Option` appears only for real absence (a backend without PID, an unfinished exit, a writer error).
4. **Behavior tests:** assertions compare OS-reported facts (`stty size`, `/proc`/`ps` tokens, exit statuses, typed kinds, decoded holder frames) against what the probe did. proptest covers the u64 encoding.
5. **No suppression or placeholders:** no `#[allow]`, no crate-level `#[expect]`, no TODOs. Scoped item `#[expect]`s: `trivially_copy_pass_by_ref` on the serde `with` serializer, and `missing_const_for_fn` on `observe_process` only for the unsupported-platform stub (`cfg_attr`).
6. **Evidence:** this body states what each test asserts versus what is only recorded. Conformance is deferred with the exit-2 attempt verbatim.

## Out-of-scope edits

- **`clippy.toml`**, still awaiting an explicit orchestrator grant: `allowed-duplicate-crates` gains `bitflags`, `syn`, `thiserror`, `thiserror-impl`. The plan-mandated `portable-pty` 0.9.0 depends on bitflags 1 (Windows), thiserror 1 and syn 2, alongside the workspace's bitflags 2 / thiserror 2 / syn 3. Without the allowance, `clippy::multiple_crate_versions` fails `-D warnings` for every crate reaching portable-pty, and the only other fix would be a forbidden crate-wide `#[expect]`. Todo 7 received the same grant for `windows-sys`.
- **Manifest seams:**
  - Root `Cargo.toml`: pins `portable-pty = "=0.9.0"` and registers `ovis-terminal-probes` / `ovis-terminal-bench` as workspace dependencies.
  - Holder `Cargo.toml`: the probe target names (`--test platform_probe` on this crate, per the QA command) point into `probes/`. It gains a `[[bin]]` fixture and a `[[test]]`, a normal `portable-pty` dependency for the fixture, dev-dependencies on probes, bench and vt, and a `cargo-machete` ignore list, because machete scans only the crate directory.
  - Todo 6 also edits this manifest; a conflict there is a trivial union.
- **`probes/src/lib.rs` / `bench/src/lib.rs`:** module slot lines only.

## Risks

- **Measured blockers handed to the owning backends (not fixed here, not mirrored around):**
  - Todo 6 (Unix holder):
    1. portable-pty's `kill()` is SIGHUP, which a program can ignore; the holder needs SIGKILL escalation.
    2. A writer blocked on a non-reading program stays blocked after the program dies; holder input writes must be non-blocking or cancellable.
    3. A missing `cwd` silently falls back to `$HOME`; the holder must validate cwd before spawn.
    4. In canonical mode, input lines longer than the line-discipline buffer are silently discarded on Linux, not blocked.
    5. On Linux, a write after the program exits is accepted without error ([portable-pty-spike.log](portable-pty-spike.log)).
  - Todos 53/54 (Windows): the ConPTY EOF-after-exit and mode passthrough are *recorded*, not asserted, in the smoke (`conpty_eof_after_exit`, `conpty_modes_observed`).
- **Windows and macOS were not run locally.** They are proven by this PR's CI only (see Platform findings). macOS passes natively in both `ci.yml` and `platform-probes.yml`. Windows is the smoke tier, and its three findings are open blockers for Todos 53/54; no full Windows QA is claimed.
- **`(dev, ino)` socket identity:** the Linux CI runner reused the inode for a rebound socket, so endpoint ownership checks in the server lanes need more than `(dev, ino)` (for example a nonce exchanged over the socket).
- **Harness scope:** the latency harness measures PTY round trips (write → `cat` → read), not the RPC admission/event contract. The 8-client/16-request part of the profile needs the server (Todo 30). The soak's 94.7 ms worst sample on this 128-core host is a capacity observation, not a verdict. Program RSS counts shared library pages per process.
- **Start tokens:** macOS `lstart` has one-second resolution, so a PID reused within the same second would collide. The proc_pidinfo implementation belongs to the Todo 6 backend.
