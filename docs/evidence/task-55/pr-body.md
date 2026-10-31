Plan todo 55 - report process incarnations consistently across operating systems.
Closes GAP-095, GAP-096. Delivers IDEAL-045, IDEAL-071, IDEAL-105. Enforces EXCL-006, EXCL-008, EXCL-010, EXCL-026 (no V-004 / V-RT-HD-02 / V-RT-HC-03 reproduction).

## Before

`docs/evidence/baseline/smoke/attempt2-herdr-h05-process-info-trap.log:18` (RT-HD-02, V-RT-HD-02): herdr `pane.process_info {target}` with a misnamed target field answers for the focused pane:

```
TRAP CHECK: pane_id=B foreground={"pane_id":"w1:p2",...} | target=B foreground={"pane_id":"w2:p1",...}
```

Gap baseline: GAP-095 (no start tokens, no launcher/foreground distinction; Windows reports only the shell) and GAP-096 (agent identity stays bound after exit or replacement). Before this PR, ovis-terminal's process roles were Linux-only (`runtime/inspection/linux.rs`), macOS start tokens came from `ps -o lstart=` (one-second resolution), Windows had no `read_start_token`, and `terminal.process_info` had no route.

## After

Host: `Linux 7.0.0-38-generic x86_64` (macOS evidence: stress workflow below). Binary `target/debug/ovis-terminal` sha256 `65990af102b64cbbd3a6c482166ad3e2c1c43d7ed50edf87376cb67992291c8b` (recorded in the evidence file).

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/process-info --isolated --case happy --evidence "$ATTEMPT_DIR/task-55-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"platform/process-info","totals":{"assertions":"62","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
EXIT 0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/process-info --isolated --case failure --inject '{"method":"terminal.process_info","params":{"pane_typo":"p2"}}' --evidence "$ATTEMPT_DIR/task-55-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"platform/process-info","totals":{"assertions":"19","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
EXIT 0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/process-info --isolated --case all --evidence "$ATTEMPT_DIR/task-55-ovis-terminal-v1.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"83","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
EXIT 0
```

Materialized requests: every request goes through `ovis-terminal connect` (the bridge) to the fixture server (real server crate, dispatcher, SQLite store, authority) whose `terminal.process_info` route is `lanes::process_info::ProcessInfoRead` over the real core lifecycle service (`Terminals`) and an in-process holder PTY running:

```
/bin/sh -c "rm -f guard; mkfifo guard; exec 3<>guard; set -m; /bin/cat <guard 3>&- & /bin/sh -c 'printf FG_READY; read gate' 3>&-"
```

Failure case, literal wire (`docs/evidence/task-55/task-55-ovis-terminal-v1.json`):

```
sent     {"id":"inject-0","jsonrpc":"2.0","method":"terminal.process_info","params":{"pane_typo":"p2"}}
received {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32602,"message":"method parameters violate the closed schema","data":{"kind":"invalid_params","hint":"correct the method parameters","retryable":false,"field_path":"params.pane_typo"}}}
```

No `result` is present (asserted `"/result": {"$present": false}`): there is no focus fallback. In the same case, after a real stop + relaunch of the program (`fixture.replace_process` -> `exit_observed: true`, `replaced: "1"`):

```
terminal.process_info {"target":{"name":"proc"},"expect":{"process_incarnation":"1"}}
  -> {"code":-32000,"data":{"kind":"process_replaced","hint":"read terminal.process_info without expect; never redirect to the new program","retryable":false}}
status.report with the old token fixture-process-token-proc-1
  -> {"code":-32000,"data":{"kind":"not_owner",...}}
```

Happy case observable: `roles.launcher` is the outer shell (pid 699278, `linux:<boot>:33550229`), `roles.foreground` is exactly one process with argv `["/bin/sh","-c","printf FG_READY; read gate"]`, `roles.background` is exactly two processes (the launcher shell and the job-control child), `roles.shell` is `{"state":"unsupported","reason":"undeclared_role"}`, top-level `foreground` is `null` (the launcher does not hold the foreground) and `supported_fields` is `["launcher","foreground","background","pid","start_token","argv","cwd"]`. After replacement the launcher is pid 699292 with start token `...:33550247`, `incarnation` `"2"`, and a report with the new incarnation's token is accepted (`process_incarnation: "2"`).

Other acceptance commands (logs in `docs/evidence/task-55/`):

| Command | Exit |
|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 |
| `cargo test -p ovis-terminal-holder -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` | 0 (560 passed, 0 failed) |
| `cargo clippy -p ovis-terminal-holder -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings` | 0 |
| `cargo test --locked --manifest-path conformance/Cargo.toml --test runner` | 0 (9 passed; suite inventory lists `platform/process-info.json` as executable) |
| `bash scripts/check.sh` | 0 (nextest 822 passed, 1 skipped; doc tests, hygiene, manifests, json) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 |
| `RUSTFLAGS=-Dwarnings cargo check -p ovis-terminal-holder -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-conformance --all-targets --target aarch64-apple-darwin --locked` | 0 |

Conformance execution is NOT deferred: the todo-21 runner has landed and `platform/process-info` is an executable `ovis-conformance/1` scenario (no `declared.json` entry).

## What changed

- `crates/ovis-terminal-holder/src/process/`: one OS backend per platform behind `read_start_token` and a new process-table API (`read_entry`, `list_pids`, `read_argv`, `read_cwd`, `ProcessEntry`, `Membership`, `ProbeError`). Linux: `/proc/<pid>/stat` starttime + boot id (unchanged token form). macOS: `proc_pidinfo(PROC_PIDTBSDINFO)` microsecond start time (`darwin:<sec>.<usec>`), pgid, `e_tdev`, `e_tpgid`, zombie state; `proc_listallpids`; `KERN_PROCARGS2`; `PROC_PIDVNODEPATHINFO`. Windows: `OpenProcess` + `GetProcessTimes` CreationTime (`windows-created:<FILETIME>`, the exact form the ConPTY holder records, now produced by one shared function) + `GetExitCodeProcess`; terminal membership, enumeration, argv and cwd are reported unsupported.
- `crates/ovis-terminal-core/src/terminals/process_info/`: `ProcessTable` seam (`NativeTable` or a scripted table), `liveness` (`Current` / `Replaced` / `Exited` / `Unknown(reason)`), `observe` (one bounded pass: launcher re-read before and after, start token re-read per sample, a foreground change mid-pass is `raced`), `ProcessInfo::roles()` wire conversion. The core lifecycle service's `process_info` / `bind_agent` / `check_agent` now use it on every OS (macOS previously reported only the launcher).
- `crates/ovis-terminal-protocol/src/domain/process_info.rs`: `Observed<T>` (`{"state":"known","value":..}` / `{"state":"unsupported","reason":..}`), `UnsupportedReason`, `ProcessSample`, `ProcessRoles`, `supported_fields`, `TerminalProcessInfoResult::new`.
- `crates/ovis-terminal-server/src/dispatch/lanes/process_info.rs`: `ProcessInfoRead` (an unrecorded Read route), `ProcessDirectory` (`launched`, `ended`), `ProcessInfoError` (`stale_target`, `process_replaced`, target errors). Proven `Replaced`/`Exited` calls `ProcessDirectory::ended` (ownership revocation) and replies `process_replaced`; `Unknown` is in-band unsupported fields, never an exit.
- `conformance/scenarios/platform/process-info.json` + fixture `conformance/src/fixture/processes{,/directory}.rs`.
- `.github/workflows/probe-stress-macos.yml`: option `process-info` and job `process-info` appended at the end.

## QA

- happy / failure / all: commands and outputs above (real PTY, real processes, real kill + relaunch through the core lifecycle service).
- No leftover processes after fixture runs (`ps` shows no `FG_READY` / guard `cat` processes; the fixture's stop runs `program_stop`, closes the lifecycle service and shuts the holder down).
- macOS native stress: see the run linked in a comment below (`gh workflow run probe-stress-macos.yml --ref lane/t55-darwin-contract -f probe=process-info`).

## Adversarial

- Stale state, deterministic PID reuse: `core ... process_info::tests::a_reused_pid_with_another_start_token_is_replacement_never_the_owned_process` and `server ... lanes::process_info::tests::a_reused_pid_is_process_replaced_and_revokes_the_old_incarnation` (same PID, different start token -> `process_replaced`, `ended` called exactly once with `Liveness::Replaced` and incarnation 1); proptest `only_a_different_start_token_on_a_running_pid_is_replacement`.
- Stale state, real replacement: the conformance happy/failure cases (kill + recover; old `expect.process_incarnation` -> `process_replaced`; old token -> `not_owner`; new token accepted).
- Misleading success, unsupported foreground is not exit: `unsupported_membership_reports_the_launcher_and_never_an_exit` (core) and `an_unsupported_foreground_answers_without_revoking_or_reporting_exit` (server): `Liveness::Current`, foreground/background `unsupported: platform`, no `ended` call, `supported_fields` omits them; `an_unreadable_launcher_is_unknown_and_every_role_says_why` (permission -> `Unknown`, not exit).
- Wrong target: `a_misnamed_target_field_is_invalid_params_at_that_field` (protocol, `params.pane_typo`), `an_unknown_name_is_not_found_and_never_another_terminal` (server), and the failure case's `not_found` injection for `{"target":{"name":"focused"}}`.
- Race: `a_foreground_change_during_the_pass_is_reported_as_raced_not_as_a_role`.
- Exit without reap: `holder ... an_exited_unreaped_child_keeps_its_token_but_is_not_running` (waitid `WNOWAIT`, then the record says `running: false` or `process_gone`).
- Host safety: no test sends a signal; the fixture script uses a fifo rather than any `kill`, and the only signals are the core lifecycle service's incarnation-checked stop (`KillRequest::immediate`) of the PTY program it launched itself (PID > 1, reserved PIDs refused by `SignalError::ReservedPid`).

## Contract checklist

1. Decimal strings: `expect.process_incarnation` / `terminal_incarnation` and every incarnation are canonical decimal strings; numeric form is refused (`expected_incarnations_are_exact_decimal_strings`, includes the u64::MAX round trip). `pid` stays the existing `ProcessPid` schema type (a u32 JSON number, the same as `ProcessRecord.pid`); it is a 32-bit OS identity, not a u64 counter.
2. Errors: `ProbeError` (holder) and `ProcessInfoError` (server) are `thiserror` enums with `kind()` / `hint()`; `#[source]` on inner errors; Display never repeats the kind token. `StartTokenError` is unchanged.
3. Identities: `ProcessPid`, `ProcessStartToken`, `ProcessHandle`, `Incarnation`, `TerminalGeneration` newtypes; `Option` only for "holder recorded an exit" (`Launched::owned`).
4. Tests assert behavior (liveness transitions, revocation calls, wire shapes, typed kinds); proptest round trip of every `ProcessRoles` shape (protocol) and of the liveness rule (core).
5. No `#[allow]`, no new `#[expect]`, no placeholder TODOs.
6. Holder wire generation stays 2; no holder protocol message changed.

## Seams (public API changes)

- protocol: `terminal.process_info` params are now `TerminalProcessInfoParams { target, expect: Option<ProcessExpect> }` (was `TerminalGetParams`; an added optional input); `TerminalProcessInfoResult` gains `roles: ProcessRoles` and its `foreground` doc now says it repeats the launcher only while the launcher holds the foreground; method errors add `ProcessReplaced`; new `domain::process_info`. `schema/ovis-terminal.openrpc.json` regenerated; `openrpc compat schema/released.openrpc.json` -> compatible.
- holder: `process` is now compiled on every OS (was `cfg(unix)` except `ProcessId`); new `table` API; macOS start tokens change from `lstart:<date>` to `darwin:<sec>.<usec>`; `windows/process.rs` delegates its token to the shared `process::creation_token` (same string).
- core: new `pub mod terminals::process_info`; `runtime/inspection/linux.rs` removed, `proof.rs` rewired (outside the literal write scope, required to make the lifecycle service use the cross-platform backend).
- server: new `pub mod dispatch::lanes::process_info`.
- conformance (outside the literal write scope, fixture seams): `fixture/processes{,/directory}.rs`, the never-advertised `fixture.replace_process` stand-in for `terminal.kill` + `terminal.recover` (todo 31 serves the real ones), `agents.rs` TERMINALS gains `proc` (terminal 1, token `fixture-process-token-proc-1`), `handler.rs`/`serve.rs` wiring, `conformance/Cargo.toml` adds `ovis-terminal-holder`, suite `EXECUTABLE` 13 -> 14.

## Risks

- origin/integration moved after this lane's base (00aeae4 -> b1c68ef: todos 23 and 78). Overlapping files: `Cargo.lock`, `conformance/src/runner_tests/suite.rs`, `schema/ovis-terminal.openrpc.json`. Per the lane's hard process rule this branch does not merge; the orchestrator's union merge must regenerate the OpenRPC document and union the `EXECUTABLE` list.
- The agent crate's `ProcessTokens` has no retire-on-exit API; the fixture's `ended` hook retires a token by binding an unguessable one to the ended incarnation. Assembly (todos 31/52) should wire `ProcessDirectory::ended` to a real retire plus `StatusService::exited`.
- Windows reports foreground/background/argv/cwd as `unsupported: platform` (ConPTY has no process groups); it proves the launcher by CreationTime only.
- Persisted macOS start tokens from an older build (`lstart:`) no longer match and would read as `process_replaced` after upgrade (pre-release).
- Core's Linux-only lifecycle QA tests (`terminals/qa/administration.rs`) were not widened to macOS here (outside the write scope).
