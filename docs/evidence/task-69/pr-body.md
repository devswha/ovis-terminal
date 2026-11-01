test(terminal50): benchmark fifty terminals against published latency budgets

Plan todo 69 (wave 17). Closes GAP-035, GAP-061, GAP-074, GAP-143; delivers IDEAL-027, IDEAL-041, IDEAL-051, IDEAL-116; enforces EXCL-019, EXCL-024, EXCL-025 and does not reproduce V-RT-HC-01, V-RT-HD-11, V-4085.

## What ships

`bench/terminal50/` adds the `terminal50` binary to `ovis-terminal-bench`:

- **IDEAL profile** (`--admission-max-ms/--event-max-ms`): accepted only for the exact shape of 16 terminals, 8 clients, 16 requests in flight and 1048576 B/s; any other shape is `usage` (exit 2). It is judged on **every** admission and event sample, warmup included. Label `ideal-16pty`.
- **Capacity profile** (`--admission-p99-ms/--event-p99-ms`): judged on the p99 of the measured samples and labelled `capacity-release-target`, never the IDEAL assumption. With 50 terminals and 8 viewers per terminal it stresses 400 output viewers against the shared `Arc` output chunks and one shared set of session budgets.
- **What is measured:**
  - Admission is reserve+admit to receipt. It never waits for delivery (EXCL-024), and rejections are sampled too.
  - Events are actor commit to the line accepted by each client's own Unix socket. Every client has its own bounded queue and writer, and no lock is held across a write (V-4085). A ninth client never reads; it loses only its own events, which are counted (IDEAL-041). Every subscription exists before the load (EXCL-025).
  - Reports give p50/p95/p99/max, sample counts, rejected work by kind, CPU of the runtime and its PTY programs, peak RSS, history disk bytes, per-client lag (delivered/lost/undelivered/deepest queue) and per-viewer lag (bytes, events lost, held).
  - Each report also states the host (OS, arch, kernel, CPU model, parallelism, workers), the build profile, the binary under test (sha256, size, `--version`) and the warmup count. Raw samples in microseconds go beside the evidence as `<stem>.samples.json`.
- **Verdict:** both profiles also fail on lost or unwritten events to a writable client, on fewer samples than asked, on RSS > `--rss-limit-mib` (1024) and on disk > `--disk-limit-mib` (4096). A broken budget prints the full report, then `budget_exceeded: ...` on stderr, and exits 1.
- **Failure fixture:** `--inject '{"fixture":"control-writer-delay","delay_ms":150}'` delays every control-event write. The JSON shape is closed; unknown fixtures or keys are `invalid_inject` (exit 2).

## Before

`docs/evidence/baseline/smoke/hc-s06-wait-output.log:6` (RT-HC-01, original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:6`):

```
$ <run-root>/bin/hc send sh1 sleep 1; echo READY_MARK
--- stdout
{"started":false,"status":"idle","turn":1}
--- stderr
--- exit=0 elapsed_ms=5213
```

Send admission waited 5213 ms. Gap baseline: GAP-035, GAP-061, GAP-074, GAP-143.

## After

The native host for every run below was Linux 7.0.0-38-generic x86_64 (AMD Ryzen Threadripper PRO 3995WX), with runs pinned by `taskset -c 2-123`. The bench was a release build (`lto = "fat"`) with 8 runtime workers and 1000 warmup samples. The binary under test was `target/release/ovis-terminal` (`ovis-terminal 0.1.0`, sha256 `756b2d6a9eeebcb759fae2690ccb14e70afb66495703f13df013f0acaf862bf3`), built by `cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal` (exit 0).

**IDEAL profile, 16 PTYs**: exit 0, verdict passed. Evidence: `docs/evidence/task-69/bench/ideal-16pty-linux.{json,samples.json,log}`.

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --evidence "$ATTEMPT_DIR/task-69-ideal-profile.json"
```

| | count | p50 ms | p95 ms | p99 ms | max ms |
|---|---|---|---|---|---|
| admission warmup | 1000 | 0.004 | 0.008 | 0.028 | 3.289 |
| admission measured | 10015 | 0.004 | 0.006 | 0.028 | 1.214 |
| event warmup | 11960 | 0.052 | 0.466 | 1.218 | 2.331 |
| event measured | 120192 | 0.045 | 0.168 | 0.774 | 3.094 |
| control (resize/cancel) | 11008 | 0.282 | 1.488 | 2.325 | 4.328 |

- 11015 admitted, 0 rejected, 16519 events committed.
- Each of the 8 writable clients delivered 16519 with 0 lost and 0 undelivered. The non-reader delivered 278, lost 12144 and left 4097 undelivered.
- Of 128 viewers, 112 read with 0 events lost; the 16 stalled viewers lost 46870 output events, reported as gaps.
- Peak RSS 284.4 MB; disk 14.7 MB. CPU: runtime 10260 ms, programs 730 ms, over 13762 ms wall.

**Capacity profile, 50 PTYs, reported separately**: exit 0, verdict passed. Evidence: `docs/evidence/task-69/bench/capacity-50pty-linux.{json,samples.json,log}`.

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 50 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-p99-ms 100 --event-p99-ms 100 --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json"
```

- Admission measured: p99 0.010 ms, max 0.066 ms (10015 samples); warmup max 7.619 ms.
- Event measured: p99 0.161 ms, max 1.002 ms (120192 samples).
- 400 viewers: 350 reading with 0 lost, 50 stalled with 137006 lost as gaps.
- Peak RSS 776.4 MB (≤ 1 GiB); disk 15.0 MB (≤ 4 GiB); max parse turn 59.1 ms.

**Failure, delayed control writer**: exit 1. Evidence: `docs/evidence/task-69/bench/failure-writer-delay-150ms-linux.{json,samples.json,log}`.

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin terminal50 -- --binary target/release/ovis-terminal --isolated --terminals 16 --clients 8 --inflight 16 --output-bytes-per-second 1048576 --samples 10000 --admission-max-ms 100 --event-max-ms 100 --inject '{"fixture":"control-writer-delay","delay_ms":150}' --evidence "$ATTEMPT_DIR/task-69-failure.json"
```

```
budget_exceeded: 3 budget checks failed (read verdict.violations in the evidence)
{"passed": false, "violations": [{"check": "event_max_ms", "limit": 100, "observed": {"ms": 53500.2, "samples_over": 1920}}, {"check": "measured_event_samples", "limit": 1, "observed": 0}, {"check": "writable_events_missing", "limit": 0, "observed": 130232}]}
```

All 1920 written event samples are kept (raw minimum 150710 µs), and the budget still reads `admission_max_ms: 100, event_max_ms: 100`. Nothing is filtered and the budget is not widened.

**Crate tests:** `cargo nextest run --locked -p ovis-terminal-bench --no-tests=fail` ran 13 tests, all passed. 11 are terminal50 tests:

- nearest-rank summary keeps every sample;
- one slow sample fails IDEAL but not the capacity p99;
- IDEAL judges warmup;
- a p99 over budget fails capacity;
- missing events, short runs, RSS and disk fail both profiles;
- max bounds only for the exact IDEAL shape, and 50×8 = 400 viewers;
- mixed pairs, a missing `--isolated` and unknown or extended fixtures are refused;
- ps clock parsing;
- `/proc/<pid>/stat` CPU parsing;
- an in-process run with a 150 ms writer breaks `event_max_ms` with every raw sample ≥ 150000 µs;
- an in-process run where every client accounts for every committed event (delivered + lost + undelivered).

**Mutants:** four, each applied alone and each killed by its named test. The logs are `docs/evidence/task-69/mutant-*.log`, and the table is in `docs/evidence/task-69/adversarial.md`.

**Gates:** `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exits 0. The Windows pre-check (`RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and the matching clippy) exits 0.

**macOS native:** stress lane `terminal50` (`probes/platform-qa/stress/terminal50.{json,sh}`) runs the crate tests and both profiles plus the failure three times on macos-latest; run URL below.

## DEFERRED VERIFICATION

`performance/terminal50` is a declared-only descriptor listed in `conformance/scenarios/declared.json`, following the precedent of `performance/fairness` from todo 56. The binary under test on integration 95e45cc routes no `input.*` or `events.*` call to scheduler actors until todo 31 assembles it. The runner therefore cannot drive these profiles over the real endpoint, and the benchmark measures the same actors in-process on real PTYs. Verbatim attempts (`docs/evidence/task-69/conformance-deferred.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/release/ovis-terminal --scenario performance/terminal50 --isolated --case happy --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"performance/terminal50 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"performance/terminal50","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
--- exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/release/ovis-terminal --scenario performance/terminal50 --isolated --case failure --inject '{"fixture":"control-writer-delay","delay_ms":150}' --evidence "$ATTEMPT_DIR/task-69-ovis-terminal-v1.json" --append
{"case":"failure","error":{"detail":"performance/terminal50 is a declaration, not a registered executable scenario",...,"kind":"scenario_not_executable"},...,"result":"failed"}
--- exit=1
```

`--case all` gives the same `scenario_not_executable` with exit 1. The acceptance criterion "conformance exits 0 with failed:0, skipped:0" is therefore **not met** by this PR. The attempts above wrote to `task-69-conformance-{all,happy,failure}.json` so they would not overwrite the 50-terminal bench evidence, which the plan's literal command also names `task-69-ovis-terminal-v1.json`.

## Seams

Files outside the write scope (`bench/terminal50/`, `conformance/scenarios/performance/terminal50/`):

- `bench/Cargo.toml`: the `terminal50` `[[bin]]` plus `clap` (CLI) and `sha2` (binary hash), both existing workspace dependencies.
- `bench/src/lib.rs`: the `#[cfg(unix)] #[path = "../terminal50/terminal50.rs"] pub mod terminal50;` mod line.
- `Cargo.lock`: the two new dependency edges of `ovis-terminal-bench`; no new crate enters the lock.
- `conformance/scenarios/performance/terminal50.json`: the runner's grammar maps scenario id `performance/terminal50` to this file, not to a directory.
- `conformance/scenarios/declared.json`: the declared-only entry, sorted.
- `probes/quality/evidence-exemptions.json`: six entries exempting the benchmark reports and raw samples, which are not conformance-runner evidence; same pattern as todo 56's tuning reports.
- `probes/quality/tests/evidence_tree.rs`: the pinned count of exempted committed files moves from 22 to 28 for those six entries.
- `probes/platform-qa/stress/terminal50.{json,sh}`: the macOS stress lane, two added files, no workflow edit.
- `docs/evidence/task-69/`: evidence.

## Risks

- **Not end to end:** the latency is the scheduler actors' and a per-client socket's, not the served binary's endpoint, which can't yet route these calls (todo 31). The binary under test is hashed and versioned but carries no measured traffic. Once todo 31 lands, the scenario should become runner-executable.
- **Host-bound budgets:** the IDEAL max ≤ 100 ms is a property of the host under load. It held on this 128-core host with the run pinned away from other lanes; a heavily oversubscribed host can break it, and the report then says so rather than hiding it.
- **Capacity headroom:** peak RSS of 776 MB at 50 terminals includes 50 emitter processes and leaves about 25% headroom to the 1 GiB budget.
- **CPU granularity:** CPU on Linux comes from utime+stime in clock ticks (10 ms granularity); on macOS it comes from `ps -o time=`.
