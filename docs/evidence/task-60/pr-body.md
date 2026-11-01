Todo 60 (wave 16): first-class Unix QA and Windows smoke CI on freshly built binaries.

Closes GAP-049, GAP-051, GAP-095, GAP-121. GAP-050 is no longer claimed by todo 60 (plan amended 2026-11-02): its Unix half is closed by todo 26 and its Windows half is DEFERRED. The Windows served-attach half of GAP-048 is also DEFERRED. See Round 3. Delivers IDEAL-001, IDEAL-034, IDEAL-071, IDEAL-093. Enforces EXCL-016, EXCL-019, EXCL-027.

## Round 2 (review of 2da8a9d)

**B1, fixed in the product.** The CLI's Windows connect (`crates/ovis-terminal-cli/src/invocation/transport.rs:62-71` at 2da8a9d) opened the pipe once. A second concurrent invocation that hit `ERROR_PIPE_BUSY` (231) failed as a generic `transport_failed`. Only my smoke test client retried, so round 1's "the product is correct" ruling was wrong for the client half. I retract it.

- `invocation/transport/pipe.rs` (new): on `ERROR_PIPE_BUSY` the CLI waits in `WaitNamedPipeW` for a free instance (on a blocking thread), bounded by what is left of `BUSY_BOUND` = 5 s (inside the 10 s exchange deadline), then opens again.
- After the bound it returns the typed `CliError::PipeBusy`: kind `pipe_busy_timeout`, a hint, `waited_ms`, and the OS error as `#[source]`. Any other open error keeps its old mapping (`NotRunning` stopped / `stale_endpoint`, or `transport_failed`).
- The pipe listener is unchanged; it already creates the next instance before serving (`listen/pipe/endpoint.rs:126`, `:139-142`).
- `two-clients` now drives the production client path. It runs `ovis-terminal-cli` `invocation::transport::pipe::tests`:
  - two `Session::open` invocations at once against the production pipe listener, both completing hello and `doc.get`, with the serve report showing 2 accepted and 0 failed;
  - a pipe whose only instance is held: the waiting open succeeds once a second instance appears;
  - a pipe that stays busy: `pipe_busy_timeout` after the bound, and an absent pipe is still `server_not_running`/`stopped`.
- The test-client `two_clients` test in `probes/platform-qa/tests/pipe_smoke.rs` is removed. That file keeps only server-side checks (generation refusal, observer denial).

**B2.** Superseded by Round 3 below: the round-2 reassignment of GAP-050 to todo 62 is withdrawn.

## Round 3 (review of ebea1b7)

**B3, GAP-050 and GAP-048 in both ledgers.** The plan was amended on 2026-11-02, and todo 60 no longer claims GAP-050:

- Todo 26 closes GAP-050's Unix half (landed).
- No v1 todo serves attach over the Windows named pipe: todo 62's attach is Unix-only, and todo 64 is consumer fixtures. GAP-050 is not reassigned to 62 or 64.

What changed:

- `conformance/traceability.json` and `conformance/ideal-map.json` were regenerated with the sanctioned command: `cargo run --locked --manifest-path probes/Cargo.toml --bin quality-matrix -- --mode write --plan <amended live plan>`.
  - It changed only GAP-050 (todos `26, 54`, scenarios `part-a/task-26, platform/windows-service`) and the plan hash.
  - `quality-matrix --mode verify` exits 0, and `--mode check --plan <amended plan>` exits 0.
- `docs/design/traceability.json` gives GAP-050 and GAP-048 exactly the owners of the regenerated plan-derived matrix, each with a `note`. So **the two ledgers agree on both rows**.
  - One caveat: GAP-050 cannot be todo 26 alone in either ledger while the plan's todo 54 Closes line still names GAP-050. That entry is recorded as deferred below. If todo 54's line is amended too, one regeneration plus removing that entry leaves todo 26 alone.
  - The two ledgers already differed on 57 other rows before this todo (they use different derivations). Those are untouched.
- `docs/design/traceability-unclaimed.json` records the Windows halves as **DEFERRED** (`status: deferred`), each with the reason "Deferred: no served attach over the Windows named pipe in v1":
  - `(GAP-050, platform/windows-service)`, owner todo 54;
  - `(GAP-048, platform/matrix)`, owner todo 60 (matrix runs on Linux and macOS; the Windows smoke proves only the holder's ordered ConPTY read).
- The checker requires a deferred entry to start "Deferred: " and to state why v1 does not deliver it.
- The conformance traceability tests pass 4/4.

**B4, the busy-wait test now proves the wait.** `a_busy_pipe_is_waited_out_and_opened_once_an_instance_is_free` holds the only instance, pins the production `open`, and polls it exactly once.

- It asserts the poll is `Pending`. That state is reachable only after `ClientOptions::open` returned `ERROR_PIPE_BUSY` and the open is waiting in `WaitNamedPipeW`; a free instance or any other error would be `Ready`.
- Only then does it create the second instance and await the open, which must succeed.
- This is deterministic, with no scheduling assumption.
- The reopen-loop mutant (after the wait, return an error instead of opening again) was run on Windows CI on a throwaway branch; the result is in `docs/evidence/task-60/adversarial-mutants-r3.log`.

**N2, observer denial through the production client.** `observer-denial` now runs `ovis-terminal-cli` `invocation::transport::pipe::tests::sessions::an_observer_invocation_is_refused_every_mutation_while_an_owner_is_served`:

- An observer `Session::open` and an owner `Session::open` are opened at once against the production pipe listener behind the production boundary policy.
- Every published mutation the observer calls returns `forbidden`, with zero handler calls; the owner's `doc.set` is served.
- `probes/platform-qa/tests/pipe_smoke.rs` keeps only the raw generation-refusal test (the production client always sends the canonical generation). Its helper no longer duplicates the busy loop, and probes no longer depends on windows-sys.

**Reverse traceability check.** `conformance/src/traceability/unclaimed.rs` now flags any row whose listed owner scenario exists but does not declare the id.

- Pinned by `a_listed_owner_that_dropped_the_row_is_flagged` on a synthetic tree.
- The 46 rows that are unclaimed today are pre-existing and all EXCL. They come from part-a declaration files written before the runner, and from executable scenarios of todos 21, 37, 43 and 52.
- Those rows are recorded in `docs/design/traceability-unclaimed.json`, each naming the scenario's own todo, which the test checks against the file.
- The test fails on any new unclaimed row and on any recorded row that no longer applies, so the list only shrinks.
- With the reassignment, no row of todo 60 is in it.

**N1 hardening.**

- `SmokeReport::verify` refuses an empty requested commit (`usage`), and any run whose stdout or stderr hash is not a 64-hex SHA-256 (`report_inconsistent`).
- New tests cover the empty-test-list refusal at manifest load and the captured-stderr hash.

**client-ts seam (not built here).** `packages/client-ts/src/transport.ts:32-34` connects with `Bun.connect` to a Unix socket path only, so the TypeScript client has no Windows named-pipe transport at all. That is recorded for the client-ts owner (todo 63, "Package generated TypeScript types and persistent Bun client") in `docs/evidence/task-60/seams.md`. When it is built, it needs the same `ERROR_PIPE_BUSY` wait-and-reopen.

**Round-2 mutants** (`adversarial-mutants-r2.log`), 5/5 killed:

- R1: GAP-050 lists `platform/matrix` again, killed by `every_listed_owner_claims_its_row_except_the_recorded_ones`.
- R2: the reverse checker ignores unclaimed rows, killed by both reverse tests.
- R3: `verify` accepts an empty commit, killed by `an_empty_commit_or_a_run_without_output_hashes_is_refused`.
- R4: a run records the hash of empty stderr, killed by `a_proven_run_passes_and_a_timed_out_run_fails_its_check`.
- R5: a manifest check with no test loads, killed by `a_manifest_check_with_no_native_test_is_refused_at_load`.

The CLI busy-wait tests are Windows-only and run in `windows-smoke` and `test (windows-latest)`.

**Merge of origin/integration 557af58 (todo 61 quality gates).**

- Conflicts resolved:
  - `probes/Cargo.toml` and `probes/src/lib.rs`: union of both sides' bins, tests, dependencies and modules.
  - `Cargo.lock`: `--theirs`, then `cargo generate-lockfile --offline`, and `cargo metadata --locked` passes; the diff from integration is my dependencies only.
  - `.github/workflows/probe-stress-macos.yml`: kept the data-driven version. Todo 61's `quality-gates` job became `probes/platform-qa/stress/quality-gates.{json,sh}`, with its script verbatim, and is added to the lane floor.
- The new gates pass on the merged tree:
  - `source-check` (64 rules): pass.
  - `evidence-audit` over `docs/evidence`: pass, `docs/evidence/task-60` included, no exemption added.
  - `quality-matrix --mode verify`: pass.
- `conformance/traceability.json` is generated from the plan, which still names todo 60 under GAP-048 and GAP-050. That file is not hand-edited here. The plan amendment and regeneration are recorded for the orchestrator in `docs/evidence/task-60/seams.md`.
- Gates on the merged tree (`round5-gates.log`): check.sh exit 0 (Summary [ 122.359s] 1349 tests run: 1349 passed (3 slow), 2 skipped), `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and the Windows clippy exit 0.

**C1/C2.** This file is the committed mirror of the PR body, with no placeholders. CI results for the round-2 head go in a PR comment and the DoneClaim, so the mirror never has to change after the push. The `platform-qa` macOS stress lane is re-dispatched on the round-2 head.

## Before

`absent`: there was no comparable baseline capability, and this is not an executed failing test. Gap baseline: GAP-048, GAP-049, GAP-050, GAP-051, GAP-095, GAP-121 (`docs/design/gap-list.md:68-71`, `:117`, `:145`). Full notes are in `docs/evidence/task-60/before.log`.

- `.github/workflows/ci.yml:4-5` (edfdb58): `push: branches: [main]`. Commits pushed straight to integration got no CI.
- `.github/workflows/probe-stress-macos.yml` (edfdb58, 1427 lines): every lane PR edited one shared `options: [...]` line and appended a job block, so open lane PRs conflicted after every landing.
- `conformance/scenarios/platform/windows-service.json:6` (edfdb58) claimed GAP-050 without any case or test that delivers it.
- `--scenario platform/matrix` did not exist, and neither did a platform QA workflow.

## After

Native host: `Linux x86_64` (`docs/evidence/task-60/host.log`). Binary sha256 in `docs/evidence/task-60/binary.sha256` (re-recorded on the round-2 build). `$B` is `$CARGO_TARGET_DIR/debug/ovis-terminal`; the lane keeps its cargo target on RAM, so that path stands in for `target/debug/ovis-terminal`.

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

### CI at head 2da8a9d (all checks green; `gh pr checks 73 --watch` exit 0)

`platform-qa` run [41313712028](https://github.com/devswha/ovis-terminal/actions/runs/41313712028):

| Runner (native OS/arch) | Command | Exit | Result |
|---|---|---|---|
| unix-full ubuntu-latest (Linux x86_64) | all/all, matrix happy, matrix failure numeric, matrix failure decimal, service tests, no-skips | 0 each | all/all 62/62, failed 0, skipped 0; matrix happy/failure/failure each failed 0, skipped 0; binary sha256 0720354c... |
| unix-full macos-latest (Darwin arm64) | same | 0 each | all/all 62/62, failed 0, skipped 0; binary sha256 622889b6... |
| windows-smoke (windows x86_64) | windows-smoke over 13 checks | 0 | `windows_smoke: pass`: named-pipe-hello, generation-refusal, conpty-run, conpty-attach, conpty-input, conpty-resize, final-output-order, observer-denial, two-clients, core-restart, job-survival, service-status, conpty-kill |

So `unsupported_generation` before any spawn holds on all three runners:

- Linux and macOS: through the conformance runner and `ovis-terminal connect`. The numeric generation is `invalid_params`, `"999999"` is `unsupported_generation`, and `terminal.run` is `unauthenticated`.
- Windows: through the production named pipe (`generation-refusal`), with zero handler calls.

ci.yml is green too, including `test (windows-latest)` (nextest on Windows runs `pipe_smoke`).

verify-ci (literal acceptance command, run at head 2da8a9d):

```
$ cargo run --locked --manifest-path probes/Cargo.toml --bin verify-ci -- --repo devswha/ovis-terminal --workflow platform-qa.yml --job windows-smoke --sha "$(git rev-parse HEAD)" --require named-pipe-hello,conpty-run,conpty-attach,conpty-input,conpty-resize,final-output-order,observer-denial,two-clients,core-restart,job-survival,conpty-kill --download "$ATTEMPT_DIR/windows" --evidence "$ATTEMPT_DIR/task-60-ovis-terminal-v1.json"
{"result":"passed","run_id":"41313712028","job_conclusion":"success", "report": {"os":"windows","arch":"x86_64","windows_smoke":"pass", ...}}
exit_status: 0
```

macOS stress, lane `platform-qa` (25 iterations of the platform_qa tests plus platform/matrix all), dispatched on ac81e38: [41311577409](https://github.com/devswha/ovis-terminal/actions/runs/41311577409), conclusion success.

## What runs where

- `.github/workflows/platform-qa.yml` triggers on `pull_request`, on `push` to integration and main, and on `workflow_dispatch`. Concurrency is grouped per PR or ref, and only PR runs are cancelled.
  - `unix-full (ubuntu-latest | macos-latest)`: runs `probes/platform-qa/unix-full.sh`. That covers all/all, the matrix happy case, both literal failure injections, and the Unix service + remote/SSH-loopback + services dispatch tests. Each command's argv, exit code, stdout, stderr (including `--manual-transcript` wire transcripts), the binary and runner sha256, and the evidence sha256 are archived. Any non-zero exit, failed case or skipped case fails the job.
  - `windows-smoke (windows-2025)`: the `windows-smoke` binary runs each check of `probes/platform-qa/windows-smoke.json` one native test at a time (`cargo test <path> -- --exact`) under a 900 s deadline. A check passes only when its output shows `test <name> ... ok` and `1 passed; 0 failed; 0 ignored`, so a missing prerequisite or an ignored test fails instead of skipping. Checks: named-pipe-hello, generation-refusal, conpty-run, conpty-attach, conpty-input, conpty-resize, final-output-order, observer-denial, two-clients, core-restart, job-survival, service-status, conpty-kill. Each records what it observes (ACL, mode, offset, cleanup, service lifetime). The report states it is the smoke tier, not full Windows manual QA.
  - `two-clients` runs the production CLI client (`ovis-terminal-cli` `invocation::transport::pipe::tests`): two concurrent `Session::open` invocations against the production pipe listener, busy-instance wait, and `pipe_busy_timeout`.
  - New `probes/platform-qa/tests/pipe_smoke.rs` runs on the production named-pipe listener, connection core and boundary policy. It covers numeric -> `invalid_params` / `"999999"` -> `unsupported_generation` / `terminal.run` -> `unauthenticated` with zero handler calls, and an observer forbidden every published mutation while the owner is served.
- `verify-ci` finds the latest completed `platform-qa.yml` run at a commit and requires the named job's conclusion to be `success`. It downloads the `windows-smoke` artifact, requires the report's sha to equal the commit and every `--require`d check to pass, and appends the outcome to the evidence file.
- `.github/workflows/ci.yml` now also runs on push to integration, with per-PR concurrency.
- `probe-stress-macos.yml` is data-driven: `probe` is a free-text lane name, and a `resolve` job reads `probes/platform-qa/stress/<lane>.json` (variants, timeout, setup) and runs `<lane>.sh`.
  - The 41 existing lanes were generated mechanically from their old job blocks, with scripts verbatim. The platform lane's `$PROBE` matrix variable became `$STRESS_VARIANT`.
  - A new lane adds two files and edits no shared line. `every_stress_lane_resolves_and_the_union_is_kept` keeps the 41 + `platform-qa` union as a floor.

Gates (round 1): `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` exit 0 (nextest 1318/1318 passed, hygiene 0 violations, `check-sh.log`). Round 2 gates: `round4-gates.log`. `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and `cargo clippy ... --target x86_64-pc-windows-gnu -- -D warnings` exit 0 (`windows-precheck.log`).

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

Per-check verdict at the time: the check encoded a Unix assumption. The round-2 review rejected the accompanying "product is correct" ruling: the server side is correct, but the CLI client lacked the busy wait (fixed in Round 2 above).

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
- `Cargo.lock`: clap and sha2 for probes, plus the Windows dependencies.
- `crates/ovis-terminal-cli/src/invocation/transport.rs`, `transport/pipe.rs`, `transport/pipe_tests.rs`, `error.rs`, `report.rs`, `crates/ovis-terminal-cli/Cargo.toml`: the round-2 B1 product fix (bounded `ERROR_PIPE_BUSY` wait and reopen, typed `pipe_busy_timeout`, and production-client tests). The reviewer named this as a seam owned by T28/T54; I made it here at the lead's direction, without changing the listener.
- `docs/design/traceability.json`, `docs/design/traceability-unclaimed.json`, `conformance/src/traceability.rs`, `conformance/src/traceability/unclaimed.rs`: the GAP-050 and GAP-048 reassignment with notes, and the reverse owner check with its shrink-only exceptions list.
- client-ts (todo 63): it has no Windows named-pipe transport (`packages/client-ts/src/transport.ts:32-34` is Unix-socket only). Recorded in `docs/evidence/task-60/seams.md`, not built here.
- `.github/workflows/probe-stress-macos.yml`: the conflict-free conversion the T60 seam note asked for.
- `.github/workflows/ci.yml`: push CI on integration plus concurrency (the T82 ledger finding).
- `docs/evidence/task-60/`: evidence.

## Risks

- GAP-050's Windows half and GAP-048's Windows served-attach half are DEFERRED: no served attach over the Windows named pipe exists in v1. Both are recorded in `docs/design/traceability-unclaimed.json` with their owning todos and reasons; per the plan amendment, the final wave (F4) lists GAP-050 as a named gap.
- The CLI busy-wait tests run only on Windows (CI `windows-smoke` and `test (windows-latest)`); locally they are checked by the Windows-target clippy only.
- On Windows the conformance fixture is Unix-only (`conformance/src/fixture.rs:86`), so `platform/matrix` declares linux and macos. On Windows, the generation refusal is proven by `generation-refusal` over the production pipe, not by the conformance runner.
- `conpty-attach` is the holder's ordered ConPTY output read with mode state. It is not a served `attach.open` over the pipe, which is unmet on Windows.
- The Windows report's `sha` is the PR head (`github.event.pull_request.head.sha`). On pull_request runs, the code that actually ran is the PR merge ref.
- The happy case ends with a single fixture serve. The runner cannot serve twice in one isolated root (`create_dir` on the effects dir in `conformance/src/fixture/serve.rs:84`), so the two-clients and observer-denial checks live in the failure case.

DEFERRED VERIFICATION

None open for round 1: verify-ci ran against 2da8a9d's green windows-smoke job and exited 0 (above). For the round-2 head, verify-ci runs after the CI watch, and its verbatim result is posted as a PR comment and in the round-2 DoneClaim.
