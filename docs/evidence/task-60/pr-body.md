Todo 60 (wave 16): first-class Unix QA and Windows smoke CI on freshly built binaries.

Closes GAP-048, GAP-049, GAP-051, GAP-095, GAP-121 (GAP-050 stays open; see Risks). Delivers IDEAL-001, IDEAL-034, IDEAL-071, IDEAL-093. Enforces EXCL-016, EXCL-019, EXCL-027.

## Before

`absent`: there was no comparable baseline capability, and this is not an executed failing test. Gap baseline: GAP-048, GAP-049, GAP-050, GAP-051, GAP-095, GAP-121 (`docs/design/gap-list.md:68-71`, `:117`, `:145`). Full notes are in `docs/evidence/task-60/before.log`.

- `.github/workflows/ci.yml:4-5` (edfdb58): `push: branches: [main]`. Commits pushed straight to integration got no CI.
- `.github/workflows/probe-stress-macos.yml` (edfdb58, 1427 lines): every lane PR edited one shared `options: [...]` line and appended a job block, so open lane PRs conflicted after every landing.
- `conformance/scenarios/platform/windows-service.json:6` (edfdb58) claimed GAP-050 without any case or test that delivers it.
- `--scenario platform/matrix` did not exist, and neither did a platform QA workflow.

## After

Native host: `Linux x86_64` (`docs/evidence/task-60/host.log`). Binary sha256 `74ddd6efa194f57c82e702473448cf5f9c687041c2a18033464536079ed6fe4b` (`docs/evidence/task-60/binary.sha256`). `$B` is `$CARGO_TARGET_DIR/debug/ovis-terminal`; the lane keeps its cargo target on RAM, so that path stands in for `target/debug/ovis-terminal`.

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `acceptance-build.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary "$B" --scenario platform/matrix --isolated --case all --evidence "$ATTEMPT_DIR/task-60-ovis-terminal-v1.json"` | 0 | cases 2, passed 2, failed 0, skipped 0, 122 assertions (`acceptance-all.log`) |
| `... --case happy --evidence "$ATTEMPT_DIR/task-60-ovis-terminal-v1.json"` | 0 | cases 1, failed 0, skipped 0, 94 assertions (`qa-happy.log`) |
| `... --case failure --inject '{"method":"hello","params":{"generation":999999}}' --evidence ... --append` | 0 | 26 assertions (`qa-failure.log`) |
| `... --case failure --inject '{"method":"hello","params":{"generation":"999999"}}' --evidence ... --append` | 0 | 25 assertions (`qa-failure-decimal.log`) |
| `conformance --scenario all/all --isolated --case all` (every non-Windows family) | 0 | cases 62, passed 62, failed 0, skipped 0, 37 declared-only |
| `python3 probes/platform-qa/no-skips.py <evidence>` | 0 | every run passed, skipped 0 (`qa-no-skips.log`) |

What the failure evidence shows (`docs/evidence/task-60/qa-failure-assertions.log`, copied from the evidence JSON):

- The literal injected request `{"method":"hello","params":{"generation":999999}}` (sent through `ovis-terminal connect`) gets `/error/data/kind` `invalid_params` with `field_path` `params.generation`, and no result.
  - That is the binding wire contract: every u64 generation is a canonical decimal string, so a numeric value is `invalid_params` (`crates/ovis-terminal-server/src/conn/core/tests.rs:180` pins `1` -> `invalid_params`).
  - The same generation in canonical form, `"999999"`, gets `unsupported_generation` and no result.
- In both runs a `terminal.run` sent afterwards on that still-unauthenticated connection is `unauthenticated` with no result, so nothing spawns.
- The same endpoint then serves an owner through the bridge and an observer on a socket at once. All four observer mutations (`terminal.run`, `input.send`, `attach.resize`, `doc.set`) are `forbidden`, the owner's `rpc.discover` equals the published schema, and the fixture reports `accepted` = `3` and `panicked` = `0`.
- Happy case: the detached holder outlives its starter, and `service status` and `server status` never start anything (EXCL-027). Through the bridge, `process_info` gives launcher, foreground and background roles with PID plus start token, and a replaced incarnation loses its token.

Shipped tests: `cargo nextest run --locked -p ovis-terminal-probes -E 'test(platform_qa) | binary(platform_qa)'` passes 19/19 (libtest verdicts, report verification, deadline kill, evidence append, manifest and lane checks). The Windows pipe smoke tests compile and lint clean for `x86_64-pc-windows-gnu`; they run on the `windows-smoke` job.

CI_RESULTS_PLACEHOLDER

## What runs where

- `.github/workflows/platform-qa.yml` triggers on `pull_request`, on `push` to integration and main, and on `workflow_dispatch`. Concurrency is grouped per PR or ref, and only PR runs are cancelled.
  - `unix-full (ubuntu-latest | macos-latest)`: runs `probes/platform-qa/unix-full.sh`. That covers all/all, the matrix happy case, both literal failure injections, and the Unix service + remote/SSH-loopback + services dispatch tests. Each command's argv, exit code, stdout, stderr (including `--manual-transcript` wire transcripts), the binary and runner sha256, and the evidence sha256 are archived. Any non-zero exit, failed case or skipped case fails the job.
  - `windows-smoke (windows-2025)`: the `windows-smoke` binary runs each check of `probes/platform-qa/windows-smoke.json` one native test at a time (`cargo test <path> -- --exact`) under a 900 s deadline. A check passes only when its output shows `test <name> ... ok` and `1 passed; 0 failed; 0 ignored`, so a missing prerequisite or an ignored test fails instead of skipping. Checks: named-pipe-hello, generation-refusal, conpty-run, conpty-attach, conpty-input, conpty-resize, final-output-order, observer-denial, two-clients, core-restart, job-survival, service-status, conpty-kill. Each records what it observes (ACL, mode, offset, cleanup, service lifetime). The report states it is the smoke tier, not full Windows manual QA.
  - New `probes/platform-qa/tests/pipe_smoke.rs` runs on the production named-pipe listener, connection core and boundary policy. It covers two concurrent clients, numeric -> `invalid_params` / `"999999"` -> `unsupported_generation` / `terminal.run` -> `unauthenticated` with zero handler calls, and an observer forbidden every published mutation while the owner is served.
- `verify-ci` finds the latest completed `platform-qa.yml` run at a commit and requires the named job's conclusion to be `success`. It downloads the `windows-smoke` artifact, requires the report's sha to equal the commit and every `--require`d check to pass, and appends the outcome to the evidence file.
- `.github/workflows/ci.yml` now also runs on push to integration, with per-PR concurrency.
- `probe-stress-macos.yml` is data-driven: `probe` is a free-text lane name, and a `resolve` job reads `probes/platform-qa/stress/<lane>.json` (variants, timeout, setup) and runs `<lane>.sh`.
  - The 41 existing lanes were generated mechanically from their old job blocks, with scripts verbatim. The platform lane's `$PROBE` matrix variable became `$STRESS_VARIANT`.
  - A new lane adds two files and edits no shared line. `every_stress_lane_resolves_and_the_union_is_kept` keeps the 41 + `platform-qa` union as a floor.

Gates: `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exit 0 (nextest 1318/1318 passed, hygiene 0 violations, `check-sh.log`). `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and `cargo clippy ... --target x86_64-pc-windows-gnu -- -D warnings` exit 0 (`windows-precheck.log`).

## Round 2: Windows smoke harness fix

On head ac81e38, `platform-qa / windows-smoke` (run 41311579837, artifact `windows-smoke`) failed three checks: `generation-refusal`, `two-clients` and `observer-denial`. Each failed check's log (`windows-smoke/<check>-0.log` in that artifact) shows the same cause:

```
error: failed to remove file `D:\a\ovis-terminal\ovis-terminal\target\debug\windows-smoke.exe`
  Access is denied. (os error 5)
```

All three are `pipe_smoke` tests in the `ovis-terminal-probes` package. `cargo test -p ovis-terminal-probes` relinks that package's binaries, and one of them, `windows-smoke.exe`, was the running smoke runner itself. Windows cannot replace a running executable, so the test never compiled.

- This was the harness, not the product or a contract: no test body ran, and every check in other packages passed on that run.
- The checks themselves are unchanged and stay as written.
- Fix in the runner: `windows-smoke` refuses to start from inside the cargo target directory its checks build into (typed `usage` error).
- Fix in the workflow: it builds the runner, copies it to `$RUNNER_TEMP` and runs the copy.
- Locally, running from the target dir exits 1 with `kind: usage`, and a copy outside it runs a real check end to end with `windows_smoke: pass`.

## Round 3: two-clients and observer-denial (real Windows findings)

On head 7643079 the harness fix held: `generation-refusal` and all native checks passed. `two-clients` and `observer-denial` failed at the second concurrent `ClientOptions::open` with `Os { code: 231, message: "All pipe instances are busy." }` (run 41312728281, artifact `windows-smoke`, `two-clients-0.log` / `observer-denial-0.log`, `pipe_smoke.rs:26` / `:82`).

Per-check verdict: the **check** was wrong. The product is correct.

- The production listener creates its following instance before it returns the accepted connection, so before serving it. See `crates/ovis-terminal-server/src/listen/pipe/endpoint.rs:126` ("the following instance exists before this returns") and `:139-142`.
- `ERROR_PIPE_BUSY` is the short window between a client taking the waiting instance and the accept loop creating the next one.
- The Windows client contract for that window is `WaitNamedPipe` with a bound, then reopen. The test opened the second client immediately, which only works with a Unix socket backlog.

The fix is in the check:

- `pipe_smoke/served.rs` `Client::open` waits on `ERROR_PIPE_BUSY` with `WaitNamedPipeW`, bounded by what is left of 20 s, and reopens. The wait is event-driven: no sleep, no rerun.
- It returns a typed `OpenError` (`pipe_busy` / `pipe_open_failed`, `kind()` and `hint()`, the OS error as `#[source]`).
- The three tests return `TestResult` instead of unwrapping the connect.
- Both clients are still opened concurrently (`tokio::join!`), so the checks still exercise two clients holding the endpoint at once.
- `generation-refusal` was unchanged and passed on 7643079.

## Adversarial

- Flaky tests: readiness is event-driven. The bounded runner completes on both pipes reaching EOF and treats the deadline only as a hang guard. The pipe tests read each reply under a 20 s bound with no sleeps. No rerun-to-green anywhere: `windows-smoke` runs each test once.
- Hung commands: every native test and every `gh` call runs under a deadline. On expiry the runner kills its own spawned child handle (pid asserted > 1) and records `timed_out`. Pinned by `a_child_still_running_at_the_deadline_is_killed_and_reported`.
- Repeated interruptions: the Windows service and account cleanup runs `if: always()`, and each conformance run uses a fresh isolated root.
- Mutants: see `docs/evidence/task-60/adversarial-mutants.log`.

| Mutant | Killed by | Exit |
|---|---|---|
| M1 `libtest::proven` accepts a failed exit | `platform_qa::libtest::tests::a_second_summary_or_a_failed_exit_is_rejected` | 100 |
| M2 `SmokeReport::verify` ignores the commit | `platform_qa::smoke::tests::a_report_for_another_commit_is_refused` | 100 |
| M3 a child killed at its deadline is reported as exit 0 | `platform_qa::bounded::tests::a_child_still_running_at_the_deadline_is_killed_and_reported` | 100 |
| M4 server `hello` skips the generation check | `conformance --scenario platform/matrix --case failure --inject '{"method":"hello","params":{"generation":"999999"}}'` (`unauthenticated` != `unsupported_generation`) | 1 |

Killed 4/4 (`adversarial-mutants.log`). A first M3 that never killed hung the harness instead of failing a test; it was replaced, and every mutant command now runs under `timeout 900` (details in the log).

## Seams

Files outside the literal write scope, and why:

- `conformance/scenarios/platform/matrix.json`: the runner resolves `platform/matrix` to `<family>/<name>.json` and classifies every file under `scenarios/` by that grammar, so a `platform/matrix/` directory would be reported invalid.
- `conformance/src/runner_tests/suite.rs`: the scenario registered in `EXECUTABLE`, in sorted order. Nothing was added to `declared.json`, since the scenario is executable.
- `conformance/scenarios/platform/windows-service.json`: GAP-050 removed from its `closes` (owner seam: deliver it or correct the list).
- `probes/src/lib.rs`: one `#[path] pub mod platform_qa;` line.
- `Cargo.lock`: clap and sha2 for probes, plus the Windows dev-dependencies.
- `.github/workflows/probe-stress-macos.yml`: the conflict-free conversion the T60 seam note asked for.
- `.github/workflows/ci.yml`: push CI on integration plus concurrency (the T82 ledger finding).
- `docs/evidence/task-60/`: evidence.

## Risks

- GAP-050 (geometry events for mirror clients on Windows) is not delivered. A served attach over the Windows pipe is still unmet (T54 notes), so `platform/matrix` does not claim it. I also removed the claim from the `windows-service` declaration. `docs/design/traceability.json` still lists both scenarios as GAP-050 owners, and a later todo has to deliver it.
- On Windows the conformance fixture is Unix-only (`conformance/src/fixture.rs:86`), so `platform/matrix` declares linux and macos. On Windows, the generation refusal is proven by `generation-refusal` over the production pipe, not by the conformance runner.
- `conpty-attach` is the holder's ordered ConPTY output read with mode state. It is not a served `attach.open` over the pipe, which is unmet on Windows.
- The Windows report's `sha` is the PR head (`github.event.pull_request.head.sha`). On pull_request runs, the code that actually ran is the PR merge ref.
- The happy case ends with a single fixture serve. The runner cannot serve twice in one isolated root (`create_dir` on the effects dir in `conformance/src/fixture/serve.rs:84`), so the two-clients and observer-denial checks live in the failure case.

DEFERRED_PLACEHOLDER
