# fix(probes): await thread teardown with a bounded deadline in the stall no-leak check

Plan todo 72 (wave 4, follow-up to todo 12). Closes V-220; delivers IDEAL-108 (regression evidence stays deterministic). Unblocks PR #10 CI.

## Cause
`unread_input_stays_killable` compared one `/proc/self/task` read to a baseline right after `probe_stalled_writer` returned. On Linux `pthread_join` returns once the kernel clears the child tid, slightly before `/proc/self/task/<tid>` disappears, so the joined `probe-stall-report` thread was sometimes still listed.

## What changed (only `probes/platform/tests/unix_failure.rs`)
- `thread_names(prefix)` lists thread names under `/proc/self/task`; where that listing fails it returns the `io::ErrorKind` (`NotFound` on macOS, which has no `/proc`).
- `await_threads_at_most(prefix, baseline, deadline)` re-reads until at most `baseline` threads match, spinning with `thread::yield_now` (no fixed sleep); at the 2 s `TEARDOWN_DEADLINE` it returns `Teardown::Leftover(names)` and the probe assertion fails with them. The outcome is three-valued: `Gone`, `Leftover(names)` and `Unavailable { reason }`; `Unavailable` is never folded into `Gone` (PR CI on macOS showed that folding it silently passes the check, the same class as the original bug). The baseline comparison is unchanged (count must be <= the count before the probe); a thread that never exits still fails. `on_this_os` states the expected outcome per OS without skipping an assertion: Linux -> `Gone`/`Leftover(["t72-parked"])`, non-Linux -> `Unavailable { reason: NotFound }`. In `unread_input_stays_killable` Linux asserts `Gone` and macOS asserts the typed `Unavailable`, so the formerly silent `None == None` comparison is now an explicit assertion.
- Two unit tests: an exited-and-joined thread is reported gone; a thread parked on a channel nothing signals is reported as `["t72-parked"]` once the deadline (200 ms in the test) passes, then it is released, joined and confirmed gone. The parked thread signals readiness after it is spawned and named, and the test waits for that signal before calling the helper, so it does not depend on start-up timing.
- No holder code touched, no `#[allow]`. The helper lives in the test file because it reads `/proc` for test purposes only and has no non-test caller; no `probes/platform/src` file was needed.

## Before
- PR #10 CI (jobs 124337043222 and 124339775023, runs on 54f19c9 and 9b7d2f1): `the probe left a thread behind`, left `Some(1)` right `Some(0)`.
- Reproduction [join-race-probe.rs](join-race-probe.rs), output [join-race-probe.out](join-race-probe.out) (rustc -O, Linux x86_64): idle, 200000 joins, `leftover_after_join=297` (max 49 yields until gone); four copies at once, 18011-18476 leftovers per 200000 (about 9%), max 87 yields.
- Negative evidence through the REAL helper code path ([negative-evidence/](negative-evidence/)): `run.sh` extracts the helper region of `unix_failure.rs` verbatim (`helper_extract.rs`) and `harness.rs` (a throwaway program, not a test, not in the workspace) loops 100000 times: spawn a `probe-stall-report` thread, join it, then check. Four copies at once (Linux x86_64):
  - single read (`thread_names` once, the old check): `leftover=8203, 8367, 8259, 8380` of 100000 each ([single-4way.log](negative-evidence/single-4way.log)).
  - bounded wait (`await_threads_at_most` with `TEARDOWN_DEADLINE`): `leftover=0` in all four ([bounded-4way.log](negative-evidence/bounded-4way.log)).
- The committed probe with the deadline reverted to `Duration::ZERO` (a single read), run 50 times 8 at a time, did **not** fail once (50/50 exit 0, [revert-committed-probe-50x.log](negative-evidence/revert-committed-probe-50x.log)): the real probe joins one thread per run, which is too rare to hit here. Recorded as observed; the harness above is the negative evidence.

## After (Linux x86_64, rustc/cargo 1.99.1)
| Command | Exit | Result |
|---|---|---|
| `P=8 bash docs/evidence/task-72/stress.sh 50 stress.log` (50 `cargo nextest run -p ovis-terminal-holder --test platform_probe --locked`, 8 concurrent) | 0 | total=50 ok=50 failed=0 ([stress.log](stress.log)); no fixture process left behind afterwards |
| CI `probe (macos-latest)` on 1e397d6 | pass | runs `platform_probe` including `failure` and both helper unit tests; the typed `Unavailable` path passes on macOS |
| CI `test (macos-latest)` on 1e397d6 (two attempts) | **fail** | NOT caused by this change: `platform_probe happy` panics at `unix_happy.rs:45` (`left: Grid { columns: 80, rows: 24 } right: Grid { columns: 132, rows: 40 }`, the post-resize size read in the todo-12 happy probe). That file is outside this todo's write scope and is left unchanged; it needs its own follow-up. |
| (superseded) first head 35418a5 macOS | the first head (35418a5) failed there with `left Ok(()) right Err(["t72-parked"])`; this head asserts the typed `Unavailable` instead. Not run locally (Linux host). |
| `bash docs/evidence/task-72/negative-evidence/run.sh` | 0 | single read: 8.2-8.4k leftovers per 100000 under 4-way load; bounded wait: 0 (above) |
| `bash scripts/check.sh` | 0 | all steps passed ([check.log](check.log)); includes both helper unit tests |

DEFERRED VERIFICATION (runner lands in Todo 21): `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-12 --isolated --case happy --evidence ...` exits 2 with `conformance: no scenarios are registered in this build` ([conformance-attempt.log](conformance-attempt.log)).

## QA
- Happy: 50/50 under 8-way load (above).
- Failure: `a_thread_that_never_exits_is_reported_at_the_deadline` asserts the leftover is reported by name only after the deadline elapsed, and the thread is then released and joined.

## Adversarial
- flaky_tests: 50x under load; the standalone race reproduction under 4-way load.
- timing: the deadline path is executed by the parked-thread test. While exploring I also found that my first parked-thread test raced thread start-up under heavy load (the thread name is set by the new thread); fixed with the readiness signal.
- A 400-run 32-way exploratory run was aborted and discarded: it overloaded the host and failed unrelated `happy` steps, so it says nothing about this change.
- Leak finding: see [leaked-fixtures.md](leaked-fixtures.md). Two `probe-holder serve` fixtures from `happy` outlived that aborted run because this lane killed nextest mid-test with `pkill`; not observed on completed runs. Not changed here.

## Contract checklist
1-3 (decimal-string u64 JSON, typed boundary errors with kind/hint, semantic identity newtypes): not exercised; this todo adds no wire/JSON, error type or identity. 4: tests assert behavior (gone vs leftover names at a deadline); no proptest because there is no round trip. 5: no `#[allow]`/`#[expect]`, no TODO, helper is the single caller's need. 6: evidence above states what is and is not shown.

## Risks
On macOS the thread-leak check cannot run (no `/proc/self/task`); it now asserts `Unavailable` explicitly rather than passing silently, so a macOS leak remains undetected by this probe. The 2 s deadline is generous against the observed <=154 yields; a thread that genuinely never exits still fails the probe.
