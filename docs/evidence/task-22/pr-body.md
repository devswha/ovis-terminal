feat(subscriptions): implement independent subscriptions replay and recoverable loss

Plan todo 22 (wave 10). Closes GAP-021, GAP-022, GAP-023, GAP-026, GAP-027, GAP-028, GAP-029, GAP-030, GAP-031 at the core layer. Delivers IDEAL-015, IDEAL-018, IDEAL-019, IDEAL-020, IDEAL-021, IDEAL-022, IDEAL-023 in `ovis-terminal-core`. The RPC routes come later, so the real-binary proof is deferred (see DEFERRED VERIFICATION).

## Before

RT-HD-08 / V-RT-HD-08 / EXCL-015: a second `events.subscribe` on one herdr connection silences both streams.

`docs/evidence/baseline/smoke/herdr-h06-events.log:15` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h06-events.log:15`):

```
TRAP CHECK: status event for A (first subscribe) received=false; for B (second subscribe on same conn) received=false
```

The plan cites `docs/evidence/baseline/herdr-h06-events.log:15`. The durable copy actually lives under `baseline/smoke/`.

## After

### Real binary: DEFERRED VERIFICATION (runner landed in Todo 21; `events.*` is not routed until todo 31)

The runner from todo 21 exists. No serving fixture or binary routes `events.subscribe/update/unsubscribe` yet. The conformance fixture (`conformance/src/fixture/`) and the server dispatcher are outside this todo's write scope, and the plan assigns binary assembly to todo 31. `conformance/scenarios/part-a/task-22.json` is therefore a declaration, listed in `conformance/scenarios/declared.json` with owner `22` and blocking todo `31`.

These are the exact plan QA invocations, run verbatim. They exit 1 with `scenario_not_executable`:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-22 --isolated --case happy --evidence "$ATTEMPT_DIR/task-22-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"part-a/task-22 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"part-a/task-22",...}
exit=1
$ cargo run ... --scenario part-a/task-22 --isolated --case failure --evidence "$ATTEMPT_DIR/task-22-ovis-terminal-v1.json"
{"case":"failure","error":{...,"kind":"scenario_not_executable",...},"result":"failed",...}
exit=1
```

The full suite still passes with the declaration allowlisted (`docs/evidence/task-22/all-suite.log`):

```
$ cargo run --locked --quiet --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence .../all-suite.json
{"case":"all","error":null,"result":"passed","scenario":"all/all","totals":{"assertions":"345","cases":"16","declared_only":"24","failed":"0","passed":"16","skipped":"0"}}
exit=0
```

### Core behaviour against the real SQLite writer

Linux 7.0.0-38-generic x86_64, rustc 1.99.1. `docs/evidence/task-22/core-happy-failure.log`:

```
$ cargo nextest run -p ovis-terminal-core --test subscriptions --locked --no-tests=fail --no-capture -E 'test(=happy) | test(=failure)'
{"effective_seq":"3","filtered_scan_seq":"2","rebuilt_equals_fresh":true,"replayed_seqs":["1","2","3","4"],"subscriptions":["01a1926d-75bc-75be-8f4e-2aebf6859acc","01a1926d-75c0-733e-95d7-07f4b706dbe4"]}
{"foreign_missed":"lifetime_mismatch","loss":{"cause":"queue_overflow","from_seq":"1","subscription_id":"01a1926d-7569-761d-ade7-71e5dae9021f"},"loss_marker_seq":"4","reconnect_missed":"cursor_expired","reconnect_snapshot_seq":"4"}
exit=0
```

## What changed

`crates/ovis-terminal-core/src/subscriptions/` contains `Subscriptions`, the hub. It holds the only coordinator delivery and runs one named fan-out worker (`ovis-subscriptions`). The worker copies every committed event into each subscription's own bounded queue.

Each subscription has its own `SubscriptionId`, a replaceable `Selector` (types, terminals, machines), a scan cursor and a delivered cursor.

- **Startup (`Request` / `Start`).**
  - `Snapshot`: the snapshot at R is captured under the hub lock, so R is at or after everything already fanned out, and delivery is strictly after R.
  - `Head`: starts at the current fan-out head with no snapshot.
  - `Resume(cursor)`: replays retained events after the cursor through the coordinator. A foreign lifetime or an expired cursor returns `missed` plus a fresh snapshot. A cursor ahead of the head returns `seq_ahead` and installs nothing.
  - `Request::decode` maps `EventsSubscribeParams`. A cursor always resumes; the `snapshot` flag only matters without one.
- **`update`** replaces the selector in place and returns `effective_seq = scan_seq + 1`, the first sequence judged by the new filter. **`unsubscribe`** closes one subscription; its consumer then reads `Closed{seq, scan_seq}`.
- **`Feed::next(deadline)`** returns one of four outcomes:
  - `Events(Batch{events, scan_seq})`;
  - `Heartbeat{seq, scan_seq, machine_id, runtime_id, boot_id}` once the subscription has been idle for the configured interval (at most 15 s, enforced by `QueueLimits::checked`) or until the caller's deadline, whichever is first;
  - `Missed(Recovery{loss, snapshot})`;
  - `Closed`.
- **Loss.** A subscription loses delivery in three ways:
  - its own event or byte bound overflows (`queue_overflow`);
  - it holds the largest queue when the session budget runs out (`session_budget`), so healthy queues are never the victim;
  - the worker itself falls behind (`fanout_overflow`), which loses every active subscription; the worker then re-subscribes.

  In every case the queue is replaced by a `LossRecord{subscription_id, from_seq, cause}`. That record is committed as an event in the durable log before the consumer is told. `Feed::close_forced` commits a `forced_close` marker for undelivered events before returning. Dropping a `Feed` with undelivered events has the worker commit the marker on its next tick, even with no further commit.
- **`QueueLimits::v1()`** gives 64 subscriptions, 10,000 events and 64 MiB per queue, a 64 MiB session budget and a 15 s heartbeat.

## Seams (public API changes outside `subscriptions/`)

- **`ovis-terminal-core::events::Coordinator::replay_floor()`** (new, additive, `events/replay.rs`). It returns the lowest resumable cursor sequence from the durable `floors` row. `Started.replay_floor` needs it, and nothing in `subscriptions/` can read the store.
- **Subscription data type bound.** `Subscriptions<D, P>` requires `D: From<LossRecord>`. The assembly's composite event enum (todo 31) needs a variant for the `subscription.lost` marker.
- **Assigned landed-code defect, hello count limits (todos 13/4).** `Limits.event_replay_events`, `history_lines` and `max_in_flight_requests` were JSON numbers. They are now the new decimal-string scalar `ovis_terminal_protocol::schema::scalar::Count`.
  - Server: `conn/connections.rs` and `dispatch/tasks.rs` read `.get()`, and two server tests parse the string form.
  - `schema/ovis-terminal.openrpc.json` is regenerated.
  - **Todo 4 compat note:** `schema/released.openrpc.json` is amended in place: the three properties now `$ref` `Count`, the `x-limits` defaults are strings, and the `Count` component is added. Against the pre-amendment baseline, `openrpc compat` reports exactly 12 breaks: `output_type_changed` and `closed_output_enum_grew` for each of the three `Count` fields, on both `hello` and `schema.capabilities` (6 + 6; recorded in `schema-gate.log`, exit 1). This is the first change to the released baseline. Generation 1 has not been released, so the baseline was corrected instead of carrying a numeric counter forever.
- **Holder wire generation stays 2.**
- **`.github/workflows/probe-stress-macos.yml`**: the option `subscriptions` and a matching job are appended at the end, following the existing pattern.

## QA

All commands ran in this worktree on Linux x86_64.

- **`bash scripts/check.sh`: exit 0** (`check.log`). fmt, workspace clippy `-D warnings`, `nextest --workspace` (578 passed, 1 skipped), doctests, hygiene (250 LOC / 3 params) and manifest/json checks all pass.
- **Windows:** `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0 with no warnings (`windows-check.log`).
- **Core suite:** `cargo nextest run -p ovis-terminal-core --test subscriptions` passes 21/21 (`core-subscriptions.log`). The full `-p ovis-terminal-core` run passed 195/195.
- **Schema gate:** `openrpc check` and `openrpc compat schema/released.openrpc.json` both exit 0. `ovis-terminal-protocol --test limits_counts` passes 2/2 (`schema-gate.log`).

What each test asserts:

- **`happy`**
  - Two subscriptions on one hub, one filtered to terminal 1. Commits go to terminal 2, then terminal 1.
  - The unfiltered stream gets both. The filtered stream gets only the terminal-1 event, with `scan_seq` covering the nonmatching one.
  - `update` returns `effective_seq = second + 1`. After it, the filtered stream gets only terminal-2 events while the other stream continues.
  - Snapshot plus all events equals a fresh snapshot at the same sequence.
  - Resuming from the snapshot cursor replays the identical events with no snapshot and no `missed`.
- **`failure`**
  - One consumer withholds reads past a two-event bound; a healthy consumer sees every event plus the committed `LossRecord{from_seq: 1, cause: queue_overflow}` naming the withheld subscription.
  - The withheld consumer is dropped without reading its error, then reconnects with its old cursor after that cursor left retention. It gets `missed: cursor_expired`, a snapshot at the marker sequence, then live delivery.
  - A foreign-lifetime cursor gets `lifetime_mismatch` plus a snapshot equal to fresh state.
  - A cursor ahead of the head is refused with kind `seq_ahead`.
- **`independence`**
  - Unsubscribing one subscription leaves the other delivering, and later calls on the closed id return `not_found`.
  - At the subscription limit, the extra request gets `subscription_limit` without disturbing installed delivery.
  - An idle filtered subscription heartbeats with `seq` unchanged, `scan_seq` at the nonmatching commit, and the runtime identity.
  - A 50 ms interval heartbeats despite a one-hour caller deadline.
  - `QueueLimits::checked` rejects a zero bound and a heartbeat over 15 s.
- **`replay`**
  - A snapshot taken while another thread commits 50 changes is followed by exactly the contiguous sequences after R and rebuilds fresh state.
  - An expired cursor misses with a complete snapshot, and `replay_floor` equals head minus retention; resuming at the floor replays exactly the retained events.
  - A cursor ahead of the head consumes no subscription slot.
- **`loss`**
  - A withheld consumer recovers with the durable marker and a snapshot, never a partial queue, then gets only later events.
  - `a_forced_close_marker_names_the_first_undelivered_event_and_reconnect_replays_it`: `close_forced` returns a `forced_close` marker from the first undelivered event, and the reconnect replays both events and the marker.
  - A dropped consumer's marker is committed with no further commit.
  - Under a 1,500-byte session budget the withheld queue is evicted (`session_budget`) while the draining consumer receives every commit.
- **`stall`**
  - The worker is held inside one fan-out (a test-only gate inside the fixture event's serializer, active only on the `ovis-subscriptions` thread) while three commits overflow a two-event coordinator delivery.
  - Both subscriptions recover with `fanout_overflow` and a snapshot containing the overflowed value, then each gets only the next commit.
- **`durability`** (round 2, gate review B1). The fan-out worker is held with `PUMP_GATE`, so its idle tick cannot commit markers; the durable log is read straight from the coordinator while it is held.
  - `close_forced_returns_only_after_its_marker_is_durable`: the marker `close_forced` returns is already in the durable log when it returns.
  - `missed_is_returned_only_after_its_marker_is_durable`: a resume whose replay exceeds the queue marks the loss on the subscribing thread; when `next()` returns `Missed`, its marker is already in the durable log.
  - Mutants M4 (drop the synchronous persist in `close_forced`) and M11 (deliver `Missed` before the marker is durable) both fail these tests; see `mutations.log`.
- **`wire`**
  - `LossRecord.from_seq` round-trips every u64 as a decimal string (proptest), and exact `u64::MAX` decodes.
  - The numeric form and unknown fields are rejected.
  - `Request::decode` covers all three start forms.
  - `labels`, `children_of`, an empty list, a malformed terminal id and machine names give `filter_invalid`; a malformed lifetime gives `invalid_params`; a numeric cursor `seq` is rejected by the protocol decoder.

## Adversarial

- **Stale state (expired or foreign cursor):** `replay::an_expired_cursor_...` and the foreign/ahead checks in `failure`.
- **Hung commands (blocked writer, withheld reader):** the withheld consumer in `failure` and `loss`. The worker held mid fan-out in `stall` shows the hub re-subscribing rather than wedging.
- **Cancel/resume (reconnect):** forced close then resume (`loss::a_forced_close_...`); drop without the final error then reconnect (`failure`, `loss::a_dropped_consumer_...`).
- **Repeated interruptions (filter replacement never kills peers):**
  - `happy` and `independence` cover replace, unsubscribe and the limit.
  - The whole subscription test binary ran 100 consecutive times, 16 threads per run, with 0 failures (`stress-100.log`).
  - The macOS stress job `subscriptions` repeats the suite 100 times: https://github.com/devswha/ovis-terminal/actions/runs/41211905797 (triggered on 4c5ea95; result reported in the DoneClaim).
- **Timing:** there are no fixed sleeps. Every wait registers first (witness subscriptions, gate channels) and is bounded at 5 s. Only the heartbeat test measures time, because time is the behaviour under test.

## Contract checklist

1. **Decimal strings.** Every u64 on the JSON surface added here is a decimal string: `LossRecord.from_seq` and the `EventSeq` fields; hello `Count` limits are now strings too. The numeric form is rejected (`wire::loss_markers_reject_numeric_and_unknown_fields`, `limits_counts`), and `u64::MAX` round-trips exactly.
2. **Errors.** `SubscriptionError` is a `thiserror` enum with `kind()` and `hint()`, and `#[source]` on the identity and event errors; Display never repeats the kind token.
   - `filter_invalid`, `invalid_params`, `subscription_limit`, `not_found`, `limit_exceeded`, `unavailable`, `coordinator_stopped`.
   - Coordinator kinds such as `seq_ahead` are delegated.
3. **Identities.** `SubscriptionId`, `TerminalId`, `MachineId`, `RuntimeId`, `ServerBootId` and `EventSeq` are distinct newtypes; `Option` appears only for real absence (snapshot, missed).
4. **Tests.** Tests assert state transitions, round trips and typed kinds, with proptest for the wire forms.
5. **Lints and placeholders.** No `#[allow]` or `#[expect]`, no TODOs, no prose-pinning tests.
6. **Evidence.** This body states what each test asserts, and the conformance attempt is recorded verbatim above.

## Risks

- **No socket proof yet.** The server dispatcher, the conformance fixture and the binary do not route `events.*` yet. The heartbeat "on every transport" holds because the core `Feed::next` produces it independently of the transport. Todo 31 (and 28 for WebSocket) must call `next` per subscription and write the notifications. Until then no real-socket transcript exists.
- **Unsupported filter selectors.** `labels`, `children_of` and machine names are rejected with `filter_invalid` instead of being evaluated. They need registry lookups (agent registry, parent tracking, machine names) that this layer does not own.
- **Replay vs. queue bound.** Replay on resume is bounded by the per-subscription queue. A replay larger than the queue bound becomes an immediate `queue_overflow` loss with a snapshot. With the v1 defaults (10,000 events equals retention) this only happens with custom small bounds.
- **Stall closing.** The 30 s stall-close timer for an unwritable connection belongs to the transport. The core provides `close_forced`, which commits the loss marker before returning.
- **Subscription ids are not yet bound to a principal.** Any holder of a `SubscriptionId` can `update` or `unsubscribe` it through the core API. Todo 31 binds subscriptions to the caller's todo-19 authority scope when it routes `events.*`.
- **Baseline amendment.** `schema/released.openrpc.json` was edited. If the orchestrator prefers keeping the baseline immutable, that part must be reverted and the three limits stay numeric.
