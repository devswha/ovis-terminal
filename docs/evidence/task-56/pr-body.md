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

- `crates/ovis-terminal-vt/src/shard/`: `Shard` feeds queued output chunks (shared `Arc<[u8]>`, never copied) to a `Vt` in turns of at most `ParseBudget` bytes; `push` refuses past `PendingCap` with `ShardError::Full` (kind `backpressure`) before queuing. Defaults: 16 KiB per turn (chosen from the measurement below) and 1 MiB pending, so 50 terminals stay inside the 64 MiB session backlog.
- `crates/ovis-terminal-server/src/streams/scheduler/` (new child of `streams`; viewer/pump untouched):
  - `spawn_terminal`: one actor per terminal on the multi-thread runtime. Its `select!` is biased: control queue (cancel/resize/stop, its own 16-slot reserved queue), then admissions, then output intake, then one parse turn followed by `yield_now`.
  - Input: `TerminalHandle::reserve(len)` reserves against the per-terminal (`input_queue_bytes`, 16 MiB) and session (`session_input_bytes`, 64 MiB) `ByteBudget`s **before** the caller allocates the bytes. A refusal is `SchedulerError::Exhausted` (kind `input_overflow`, retryable) naming the scope and holding nothing. A session refusal returns the terminal reservation.
  - A separate writer task drains the queue into the program's stdin, so a program that never reads stdin stalls only that task; `cancel` withdraws queued entries (`Withdrawn`) or reports `InFlight`; stop aborts a writer stuck mid-write and reports `unsent` entries and the `cut_off` ticket (outcome unknown).
  - Committed control events (`Admitted`/`Writing`/`Written`/`Withdrawn`/`Resized`) and output fanout (`OutputEvent::Chunk` sharing one `Arc` per chunk, `Parsed`) are separate bounded broadcast channels. A lagging subscriber gets an explicit `Lagged(n)` loss count; output volume cannot push out control events.
  - `PumpThread`: owned named thread per blocking reader (portable-pty master), bounded feed (a full feed blocks the pump and so the program), EOF/EIO end, joined.
  - `DbWriter<S>`: dedicated named thread draining a bounded `sync_channel`; a full queue is `WriterBusy` (`backpressure`) before acceptance; `shutdown` joins and returns the state; a panicking job is reported as `internal`.
  - `BlockingGate`: bounded `spawn_blocking` admission (semaphore `try_acquire`), one past the limit is `BlockingSaturated`, never queued.
- `bench/tuning/` (`fairness-tuning` binary; library module via `#[path]` like `bench/harness`): `parse`, `load <terminals> <viewers> <bytes/s> <seconds> [parse-budget [workers]]` over real portable-pty terminals, and `emit`.
- `conformance/scenarios/performance/fairness.json` + `declared.json` entry (owner 56).
- macOS stress job `fairness` (option + job appended at the end of `.github/workflows/probe-stress-macos.yml`) was committed in ba9f3e4 and dispatched on this branch: https://github.com/devswha/ovis-terminal/actions/runs/41236348670. The file was then restored to base in the follow-up commit, because the shared options line conflicts with integration 436506e; the orchestrator union-merges the job block from ba9f3e4.

## Seams

- New public API: `ovis_terminal_vt::{ParseBudget, PendingCap, Shard, ShardError, Turn}`; `ovis_terminal_server::streams::scheduler::*` (new `pub mod`).
- No landed public API changed. Holder wire generation stays 2.
- `bench` gains dependencies on `ovis-terminal-protocol`, `ovis-terminal-server`, `tokio` (`DEPENDS_ON` updated).

## After

Native host: Linux 7.0.0-38-generic x86_64, AMD Ryzen Threadripper PRO 3995WX (128 cores, shared host, load average ~31), rustc 1.99.1 (`docs/evidence/task-56/tuning/host.log`).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                                  exit=0
$ cargo test -p ovis-terminal-vt -p ovis-terminal-server --all-targets                           exit=0 (161 passed)
$ cargo clippy -p ovis-terminal-vt -p ovis-terminal-server --all-targets -- -D warnings          exit=0
$ bash scripts/check.sh                                                                          exit=0 (nextest 828 passed, 1 skipped; hygiene 0 violations)
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked   exit=0
```

In-crate profile (`docs/evidence/task-56/scheduler-qa.log`, debug build, 4 workers, 4 KiB test parse budget):

```
{"profile":"16-pty/8-client/16-inflight/1MiB-s","requests":6560,"commits":4928,"admissions":3280,"controls":3280,"admission_max_ms":2.063,"control_max_ms":1.868,"events":39424,"event_max_ms":3.050,"events_lost":0,"rejected":0,"output_bytes":4216832,"parsed_bytes":4216832,"parse_turns":4118,"max_turn_ms":2.453}
```

Real PTYs, release (`docs/evidence/task-56/tuning/*.json`, each with its `.log` holding the command and exit status):

| run | workers | turn budget | admissions | admission max | control max | event max | events lost | max turn | RSS peak |
|---|---|---|---|---|---|---|---|---|---|
| 16 PTYs, 1 MiB/s (profile) | 4 | 16 KiB | 8016 | 1.29 ms | 5.78 ms | 9.29 ms | 0 | 4.31 ms | 247 MB |
| 16 PTYs, 64 MiB/s overload — **before** (8 MiB turns) | 4 | 8 MiB | 3912 | **319.7 ms** | 368.5 ms | 276.0 ms | 0 | 99.8 ms | 234 MB |
| 16 PTYs, 64 MiB/s overload — **after** | 4 | 16 KiB | 7985 | **62.1 ms** | 70.6 ms | 50.8 ms | 0 | 16.4 ms | 265 MB |
| 50 PTYs, 400 viewers, 1 MiB/s | 4 | 16 KiB | 8000 | 51.8 ms | 54.1 ms | 52.8 ms | 0 | 41.5 ms | 700 MB |
| 50 PTYs, 400 viewers, 1 MiB/s | 8 | 16 KiB | 8016 | 32.3 ms | 63.4 ms | 27.7 ms | 0 | 192.2 ms | 701 MB |

- Every run uses 16 lanes firing together every 10 ms (16 requests in flight) and eight control-event clients per terminal. `admission_max`/`event_max` are the maximum over **every** sample, not a percentile; sample counts are in the JSON (`admissions`, `controls`, `events`), and `rejected` is 0 in every run.
- 50×400: RSS peak (harness + 50 emitter processes) 700 MB < 1 GiB. The retained-data bound reported as `data_bound_bytes` is 382 MB < 4 GiB. One viewer per terminal never reads; those 50 stalled viewers were told of 87,055 / 88,133 lost output events (explicit `Lagged` floor) instead of growing memory.
- `max_turn_ms` is wall time and includes OS preemption on this shared host: a 16 KiB turn costs about 1 ms of CPU (`parse-budgets.json`), and the 192 ms outlier in the 8-worker stress run is a preempted worker. Control requests still stayed at or below 63 ms because other workers served them.
- Overload before/after is the latency evidence: the same 64 MiB/s on 4 workers breaks the 100 ms contract with 8 MiB turns (320 ms) and holds it with 16 KiB turns (62 ms, with 31,001 output-backpressure waits instead).

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

- happy: `scheduler::qa::profile::sixteen_ptys_eight_clients_keep_every_control_within_100_ms`. 16 actors, 8 clients × 2 lanes, output paced at 1 KiB per terminal every 16 ms (1 MiB/s aggregate) until at least 4 MiB was sent. It asserts the max of every admission, control and committed-event sample is ≤ 100 ms, every client saw every commit (`events == commits × 8`), 0 lagged, and parsed ≥ 4 MiB.
- failure: `scheduler::qa::failure::overflow_is_refused_before_admission_and_cancel_stays_serviceable`. A 1 KiB pipe nobody reads is the program's stdin. `reserve(16 MiB + 1)` → `input_overflow`, scope `TerminalInput`, `held: 0`, `input_held() == 0`, no receipt. Then 3 × 4 KiB entries are admitted (each ≤ 100 ms). After the product's `Writing{first}` event, `cancel(second) == Withdrawn`, `cancel(first) == InFlight` and a resize are served together within 100 ms, and the withdrawn bytes are released. Stop joins with `admitted 3, withdrawn 1, unsent 1, cut_off first, writer None`.
- Also: `the_session_bound_refuses_and_returns_the_terminal_reservation` (scope `SessionInput`, terminal reservation returned, over-reservation `invalid_params` releases), `every_refusal_maps_to_a_typed_retryable_wire_kind` (wire `kind`/`retryable` via `BoundaryError::data`, Display never repeats the kind), `every_viewer_receives_the_same_shared_chunk` (8 viewers, `Arc::ptr_eq`), DB writer busy/join/panic, blocking gate saturation, pump EOF/actor-stopped, budget proptest (held never passes the limit, a refusal holds nothing), shard proptest (budgeted turns over any chunking equal one whole feed: offset, screen, cursor, modes, query offsets, OSC; every turn ≤ budget; turn count = ⌈len/budget⌉).

## Adversarial

- Hung commands / stdin non-reader: the failure test (never-read pipe) and the real-PTY runs (emitters run `stty -echo` and never read stdin) keep cancel/resize within bound. Stop never waits on a stuck write.
- Misleading success: every report carries sample counts, `rejected`, `events_lost`, `output_waits`, `unsent`/`cut_off`. Output still queued at stop is reported as not parsed (`parsed <= output`), never claimed.
- Stale state / impossible credit: the stream credit refusal for acknowledgements beyond sent or regressed is the landed `streams::credit` (todo 23), which this PR does not change. Scheduler tickets are never revived after `NotQueued`.
- No timing sleeps for synchronization: tests wait on product replies or events with a 20 s bound. The two `interval`s in the profile test pace the declared workload (1 MiB/s, 16 requests per tick); they are not synchronization.

## Contract checklist

1. u64 on the wire: this PR adds no wire fields (scheduler events are in-process). `Ticket` is a newtype not yet serialized; its decimal-string encoding is for todo 31 to add when routed.
2. Boundary errors: `SchedulerError`, `ShardError`, `TuningError` are `thiserror` enums with `kind()`/`hint()`, `#[source]` on inner errors, and Display without the kind token (asserted).
3. Identities: `Ticket` newtype; `OutputOffset` reused from the VT crate.
4. Behavior tests incl. two proptests; no read-back of locally assigned literals.
5. No `#[allow]`/`#[expect]`, no TODOs.
6. Holder wire generation stays 2; checklist items unchanged.

## Risks

- `origin/integration` moved after dispatch (81cb6bf → 436506e); per lane rules this branch does not merge it. `git merge-tree` against 436506e conflicted only in the shared stress workflow, which is now restored to base; the re-check is clean.
- The scheduler is not yet wired into served routes or into `TerminalStreams` (todo 31 assembles). The in-crate tests prove the component, not the served binary.
- Disk-backed history is not part of this PR: retention here is bounded memory with explicit loss counts (`Lagged`, `unsent`, `cut_off`). Disk spill of history remains with the checkpoint/history owners (todo 49).
- The debug-build profile test asserts a hard 100 ms max on shared CI runners. Locally on 2 pinned cores it stayed ≤ 77 ms in 12 of 13 runs. One closed-loop variant hit 113 ms before lanes were paced, and one 1-core run hit 161 ms. The macOS `fairness` stress run https://github.com/devswha/ovis-terminal/actions/runs/41236348670 (Apple M1 virtual, 3 vCPU) passed iterations 1–9 (admission max ≤ 22.5 ms, event max ≤ 14.8 ms) and failed iteration 10. In that iteration admission was 279.5 ms, control 298.7 ms and event 247.3 ms, with the longest 4 KiB parse turn at 217.0 ms of wall time. That turn costs well under 1 ms of CPU, so the VM descheduled the workers: every metric moved together, and turns and events were not lost or misordered. The contract was not loosened; the assertion messages now print these figures so a failing CI log explains itself. PR CI (ubuntu/macos/windows test, check.sh) passed.
