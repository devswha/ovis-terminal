## Round 2 (gate blockers B1-B3)

- **B1, typed restart close (EXCL-016).**
  - `conn::Ending { Shutdown, Restart }` now carries the server's ending to every connection.
  - `ServerHandle::restart()` closes each connection with `$/closing{"reason":"server_restart","incarnations_preserved":true,"hint":...}`. `shutdown()` keeps `server_shutdown`.
  - `TerminalStreams::restarting(op)` revokes input and closes every viewer with `StreamCloseReason::ServerRestart`; attaches then wait `recovering` until `recovered()`. It shares one close path with `recovering`, which keeps closing with `lost`.
  - Tests:
    - `lanes::restart::qa::wire::a_restart_over_the_socket_keeps_the_child_and_the_lifetime_and_changes_the_boot` asserts on the wire `/params/reason == "server_restart"` and `/params/incarnations_preserved == true`, that the connection then ends, and that after reconnect the terminal's process has the same pid, start token and generation.
    - The streams failure flow (`tests/streams.rs` `failure`) gains `planned_restart`: `close.reason == "server_restart"`, an attach in between gets `recovering`, and the successor serves a new `stream_id`.
- **B2, intent persistence proven through `restart()` itself.**
  - `Control::open` now returns `Boot::Planned(receipt)` only when an intent was on disk, else `Boot::Unplanned`.
  - A `#[cfg(test)]` crash point makes `restart()` die right after each barrier (`intent_persisted`, `suspended`, `announced`, `handed_over`, `booted`). `crash_at` then reads back the intent that `restart()` itself wrote and asserts its phase equals the barrier.
  - `a_crash_at_any_barrier_is_finished_by_the_next_open_with_the_same_processes` requires `Boot::Planned`.
  - The mirror `a_control_that_dies_without_an_intent_reopens_unplanned_as_the_core_crash_row_says` requires `Boot::Unplanned` and no intent.
- **B3, holder loss proven.**
  - A lost incarnation carries typed `LossEvidence`: `exited{code,signal}`, `exit_unknown`, `gone`, `replaced{found}` or `not_adopted`. It is never reported preserved.
  - `a_child_killed_during_the_restart_is_lost_with_its_exit_and_never_preserved`:
    - the restart crashes after `handed_over`;
    - the test kills its own child (pid > 1) through `Holder::signal(Kill)` on its exact incarnation;
    - it waits for the holder's reap evidence on a probe link (event-driven, no polling);
    - the successor reports `lost == [exited{signal}]`, `preserved == []` and `ended_incarnations == [gen]`.
  - `a_recorded_identity_that_does_not_match_the_held_process_is_not_adopted_and_is_typed` forges the intent's start token. The result is `replaced{found: actual}`, and the real process runs on untouched.
  - The tautological protocol table tests were removed. `restart::qa::fates` now checks observed identity movement against `identity::fate` for:
    - `control_restart` (a real planned restart);
    - `core_crash` (unplanned reopen under the next boot);
    - `holder_loss` (the holder is shut down and replaced, and the kept terminal recovered).
- **Mutants**, applied by me, in `docs/evidence/task-48/mutants-r2.log`:
  - `skip_intent_marker` (no intent store in `restart()` and `advance`): 5 tests fail.
  - `keep_dead_holder` (verdict forced to preserved, the "if true" adoption): 2 tests fail.
- **Gate:**
  - `bash scripts/check.sh` exit 0: nextest 866 run, 866 passed; hygiene 0 violations in 850 files.
  - `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0.
  - `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0.
- **Seams** (disclosed, outside the literal write scope):
  - `crates/ovis-terminal-server/src/conn/{outbound,connections,driver}.rs` and `conn.rs`: `Ending`, `Connections::end`, and the typed `$/closing`.
  - `listen/server.rs`: `ServerHandle::restart`. Its test gate `listen/server/shutdown_race.rs` gets the watch type.
  - `streams/hub.rs`: `TerminalStreams::restarting`. `streams/qa/failure.rs` gets the new step.
  - `Control::open` returns `(Control, Boot)`.
  - `RestartReceipt.lost` is now `Vec<Lost>`.

## Before

`docs/evidence/baseline/smoke/attempt2-herdr-h03-workspace.log:5` (RT-HD-05): `workspaces before: 1`. herdr's persisted workspace metadata reappears after a server restart, and nothing proves that any program survived. Gap baseline: GAP-065 (a live handoff has no structured relookup signal), GAP-122 (a planned restart cuts waits without structured outcomes), GAP-126 (stop/restart/kill/removal receipts do not enumerate affected processes).

## After

Todo 48 adds:

- `ovis_terminal_core::restart`: `Control` owns the terminal lifecycle actor and replaces it through an explicit state machine: `running -> intent_persisted -> suspended -> announced -> handed_over -> booted -> recovered -> running`.
  - The restart intent, the preserved incarnations, the in-flight operations and the interrupted waits go to a durable intent file (staged write, fsync, rename, directory fsync) before each step.
  - Readiness is withdrawn: work offered inside the boundary gets the typed `restarting` error.
  - Boundaries are announced on bounded listeners; a capacity-0 listener holds the restart at each boundary.
  - Armed waits end `cancelled` with their resume cursor.
  - Holder ownership is handed over through `Terminals::prepare_restart`, which signals no program.
  - The successor adopts the holder under the boot id the intent names, in the lifetime the intent recorded. The lifetime is never taken from an endpoint or a hello reply.
  - Each preserved incarnation is verified by holder record plus a live OS start token. Interrupted operations are classified from the committed ledger and holder write evidence only.
  - A crash at any barrier leaves the intent on disk, and the next `Control::open` finishes it. An intent from another host boot is refused as `host_rebooted`; one from another lifetime as `lifetime_mismatch`.
- `Control::stop(op, scope)`:
  - `control_only` signals nothing and lists the incarnations as preserved.
  - `control_and_programs` lists reaped exits as ended. If any exit stays unproven it returns `outcome_unknown` with both lists, and control stays up.
- `ovis_terminal_protocol::domain::restart`: `RestartPhase`, `RestartBoundary`, `InterruptedOp` / `InterruptedOutcome` / `Delivery`, `InterruptedWait`, constructors for the published `ServerTransitionResult` / `RecoveryEvidence`, and the identity transition table `identity::{Cause, Field, Fate, fate, markdown}`.
- `docs/identity-transitions.md`: the table, generated by `identity::markdown()`. A protocol test fails if the doc copy differs.
- `ovis_terminal_server::dispatch::lanes::restart`: `ServerRestart` (`server.restart`) and `ServerStop` (`server.stop`). Both are `Administer` on the machine. Both refuse `restarting` before any effect.

Native host: `Linux 7.0.0-38-generic x86_64`. Base: origin/integration 81cb6bf.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal        # exit 0
$ cargo clippy -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings   # exit 0
$ cargo test -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets --no-fail-fast      # exit 101, see "Pre-existing failure"
  37 test targets ok; 25 todo-48 tests pass (9 protocol, 14 core, 2 server lane)
$ bash scripts/check.sh steps (fmt, clippy --workspace, nextest --workspace, doc, hygiene, manifests, json)   # every step exit 0
  nextest: 838 tests run: 838 passed (1 slow), 1 skipped
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked             # exit 0
```

The happy observable runs over a real Unix socket with a real dispatcher, SQLite ledger and holder: `dispatch::lanes::restart::qa::wire::a_restart_over_the_socket_keeps_the_child_and_the_lifetime_and_changes_the_boot`.

- A live child (`/bin/sh -c "printf ARMED; read gate"`) and an armed wait exist.
- `server.restart` replies `server_boot_id_old: "boot:1"`, `server_boot_id_new: "boot:2"`, `preserved_incarnations: ["1"]`, `ended_incarnations: []`.
- The wait scope is cancelled.
- The successor listener's `hello` reports the same `runtime_lifetime` and `server_boot_id: "boot:2"`. `listen::continuity` classifies the pair as `ControlRestarted`.
- The same PID runs with the same start token.

The core test `restart::qa::happy::a_live_child_keeps_pid_token_and_lifetime_while_boot_changes_and_a_wait_ends_cancelled` additionally asserts:

- `seq_after > seq_before`;
- the wait ticket resolves `OutputOutcome::Cancelled`, never a match or a timeout;
- boundaries `started` then `completed`, carrying `RecoveryEvidence.restored`;
- the intent file is removed.

Failure observable, `...::an_unknown_stop_scope_is_invalid_params_and_nothing_stops_while_control_only_keeps_programs`:

- `server.stop {"scope":"everything-implicitly"}` and a stop with no scope both reply `invalid_params` at `params.scope`, with no result.
- The child keeps its PID and start token, and control still lists it.
- A `control_only` stop returns `preserved_incarnations: ["1"]`, `ended_incarnations: []` and `server_boot_id_new: null`, and the child still runs.
- A later `server.restart` is refused `control_stopped`.

### DEFERRED VERIFICATION (runner lands in Todo 21; served binary in Todo 31)

`conformance/scenarios/recovery/restart.json` is a declaration listed in `declared.json`, with owner 48 and blocked on 31 and 21. The runner's only fixture cannot hold a holder-owned program, and `protocol/discovery.json` asserts `server.stop` is unserved there. The attempts, verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario recovery/restart --isolated --case happy --evidence "$ATTEMPT_DIR/task-48-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"recovery/restart is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t48/.omo/evidence/ovis-terminal-v1/task-48/task-48-ovis-terminal-v1.json","evidence_written":null,"result":"failed","scenario":"recovery/restart","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
happy exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario recovery/restart --isolated --case failure --inject '{"method":"server.stop","params":{"scope":"everything-implicitly"}}' --evidence "$ATTEMPT_DIR/task-48-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"recovery/restart is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"/home/devswha/workspace/ovis-terminal-wt/lane-t48/.omo/evidence/ovis-terminal-v1/task-48/task-48-ovis-terminal-v1.json","evidence_written":null,"result":"failed","scenario":"recovery/restart","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
failure exit=1
```

The acceptance criterion "conformance exits 0 with failed:0, skipped:0" is **not met** in this PR; it needs the todo-31 assembly.

## QA

- macOS stress: `gh workflow run probe-stress-macos.yml --ref lane/t48-planned-restart -f probe=restart` ran https://github.com/devswha/ovis-terminal/actions/runs/41235872273. Result: success, `RESULT 25/25` on Darwin 25.6.0 arm64 (core `restart::`, server `lanes::restart`, protocol `domain::restart`, each repeated 25 times).
- PR CI: none ran. The PR is `CONFLICTING` against origin/integration b1c68ef in exactly one file, the shared `.github/workflows/probe-stress-macos.yml` (both sides appended an option and a job; union-merge). Per the lane rule, this lane did not merge integration.

- happy: `cargo test -p ovis-terminal-server --lib lanes::restart` and `cargo test -p ovis-terminal-core --lib restart` (real holder PTY, real SQLite, real socket). Exit 0.
- failure: the invalid `server.stop` scope over the socket, as above. No process ended, which is asserted with the OS start token.
- Logs: `docs/evidence/task-48/`.

## Adversarial

- Crash at each commit barrier: `a_crash_at_any_barrier_is_finished_by_the_next_open_with_the_same_processes` crashes after `intent_persisted`, `suspended`, `announced`, `handed_over` and `booted`. The next `Control::open` finishes recovery on the intent's boot with the same process.
- Cancel/resume and repeated interruptions: `repeated_interruptions_never_mint_another_boot_or_lose_the_process` (the successor crashes too; the third open still uses the intent's boot). `work_offered_inside_the_boundary_is_refused_as_restarting_not_queued_or_lost` holds the restart with a capacity-0 listener, so the check is deterministic, not timing-dependent.
- Misleading success:
  - `a_launch_interrupted_before_spawn_evidence_stays_outcome_unknown_and_is_adopted`: the receipt and `op.get` say `outcome_unknown` with `delivery: unproven`, never committed.
  - The `classify` proptest: only a ledger commit is `committed`, and anything with write intent or acknowledgement is never `not_started`.
  - An unknown ledger state is uncertain, never success.
- Host reboot / foreign lifetime: `an_intent_from_another_host_boot_or_lifetime_is_refused_and_kept`.
- Rollback: `a_failure_before_handover_keeps_the_old_boot_serving_and_signals_nothing`.
- Host safety: tests signal only processes they spawned. Every inspected pid is asserted `> 1`, and inspection only reads the start token.

## Contract checklist

1. Every u64 on the wire is a decimal string: `written_bytes` / `output_offset` / incarnations / `seq` / intent `seq`. Numeric forms are refused, and u64::MAX round-trips (protocol and intent tests).
2. `RestartError` is a thiserror enum with `kind()` / `hint()` and `#[source]` on inner errors; Display never repeats the kind. The lane maps it to `DispatchError::Domain` with the same kind and hint.
3. Identities stay distinct newtypes (`RuntimeId`, `ServerBootId`, `HostBootId`, `ProcessIdentity`, `TerminalGeneration`, `OpId`).
4. Tests assert behavior (process liveness by start token, typed kinds, round trips). Proptest covers the outcome records and `classify`.
5. No `#[allow]`/`#[expect]`, no TODOs. Holder wire generation 2 is unchanged.
6. The conformance limitation is stated under DEFERRED VERIFICATION.

## Seams

- New public modules: `ovis_terminal_core::restart`, `ovis_terminal_protocol::domain::restart`, `ovis_terminal_server::dispatch::lanes::restart` (cfg(unix), symmetric with `unix_service`). Additive constructors on the published `ServerTransitionResult` and `RecoveryEvidence`. The published schema is unchanged.
- Files at the edge of the literal write scope:
  - `crates/ovis-terminal-core/src/restart.rs`, a module root required by `clippy::mod_module_files`;
  - child modules under `domain/restart/` and `lanes/restart/`;
  - `conformance/scenarios/recovery/restart.json`: the scenario id grammar is `<family>/<name>.json`, so `recovery/restart/` cannot hold `recovery/restart`;
  - slot lines in `lib.rs`, `domain.rs`, `lanes.rs`, `declared.json`;
  - the `restart` option and job appended to `.github/workflows/probe-stress-macos.yml`.

## Risks

- **Assembly obligation (todo 31):** the served binary must call `ServerHandle::restart()` (not `shutdown()`) and `TerminalStreams::restarting()` for each terminal when `Control` announces `started`. `Control::boundaries(0)` lets it hold the restart until streams are closed. Both closes are typed and tested here; the wiring into one served process is not.
- The published `server.restart` result lists process incarnation numbers only. Terminal id, PID and start token are in the core `RestartReceipt`.
- `Terminals::new` always tells the holder `PreviousCoreState::Crash`, even on a planned handover (terminals runtime, todo 16 scope). The holder does not change behavior on it today.
- Interrupted operations are enumerated from the lifecycle ledger (pending launches). Input sends become interruptible once todos 28-31 route them through `Control`; `classify` already handles their write evidence.
- Round 2 is pushed on top of the orchestrator's merge cb74619. origin/integration has since moved to 0d3022c; it was not merged here, per the lane rule.

## Pre-existing failure (not this change)

`cargo test -p ovis-terminal-core --test subscriptions` fails in-process on this host: `durability::close_forced_returns_only_after_its_marker_is_durable` or `durability::missed_is_returned_only_after_its_marker_is_durable` (`Error: Disconnected`).

- It reproduces on clean 81cb6bf with this diff stashed.
- It passes under nextest's per-process isolation (838/838).
- Reported, not touched.
