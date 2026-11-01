# Task 69 adversarial pass (round 2)

Native host: Linux 7.0.0-38-generic x86_64 (AMD Ryzen Threadripper PRO 3995WX). Runs were pinned with `taskset -c 2-123`, used a release build (`lto = "fat"`) with 8 runtime workers and 1000 warmup samples, and ran against binary `de70674c12b023ad71908dba5340c1f78ebbb4832072e34718ef19a8a1286211`. Every number below comes from `docs/evidence/task-69/bench/*.json`, produced by the round-2 head code.

## Round-2 gate findings and their fixes

### B1: the verdict is now bound to what the run measured

Mutants M-a, M-b and M-c from the gate review are applied exactly as reviewed. Each is now killed by name:

| Mutant (log) | Change | Killed by |
|---|---|---|
| `r2-mutant-M-a-fleet-capped-at-two.log` | `fleet.rs`: `0..plan.shape.terminals.min(2)` | `bound::the_verdict_reads_the_fleet_that_actually_ran`, `runs::every_committed_event_is_delivered_lost_or_undelivered_for_each_client` |
| `r2-mutant-M-b-admission-latency-zero.log` | `lanes.rs`: admission sample `Duration::ZERO` | `bound::an_admission_delay_breaks_the_admission_budget_through_the_run` |
| `r2-mutant-M-c-verdict-rss-zero.log` | `account.rs` `observed()`: `rss_peak_bytes: 0` | `bound::footprint_limits_below_the_measured_footprint_fail_the_run` |

The `bound::*` tests drive a real in-process run through `drive::run`, never through `judge` on a hand-built value.

- **M-b:** an `admission-delay` fixture of 150 ms must fail `admission_max_ms`. The test checks that the judged value equals the reported `admission.measured.max_ms` and that every raw admission sample is at least 150000 µs.
- **M-c:** `rss_limit_bytes = 1` and `disk_limit_bytes = 1` must fail, with the judged values equal to `footprint.rss_peak_bytes` and `footprint.disk_bytes`.
- **M-a:** the verdict now reads the realized fleet. That is:
  - terminals whose actor parsed output;
  - viewers alive at the end, with reading viewers counted only once they received output;
  - writable clients that delivered a measured event;
  - lanes that sampled.

  It compares these against the requested shape (`terminals_realized`, `viewers_realized`, `clients_realized`, `lanes_realized`). Both shapes are in every report.

M-a end to end on the real PTY path (`bench/mutant-M-a-fleet-capped-at-two-linux.{json,log}`): exit 1 after 13 s with `budget_exceeded`, `terminals_realized` 2 of 50 and `viewers_realized` 16 of 400. In round 1 this hung.

### B2: the unbounded teardown that hung M-a, and the macOS lane

**Root cause, captured on the M-a binary.** After 60 s the stalled process had no children. Both `terminal50-stdin` forwarder threads were sleeping in `wait_woken`, a TTY write waiting for buffer room, while the main thread was parked in a join. The emitter never read stdin, so admitted input filled each PTY. Once teardown killed the emitter, the forwarder was stuck writing to that PTY forever, and `settle` joined it with no bound.

At 2 terminals each PTY receives 25 times the input of the 50-terminal run, which is why M-a hit it. macOS PTY buffers are smaller, so the same wait is the likely cause of run 41327470005 sitting in `run lane terminal50` for over 30 minutes.

**Fixes:**

- **Emitter:** it now reads and discards stdin, so admitted input is consumed and a PTY cannot wedge its forwarder.
- **Bounded teardown:** every teardown step is bounded by `settle::bounded`, and the 10 s bound is held in `Plan::teardown`. That covers program stop, actor join, stdin-forwarder join, client finish and viewer joins. A step that outlives its bound fails the run as `shutdown_stalled`, naming the stage and the terminal or client index.
- **Runtime:** it shuts down with `shutdown_timeout(5 s)`, so a stuck blocking thread cannot hold the process.
- **Test:** `bound::a_teardown_step_that_never_finishes_fails_naming_its_terminal` pins the stall path with a never-ready future.

## Classes from the todo block

**Hung commands: one non-reader.** Client 8 subscribes to every terminal and never reads its socket, and one output viewer per terminal never reads. In the IDEAL run:

- Client 8 delivered 278, lost 12144 (counted) and left 4097 undelivered when its end was hung up.
- The 8 writable clients each delivered all 16519 committed events with 0 lost, and the event max was 3.475 ms.
- The 16 stalled viewers lost 47248 output events, reported as gaps, while the 112 reading viewers lost none.

**Misleading success: delayed writer.** `{"fixture":"control-writer-delay","delay_ms":150}` gives exit 1 with `budget_exceeded`:

- `event_max_ms` is 53492 ms against the unchanged 100 ms budget, with all 1920 written samples kept.
- Also reported: `measured_event_samples` 0, `writable_events_missing` 130232 and `clients_realized` 0 of 8.

**Misleading success: delayed admission (new).** `{"fixture":"admission-delay","delay_ms":150}` gives exit 1 with `budget_exceeded`: `admission_max_ms` 153.8 ms with all 11015 samples over budget. The budget is unchanged at 100/100.

**Relabelling.** `--terminals 49` with the p99 bounds is refused with exit 2 `usage` (`bench/capacity-49pty-refused-linux.log`), just as 15 terminals with the max bounds are.

**Flaky tests: readiness barriers and monotonic timestamps.** Every subscription and viewer exists before the first request, the run waits for each terminal's first output (bounded at 30 s), and latencies are `Instant` differences from the actor's commit instant. The tests assert only facts that host speed cannot change:

- a 150 ms delay can never pass a 100 ms bound;
- limits of 1 byte are always exceeded;
- an in-process fleet either runs at full size or the verdict says so;
- a pending future is always stalled.

## Earlier mutants, re-applied on the round-2 code

| Mutant (log) | Killed by |
|---|---|
| `r2-mutant-ideal-ignores-warmup.log` (`verdict.rs`) | `the_ideal_profile_judges_warmup_samples_too` |
| `r2-mutant-capacity-judged-on-max.log` (`verdict.rs`) | `one_slow_sample_fails_the_ideal_profile_but_not_the_capacity_p99` |
| `r2-mutant-filter-slow-event-samples.log` (`delivery.rs`) | `runs::a_delayed_control_writer_breaks_the_event_budget_with_every_sample_kept` |
| `r2-mutant-shape-lock-removed.log` (`cli.rs`, both shape locks) | `the_max_bounds_judge_only_the_exact_ideal_shape` |

## Also folded in

- **R3:** `probes/quality/tests/evidence_tree.rs` compares the exempted count with `exempted_paths()?.len()` instead of a literal.
- **N1:** `count(.., at_most: bool)` is replaced by the named `at_most` and `at_least` in `verdict.rs`.
- **Capacity shape:** the capacity profile is locked to 50 terminals, 8 clients, 16 in flight and 1048576 B/s.
- **Baseline path:** the durable Before copy is `docs/evidence/baseline/smoke/hc-s06-wait-output.log:6`. The plan's `docs/evidence/baseline/hc-s06-wait-output.log` lacks `smoke/`.
