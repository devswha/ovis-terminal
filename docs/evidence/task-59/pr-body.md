feat(health): expose read only health and per consumer metrics (plan todo 59)

Closes GAP-016, GAP-017, GAP-142, GAP-143. Delivers IDEAL-014, IDEAL-115, IDEAL-116. Enforces EXCL-016, EXCL-021, EXCL-027 and does not reproduce V-RT-HC-04, V-4078, V-005.

## Before

`docs/evidence/baseline/smoke/hc-s18-stop-cleanup.log:9-11` (RT-HC-04): after `hc server stop` returned `{"stopped":true}`, the read-only status call started a new core:

```
$ <run-root>/bin/hc server status --json
--- stdout
{"core":{...},"finished":1,"holder":null,"holders":[],"installed_by":null,"live":0,"pid":2678504,"uptime_ms":9,"version":"0.1.14"}
```

(a fresh `pid` with `uptime_ms: 9`). Gap baseline: GAP-016, GAP-017, GAP-142, GAP-143 (no per-consumer queue/lag/loss metrics, no RPC latency, no recovery state that survives reads, and status that is not observational).

## What changed

- `ovis-terminal-core::health`: bounded tables that producers publish into. These are `Consumers` (at most 1024 rows; a refused consumer is counted in `consumers_untracked`; at most 16 retained loss ranges per consumer, older ones counted in `losses_omitted`), `Latencies` (keyed only by catalog methods, so client-chosen names cannot grow it), and `RecoveryLedger` (the incident journal plus the boot's crash-recovery report; facts are only added). `Health::status` / `Health::metrics` copy published values and do no I/O, so a read cannot wait on producer work and cannot reset, acknowledge or erase anything.
- `ovis-terminal-protocol::domain::health`: `ConsumerRef` (stream+terminal or subscription), `LossRange` (events or bytes), `ConsumerLoss`, `MethodLatency`, `RecoveryPhase` / `RecoveryState`. Every u64 is a decimal string (`Count`, `ByteCount`, `DurationMs`, `EventSeq`, `ByteOffset`). `ServerStatusResult` gains an optional `recovery_state`. `ServerMetricsResult` gains `runtime_lifetime`, `server_boot_id`, `collected_ms`, `consumers_untracked`, `rpc_latency` and `recovery_state`, and its consumer rows gain `consumer`, `losses` and `losses_omitted`.
- `ovis-terminal-server::dispatch::lanes::health`: `server.status` and `server.metrics` as unrecorded READs with machine Read-scope authz over one committed snapshot. Params are the closed empty object, so `{"reset": true}` is `invalid_params` before anything is read. `Measured<H>` wraps the connection handler and records each call's latency after it settles; a pending call holds no lock that a metrics read needs.
- Status of a stopped or stale endpoint stays in the CLI's existing observational path (no server is started). The scenario proves `stale_endpoint` is distinct from `stopped`.

## After

Host: Linux x86_64 (AMD Ryzen Threadripper PRO 3995WX). Binary `target/debug/ovis-terminal` sha256 `f15bf6f3c790e7f12030ccce522435a70413bde2beff059884a588ff17892a5d` (also recorded in the evidence JSON `binary` block).

- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal`: exit 0 (`build.log`).
- `cargo test -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets --no-fail-fast`: exit 0, 702 passed, 0 failed, including 19 health tests (`test-lanes.log`). The first run without `--no-fail-fast` hit the known `subscriptions` scheduling flake (`durability::missed_is_returned_only_after_its_marker_is_durable` -> `Error: Disconnected`, `test-lanes-run1-subscriptions-flake.log`). That flake was fixed on integration by c6c7aaa (PR #63), which this lane's base 44e1660 predates. This lane does not touch subscriptions.
- `cargo clippy -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0 (`clippy.log`).
- Happy: `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario observability/health --isolated --case happy --evidence "$ATTEMPT_DIR/task-59-ovis-terminal-v1.json"`: exit 0

  ```
  {"case":"happy","error":null,"result":"passed","scenario":"observability/health","totals":{"assertions":"67","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
  ```

  The happy case runs these materialized CLI/socket steps:
  1. `ovis-terminal --json server status` on a root with no server returns `state: stopped` and `recovery_state: null`.
  2. Over the socket, `server.status` returns `running` with `server_boot_id: boot:1` and `runtime_lifetime: runtime:21` (the hello's identities).
  3. A stalled viewer (`fixture.viewer`) is visible in `server.metrics`: `streams: 1`, `consumers/0/consumer = {kind: stream, stream_id: stream-v1, terminal_id: t-v1}`, `queue_bytes: "4096"`, `event_lag_ms >= 1`, `losses = [{range: {unit: bytes, first_offset: "4096", last_offset: "8191"}, resynced: true}]`, `rpc_latency/0 = server.status calls "1" failures "0"`.
  4. A recorded `store_corrupted` incident reads `{phase: degraded, corrupted: true, incidents: "1"}` on two consecutive metrics reads (the corrupted flag remains after a metrics read), and `ovis-terminal --json server status` shows `/recovery_state/corrupted: true`.
  5. A dead socket at the default endpoint makes the CLI report `state: stale_endpoint, server_boot_id: null`.
  6. After stop, two consecutive `ovis-terminal --json server status` calls report `stopped` with `uptime_ms: null` and no new socket or process.

  Every metrics reply arrives within 2000 ms.
- Failure: `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario observability/health --isolated --case failure --inject '{"method":"server.metrics","params":{"reset":true}}' --evidence "$ATTEMPT_DIR/task-59-ovis-terminal-v1.json" --append`: exit 0

  ```
  {"case":"failure","error":null,"result":"passed","scenario":"observability/health","totals":{"assertions":"21","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
  ```

  Evidence checks: `inject/planned` actual `{"method":"server.metrics","params":{"reset":true}}`, `inject[0]/error/data/kind` actual `invalid_params`, `inject[0]/result` absent. `server.status {"verbose": true}` is also `invalid_params`. The read after the injection still reports `recovery_state {phase: degraded, corrupted: true, incidents: "1", unsettled: "0"}`, and its `server.metrics` latency row keeps the earlier calls and counts the refusal as a failure. No counter or recovery evidence was erased.
- All: `... --case all --evidence "$ATTEMPT_DIR/task-59-all.json"`: exit 0, `assertions 90, cases 2, failed 0, skipped 0`.
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0. `1103 tests run: 1103 passed (1 slow), 1 skipped`; `workspace-hygiene: 0 violation(s) in 1092 file(s) checked`; `check.sh: all steps passed` (`check.log`).
- Windows pre-check (through the heavy gate, `windows-check.log`): `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0.

Raw evidence in `docs/evidence/task-59/`: `task-59-ovis-terminal-v1.json` (happy + failure, with binary hash, host and every request/result check), `task-59-all.json`, `qa-{happy,failure,all}.{stdout,stderr,exit}`.

## Adversarial

`docs/evidence/task-59/adversarial.md`:
- Misleading success: the scenario reports `stale_endpoint` distinct from `stopped`.
- Hung commands: `metrics_answer_while_a_call_hangs_and_name_the_current_boot` answers metrics while another call is pending; every scenario read is bounded by `within_ms: 2000`.
- Stale state: every reply asserts the hello's `boot:1` / `runtime:21`.
- Mutants: `bash docs/evidence/task-59/mutants.sh` (`mutants.log`). 5/5 were killed by a failing shipped test, none by a compile error: resync-never-marked, corruption-forgotten, failures-uncounted, latency-unrecorded, queue-bytes-written.

## Contract checklist

1. Every u64 on the wire is a decimal string. `counters_are_decimal_strings_up_to_u64_max` round-trips u64::MAX, and `numeric_counters_and_unknown_fields_are_rejected` rejects numeric forms.
2. `HealthError` is a `thiserror` enum with `kind()`/`hint()` and `#[source]` on the journal errors. Display does not repeat the kind token.
3. Distinct newtypes: `StreamId`, `SubscriptionId`, `TerminalId`, `ClientId`, `ServerBootId`, `RuntimeLifetimeId`, `MethodName`, `OpId`, `ByteOffset`, `EventSeq`, `Count`.
4. Reads are unrecorded READs with machine Read-scope authz; nothing is written to the ops ledger.
5. No `#[allow]`/`#[expect]`, TODO/FIXME, sleeps or retries. Waits in tests are event-driven with bounded hang guards only.
6. Files stay under the 250 pure-LOC hygiene limit and the 3-parameter/100-line clippy limits (grouped tuple parameters, child modules `health/qa`, `domain/health/results`, `lanes/health/qa`).

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-core/src/lib.rs`, `crates/ovis-terminal-protocol/src/domain.rs`, `crates/ovis-terminal-server/src/dispatch/lanes.rs`: one `pub mod health;` line each.
- `crates/ovis-terminal-core/src/health.rs`, `crates/ovis-terminal-protocol/src/domain/health/`, `crates/ovis-terminal-server/src/dispatch/lanes/health/`: module roots and child modules of the scoped paths.
- `crates/ovis-terminal-protocol/src/schema/methods/server.rs`: the existing `ServerStatusResult` / `ServerMetricsResult` / `ConsumerQueue` records gain the fields listed above. `recovery_state` on status is optional, so older status payloads still decode (`a_status_written_before_recovery_state_existed_still_decodes`).
- `schema/ovis-terminal.openrpc.json`: regenerated for those fields.
- `conformance/src/fixture.rs`, `conformance/src/fixture/handler.rs`, `conformance/src/fixture/serve.rs`, `conformance/src/fixture/health.rs`: the protocol fixture registers the real health reads, wraps its handler in `Measured`, and serves three unadvertised stand-ins. `fixture.viewer` reports a stalled viewer into the real consumer table, because there are no attach streams until todo 31. `fixture.incident` records through the real journal. `fixture.stale_endpoint` leaves a dead socket at the default endpoint.
- `conformance/src/runner_tests/suite.rs`: `observability/health.json` was added to the sorted EXECUTABLE slice. It was never listed in `conformance/scenarios/declared.json`, so that file is unchanged.
- `.github/workflows/probe-stress-macos.yml`: `health` option plus a job appended last. It runs 25 iterations of the health lib tests and the `observability/health` scenario.
- `docs/evidence/task-59/`.

Seams for later todos: the attach stream (todo 31) and subscription pumps should call `Health::consumers().track(...)` and report `queued` / `written` / `lost` / `resynced` on their `ConsumerGauge`. The production server should install `Measured` around its handler and call `Health::publish_census` from the terminal service. Todos 60 and 69 consume `server.metrics`.

## Risks

- Production producers are not wired yet. The real attach/subscription paths land in other todos, so the fixture's `fixture.viewer` stand-in feeds the real table. Until those todos call `track()`, production `consumers` stays empty.
- `event_lag_ms` keeps at most 64 queue marks; beyond that the newest mark is extended. This can only overstate lag, never understate it.
- `origin/integration` moved to b77f0fb after this lane's base 44e1660. It was not merged here (process rule); the orchestrator union-merges.

## DEFERRED VERIFICATION

None: the conformance runner executes `observability/health` (happy, failure and all exit 0 above).

macOS stress: MACOS_RUN
