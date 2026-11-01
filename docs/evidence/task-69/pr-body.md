test(terminal50): benchmark fifty terminals against published latency budgets

Plan todo 69 (wave 17). Closes GAP-035, GAP-061, GAP-074, GAP-143; delivers IDEAL-027, IDEAL-041, IDEAL-051, IDEAL-116; enforces EXCL-019, EXCL-024, EXCL-025 and does not reproduce V-RT-HC-01, V-RT-HD-11, V-4085.

## Round 2 (gate review of 1d1e0a8)

- **B1, verdict bound to measurements:** fixed. The verdict now reads the realized fleet as well as the measured samples and footprint. Real in-process runs through `drive::run` must fail on:
  - an injected admission delay;
  - RSS and disk limits below the measured footprint;
  - a fleet smaller than requested.

  The reviewed mutants M-a, M-b and M-c are each killed by a named test (`docs/evidence/task-69/adversarial.md`, `r2-mutant-*.log`).
- **B2, unbounded teardown and macOS:** fixed. The M-a hang was captured with both stdin forwarder threads sleeping in `wait_woken`, writing to a full PTY after their emitter was killed, while teardown joined them with no bound. The emitter now consumes stdin, every teardown step is bounded (`shutdown_stalled` names the stage and terminal), and the runtime shuts down with a timeout. M-a end to end now exits 1 after 13 s with `terminals_realized` 2/50, where before it hung. The macOS lane is re-dispatched on the round-2 head (see After).
- **Also folded in:**
  - R3: `Some(28)` becomes `exempted_paths()?.len()`.
  - N1: `count(.., at_most: bool)` becomes the named `at_most` and `at_least`.
  - The capacity profile is locked to 50/8/16/1048576; `--terminals 49` exits 2 `usage`.
  - The baseline path is cited as `docs/evidence/baseline/smoke/...`.
  - New `admission-delay` failure fixture.

## What ships

`bench/terminal50/` adds the `terminal50` binary to `ovis-terminal-bench`:

- **IDEAL profile** (`--admission-max-ms/--event-max-ms`): accepted only for the exact shape of 16 terminals, 8 clients, 16 in flight and 1048576 B/s. It is judged on **every** admission and event sample, warmup included. Label `ideal-16pty`.
- **Capacity profile** (`--admission-p99-ms/--event-p99-ms`): accepted only for 50 terminals, 8 clients, 16 in flight and 1048576 B/s, and judged on the p99 of the measured samples. Label `capacity-release-target`, never the IDEAL assumption. It runs 400 output viewers over shared `Arc` chunks and one shared set of session budgets.
- **What is measured:**
  - Admission is reserve+admit to receipt. It never waits for delivery (EXCL-024), and rejections are sampled too.
  - Events are actor commit to the line accepted by each client's own Unix socket. Each client has its own bounded queue and writer, and no lock is held across a write (V-4085). A ninth client never reads and loses only its own events, counted (IDEAL-041). Every subscription exists before the load (EXCL-025).
  - Reports give p50/p95/p99/max for warmup and measured samples, rejected work by kind, CPU of the runtime and its PTY programs, peak RSS, disk, per-client and per-viewer lag, and the **requested and realized** shapes.
  - Reports also state the host, the build, the binary under test (sha256, size, `--version`) and the warmup count. Raw samples go beside the evidence as `<stem>.samples.json`.
- **Verdict:** both profiles also fail on:
  - lost or unwritten events to a writable client;
  - fewer samples than asked;
  - RSS over `--rss-limit-mib` (1024) or disk over `--disk-limit-mib` (4096);
  - fewer terminals, viewers, writable clients or lanes than requested.

  A broken budget prints the report, then `budget_exceeded: ...` on stderr, and exits 1. A stalled teardown exits 1 with `shutdown_stalled`.
- **Failure fixtures:** `{"fixture":"control-writer-delay","delay_ms":N}` and `{"fixture":"admission-delay","delay_ms":N}`. The shape is closed; anything else is `invalid_inject` (exit 2).

## Before

`docs/evidence/baseline/smoke/hc-s06-wait-output.log:6` (RT-HC-01; the original was `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:6`; the plan's `docs/evidence/baseline/hc-s06-wait-output.log` lacks `smoke/`):

```
$ <run-root>/bin/hc send sh1 sleep 1; echo READY_MARK
--- stdout
{"started":false,"status":"idle","turn":1}
--- stderr
--- exit=0 elapsed_ms=5213
```

Send admission waited 5213 ms. Gap baseline: GAP-035, GAP-061, GAP-074, GAP-143.

## After

All runs below are on the round-2 code. The native host was Linux 7.0.0-38-generic x86_64 (AMD Ryzen Threadripper PRO 3995WX), with runs pinned by `taskset -c 2-123`. The bench was a release build (`lto = "fat"`) with 8 workers and 1000 warmup samples. The binary under test was `target/release/ovis-terminal` (`ovis-terminal 0.1.0`, sha256 `de70674c12b023ad71908dba5340c1f78ebbb4832072e34718ef19a8a1286211`), built by `cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal` (exit 0). Evidence is in `docs/evidence/task-69/bench/`.

**IDEAL profile, 16 PTYs**: exit 0, verdict passed (`ideal-16pty-linux.*`).

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --evidence "$ATTEMPT_DIR/task-69-ideal-profile.json"
```

| | count | p50 ms | p95 ms | p99 ms | max ms |
|---|---|---|---|---|---|
| admission warmup | 1000 | 0.004 | 0.006 | 0.009 | 2.423 |
| admission measured | 10015 | 0.004 | 0.006 | 0.008 | 0.449 |
| event warmup | 11968 | 0.052 | 0.777 | 1.683 | 3.136 |
| event measured | 120184 | 0.045 | 0.218 | 1.000 | 3.475 |
| control (resize/cancel) | 11008 | 0.233 | 1.716 | 2.771 | 4.854 |

- Requested equals realized: 16 terminals, 128 viewers, 8 clients, 16 lanes.
- 11015 admitted, 0 rejected, 16519 events committed.
- Each writable client delivered 16519 with 0 lost and 0 undelivered. The non-reader delivered 278, lost 12144 and left 4097 undelivered.
- The 112 reading viewers lost 0 events; the 16 stalled viewers lost 47248, reported as gaps.
- Peak RSS 287.6 MB; disk 14.7 MB. CPU: runtime 11050 ms, programs 930 ms, over 13761 ms wall.

**Capacity profile, 50 PTYs, reported separately**: exit 0, verdict passed (`capacity-50pty-linux.*`).

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 50 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-p99-ms 100 --event-p99-ms 100 --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json"
```

- Admission measured: p99 0.008 ms, max 0.091 ms (10015 samples).
- Event measured: p99 0.147 ms, max 0.410 ms (120184 samples).
- Realized equals requested: 50 terminals, 400 viewers (350 reading with 0 lost, 50 stalled with 137163 lost as gaps), 8 clients, 16 lanes.
- Peak RSS 758.1 MB (≤ 1 GiB); disk 15.0 MB (≤ 4 GiB); max parse turn 49.1 ms.

**Capacity profile with 49 terminals**: exit 2, `usage: ... the p99 bounds judge only the capacity shape: 50 terminals, 8 clients, 16 in flight, 1048576 B/s` (`capacity-49pty-refused-linux.log`).

**Failure, delayed control writer**: exit 1 (`failure-writer-delay-150ms-linux.*`).

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --inject '{"fixture":"control-writer-delay","delay_ms":150}' --evidence "$ATTEMPT_DIR/task-69-failure.json"
budget_exceeded: 4 budget checks failed (read verdict.violations in the evidence)
violations: event_max_ms 53492.3 ms over 100 (1920 samples over, all kept); measured_event_samples 0; writable_events_missing 130232; clients_realized 0 of 8
```

**Failure, delayed admission**: exit 1 (`failure-admission-delay-150ms-linux.*`).

```
... --inject '{"fixture":"admission-delay","delay_ms":150}' --evidence "$ATTEMPT_DIR/task-69-failure-admission.json"
budget_exceeded: 1 budget checks failed (read verdict.violations in the evidence)
violations: admission_max_ms 153.8 ms over 100 (11015 samples over, all kept)
```

In both failures the budget field still reads `admission_max_ms: 100, event_max_ms: 100`: nothing is filtered and nothing is widened.

**Reviewed mutant M-a end to end** (fleet capped at 2, built as a mutant binary and reverted): exit 1 after 13 s with `budget_exceeded`, `terminals_realized` 2 of 50 and `viewers_realized` 16 of 400 (`mutant-M-a-fleet-capped-at-two-linux.*`). It completes; in round 1 it hung.

**Crate tests:** `cargo nextest run --locked -p ovis-terminal-bench --no-tests=fail` ran 18 tests, all passed. The round-2 additions are:

- `bound::an_admission_delay_breaks_the_admission_budget_through_the_run`;
- `bound::footprint_limits_below_the_measured_footprint_fail_the_run`;
- `bound::the_verdict_reads_the_fleet_that_actually_ran`;
- `bound::a_teardown_step_that_never_finishes_fails_naming_its_terminal`;
- `shape::fewer_terminals_viewers_clients_or_lanes_than_requested_fail_the_verdict`;
- capacity 49 refused and `admission-delay` accepted, inside the existing CLI tests.

**Mutants:** seven, each applied alone, each killed by a named test, and each reverted: M-a, M-b and M-c from the review plus four earlier ones re-applied (`docs/evidence/task-69/r2-mutant-*.log`, with the table in `adversarial.md`).

**Gates:** `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exits 0 (`docs/evidence/task-69/check.log`). The Windows pre-check (`RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and `cargo clippy ... -D warnings`) exits 0 for both (`windows-precheck.log`).

**macOS native:** stress lane `terminal50` runs, three times on macos-latest, the crate tests, both profiles, both failures and the 49-terminal refusal. It was re-dispatched on the round-2 head: https://github.com/devswha/ovis-terminal/actions/runs/41332637305 (head 97fabf1; the round-1 run 41327470005, stuck in the old unbounded teardown, was cancelled). Result: **success, `RESULT 3/3`** on macOS 26.6.2 arm64 (Apple M1, virtual), rustc 1.99.1, with crate tests 16/16 (`docs/evidence/task-69/macos/terminal50-stress-macos-41332637305.log`):

| iteration | IDEAL 16 PTY: admission max / event max | capacity 50 PTY: admission p99 / event p99 | writer delay | admission delay | 49 terminals |
|---|---|---|---|---|---|
| 1 | 12.43 / 30.66 ms, passed | 3.70 / 9.34 ms, passed | exit 1 budget_exceeded | exit 1 budget_exceeded | exit 2 usage |
| 2 | 4.50 / 9.44 ms, passed | 1.65 / 2.74 ms, passed | exit 1 | exit 1 | exit 2 |
| 3 | 3.77 / 8.99 ms, passed | 1.01 / 1.89 ms, passed | exit 1 | exit 1 | exit 2 |

Realized equalled requested in every passing run (16/128/8/16 and 50/400/8/16). Peak RSS was about 0.29 GB at 16 PTYs and about 0.76 GB at 50 PTYs.

## DEFERRED VERIFICATION

`performance/terminal50` is a declared-only descriptor listed in `conformance/scenarios/declared.json`, following the precedent of `performance/fairness` from todo 56. The gate accepted this as deferred, with a mandatory re-verify of happy, failure and all once todo 31 lands, driving the real endpoint.

The binary on integration 95e45cc routes no `input.*` or `events.*` call to scheduler actors until todo 31. The benchmark therefore measures those actors in-process on real PTYs, and its `surface` field says so. Verbatim attempts (`docs/evidence/task-69/conformance-deferred.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/release/ovis-terminal --scenario performance/terminal50 --isolated --case happy --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"performance/terminal50 is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},...,"result":"failed",...}
--- exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/release/ovis-terminal --scenario performance/terminal50 --isolated --case failure --inject '{"fixture":"control-writer-delay","delay_ms":150}' --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"performance/terminal50 is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},...,"result":"failed"}
--- exit=1
```

`--case all` gives the same result. The conformance acceptance (exit 0, failed:0, skipped:0) is **not met** by this PR.

## Seams

Files outside the write scope (`bench/terminal50/`, `conformance/scenarios/performance/terminal50/`):

- `bench/Cargo.toml`: the `terminal50` `[[bin]]`, plus the `clap` and `sha2` workspace dependencies.
- `bench/src/lib.rs`: the `terminal50` mod line.
- `Cargo.lock`: two dependency edges of `ovis-terminal-bench`; no new crate.
- `conformance/scenarios/performance/terminal50.json`: the runner maps the id `performance/terminal50` to this file.
- `conformance/scenarios/declared.json`: the declared-only entry.
- `probes/quality/evidence-exemptions.json`: nine entries for the benchmark reports and raw samples, which are not conformance-runner evidence.
- `probes/quality/tests/evidence_tree.rs`: R3, comparing the exempted count against the exemption list instead of a literal.
- `probes/platform-qa/stress/terminal50.{json,sh}`: the macOS stress lane.
- `docs/evidence/task-69/`: evidence.

## Risks

- **Not end to end:** the latency belongs to the scheduler actors and per-client sockets, not the served endpoint (todo 31 is owed the re-verify).
- **Host-bound budgets:** the IDEAL max ≤ 100 ms is a property of the host under load. A heavily oversubscribed host can break it, and the report then says so.
- **Capacity headroom:** peak RSS of 758 MB at 50 terminals leaves about 25% headroom to the 1 GiB budget.
- **CPU granularity:** CPU on Linux comes from utime+stime in clock ticks; on macOS it comes from `ps -o time=`.
- **Linux sample size:** Linux evidence is one run per profile on the round-2 code; the round-1 runs and the gate's independent reruns agree with it.
