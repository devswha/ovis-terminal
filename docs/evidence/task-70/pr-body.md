# test(soak): soak fifty terminals for seventy two hours (todo 70)

Ships `bench/soak/` (binary `soak`), the scheduled 72 h workflow `.github/workflows/soak.yml`, and the declared `performance/soak` scenario. Proves the harness with one continuous 1800 s, 50-terminal native Linux run.

**What the harness drives.** Every program runs in the holders of the binary under test, started with its own verb `ovis-terminal --json service run --session <s> --manager detached`. There is one shared holder and one dedicated holder for the last terminal (the internal isolation placement). The control process is crashed with SIGKILL, handed over and observed; it is the soak's own `control` mode, built from `ovis-terminal-core` crash recovery (`recovery::recover`), checkpoints and `restart::classify`. The binary does not serve a control endpoint yet (todo 31), so the crashed process is that library-assembled core, not `ovis-terminal` itself; see Risks.

## Before

`/tmp/ulw-ovis-yCJzPH/evidence/smoke/attempt2-herdr-h03-workspace.log:5` (RT-HD-05), durable copy `docs/evidence/baseline/smoke/attempt2-herdr-h03-workspace.log:5`:

```
workspaces before: 1
```

and lines 9-10 of the same file: `root_pane from create: w2:p1 ...`, `workspace id: w2 result keys: type,workspace,tab,root_pane`. herdr restores and reports workspace/pane metadata with no process incarnation, start token, output offset, gap or outcome record, so nothing proves process continuity. Gap baseline: GAP-061, GAP-119, GAP-121, GAP-122, GAP-123, GAP-124, GAP-143 (`docs/evidence/task-70/before.log`).

## After

Native Linux, x86_64 (`Linux 7.0.0-38-generic x86_64`, 122 usable cores, run pinned to cores 2-123 at nice 10). Binary under test `/dev/shm/ovt-target-lane-t70/release/ovis-terminal`, `ovis-terminal 0.1.0`, sha256 `98f7abe5e9b4aa38722b03fdaba12ef4a404045728abdbaf4a737e24290710c6`.

### Acceptance: one continuous 1800 s run (tracked background session, completion subscribed)

```
cargo build --locked --release -p ovis-terminal-cli --bin ovis-terminal                 # exit 0
cargo run --locked --release --manifest-path bench/Cargo.toml --bin soak -- --binary "$CARGO_TARGET_DIR/release/ovis-terminal" --isolated --terminals 50 --duration-seconds 1800 --core-crashes 2 --rss-limit-mib 1024 --disk-limit-mib 4096 --evidence "$ATTEMPT_DIR/task-70-ovis-terminal-v1.json"
ACCEPTANCE_EXIT=0      started 2026-11-01T18:46:07Z, ended 2026-11-01T19:16:08Z
```

(`target/` is on RAM on this host, so `target/release/ovis-terminal` is `$CARGO_TARGET_DIR/release/ovis-terminal`.) Report: `docs/evidence/task-70/runs/soak-1800s-50pty-linux.json`; raw per-second samples (1800 lines): `runs/soak-1800s-50pty-linux.samples.ndjson`; last minute checkpoint: `runs/soak-1800s-50pty-linux.checkpoint.json`.

- `verdict: {"passed": true, "violations": []}`, `elapsed_ms: 1800000` against `duration_ms: 1800000`.
- Disruptions done: `[['core_crash', 2], ['planned_restart', 1], ['remote_interruption', 2], ['stalled_viewer', 1]]`. Time from each disruption to the next boot's `ready`, in ms: `[['core_crash', 507], ['planned_restart', 494], ['core_crash', 475], ['core_crash', 482]]` (the last entry is the post-run holder-loss probe's crash). Planned-restart exit code: `[0]`.
- Recoveries: every survivor kept its first PID and start token (`identity_changes: 0`, `losses: []`).

| boot | mode | survived | not survived | exact screens | gaps |
|---|---|---|---|---|---|
| 2 | crash | 50 | 0 | 50 | 0 |
| 3 | handover | 50 | 0 | 50 | 0 |
| 4 | crash | 50 | 0 | 50 | 0 |
| 5 | crash | 49 | 1 | 49 | 0 |

  The probe boot (the last row) lost exactly the dedicated terminal: `probe: {"dedicated": "unknown:holder_unreachable", "shared_expected": 49, "shared_identities_unchanged": true, "shared_survived": 49}`.
- Output offsets never moved backwards (`offset_regressions: 0`). Observer continuity: `{"gapped": 0, "holes": 0, "received": 1805, "resumes": 2}` (2 resumes after interruptions, 0 holes).
- Control: `pongs: 1800`, max `49` ms, `late_pongs: 0` (bound 1000 ms).
- Queues: max viewer queue `181417` B against a 262144 B bound. The stalled viewer had reached 181417 B when the second crash ended its boot, so it was never cut in this run. The cut itself is proven by the shipped hub test below.
- Memory: runtime (control + holders) RSS peak `222765056` B (<= 1 GiB); post-warmup median `221421568` -> final median `195584000`, growth `0` B against an allowance of `18991431` B (16 MiB + 1% of the baseline). Retained disk peak `26135490` B (<= 4 GiB).
- Materialized `ovis-terminal` CLI commands, run by the harness with their exit codes: `ovis-terminal --json service status --session soak` -> 0, `ovis-terminal --json service stop --session soak` -> 0, `ovis-terminal --json service status --session soak` -> 0.

### Failure fixture (final code)

```
cargo run --locked --release --manifest-path bench/Cargo.toml --bin soak -- --binary "$CARGO_TARGET_DIR/release/ovis-terminal" --isolated --inject '{"fixture":"holder-tail-gap-and-lost-receipt"}' --evidence "$ATTEMPT_DIR/task-70-failure.json"
FIXTURE_EXIT=0
```

`docs/evidence/task-70/runs/fixture-holder-tail-gap-and-lost-receipt-linux.json`: `{"auto_resend": false, "exact_recovery_claimed": false, "outcome": {"checkpoint_offset": 552639, "gaps": [{"from": "552639", "to": "854943"}], "input_sequence_after_crash": 1, "input_sequence_after_handover": 1, "offset_at_crash": 1900551, "op_id": "op:soak-input-terminal:soak-0-1", "outcome": "outcome_unknown", "resent": false, "screen_exact": false}, "passed": true, "screen_exact": false}`. Terminal 0's only checkpoint is at the reported offset. Its holder's 1 MiB tail had rolled past that checkpoint when the control process was killed, so crash recovery reports `screen_exact: false` with the gap. The `input.send`, whose intent was journalled before the holder saw it and whose receipt was never committed, comes back `outcome_unknown` with `delivery: unproven`. The holder's input sequence stays 1 through the crash boot and a later handover boot, so nothing resent it. The report never claims exact recovery (`exact_recovery_claimed: false`, `auto_resend: false`).

### Tests and mutants

`cargo nextest run -p ovis-terminal-bench soak::`: 15 tests run, 15 passed. They include a vanished run-root entry counted as gone, and a batched `ps` listing parsed with its missing PIDs counted. They cover the schedule (20 crashes over 72 h; the 1800 s crash times 615 s and 1365 s), every verdict rule (incomplete duration, skipped disruption, RSS limit and growth edges, planned exit, probe isolation, late/missing control, queue overrun), the fixture pass rule, the tally (changed PID, unexcused loss, offset regression), decimal counters (u64::MAX round-trip, numeric and `01` rejected), and the hub on real sockets: a stalled viewer is cut at its bound with a `viewer_cut` record while a reader gets every sequence in order, and a late resume gets an explicit gap.

Each mutant was applied, run and reverted; each fails exactly one shipped test (`docs/evidence/task-70/mutant-*.log`):
- M1, an incomplete run passes: `a_complete_clean_run_passes_and_each_broken_fact_fails_its_own_rule` fails.
- M2, exact recovery accepted: `the_fixture_passes_only_without_exact_recovery_and_without_a_resend` fails.
- M3, the stalled viewer is never cut: `a_stalled_viewer_is_cut_at_its_bound_while_a_reader_misses_nothing` fails.
- M4, a changed PID counts as a survivor: `a_survivor_under_another_pid_or_an_unexcused_loss_is_counted` fails.
- M5, a relative `--binary` is not pinned: `a_relative_binary_is_pinned_to_its_absolute_path_before_any_process_starts` fails.

Adversarial classes and legacy defects: `docs/evidence/task-70/adversarial.md`.

### Gates

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0. nextest reports 1377 tests run, 1377 passed; workspace-hygiene 0 violations in 1387 files; source-check and evidence-audit pass (`docs/evidence/task-70/check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: both exit 0 (`docs/evidence/task-70/windows-precheck.log`). The soak modules are `cfg(unix)`; the Windows `main` prints a usage line and exits 2.

## DEFERRED VERIFICATION

**72-hour soak.** `.github/workflows/soak.yml` (workflow_dispatch plus a weekly cron `17 3 * * 6`) runs 50 terminals for 259200 s with 20 control crashes, 6 handovers, 12 interruptions, 6 stalled viewers and the probe. It runs on existing self-hosted runners labelled `[self-hosted, Linux, ovis-soak]` and `[self-hosted, macOS, ovis-soak]`, with no new paid service, and uploads external artifact checkpoints every 6 h plus the final evidence. First dispatch, attempted from this lane:

```
$ gh workflow run soak.yml --ref lane/t70-ship-the-72-hour-soak-harnes
HTTP 404: Not Found (https://api.github.com/repos/devswha/ovis-terminal/actions/workflows/soak.yml)
exit=1
```

GitHub dispatches and schedules a workflow only once its file is on the default branch, which this lane does not change. The first dispatch therefore happens after the union merge reaches it, and is reported then. The 72 h result is reported when it lands; this PR claims nothing about it.

**Conformance runner.** `performance/soak` is declared (`conformance/scenarios/declared.json`), not executable. The literal attempts, verbatim:

```
FIXTURE_EXIT=0
{"case":"happy","error":{"detail":"performance/soak is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"/home/devswha/.ovis-run/scratch/t70/attempt/task-70-ovis-terminal-v1.conformance.json","evidence_error":null,"evidence_written":true,"result":"failed","scenario":"performance/soak","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
CONFORMANCE_happy_EXIT=1
{"case":"all","error":{"detail":"performance/soak is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"/home/devswha/.ovis-run/scratch/t70/attempt/task-70-ovis-terminal-v1.conformance.json","evidence_error":null,"evidence_written":true,"result":"failed","scenario":"performance/soak","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
CONFORMANCE_all_EXIT=1
{"case":"failure","error":{"detail":"performance/soak is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"/home/devswha/.ovis-run/scratch/t70/attempt/task-70-ovis-terminal-v1.conformance.json","evidence_error":null,"evidence_written":true,"result":"failed","scenario":"performance/soak","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
CONFORMANCE_failure_EXIT=1
```

It becomes executable when todo 31 serves the control endpoint (terminal.run, input.*, events.*) and the runner can run this schedule over it, or gains a step that runs an external harness. The `--inject` value has no `method`, so today's inject step cannot plan it. **Seam note (performance/terminal50):** still declared-only. Todo 31 (PR #57) has not landed, so the flip to executed stays with todo 70 or the orchestrator's re-verify after #57.

### CI disk exhaustion on e54c967 (root cause)

`test (ubuntu-latest)` of run 41342888203 died with `No space left on device` in the runner's own `_diag` log. Every build/test step had already passed, so no job log was uploaded. The full analysis is in `docs/evidence/task-70/ci-disk-root-cause.md`.

- **Runtime writes, measured.** The soak tests write 0 bytes under fresh `OVIS_FIXTURE_ROOT` and `TMPDIR`, and nothing under the repo tree. They did leave 2 empty directories per run, also on panic; a `Scratch` drop guard now removes them (0 left afterwards).
- **The soak binary does not run in CI.** No CI job executes it.
- **Build footprint, measured under CI's full-debug profile.** The PR adds one 210,816,152 B `soak` binary, the same size as `terminal50`'s 207,287,424 B, plus a 7 MB test harness for `main.rs`. `test = false` on that `[[bin]]` now removes the harness.
- **Not caused by e54c967.** The same job passed on d7255d8 with the identical footprint, and e54c967 changed nothing it builds. That job runs near the runner's about 14 GB limit. Shrinking the workspace debug footprint (`[profile.dev] debug = true`, every bench binary linking the whole bench lib) is a shared-profile follow-up.
- **`soak.yml` is valid.** actionlint 1.7.7 reports no findings for it or any other workflow; `gh workflow view/run` answer 404 only because the file is not on the default branch.

### Minimum run length for the RSS rule

`rss_growth` compares the median runtime RSS of the window that starts when warmup ends with the median of the final window. Holders keep a 1 MiB output tail per terminal; at the default 8192 B/s per terminal that tail fills in 128 s, and the core's screens and checkpoints fill in about the same time. The runtime's RSS therefore climbs for roughly the first 130 s and then plateaus. The 1800 s acceptance report shows this: baseline median 221421568 B, final median 195584000 B, growth 0. A baseline taken during the climb measures the fill, not a leak.

The default warmup is `min(180 s, duration / 10)`, and the window is `min(60 s, duration / 10)`. The baseline starts after the fill only when warmup is at least about 150 s, which means a duration of at least 1500 s with the defaults, or an explicit `--warmup-seconds 150` (or more) with a schedule that still fits. Shorter runs, such as the 180 s profile (18 s warmup) and the 40 s `--self-test`, are expected to trip `rss_growth` or to pass it only by chance. The macOS stress lane's 600 s run uses a 60 s warmup and passed with growth 0 for that reason, not by the rule's design. The rule binds the 1800 s acceptance run and the 259200 s profile, both of which warm up for 180 s.

## Seams

- `bench/Cargo.toml` adds the `[[bin]] soak` entry and the `ovis-terminal-holder` dependency; `bench/src/lib.rs` adds the module and layering line; `Cargo.lock` changes by one line.
- `conformance/scenarios/performance/soak.json` is the declared descriptor. The scenario id `performance/soak` maps to this file name, while the block's write scope names a `performance/soak/` directory. `conformance/scenarios/declared.json` gains its entry.
- `probes/quality/evidence-exemptions.json` exempts the three `ovis-soak/1` JSON reports (not runner evidence), matching the task-69 precedent.
- `probes/platform-qa/stress/soak.{json,sh}` add the macOS stress lane (the data-driven lane seam; the workflow is untouched).
- `docs/evidence/task-70/` holds the evidence.

## Risks

- **The crashed control process is not the `ovis-terminal` binary.** The binary serves no control endpoint before todo 31. The holders (and every program) are the binary's own `service run`, and the control process is assembled from `ovis-terminal-core` recovery. Re-run this soak against the served endpoint once #57 lands.
- **`timeout-minutes`.** The workflow puts `timeout-minutes: 4320` on the steps that observe the 259200 s measured phase. The job is allowed 4380 minutes, because a job-level 4320 would cancel the run before its report: the build plus the post-run probe and teardown sit outside the measured 72 h.
- **GAP-119/IDEAL-092 coverage.** The soak's "remote interruption" cuts and resumes the observing client's connection; it is not an SSH machine transport. Orphan acknowledgement replay by parent incarnation is proven by `recovery/e2e` (todo 52) and is not re-run inside this soak.
- **Source state of the 1800 s run.** The soak binary of the acceptance run was built before a behaviour-free module split, needed for the 250-pure-LOC rule. `control.rs` was split into `io/ops/shutdown` and `drive.rs` into `tick`. The fixture run, the tests and the mutants ran on the final code.
- **The stalled viewer was not cut inside the 1800 s run** (181417 of 262144 B before its boot ended). The cut is proven by the hub test; the 72 h profile attaches 6 stalled viewers.
- **macOS stress finding, fixed (diagnosis corrected).** Two stress dispatches failed at the self-test: https://github.com/devswha/ovis-terminal/actions/runs/41341596094 and https://github.com/devswha/ovis-terminal/actions/runs/41342883031. The second printed the report, whose `error.kind` was `binary_unavailable`. Holders start with `/` as their working directory, and the lane (like `soak.yml` and the literal acceptance command) passes the relative `--binary target/release/ovis-terminal`. A relative program path is resolved against the child's directory, so the binary was not found. My first explanation, a socket path over the limit under macOS's `$TMPDIR`, was a guess and was wrong. `--binary` is now pinned with `std::fs::canonicalize` while the plan is built. This is proven by `a_relative_binary_is_pinned_to_its_absolute_path_before_any_process_starts`, which mutant M5 makes fail, and by a local self-test with a relative `--binary` (exit 0). The 1800 s acceptance run passed an absolute `$CARGO_TARGET_DIR/...` path and was not affected. The `/tmp` default root stays, so holder sockets keep clear of the long macOS `$TMPDIR`. The third dispatch, https://github.com/devswha/ovis-terminal/actions/runs/41344754982, passed the self-test and the fixture on macOS. Its 600 s, 50-terminal run stopped at about 385 s with six violations; the full classification is in `docs/evidence/task-70/macos-stress-root-cause.md`. Every one is a harness defect or a consequence of one, and none is a product defect: both recoveries had 50/50 survivors with exact screens, 0 losses, 0 identity changes, 0 offset regressions and 0 sequence holes. The root was `run_error` `io` ("stat the run root failed"). The per-second disk walk raced entries that vanish during a live run (staged checkpoints renamed into place, sockets removed at a handover). The other violations follow from it: `incomplete_duration` misreported 0 ms although 385 samples were taken, `rss_windows` never reached its final window, and the probe never ran. The single late pong came from the harness's own stall, one `ps` spawn per PID on 3 cores; control latency peaked at 117 ms against the 1000 ms budget, so no host-derived budget is needed. Fixes: vanished entries count as gone, and other failures are `PathIo` errors with the path and a `causes` chain in the report. Elapsed is recorded every tick. A requested probe that never ran is `holder_loss_probe_not_run`. Observer lines carry their arrival time, and pings go out before sampling. macOS RSS is read with one `ps` per group. A local 180 s, 50-terminal run with a relative `--binary` exited 0 (180 pongs, max 92 ms, 0 late, probe ran), but its artifact was not kept, so this PR claims nothing from it. The gate's rerun of the same profile exited 1 on `rss_growth`: 35.3 MB of growth against an 18.2 MB allowance. That is expected for so short a run: see "Minimum run length for the RSS rule" below. Final re-dispatch: STRESS4_PLACEHOLDER.
