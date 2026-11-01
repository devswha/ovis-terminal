# Task 69 adversarial pass

Native host: Linux 7.0.0-38-generic x86_64, AMD Ryzen Threadripper PRO 3995WX, runs pinned
with `taskset -c 2-123`, release build (`lto = "fat"`), 8 runtime workers, 1000 warmup samples.

## Classes from the todo block

| Class | Attack | Result |
|---|---|---|
| Hung commands: one non-reader | Client 8 subscribes to every terminal and never reads its socket; one output viewer per terminal never reads. | IDEAL run: client 8 delivered 278, lost 12144 (counted), 4097 still queued and reported undelivered when its end was hung up; the eight writable clients each delivered all 16519 committed events with zero loss and event max 3.094 ms. Stalled viewers lost 46870 output events (reported as gaps) while the 112 reading viewers lost none. `docs/evidence/task-69/bench/ideal-16pty-linux.json` `clients`, `viewers`. |
| Misleading success: delayed writer | `--inject '{"fixture":"control-writer-delay","delay_ms":150}'` delays every control-event write by 150 ms. | Exit 1, stderr `budget_exceeded: 3 budget checks failed`; `event_max_ms` 53500.2 ms over the unchanged 100 ms budget with all 1920 written samples counted (raw minimum 150710 us), plus `measured_event_samples` 0 and `writable_events_missing` 130232. The budget field still reads `admission_max_ms: 100, event_max_ms: 100`. `failure-writer-delay-150ms-linux.{json,log,samples.json}`. |
| Flaky tests: readiness barriers and monotonic timestamps | Every subscription and viewer exists before the first request; the run waits for each terminal's first output chunk (bounded 30 s) before the first lane starts; all latencies are `Instant` differences from the actor's commit instant. | The crate tests assert only host-speed-independent facts: a 150 ms delayed writer can never deliver under 100 ms, and per client `delivered + lost + undelivered` equals the committed events. |

## Relabelling and filtering guards

- `--admission-max-ms/--event-max-ms` are refused unless the shape is exactly 16 terminals,
  8 clients, 16 in flight and 1048576 B/s (`usage`); mixing a max and a p99 bound is refused.
- The IDEAL verdict reads every sample including warmup; warmup is reported as its own
  distribution, never discarded. A rejected admission is an answered request and is sampled.
- Writable-client events that were lost or never written fail the verdict
  (`writable_events_missing`), so a slow sample cannot disappear by being dropped.

## Mutants (each applied alone, then reverted; logs beside this file)

| Mutant | Killed by |
|---|---|
| `mutant-ideal-ignores-warmup.log`: IDEAL max ignores warmup samples | `the_ideal_profile_judges_warmup_samples_too` |
| `mutant-capacity-judged-on-max.log`: capacity judged on max instead of p99 | `one_slow_sample_fails_the_ideal_profile_but_not_the_capacity_p99` |
| `mutant-filter-slow-event-samples.log`: writer drops samples over 100 ms | `a_delayed_control_writer_breaks_the_event_budget_with_every_sample_kept` |
| `mutant-ideal-bounds-any-shape.log`: max bounds accepted for any shape | `the_max_bounds_judge_only_the_exact_ideal_shape` |

## Defect found during the pass

The first IDEAL run reported `runtime_cpu_ms` 220 for a 13.7 s run: Linux CPU time was read
from `/proc/<pid>/schedstat`, which covers the main thread only. Fixed to utime+stime of
`/proc/<pid>/stat` (every thread), covered by
`proc_stat_cpu_sums_user_and_system_ticks_after_the_command_name`; the committed runs are the
re-runs after the fix (`runtime_cpu_ms` 10260).
