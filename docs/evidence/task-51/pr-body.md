feat(unix-service): separate unix service lifetime and endpoint discovery (plan todo 51)

Closes GAP-001, GAP-016, GAP-017, GAP-121 on Linux/macOS. Delivers IDEAL-001, IDEAL-014, IDEAL-093, IDEAL-099 on Linux/macOS (Windows: todo 54). Enforces EXCL-027 (status never starts a runtime), EXCL-001/EXCL-004 (no presentation or foreground side effects: every verb is a plain JSON command). It does not reproduce V-RT-HC-04 (status auto-start), V-4075 (env-driven timing silently defaulted: every bound is a constant, every path variable is explicit and an invalid one is a typed error) or V-4076 (detached endless threads: manager-call readers are joined, the holder service joins its threads).

## Before

- `docs/evidence/baseline/hc-s18-stop-cleanup.log` (source `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s18-stop-cleanup.log:11`, RT-HC-04): `hc server status` after `hc server stop` printed `"pid":2678504,"uptime_ms":9`: inspection started a new core.
- Gap baseline: GAP-001 (no endpoint discovery), GAP-016 (status starts a runtime), GAP-017 (dead daemon vs stale socket indistinguishable), GAP-121 (holders do not outlive SSH logout/the client).

## After

Host: Linux 7.0.0-38-generic x86_64 (Ubuntu, systemd 259), isolated toolchain 1.99.1. Binary `target/debug/ovis-terminal` sha256 `da46d3ea976417b7f9e289e0402a6ee2dc81d92a24914bf27099fdc531835b5f` (recorded in the evidence JSON).

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/unix-service --isolated --case happy --evidence "$ATTEMPT_DIR/task-51-ovis-terminal-v1.json"
{"case":"happy","error":null,"result":"passed","scenario":"platform/unix-service","totals":{"assertions":"32","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ ... --case all ...      -> "assertions":"48","cases":"2","failed":"0","skipped":"0"   exit 0
$ ... --case failure ...  -> "assertions":"16","cases":"1","failed":"0","skipped":"0"   exit 0
```

Materialized binary commands of the happy case (each a separate process in a fresh isolated root, `HOME`=`XDG_RUNTIME_DIR`=root, env otherwise cleared): `ovis-terminal --json service status` -> `state:stopped`, `missing_prerequisites:["service_manager"]`; `service start --manager detached` -> `state:running` with a record (`format ovis-terminal-holder-endpoint/1`, `transport unix`, `manager null`, `boot_proof {pid,start_token}`), after which that invocation exits; two later `service status` invocations -> `state:running`; `server status` -> `state:stopped`, `holder_count:0` (the control server is separate and inspecting it starts nothing); `service stop` -> receipt `ended_holder {pid,start_token}`, `ended_terminals []`; `service status` -> `stopped`. Failure case: two statuses `stopped`, `service stop` exit 11 `already_stopped` (inspection started nothing), `--session ../escape` exit 2 `invalid_session`, `service start` without `--manager` exit 2 `usage`, `server status` `stopped`.

Same-PID/start-token across reconnect, the stale descriptor and a real systemd user unit, with the real binary (`.omo/.../manual-qa.log`, copied to `docs/evidence/task-51/manual-qa.log`):

```
$ ovis-terminal --json service install --manager systemd-user --session t51qa74428     exit=0  (unit with X-OvisTerminalLabel marker, Type=notify, Restart=no)
$ ovis-terminal --json service start --manager systemd-user --session t51qa74428       exit=0  state:running pid 2656928
systemctl --user show: ActiveState=active MainPID=2656928 ControlGroup=.../app.slice/ovis-terminal-holder-t51qa74428.service
$ ovis-terminal --json service status --session t51qa74428                             exit=0  state:running pid 2656928, same start_token
$ ovis-terminal --json service stop --session t51qa74428                               exit=0  ended_holder pid 2656928 ; ActiveState=inactive
$ ovis-terminal --json service uninstall --manager systemd-user --session t51qa74428   exit=0  ; 0 matching files left in ~/.config/systemd/user
# stale descriptor: detached holder started by this run, then kill -9 of that pid (2657112 > 1, owned)
$ ovis-terminal --json service status   (x3)  exit=0  {"state":"stale_endpoint","reason":"process_gone","record":{... "pid":2657112 ...}}
$ ovis-terminal --json service stop            exit=4  {"error":{"kind":"stale_endpoint","reason":"process_gone",...}}
record + socket left in place (evidence, not repaired); no holder process started by status/stop
$ ovis-terminal --json service start --manager detached   exit=0  new pid 2657175 replaces the proven-dead endpoint; stop exit=0
```

### DEFERRED VERIFICATION (runner lands in Todo 21)

The runner from todo 21 has no `--inject` or `--append` flag and no `stale-descriptor` fixture (the runner is outside this todo's write scope). The plan's failure command was run verbatim and exited 2:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario platform/unix-service --isolated --case failure --inject '{"method":"server.status","params":{"endpoint":"stale-descriptor"}}' --evidence "$ATTEMPT_DIR/task-51-ovis-terminal-v1.json" --append
Usage: conformance --binary <BINARY> --scenario <SCENARIO> --isolated --case <CASE>
exit 2
```

`stale_endpoint` with no new process or registration is therefore proven by the manual QA above (real binary) and by the tests `observe::tests::a_record_naming_a_reaped_or_replaced_process_is_stale_and_is_left_in_place`, `observe::tests::an_unrecorded_socket_nobody_accepts_on_is_stale_and_a_plain_file_is_not_a_socket` and `lifecycle_tests::a_stale_endpoint_refuses_stop_and_is_replaced_only_by_an_explicit_start`, not by the runner. The scenario's failure case runs without injection.

macOS: the `probe-stress-macos` workflow gained a `unix-service` option/job (10x holder service tests + scenario, then a live launchd agent with a private label: install/start/status/stop/uninstall). Run URL: see the comment below / DoneClaim. The launchd path is otherwise covered only by rendering/ownership unit tests on Linux.

## QA

- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` exit 0.
- `cargo test --locked -p ovis-terminal-holder -p ovis-terminal-cli -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` exit 0.
- `cargo clippy --locked -p ovis-terminal-holder -p ovis-terminal-cli -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings` exit 0.
- `bash scripts/check.sh` exit 0 (fmt, clippy, nextest 579 passed / 1 skipped pre-existing, doctests, hygiene, manifests, json).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0.

New tests and what they assert:
- protocol `domain::unix_service::tests` (5): record round trip (proptest over pid/token/address); another format -> `descriptor_unsupported`, extra field -> `descriptor_malformed`; `Observed` tag equals the `ServerState` token and round-trips; incarnation `u64::MAX` is `"18446744073709551615"` and a numeric incarnation is refused; prerequisite tokens equal their serde form.
- holder `service::unix` (16): paths follow explicit dirs in order with no `/tmp` fallback, relative dirs refused; session grammar and 103-byte socket limit; record is 0600, replaced whole, withdrawn only by its owner, no staging leftovers; corrupt record is an error never read as absent; empty root is `stopped` 3x and observing creates no file; reaped pid -> `process_gone`, reused pid -> `process_replaced`, record left in place; live pid is `running` only while the socket accepts; unrecorded refused socket -> `socket_refused`, plain file -> `not_a_socket`; unit quoting/escaping, `Type=notify`, `Restart=no`, owner marker, newline path refused; plist XML escaping, `KeepAlive false`, owner marker; hung manager (`/bin/sleep 30`, 200 ms bound) -> `manager_timeout` after kill+reap; exit status and diagnostics reported; detached placement always lacks `service_manager`, other-OS manager -> `service_unsupported`; a foreign definition is neither reported nor removed; `READY=1` datagram; run/observe/stop lifecycle (same record across two observes, second `run` -> `already_running`, receipt names the holder, stop of a stopped service -> `already_stopped`, nothing left in the directory); stale endpoint refuses stop, explicit start replaces it.
- server `dispatch::lanes::unix_service` (2): only definitions this runtime wrote are described (endpoint address, no boot id), describing creates nothing; the lane registers once against the published `service.describe` catalog entry and a duplicate is `method_not_found`.

## Adversarial

- stale state: stale socket (unrecorded, refused), non-socket file, record naming a reaped pid, record naming a reused pid -> all `stale_endpoint` with a reason, never repaired by status or stop (tests above + manual QA kill -9).
- repeated interruptions / service reconnect: starter exits, then repeated status invocations re-observe the same pid/start_token (scenario happy + manual QA); a second `run` against a live holder refuses; concurrent starts are serialized under the session start lock (round 2, below).
- hung commands: every manager call is bounded (5 s queries, 20 s start) and an expired call kills only its own child (`manager_timeout`, exit 13); the observation's connect is non-blocking with a 500 ms bound.
- host safety: no test or command signals pid 0/1/-1 or a group; the only kill in manual QA targeted the holder pid this run started (asserted > 1).

## Contract checklist

1. u64 on the wire: incarnations in stop receipts are decimal strings (`u64::MAX` round-trip + numeric refusal test). PIDs reuse the protocol's `ProcessPid` (u32). Holder wire generation stays 2; no holder message changed.
2. Errors: `ServiceError` (holder) and `DiscoveryError` (protocol) are `thiserror` enums with `kind()`/`hint()` and `#[source]`; Display never repeats the kind; CLI wraps via `#[error(transparent)]`.
3. Identities: reuse `ServiceLabel`, `ProcessPid`, `StartToken`, `TerminalId`, `Incarnation`, `ServerState`, `ServiceManager`; `Option` only for real absence (no record, detached = no manager).
4. Tests assert behavior and typed kinds; proptest for the record round trip.
5. No `#[allow]`/`#[expect]` added, no TODO placeholders.
6. Claims above are limited to what the listed tests and runs show.

## Seams

- `ovis-terminal-protocol`: new public `domain::unix_service` (records) plus inherent constructors `ServiceRecord::registration/with_missing`, `ServiceDescribeResult::new` (impls live in the domain module; `schema/methods/service.rs` unchanged).
- `ovis-terminal-holder`: new public `service::unix` module; rustix features `event`, `fs`, `net` added.
- `ovis-terminal-cli`: depends on `ovis-terminal-holder`; new public `service::unix` module (needed to satisfy `redundant_pub_crate` + `unreachable_pub`); slot edits in `invocation/{args,plan,error,report,exit}.rs` and `invocation.rs` (`Command::Service`, `Plan::Service`, `CliError::Service`, typed `state/reason/missing_prerequisite` error details); exit-code table gains `unsupported_path` (2), `not_registered` (3), `service_manager_unavailable`/`service_unsupported` (4), `foreign_registration`/`identity_mismatch` (11), `manager_timeout`/`holder_not_ready` (13); `skill.md` documents the verbs.
- `ovis-terminal-server`: depends on `ovis-terminal-holder`; `dispatch::lanes` module with `unix_service::ServiceDescribe`; not registered by any served binary yet (todo 31 assembles it).
- Outside the listed write scope: `conformance/scenarios/platform/unix-service.json` (the runner grammar requires `<family>/<name>.json`, so a `unix-service/` directory cannot be a scenario id) and one entry in `conformance/src/runner_tests/suite.rs`'s pinned `EXECUTABLE` list (required for any new executable scenario); `.github/workflows/probe-stress-macos.yml` option + job appended at the end.

## Risks

- `service run` is the holder entry the managers execute; todo 31 owns the real `serve`/`hold` commands and may rename or absorb it.
- "Stop control only" is not demonstrated: no control-server process exists yet (todo 31). Here `server status` stays `stopped` while the holder runs, and `service stop` (holder stop) is a distinct verb whose receipt enumerates the ended holder and terminals.
- A holder ended by `systemctl --user stop` (SIGTERM) or a crash leaves its record: status reports `stale_endpoint` until an explicit start. No SIGTERM handler withdraws it.
- launchd uses the `gui/<uid>` domain; over SSH without a GUI login it is reported as missing `launchd_user_domain`. Live launchd behaviour is proven only by the macOS workflow job.
- If a scenario step fails mid-case, a detached holder started by that case outlives the removed isolated root (the runner only cleans processes it spawned).
- Readiness waits for launchd start and for record withdrawal after `Goodbye` are bounded polls (20 ms/10 ms) in production code; systemd uses `READY=1` and the detached launch a ready line.

## Round 2 (gate review: B1 start race, B2 untyped loser, N1 identity check)

- **B1 fix:** `run` now takes an exclusive `flock` on `<state dir>/holder-S.lock` (0600, in the 0700 directory, never removed) around claim -> stale cleanup -> bind -> serve -> record publish (`service/unix/lock.rs`, `run::publish`). Record withdrawal takes the same lock. A waiting starter observes the winner's record and refuses.
- **B2 fix:** `ServiceError::AlreadyRunning { holder: Option<Box<BootProof>> }`. The CLI error record carries `"holder":{"pid","start_token"}`. The detached launcher runs the child with `--json`, parses its stderr refusal and returns the typed `already_running` (exit 11) instead of `manager_failed`. `already_running` was already in the protocol error catalog (`schema/error_kinds.rs` `AlreadyRunning`) and the exit table (conflict, 11), so no catalog change was needed. `service start` post-check no longer returns `NotReady{bound_ms:0}`: it maps the observed state to `stale_endpoint` / `already_stopped` / `unavailable`.
- **New tests:**
  - `start_race_tests::concurrent_starts_over_a_stale_endpoint_leave_exactly_one_holder_and_type_the_loser`: 50 iterations. Each one leaves crash leftovers (a stale socket file plus a record naming a reaped pid), releases two `run` calls through a barrier and awaits results over channels (no sleeps). It asserts exactly one ready announcement, a loser `AlreadyRunning{holder: Some(winner proof)}`, status `running` with the winner's record, a stop receipt naming the winner, and then `stopped`.
  - `launch::tests::a_losing_holder_refusal_becomes_typed_already_running_with_the_winner_proof`.
  - `lifecycle_tests::stop_refuses_a_holder_that_is_not_the_recorded_incarnation_and_leaves_it_serving` (N1): a live, recorded process answers with a different holder instance, stop returns `identity_mismatch`, and that holder keeps serving.
- **Mutation check:**
  - Removing the lock from `publish` makes the race test fail (two holders / untyped loser).
  - Neutralising the `Welcome.instance` check makes the N1 test fail.
  - Both were restored afterwards.
- **Real-binary probe** (`docs/evidence/task-51/race-probe-r2.log`, Linux x86_64): 50 iterations of kill -9 on this run's own detached holder (pid asserted > 1), then two concurrent `ovis-terminal --json service start --manager detached`.
  - Result: `iterations=50 multi_holder=0 untyped_loser=0 status_mismatch=0 leftover_holders=0`.
  - Loser sample: `{"error":{"exit_code":11,...,"holder":{"pid":2472151,"start_token":"linux:...:32233960"},"kind":"already_running",...}}`.
  - Status names the same pid.
- **Pushed on top of 82dbad5** with no merge of the moved origin/integration (99e5c52), per instructions.
