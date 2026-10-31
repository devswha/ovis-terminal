test(subscriptions): make the qa pump handshakes event-driven so the suite cannot fail by scheduling order

Todo 81 (closes GAP-REGRESSION flake; keeps IDEAL-050 ordering guarantees testable). Test-only change: no product code, no public API, no wire or holder change (holder wire generation stays 2).

## Before

Ledger sightings (`.omo/ulw-execute/ledger.jsonl`, `"kind":"flake"`, task 22), quoted:

- 2026-10-31T17:27:00Z: "core::subscriptions stall::a_stalled_fanout_worker_loses_every_subscription_and_each_recovers failed once in a 324-test 3-crate nextest run on the loaded host (6 lanes building) in the lane-t50 merge tree; 10/10 alone and 21/21 in its file afterwards. Second subscriptions-qa timing sighting (first: PUMP_GATE process-global note)."
- 2026-10-31T20:12:46Z: "core subscriptions stall::a_stalled_fanout_worker_loses_every_subscription_and_each_recovers failed once in T28's check.sh then 20/20 alone - THIRD sighting of this stall-timing flake (todo 61)."
- 2026-10-31T20:13:11Z: "core subscriptions durability::close_forced_returns_only_after_its_marker_is_durable failed once under `cargo test` parallel load in lane-t35 (10/10 alone) - FOURTH subscriptions-qa timing sighting"
- 2026-10-31T22:03:04Z: "core subscriptions loss::a_withheld_consumer_recovers_from_its_durable_marker_and_continues failed once in the lane-t48 merge gate (446-test run under host load); alone pass=10/10 - FIFTH subscriptions-qa timing sighting."
- Task-22 gate review R2-N1: "`PUMP_GATE` is a process-global static used by `durability` (2 tests) and `stall`. [...] Under `cargo test`'s threaded harness, one test's worker could take another test's gate."
- T57 verifier: `durability::close_forced_returns_only_after_its_marker_is_durable` 'Error: Disconnected' 4/8 on integration 759c85a.

Reproduced on the pre-fix tree (804e508) before any change:

- `docs/evidence/task-81/before-stress.log`: the acceptance loop (8 x `taskset -c 0-1 sh -c 'while :; do :; done'`, then `taskset -c 0-1 cargo nextest run -p ovis-terminal-core --test subscriptions --locked` per run): `# result: 48 passed, 2 failed of 50 runs`. Runs 18 and 27 failed `failure` with `expected a loss marker, got Heartbeat(... seq: EventSeq(4), scan_seq: EventSeq(4) ...)`.
- `docs/evidence/task-81/before-cargo-test.log`: `cargo test -p ovis-terminal-core --test subscriptions --locked` (one process, threaded libtest): 9/10 runs failed, each `durability::{close_forced_returns_only_after_its_marker_is_durable,missed_is_returned_only_after_its_marker_is_durable}` with `Error: Disconnected`.
- A targeted pinned loop of the three named tests (CPUs 16-17, 8 busy loops) failed `loss::a_withheld_consumer_recovers_from_its_durable_marker_and_continues` with the same `expected a loss marker, got Heartbeat(... seq: EventSeq(4) ...)` (`docs/evidence/task-81/before-targeted.log`; the loop was stopped early once the cause was captured).

## Root cause per test

Each of the three tests assumed an order the product never promised (the todo-80 method).

**durability::close_forced_returns_only_after_its_marker_is_durable (and its sibling missed_is_returned_...)**. The worker hold used one process-global slot, `PUMP_GATE: Mutex<Option<Armed>>`. Every `World` gate event (`Gated(1)`) sprang whichever gate was armed. When two tests share a process (`cargo test`'s threaded harness), the second test's `*PUMP_GATE = Some(..)` replaces and drops the first test's `(entered, released)` pair. The first test's `reached.recv_timeout(BOUND)` then returns `RecvTimeoutError::Disconnected`, printed as `Error: Disconnected`. Another test's worker can also take the gate. The worker side also waited for release with `recv_timeout(BOUND)`, so a hold longer than 5 s turned into a serialization error that killed the fan-out worker. **Fix**: `World::hold_worker()` arms a gate under a fresh key (`NEXT_GATE`) in a keyed map (`GATES: Mutex<BTreeMap<u32, Armed>>`), commits `Gate(Gated(key))`, and returns once that world's worker reports it is held. The worker removes only the gate its own event names and waits with `recv()`. It resumes on the release message or when the test drops the sender (test failure or panic), never because a timer ran out.

**stall::a_stalled_fanout_worker_loses_every_subscription_and_each_recovers**. After the overflow, the worker's `refill` installs a fresh coordinator delivery (bound: 2 events) and commits one loss-marker unit with two events, one per subscription. Those two markers fill the fresh delivery. Consumers can recover, and the test can commit `z`, before the worker has drained them, because recovery only needs the markers to be durable. `z` then overflows the delivery a second time and `scan_to(next)` ends with `Missed(... FanoutOverflow ...)`. The test assumed the worker had drained its markers by the time the consumers recovered. Proven deterministically in a scratch copy: with the worker descheduled for 300 ms after `refill`, the pre-fix test fails with `scan to 7 ended with Missed(Recovery { loss: ... cause: FanoutOverflow ... })`. **Fix**: before committing `z`, `pulls::until_fanned_out(&world)` waits for the event itself. It takes the durable head (`World::fresh`) and subscribes a probe at the worker's fan-out head (`Start::Head`). If the probe starts below the durable head, it scans until the worker has fanned out up to that head. Assertions are unchanged.

**loss::a_withheld_consumer_recovers_from_its_durable_marker_and_continues (and `failure` in tests/subscriptions.rs)**. The overflow at "three" makes the worker commit a loss marker right after it fans out "three". When the witness pulls late, one batch holds both event 3 and marker 4. `scan_to(&witness, 3)` returned that batch, and `until_loss_event` then waited for a marker it had already consumed, until the 5 s bound gave a `Heartbeat` at `seq 4`. The test assumed the marker arrives in a later batch than the event before it; the product batches every queued event. **Fix**: the tests keep the events `scan_to` returned (`seen`), and `until_loss_event(feed, pulled)` first looks among the events this feed already delivered, then pulls. The two other callers, whose marker is committed only after their scan returned, pass `&[]`.

No sleeps, retries, larger bounds or buffers, serialization, or weakened or removed assertions. `BOUND` stays 5 s as a hang guard only.

## After

- `docs/evidence/task-81/stress-200.log`: the same loop on the fixed tree, 200 runs of `taskset -c 0-1 cargo nextest run -p ovis-terminal-core --test subscriptions --locked` beside 8 busy loops pinned to CPUs 0-1: `# result: 200 passed, 0 failed of 200 runs`.
- `docs/evidence/task-81/after-cargo-test.log`: `cargo test -p ovis-terminal-core --test subscriptions --locked` (threaded harness, the R2-N1 condition): 20/20 runs `ok. 21 passed; 0 failed`.
- `bash scripts/check.sh` (fmt, clippy -D warnings, `cargo nextest run --workspace --all-targets --all-features --locked --no-tests=fail`: `1025 tests run: 1025 passed (1 slow), 1 skipped`, doc tests, hygiene `0 violation(s)`, manifests, json): exit 0.
- Windows pre-check `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0.
- Host: Linux 7.0.0-38-generic x86_64 (AMD Ryzen Threadripper PRO 3995WX).
- macOS: existing `probe-stress-macos.yml` job `subscriptions` (100 nextest iterations) dispatched on this branch: https://github.com/devswha/ovis-terminal/actions/runs/41254376898 (result recorded in the DoneClaim).

## QA

- happy: `bash docs/evidence/task-81/stress.sh 200 docs/evidence/task-81/stress-200.log` (8 busy loops on CPUs 0-1, then the pinned nextest loop): `# result: 200 passed, 0 failed of 200 runs`.
- failure: the same loop on the pre-fix tree (804e508, no diff) before any edit: `# result: 48 passed, 2 failed of 50 runs` (`before-stress.log`).

## Adversarial

`docs/evidence/task-81/adversarial.log`. A scratch copy (git archive of 804e508, outside the worktree) with diagnosis-only injections. Neither injection is committed.
- A: worker sleeps 300 ms after `refill` (forces the stall window open). Fixed qa: `21 tests run: 21 passed`. Pre-fix qa: stall fails with the second `FanoutOverflow`.
- B: A plus `scan_to` sleeping 200 ms before pulling (forces the loss marker into the scanned batch). Fixed qa: `21 tests run: 21 passed`. Pre-fix qa: `stall`, `loss::a_withheld_consumer...` and `failure` fail (`18 passed, 3 failed`).
- Threaded harness (cross-test gate sharing): pre-fix 1/10 clean, fixed 20/20 clean.

## Contract checklist

1. Decimal-string u64 on the wire: unchanged; no wire or JSON surface touched (the `wire.rs` u64::MAX round-trip tests still pass).
2. Boundary errors: none added or changed.
3. Identities: none added; the gate key is test-fixture payload only.
4. Tests assert behavior: no assertion changed. The fix changes only how tests wait for product events.
5. No `#[allow]`/`#[expect]`, no TODO placeholders, no new abstractions outside the qa harness.
6. Holder wire generation stays 2; checklist items unchanged.

## Seams

None. `World::gate()` (qa fixture, test-only) is replaced by `World::hold_worker()`, `PUMP_GATE` is removed, and `pulls::until_loss_event` takes the already-pulled events. All are test-harness items inside the write scope; no crate public API changed.

## DEFERRED VERIFICATION (runner lands in Todo 21)

Not applicable: this todo changes only the subscriptions qa harness and adds no RPC surface or conformance scenario.

## Risks

- `World::hold_worker` waits for the worker with `recv()` and no timer, so a test that blocks forever while holding the worker would hang instead of failing after 5 s. Every wait inside a hold is bounded on the test side, and dropping the sender (any early return or panic) releases the worker.
- `origin/integration` moved to f835935 (lane t41) after dispatch, with no change under `crates/ovis-terminal-core/src/{subscriptions,events}`. Not merged into this lane (process rule); the orchestrator merges.
