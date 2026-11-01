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

macOS stress: `gh workflow run probe-stress-macos.yml -f probe=health --ref lane/t59-expose-read-only-health-and` dispatched once: https://github.com/devswha/ovis-terminal/actions/runs/41269440978 (25 iterations of the health lib tests plus the `observability/health` scenario; result recorded in the DoneClaim).

## Merge round (origin/integration 723d101, authorized)

Conflicts were resolved keeping both sides:
- `conformance/src/fixture/handler.rs`: integration's `Programs` probe plus the health stand-ins.
- `conformance/src/fixture/serve.rs`: integration's `processes`/`update` registrations and holder shutdown, plus `health::register` and the `Measured` wrapper. The merged `serve()` reached 102 lines (`clippy::too_many_lines`), so the stdin wait moved into `runner_closed_stdin()` with no behaviour change.
- `.github/workflows/probe-stress-macos.yml`: integration's file, with `health` appended last to the options list and its job appended last. It parses: the last jobs are `recovery, update, health`.
- `schema/ovis-terminal.openrpc.json`: regenerated with `openrpc write`. `schema/released.openrpc.json` and `schema/BASELINE-AMENDMENTS.md` stay as on integration, because this lane amends no baseline and the ledger head stays `sha256:53782aab…`.

`packages/client-ts` is untouched.

Gates on the merged tree (`docs/evidence/task-59/merge-*.log`):
- `openrpc write`, `check` and `compat schema/released.openrpc.json`: exit 0 (`merge-schema.log`).
- `cargo fmt`: exit 0. Conformance clippy `-D warnings`: exit 0 (`merge-qa-2.log`).
- `observability/health`: happy exit 0 (67 assertions), failure with `--inject '{"method":"server.metrics","params":{"reset":true}}' --append` exit 0 (21), `--case all` exit 0 (90, 2/2).
- `all/all --case all`: exit 0, 46/46 cases, 1939 assertions, failed 0, skipped 0 (`merge-qa.log`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, `1160 tests run: 1160 passed (1 slow), 1 skipped`, hygiene `0 violation(s) in 1139 file(s)` (`merge-check.log`).
- Windows `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0. `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: exit 0 (`merge-windows.log`).

## Round 2 (gate review REJECT: B1, B2, B3)

Report: `.omo/evidence/ovis-terminal-v1-task-59-gate-review.md` on head 55a7c6c. This round is pushed on top of 55a7c6c with no merge.

### B1: every health counter on the wire is a canonical decimal string

- `ServerMetricsResult.streams`, `.subscriptions` and `ConsumerQueue.resynced_events` changed from `u32` JSON numbers to `Count`. `resynced_events` no longer clamps at `u32::MAX`; it saturates at `u64::MAX`.
- `ServerStatusResult.holder_count` and `.live_terminals` changed from `u32` to a required `Count`-or-null.
  - A new `census: OpenEnum<CensusState>` (`reported` / `not_reported`) states whether a census was published.
  - `Health` publishes no census until `publish_census` is called, so status reports `not_reported` with null counts rather than fabricated zeros (the B2 placeholder).
  - The schema keeps both fields required through `schema_with = nullable_count`, so the published type is `$ref Count` or `null` and never a bare string.
- Tests (`crates/ovis-terminal-protocol/src/domain/health/tests/wire.rs`):
  - `metric_counters_round_trip_u64_max_as_decimal_strings`: `/streams`, `/subscriptions` and `/consumers/0/resynced_events` encode `"18446744073709551615"` and decode back equal.
  - `numeric_metric_counters_are_refused_at_their_exact_path`: a JSON number at each field is refused by `serde_path_to_error` at exactly `streams`, `subscriptions` and `consumers[0].resynced_events`.
  - `a_reported_census_round_trips_u64_max_and_numbers_are_refused`: the same checks for `holder_count` and `live_terminals`.
  - `an_unpublished_census_is_not_reported_never_zero`.
  - Core `metrics_name_the_current_lifetime_and_boot` now also asserts `not_reported`/null before `publish_census` and `reported`/`"2"`/`"3"` after.
- OpenRPC:
  - `openrpc write` regenerated the document.
  - Before the amendment, `openrpc compat schema/released.openrpc.json` exited 1 with exactly 10 breaks, all `output_type_changed`/`closed_output_enum_grew` on those five fields (`r2-schema-before-amend.log`).
  - `schema/released.openrpc.json` was amended for exactly those five property schemas (24+/20-), and `schema/BASELINE-AMENDMENTS.md` gained the entry chaining `previous: sha256:53782aab…` (integration's head) to `current: sha256:0cf8102b…`.
  - `openrpc check` exits 0 and `openrpc compat` exits 0 (`r2-schema.log`).
- SDK: `packages/client-ts` was regenerated. `bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test` gave `{"generated":"fresh","files":31}`, `30 pass`, `0 fail`, exit 0 (`r2-sdk-regen.log`). Only `src/generated` changed.
- Scenario `observability/health`:
  - It now expects `"/result/streams": "1"`, `"/result/subscriptions": "0"` and `"/result/consumers/0/resynced_events": "0"`.
  - It also expects `"/result/census": "not_reported"`, `"/result/holder_count": null` and `"/result/live_terminals": null` on the running status, and `"/census": "not_reported"` from the CLI on a stopped root.

### B2: real producers report into health

The served binary has no assembly yet: todo 31 (`crates/ovis-terminal-cli/src/assembly/`, not landed) owns it. No production code calls `serve_unix`, `Subscriptions::start` or `TerminalStreams::new` today; this round's grep confirms the only callers are tests and fixtures. So health is wired **inside the real producers**, and the assembly only has to hand them one `Arc<Health>`.

- Subscription hub (`crates/ovis-terminal-core/src/subscriptions/`):
  - `Subscriptions::report_to(Arc<Health>)` attaches health.
  - `Request::for_client(ClientId)` names the row's client. A subscription without a client is counted in `consumers_untracked`, never given an invented client.
  - The new child module `subscriptions/gauge.rs` hooks five points:
    - fan-out `push` reports queued bytes;
    - consumer `take` reports written bytes;
    - `lose` drops the queue and records the `Events{from..=offered}` range;
    - `restart` from a fresh snapshot marks that same range resynced;
    - `close` and `orphan` drop the row.
- Stream hub (`crates/ovis-terminal-server/src/streams/`):
  - `StreamSetup::reporting_to(Arc<Health>)` attaches health.
  - `attach` tracks the viewer under its owner's client as `Stream{stream_id, terminal_id}`.
  - `publish` reports appended bytes for every open viewer, and `Viewer::data` reports written bytes.
  - `resync` reports the dropped unsent backlog plus the gap's `Bytes{from..=paint-1}` range as lost and resynced.
  - Removing a viewer drops its row.
- Real-producer tests (no fixture stand-in):
  - `crates/ovis-terminal-core/src/subscriptions/qa/health.rs::a_real_stalled_subscription_reports_its_queue_loss_and_resync`: real SQLite store, coordinator and hub. It checks a stalled consumer's queued bytes, the exact dropped `Events` range after its 2-event queue overflows, the same range `resynced: true` with `resynced_events == range.events()` after recovery, `consumers_untracked == 1` for the client-less subscription, and that the row disappears when the feed drops.
  - `crates/ovis-terminal-server/src/streams/qa/health.rs::a_real_stalled_viewer_reports_its_backlog_and_resynced_ranges`: real socket, authority, leases, screen and hub. The viewer that never acknowledges has `queue_bytes > 0` and only resynced `Bytes` ranges inside the published output. The viewer that keeps up has `queue_bytes == 0` and no losses. Both rows carry the attaching client.
- Seams left for todo 31 (assembly), all exact:
  - `Subscriptions::report_to` and `Request::for_client` at `crates/ovis-terminal-core/src/subscriptions/hub.rs` (`report_to`) and `start.rs` (`for_client`);
  - `StreamSetup::reporting_to` at `crates/ovis-terminal-server/src/streams/hub.rs`;
  - `dispatch::lanes::health::register` plus `Measured::new` around the connection handler at `crates/ovis-terminal-server/src/dispatch/lanes/health.rs`;
  - `Health::publish_census` at `crates/ovis-terminal-core/src/health/monitor.rs`, fed by the terminal service.
  
  Until then the census reports `not_reported` and the tables report only what the producers observed.
- The conformance fixture keeps `fixture.viewer` as a protocol-level stand-in, because the protocol fixture serves no attach streams. The two real-producer tests above are the integration proof.

### B3: the four named mutants (`bash docs/evidence/task-59/mutants-r2.sh`, log `mutants-r2.log`)

| Mutant | Change | Killed by (shipped test that fails) |
|---|---|---|
| record reads in the ledger | `server.metrics` writes a durable `store_corrupted` incident into the recovery ledger on every read (`lanes/health.rs` `Metrics::read`) | `dispatch::lanes::health::qa::reset_is_invalid_params_and_erases_no_counter_or_recovery_fact` (panics at qa.rs:251: recovery_state changed across reads) |
| key metrics by socket | consumer rows are keyed by the client connection instead of the stream/subscription (`consumers.rs` duplicate check on `client_id`) | `health::qa::rows_follow_gauge_lifetimes_and_the_table_is_bounded` (qa.rs:167: a second consumer of one client is refused) |
| auto-start on health | `ovis-terminal server status` on a missing endpoint binds a listener there, as a starting server would (`cli/src/invocation/execute.rs` `status`) | `ovis-terminal-cli --test cli happy` (cli.rs:28 "status created a file") and `observability/health --case all` (result failed: a later status is no longer `stopped`) |
| drop the cardinality cap | the `MAX_CONSUMERS` bound never refuses (`consumers.rs` `track`) | `health::qa::rows_follow_gauge_lifetimes_and_the_table_is_bounded` (qa.rs:174: the 1025th consumer is tracked) |

4/4 killed by a failing test, none by a compile error, and every source was restored (`sources restored`).

### Seams added this round (outside the original write scope)

- `crates/ovis-terminal-core/src/subscriptions/{gauge.rs (new), hub.rs, shared.rs, start.rs, table.rs, fanout.rs, consume.rs}` and `subscriptions.rs` (mod line): subscription producer hooks.
- `crates/ovis-terminal-core/src/subscriptions/qa/health.rs` (new) and `crates/ovis-terminal-core/tests/subscriptions.rs` (one `#[path]` mod).
- `crates/ovis-terminal-server/src/streams/{hub.rs, viewer.rs, resync.rs, open.rs}`: stream producer hooks.
- `crates/ovis-terminal-server/src/streams/qa/{health.rs (new), recorder.rs (new), fixture.rs}` and `crates/ovis-terminal-server/tests/streams.rs`: the real-stream test and the fixture's `reporting` constructor. The fixture's lease-journal `Recorder` and `NoSections` producer moved unchanged into `recorder.rs`, because the new constructor took `fixture.rs` to 263 pure LOC (hygiene limit 250).
- `crates/ovis-terminal-cli/src/invocation/execute.rs`, `crates/ovis-terminal-cli/tests/cli.rs` and `crates/ovis-terminal-cli/tests/cli/support.rs`: the CLI decodes status through `ServerStatusResult`, so its locally reported stopped/stale status now carries null counts and `census: not_reported`, and its fixtures use decimal strings.
- `conformance/scenarios/platform/unix-service.json` and `conformance/scenarios/part-a/task-20.json`: the stopped status expects `"/holder_count": null` instead of `0`. task-20 is declaration-only (the runner reports `scenario_not_executable` with or without this change); unix-service passes with 48 assertions.
- `schema/released.openrpc.json` and `schema/BASELINE-AMENDMENTS.md`: the five-field amendment and its ledger entry.
- `packages/client-ts/src/generated/*`: regenerated, never hand-edited.

### Gates on this head (Linux x86_64, AMD Ryzen Threadripper PRO 3995WX)

- `cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance`: exit 0 (`r2-build.log`).
- `cargo test --locked -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server -p ovis-terminal-cli --all-targets --no-fail-fast`: exit 0, 755 passed, 0 failed (`r2-test-lanes.log`).
- `cargo clippy --locked -p ovis-terminal-core -p ovis-terminal-protocol -p ovis-terminal-server -p ovis-terminal-cli --all-targets -- -D warnings`: exit 0 (`r2-clippy.log`).
- `observability/health` (`r2-qa-*.stdout`, evidence `r2-task-59-ovis-terminal-v1.json`, `r2-task-59-all.json`):
  - `--case happy`: exit 0, 73 assertions.
  - `--case failure --inject '{"method":"server.metrics","params":{"reset":true}}' --append`: exit 0, 21 assertions (invalid_params at reset; recovery_state `{phase: degraded, corrupted: true, incidents: "1"}` unchanged).
  - `--case all`: exit 0, 96 assertions, 2/2, failed 0, skipped 0.
- `platform/unix-service --case all`: exit 0, 48 assertions (`r2-qa.log`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, `1166 tests run: 1166 passed (1 slow), 1 skipped`, hygiene `0 violation(s) in 1144 file(s)`, `json.sh: 136 JSON file(s) checked` (`r2-check.log`).
- Windows: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`r2-windows.log`).
- SDK: `bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test`: exit 0, 30 pass (`r2-sdk-regen.log`).

### Risks (round 2)

- The production assembly (todo 31) must call `Subscriptions::report_to`, `Request::for_client`, `StreamSetup::reporting_to`, `health::register` + `Measured::new` and `Health::publish_census`. Until it does, a served binary would report empty consumer tables and `census: not_reported`; it would never report invented zeros.
- A subscription's reported loss range ends at the sequence that overflowed it (the last one known dropped); events dropped after that and before recovery are covered by the fresh snapshot but not listed.
- The baseline amendment changes five released output types. Generation 1 has no released client of `server.status`/`server.metrics`, which follows the todo-22 and todo-47 precedents.

