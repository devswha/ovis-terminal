Plan todo 56: bound VT scheduling and preserve control latency.

Closes GAP-030, GAP-035, GAP-061, GAP-063, GAP-074, GAP-128 at the component level (see DEFERRED VERIFICATION); delivers the scheduling side of IDEAL-021, IDEAL-027, IDEAL-040, IDEAL-041, IDEAL-044, IDEAL-051, IDEAL-101.

## Before

`docs/evidence/baseline/smoke/hc-s06-wait-output.log:6` (RT-HC-01; source `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:6`):

```
--- exit=0 elapsed_ms=5213
```

`hc send sh1 sleep 1` took 5213 ms because send waited for a status transition (EXCL-024). Gap baseline: GAP-030, GAP-035, GAP-061, GAP-063, GAP-074, GAP-128.

Measured "before" for the parse budget (`docs/evidence/task-56/tuning/parse-budgets.json`, release build): one unbudgeted feed of 8 MiB blocks the engine for 570 ms (build log), 252 ms (SGR on every cell) and 430 ms (wide/combining). The spec's 1 MiB turn takes 28–68 ms.

## What changed

- `crates/ovis-terminal-vt/src/shard/`: `Shard` feeds queued output chunks (shared `Arc<[u8]>`, never copied) to a `Vt` in turns of at most `ParseBudget` bytes; `push` refuses past `PendingCap` with `ShardError::Full` (kind `backpressure`) before queuing; `cap()` exposes the bound. Defaults: 16 KiB per turn (from the measurement below), 1 MiB pending.
- `crates/ovis-terminal-server/src/streams/scheduler/` (new child of `streams`; viewer/pump untouched):
  - `spawn_terminal(Engine, sink, &TerminalSetup) -> Result<…>`: one actor per terminal on the multi-thread runtime. Its `biased` `select!` order is: control queue (cancel/resize/read_history/stop), admissions, output intake, then one parse turn followed by `yield_now`. Control events (`Admitted`/`Withdrawn`/`Resized`) carry `after_turns`, the parse turns run before they committed.
  - **Session caps** (`SessionBudgets`, one `Budget` type, typed `SchedulerError::Exhausted{scope,…}` before anything is queued or allocated):
    - input: terminal 16 MiB plus session 64 MiB (`input_overflow`);
    - control: 1024 queued requests per session (`backpressure`). Each control request holds a slot until served; `stop` needs no slot;
    - output: session 64 MiB of output waiting to be parsed. A chunk without room is stashed and retried on release, and the pump/PTY is held back; nothing is dropped;
    - events: each terminal reserves its retention (64 output slots × 16 KiB chunk + 1024 control slots) against the 64 MiB session event budget at spawn. A refusal is `backpressure`, and the reservation is returned at stop.
  - **Disk-backed history** (`history/`): append-only segment files named by the 20-digit offset of their first byte. The index is in memory, rebuilt from the directory on open, and bounded by retention. Once the retained bytes pass the bound (advertised: `output_ring_bytes` 64 MiB in 16 segments), the oldest segments are deleted and the floor advances.
    - `read(offset, max)` below the floor is `history_gap` (with the floor), past the end is `invalid_params`, and a gap between segment files on open is `history_unavailable`.
    - A failed append stops recording with a typed fault.
    - The actor queues every accepted chunk to the terminal's **`DbWriter<HistoryStore>`** (owned thread, bounded queue; busy → the chunk waits) before parsing it. `TerminalHandle::read_history` reads on that thread via the control lane.
  - Input: reserve-before-allocate, separate writer task (stdin non-reader cannot stall the actor), cancel → `Withdrawn`/`InFlight`, stop aborts a stuck write and reports `unsent`/`cut_off` (unchanged from round 1).
  - Fanout: control events and output events are separate bounded broadcasts; output shares one `Arc` per chunk; a lagging subscriber gets `Lagged(n)`.
  - `PumpThread` (owned, joined), `DbWriter<S>` (owned thread, bounded queue, joined; now used by the history path), `BlockingGate` (bounded `spawn_blocking`).
- `bench/tuning/` (`fairness-tuning`): `parse`, `load … [parse-budget [workers]]` over real portable-pty terminals; each load terminal keeps an advertised disk history, and the report gives the **measured** history footprint (`history_disk_bytes`, a walk of the files) in place of round 1's computed bound.
- `conformance/scenarios/performance/fairness.json` + `declared.json` entry (owner 56).

## Round 2 (gate review 2026-11-01)

- **B1:** `an_owned_pump_carries_output_until_eof_and_is_joined` now reads the output broadcast **concurrently** with the pump in a lag-tolerant reader (`qa::parsed_through`), and asserts the pump's counters and the actor's `parsed_bytes`. The final `Parsed` event is the newest one published, so lag cannot hide it. Capacity is unchanged. 25/25 runs of the full server lib binary pinned to 2 CPUs pass (`docs/evidence/task-56/stress-lib-2cpu.log`).
- **B2:** new `qa::ordering` (current-thread runtime, so observed turns equal actual turns):
  - `a_resize_waits_behind_at_most_one_parse_turn`: 32 one-KiB turns pending; asserts `Resized.after_turns <= turns_seen + 1` and `< 32`.
  - `admissions_and_cancels_overtake_pending_parse_turns_while_stdin_is_stuck`: the writer is stuck on a never-read pipe; asserts admission/withdrawal `after_turns` within one turn, and the outcomes `Withdrawn`/`InFlight`/`cut_off`.
  - Mutant M1 (parse arm moved ahead of control) fails both: "resize committed after 16 turns; 1 had run when it was sent" (`docs/evidence/task-56/mutant-parse-before-control.log`, exit 101).
  - Wall-clock bounds are reported, not asserted (round 3 also removed the profile's 1 s margin; see below).
- **B3:** session caps for control/output/events and disk-backed history as above, with tests in `qa::caps`, `qa::history`, `history::tests` (retention eviction and floor, reopen rebuild, discontinuity refusal, typed fault, proptest over chunking/segment size/read windows).
- Slow subscriber: `a_slow_subscriber_is_told_exactly_how_many_events_it_lost` asserts `Lagged(2·100 − 64)` exactly, and that the newest 64 events are still delivered.
- Unused API removed: `ActorConfig::with_terminal_input`, `with_events`, `TerminalSetup::session`, `ByteBudget::limit` (and `ByteBudget` is renamed `Budget`). Every remaining public item has a caller in tests or bench.

## Round 3 (verification note N5/N6)

- **N5:** the profile test no longer depends on host speed. A lag-tolerant tracker follows each terminal's newest parsed offset. Before stopping, the test waits (event-driven, 300 s hang guard only) until every terminal has parsed exactly what its feed accepted, then asserts `parsed_bytes == output_bytes`.
  - The wall-clock bound is no longer asserted anywhere: the profile prints `within_contract` and `worst_ms` against the 100 ms contract.
  - Under the verifier's recipe, the round-2 1 s margin itself failed once (run 12 of 25: `profile.rs:204`, `worst <= CI_MARGIN`). Control-first ordering is asserted by counts (`qa::ordering`, kills M1 and N6).
  - The test is renamed `sixteen_ptys_eight_clients_get_every_request_and_commit_with_latency_reported`; its workload helpers moved to `qa/workload.rs`.
  - Recipe (taskset -c 0-1, 8 busy loops on CPUs 0-1, 25× full server lib): **25/25** pass (`docs/evidence/task-56/stress-lib-2cpu-8busy.log`).
- **N6:** the admissions/cancels ordering test waits for the end of output and asserts `parse_turns == 32` and `before < 32`. With the drain-whole-backlog mutant (a turn ignores its budget), both ordering tests fail (`docs/evidence/task-56/mutant-drain-whole-backlog.log`, exit 101).

## Seams

- New public API: `ovis_terminal_vt::{ParseBudget, PendingCap, Shard, ShardError, Turn}` (+ `Shard::cap`); `ovis_terminal_server::streams::scheduler::*` (new `pub mod`).
- No landed public API changed. Holder wire generation stays 2.
- `bench` depends on `ovis-terminal-protocol`, `ovis-terminal-server`, `tokio` (`DEPENDS_ON` updated).
- **Disclosed, not changed:** existing synchronous DB work outside this lane still uses raw, unbounded `tokio::task::spawn_blocking`: `dispatch/execution.rs:128,159`, `dispatch/job.rs:63,114`, `streams/pump.rs:119,128`. Routing them through `BlockingGate`/`DbWriter` is outside this lane's write scope and belongs to assembly (todo 31/42). `BlockingGate` is therefore exercised only by its tests. The actor's own stop joins the history thread through one `spawn_blocking` per terminal stop.

## After

Native host: Linux 7.0.0-38-generic x86_64, AMD Ryzen Threadripper PRO 3995WX (128 cores, shared host), rustc 1.99.1 (`docs/evidence/task-56/tuning/host.log`).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                                  exit=0
$ cargo test -p ovis-terminal-vt -p ovis-terminal-server --all-targets                           exit=0 (177 passed)
$ cargo clippy --workspace --all-targets --all-features --locked -- -D warnings                  exit=0
$ bash scripts/check.sh                                                                          exit=0 (nextest 844 passed, 1 skipped; hygiene 0 violations in 853 files)
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked   exit=0
```

In-crate profile (debug, 4 workers, 4 KiB test budget): `admission_max_ms 1.543, control_max_ms 1.464, event_max_ms 2.923, events_lost 0, within_contract true`.

Real PTYs, release, bench pinned to CPUs 2-127 (`docs/evidence/task-56/tuning/*.json` + `.log` with command and exit). The latency columns are the max over every sample:

| run | workers | turn | admissions | admission max | control max | event max | lost | RSS peak | history on disk (measured) |
|---|---|---|---|---|---|---|---|---|---|
| 16 PTYs, 1 MiB/s (profile) | 4 | 16 KiB | 8016 | 4.4 ms | 16.1 ms | 21.4 ms | 0 | 247 MB | 10.7 MB |
| 16 PTYs, 64 MiB/s — **before** (8 MiB turns) | 4 | 8 MiB | 4086 | **100.7 ms** | 97.6 ms | 64.2 ms | 0 | 237 MB | 683 MB |
| 16 PTYs, 64 MiB/s — **after** | 4 | 16 KiB | 8000 | **21.6 ms** | 23.7 ms | 23.4 ms | 0 | 280 MB | 682 MB |
| 16 PTYs, 64 MiB/s for 20 s (retention) | 4 | 16 KiB | 16000 | 98.9 ms | **108.0 ms** | 78.9 ms | 0 | 288 MB | 1,030 MB of 1,365 MB recorded; 80 segments evicted |
| 50 PTYs, 400 viewers, 1 MiB/s | 4 | 16 KiB | 8016 | 0.44 ms | 10.8 ms | 10.8 ms | 0 | 704 MB | 10.7 MB |
| 50 PTYs, 400 viewers, 1 MiB/s | 8 | 16 KiB | 8016 | 0.08 ms | 11.0 ms | 4.3 ms | 0 | 726 MB | 10.7 MB |

- The 1 MiB/s profile and the 50×400 stress stay within 100 ms. Their RSS stays below 1 GiB, and history on disk is far below the 4 GiB data budget. The 50 never-reading viewers were told of 96,810 / 97,106 lost events.
- In the 20 s overload run (64× the profile rate, 16 terminals writing history at 4 MiB/s each), control max reached 108 ms. That run is reported as overload evidence, not as the profile. History stayed bounded by retention: 1.03 GB on disk for 16 × 64 MiB = 1.07 GB allowed.
- `max_turn_ms` is wall time including preemption; a 16 KiB turn costs about 1 ms of CPU (`parse-budgets.json`).

## DEFERRED VERIFICATION (runner lands in Todo 21)

The runner has landed, but no served route feeds `input.*` admission or attach output through terminal actors yet. `performance/fairness` is therefore a declaration listed in `conformance/scenarios/declared.json` (owner 56); it becomes executable when todo 31 assembles the binary over the input routes of todos 28–31. Attempts, verbatim (`docs/evidence/task-56/conf-*.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario performance/fairness --isolated --case all --evidence "$ATTEMPT_DIR/task-56-ovis-terminal-v1.json"
{"case":"all","error":{"detail":"performance/fairness is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"performance/fairness","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
exit=1
$ ... --case happy ...                                                  -> scenario_not_executable, exit=1
$ ... --case failure --inject '{"method":"input.raw","params":{"data_b64":"fixture:input-queue-cap-plus-one"}}' --append   -> scenario_not_executable, exit=1
```

So there is no materialized `ovis-terminal` CLI/socket invocation for this todo yet. The binary hash and request/result evidence the plan asks for are deferred to todo 31.

## QA

- Ordering (B2): `qa::ordering::*` as above; mutant M1 log committed.
- failure: `qa::failure::overflow_is_refused_before_admission_and_cancel_stays_serviceable`. `reserve(16 MiB + 1)` → `input_overflow`, scope `TerminalInput`, `held 0`, no receipt. With stdin never read: `Withdrawn`/`InFlight`, resize served, withdrawn bytes released. Stop joins with `unsent 1, cut_off stuck, writer None`. Elapsed time is printed, not asserted.
- caps: `qa::caps::*`:
  - exhausted control slots → `backpressure` (scope `SessionControl`), served once a slot frees, stop accepted without a slot;
  - output held by another terminal → nothing accepted (no `Chunk` event), accepted and parsed after release, `output_waits 1`, budget back to 0;
  - a third terminal's event retention refused, and accepted after one stops.
- history: `qa::history::*` (actor records to disk; tail read equals the bytes; read at 0 → `history_gap`; floor readable; `report.history.disk_bytes == end − floor`; evicted > 0; no fault; a terminal without history → `invalid_params`) and `history::tests::*`.
- fanout: 8 viewers share one allocation (`Arc::ptr_eq`); slow subscriber exact lost count.
- threads: DB writer busy/join/panic; blocking gate saturation; pump EOF (concurrent lag-tolerant reader) / actor-stopped.
- profile: 16 actors × 8 clients × 16 lanes at 1 MiB/s until 4 MiB. Asserts every client sees every commit, 0 lost, and every accepted byte parsed before stop. `within_contract`/`worst_ms` are reported, not asserted.

## Adversarial

- stdin non-reader: the failure and ordering tests (never-read pipes) and the real-PTY runs (emitters with `stty -echo` never read).
- misleading success: reports carry sample counts, `rejected`, `events_lost`, `output_waits`, `unsent`, `cut_off`, and history `fault`/`evicted_segments`. A read below the floor is `history_gap`, never empty success.
- stale state / impossible credit: the stream credit refusal is the landed `streams::credit` (unchanged). Session slots are returned exactly once (budget proptest).
- no sync sleeps: tests wait on replies or events with a 20 s bound. The ordering tests use the current-thread runtime instead of timing. The profile's intervals pace its workload only.

## Contract checklist

1. u64 on the wire: this PR adds no wire fields (scheduler events are in-process). `Ticket` is a newtype not yet serialized; its decimal-string encoding is for todo 31 to add when routed.
2. Boundary errors: `SchedulerError`, `ShardError`, `HistoryError`, `TuningError` are `thiserror` enums with `kind()`/`hint()`, `#[source]` on inner errors, and Display without the kind token (asserted).
3. Identities: `Ticket` newtype; `OutputOffset` reused from the VT crate (history offsets and floors included).
4. Behavior tests incl. four proptests (shard turns, budget holds, history windows) and a committed mutant run; no read-back of locally assigned literals.
5. No `#[allow]`/`#[expect]`, no TODOs.
6. Holder wire generation stays 2; checklist items unchanged.

## Risks

- `origin/integration` is at 1c21d63; per lane rules this branch does not merge it (the orchestrator union-merges).
- The scheduler is a component; served routes and `TerminalStreams` do not use it until todo 31. Conformance `performance/fairness` stays declared.
- Raw `spawn_blocking` in dispatch/streams pump (listed under Seams) is not bounded by this PR.
- Under sustained 64× overload with history, control max reached 108 ms (reported above). The profile and stress runs stay below 100 ms.
