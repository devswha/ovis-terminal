feat(recovery): recover holders checkpoints and honest reboot metadata (plan todo 49)

Closes GAP-121, GAP-123, GAP-124, GAP-125. Delivers IDEAL-093, IDEAL-095, IDEAL-096, IDEAL-097, IDEAL-117. Enforces EXCL-019, EXCL-021, EXCL-026, EXCL-027 and does not reproduce V-4077, V-4084, V-RT-HD-05.

## Before

`docs/evidence/baseline/smoke/attempt2-herdr-h03-workspace.log:5` (RT-HD-05, source `/tmp/ulw-ovis-yCJzPH/evidence/smoke/attempt2-herdr-h03-workspace.log:5`):

```
workspaces before: 1
```

Workspace w1 from an earlier herdr run reappeared after a server restart, but the restored metadata carries no process-survival verdict. Gap baseline: GAP-121, GAP-123, GAP-124, GAP-125.

## What changed

- `crates/ovis-terminal-protocol/src/domain/recovery.rs`: `Survival` (`survived | restored_metadata | relaunched | lost{reason} | unknown{reason}`), `ScreenRecovery` (checkpoint offset, replayed range, `gaps[]`, `exact`), `TerminalRecovery`, `RecoveryIncident`/`IncidentKind`/`IncidentId`, `CrashRecovery` (cause is `None` when the boot evidence is inconclusive). `CrashRecovery::evidence` lists only survivors and relaunched incarnations as `restored`. All offsets and times are decimal strings.
- `crates/ovis-terminal-core/src/recovery/`:
  - `boot`: independent OS boot identity (Linux `/proc/sys/kernel/random/boot_id`, macOS `kern.boottime`, otherwise unreadable) compared with a durable boot stamp. A missing stamp or unreadable identity is `Inconclusive`, never `Same`.
  - `verdict`: pure survival verdict from the boot check, the holder group's welcome (same terminal and process incarnation, PID, start token, no retained exit), and the OS start token for the PID. PIDs 0 and 1 are never probed.
  - `groups`: `Placement::{Shared, Dedicated}` and `GroupId`. Dedicated is a documented internal test option, never a v1 schema field. Each group is adopted over its own socket (`adopt::Adopted`, crash hello), so an unreachable group affects only its own terminals.
  - `checkpoint`: durable checkpoint files with a header binding terminal incarnation, offset and SHA-256. Writes are atomic (staged, fsync, rename, directory sync) and an older offset is refused (`checkpoint_stale`). Damaged or other-format files are moved to `quarantine/` and reported, never imported.
  - `replay`: `restore` = checkpoint (both buffers, cursor, modes and pending parser/UTF-8 bytes, all owned by the VT crate) plus the holder's retained tail. Queries at or before the answered fence are not re-answered, and OSC records at or before the committed fence are not re-published. Bytes nobody retains become a `gap` and the result is `exact: false`. `Splice` admits live output after the replay exactly once.
  - `journal`: append-only, fsynced incident journal. `list` never clears; a damaged line is `journal_corrupt` and the file is left unchanged. `Finding::of_store` records the store's own DB/WAL quarantine location (todo 8 keeps the material).
  - `recover`: one unplanned boot. Order: boot evidence, incident, adopt groups only on the same boot, verdicts, rebuild survivor screens, then record the new boot stamp last. It never relaunches or signals a program.
- `crates/ovis-terminal-server/src/dispatch/lanes/recovery.rs`: `RecoveryLane`.
  - `admit_input` returns `recovering` for unknown survival.
  - `admit_relaunch` allows only a proven loss or restored metadata; otherwise it returns `recovering` / `not_lost` / `not_found`.
  - `evidence` feeds the `server.metrics` recovery list, and `incidents` reads the journal without clearing it.
- `conformance/scenarios/recovery/crash.json` (declared) plus its `declared.json` entry.
- macOS stress: the `recovery` option and job are appended at the end of `.github/workflows/probe-stress-macos.yml` (26 jobs / 26 options, YAML parses) in commit aff08e3, kept on branch `lane/t49-stress`. Integration has since added other lanes' jobs to the same lists, which made this PR CONFLICTING. Because lanes never run `git merge`, commit 7e9a584 restores the merge-base workflow file here so CI can run, and the orchestrator unions the `recovery` block from aff08e3.

## After

Host: Linux x86_64 (`uname -sm`). Binary `target/debug/ovis-terminal` sha256 `4cba9be7b6e618bdc4dc4eabc91586b954bf432c40a639b97d06f07f39cb97f8`.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
exit=0
$ bash scripts/check.sh
     Summary [ 136.555s] 986 tests run: 986 passed (1 slow), 1 skipped
check.sh: all steps passed            (exit 0)
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked
exit=0
$ cargo clippy -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence .../task-49-all.json
{"result":"passed","totals":{"assertions":"1169","cases":"32","declared_only":"36","failed":"0","passed":"32","skipped":"0"}}   (exit 0)
```

Lane tests:
- `cargo test -p ovis-terminal-core --lib recovery::`: 19 passed. Run 20 times in a row with 0 failures (`docs/evidence/task-49/recovery-loop-20x.log`).
- `cargo test -p ovis-terminal-protocol --lib domain::recovery`: 6 passed.
- `cargo test -p ovis-terminal-server --lib lanes::recovery`: 2 passed.

Note: `check.sh` ran before the last edit to `recovery/qa/exact.rs`, which strengthened the fence test after mutation probing. That file was then rechecked with `cargo +nightly fmt`, `cargo clippy -p ovis-terminal-core --all-targets -D warnings` (exit 0) and the 20× loop.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The todo-21 runner exists, but `recovery/crash` is declared-only (no format marker), following the todo-48 `recovery/restart` precedent. No served binary runs recovery at an unplanned boot, registers `terminal.run`, or holds a holder-owned program until todo 31 (assembly), and todo 52 owns the real-endpoint fault injection (core kill, dedicated holder loss, reboot metadata). Attempts recorded verbatim (`docs/evidence/task-49/conformance-attempts.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario recovery/crash --isolated --case happy --evidence "$ATTEMPT_DIR/task-49-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"recovery/crash is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"recovery/crash","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario recovery/crash --isolated --case failure --inject '{"method":"terminal.run","params":{"isolation":"dedicated"}}' --evidence "$ATTEMPT_DIR/task-49-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"recovery/crash is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable","step":null},...,"result":"failed",...}
exit=1
```

The acceptance criterion "exit 0 with failed:0, skipped:0" for `recovery/crash` is therefore **not met** in this PR. These tests cover the same observables in the meantime:
- **Happy:** `recovery::qa::crash::a_core_killed_mid_output_is_succeeded_by_an_exact_screen_without_duplicate_bytes`. A real `HolderService` on a Unix socket runs a writer. The core checkpoints at the signalled `MARK` boundary and is dropped without a handover while the writer keeps printing. A successor adopts the holder with a crash hello, and the test asserts `survived`, `exact`, no gaps, the checkpoint offset, the same PID and start token, and a `core_crash` incident. Live output is spliced on through `DONE`, and the screen lines, cursor and exported checkpoint equal an independent VT fed the holder's whole retained stream.
- **Reboot metadata:** `recovery::qa::reboot::a_changed_boot_identity_lists_restored_metadata_and_adopts_nothing` asserts `restored_metadata` with no holder adopted even though one answers, `host_reboot`, and an empty `restored` evidence list.
- **Failure, isolation:** `domain::recovery::tests::dedicated_isolation_is_not_a_terminal_run_field_and_is_refused_at_its_path` decodes `terminal.run` with `"isolation":"dedicated"` through the protocol's real `decode_params`. It asserts `invalid_params@params.isolation` and that the `TerminalRunParams` schema has no `isolation` property.
- **Failure, inconclusive reachability:** `recovery::qa::groups::an_unreachable_dedicated_holder_is_unknown_without_relaunch_and_the_shared_group_survives` asserts that only the dedicated terminal is `unknown{holder_unreachable}` and not relaunchable. The shared group's welcome lists exactly 1 terminal (no duplicate spawn), and both programs still run as recorded. `lanes::recovery` turns that verdict into `recovering` for input and relaunch.

## QA

- Happy: the crash test above, plus `a_damaged_checkpoint_is_quarantined_and_a_repeated_recovery_reaches_the_same_verdict`.
- Failure: the isolation decode test and the unreachable dedicated holder test above.
- macOS stress, first run (aff08e3): https://github.com/devswha/ovis-terminal/actions/runs/41250874516 failed at iteration 1 with `path must be shorter than SUN_LEN`. The Darwin temp directory made the test's holder socket path exceed the 104-byte `sun_path` limit (test-only). Fixed in 353f116 by moving test state and sockets to short `/tmp/t49-*` paths.
- macOS stress, rerun on `lane/t49-stress` (60500ea = 353f116 plus the stress job): https://github.com/devswha/ovis-terminal/actions/runs/41251157995 succeeded with `RESULT 25/25` (25 PASS lines).
- CI on 353f116: `test (windows-latest)` failed with `clippy::unnecessary_wraps` at `recovery/durable.rs:48` (the non-Unix `sync_directory` stub). Fixed in f81c64b by syncing the directory only under `cfg(unix)` at the call site, with no Windows stub. Local `cargo clippy --workspace --all-targets --all-features --target x86_64-pc-windows-gnu -- -D warnings` exits 0. On f81c64b every check passed (check.sh ubuntu, test ubuntu/macos/windows, probes, msrv, deny, rustdoc, machete).
- `crates/ovis-terminal-core/proptest-regressions/recovery/qa/{exact,verdicts}.txt` are the seeds proptest saved while the mutation probes ran. They replay as extra passing cases on the real code.

## Adversarial

- **Repeated interruptions:**
  - Recovery is run twice with the first run's links dropped (a crash after adoption). It reaches the same verdict and the same screen, and the boot stamp is written last.
  - A torn `.staged` checkpoint left by a crash mid-write is never read.
  - The exactness property test lets the crashed core run past its last checkpoint (any cut, any chunk size, 192 cases), and the restored engine equals the uninterrupted one whenever the checkpoint is exact.
- **Stale state / PID reuse:**
  - The verdict property test covers every combination of boot, reachability, held identity, PID (0, 1, 4242) and OS token: `survived` only on the full match, and PIDs 0 and 1 are never probed.
  - A reused PID with another start token is `lost{replaced}`.
  - An older checkpoint offset is refused (`checkpoint_stale`).
- **Malformed input:**
  - A checkpoint with a flipped byte is `checkpoint_corrupted`, kept verbatim in quarantine and not imported.
  - Another header format is `checkpoint_unsupported` and kept.
  - A checkpoint naming another incarnation is `checkpoint_target_mismatch`.
  - A damaged journal line is `journal_corrupt` and the file stays byte-identical.
  - A corrupt `state.db` plus WAL is quarantined by the store with both files kept and recorded as a `store_corrupted` incident.
  - A numeric offset in the wire records is refused.
- **Misleading success:**
  - Inconclusive boot evidence gives `unknown{boot_evidence_unreadable}` with `cause: null` and no adoption.
  - A reboot never yields `survived`, even with a live holder.
  - `evidence.restored` never lists restored metadata, lost or unknown terminals.
  - Tail overflow past the checkpoint is a `gap` with `exact: false` (property test).
- **Mutation probes** (`docs/evidence/task-49/mutation.log`): removing the OSC fence, removing the query fence, reading inconclusive boot as survived, mislabelling the unreachable incident, and accepting a reused PID are each killed by a failing test.
- **Host safety:** no test signals any process. Only `read_start_token` is used, PIDs ≤ 1 are refused before probing, and teardown uses the holder's own shutdown of its own children.

## Contract checklist

1. Every offset and time in `domain::recovery` is a decimal string, and a numeric form is refused. A `u64::MAX` round trip is tested (`offsets_are_decimal_strings_and_a_numeric_offset_is_refused`). The checkpoint header reuses `ByteOffset`.
2. `RecoveryError` (core) and `RecoveryRefusal` (lane) derive `thiserror::Error` with `kind()`/`hint()` and `#[source]` on inner errors. Display never repeats the kind.
3. Distinct newtypes: `OsBootId`, `GroupId`, `IncidentId`; holder `TerminalTarget`/`ProcessStartToken`; protocol `ByteOffset`/`Incarnation`. `Option` is used only for real absence (no checkpoint, inconclusive cause, no quarantine).
4. Tests assert behaviour: real holders and sockets, property tests (exactness, overflow, splice, verdict matrix, record round trip) and typed error kinds.
5. No `#[allow]`/`#[expect]`, no TODOs, no placeholders.
6. The conformance status is stated above as deferred, not passed. Holder wire generation stays 2.

## Seams

- New public modules: `ovis_terminal_protocol::domain::recovery`, `ovis_terminal_core::recovery`, `ovis_terminal_server::dispatch::lanes::recovery` (slot lines in `domain.rs`, `lib.rs`, `lanes.rs`). No landed public API changed.
- Todo 31 must:
  - call `recovery::recover` at an unplanned boot (`Boot::Unplanned` from `restart::Control::open`) with the store's recorded processes, holder groups and fences;
  - install each `Restored` engine as the terminal's screen (`Restored::lag` > 0 means the engine's offset lags the stream after a gap);
  - feed live output through `Splice`;
  - save `Screen::checkpoint()` through `RecoveryPlan::checkpoints()`;
  - put `RecoveryLane::admit_input`/`admit_relaunch` in front of `input.*`/`terminal.recover` and add `evidence` to `server.metrics`.
- Todo 52 makes `recovery/crash` executable.

## Risks

- `recovery/crash` is declared-only until todos 31/52, so there is no real-binary evidence in this PR.
- The macOS boot identity comes from `sysctl -n kern.boottime` text (one-second resolution of the boot time). If it is unreadable, survival is `unknown`, which is safe but blocks automatic adoption. Windows has no independent boot evidence yet (`unknown`).
- After a tail gap the restored engine's offset lags the stream (`lag`). That is reported, but todo 31 must account for it when feeding live output.
- `origin/integration` moved to 07895cf after this branch was cut. `git merge-tree` reports no conflicts, so this lane did not merge it (the orchestrator merges).
