# fix(streams): repaint checkpoints never trail output a completed wait already reported

Product-fix lane P6 (integration RED at 6de1d0e). Base: origin/integration 6de1d0e.

## Root cause

`crates/ovis-terminal-cli/src/assembly/live.rs:186-187` at 6de1d0e fed every holder chunk to the output waits **before** the attach hub:

```rust
live.waits.ingest(output)?;                 // the waits' own Screen; resolves wait tickets
let replies = live.hub.publish(output)?;    // the attach hub's Screen (repaint, first paint, terminal.screen/output)
```

`terminal.wait_output` awaits its ticket on `spawn_blocking` (`assembly/waited.rs` / `routes.rs:86`) and replies as soon as `waits.ingest` resolves it. `attach.repaint` is served by `AttachHandler::stream_call -> TerminalStreams::repaint` (`server/src/streams/handler.rs:197`, `ops.rs:70`) under the hub lock only, off the actor. So a client could see the match, send the repaint, and get a paint of a hub screen that had not taken in the matched chunk yet. That screen's tracker still held the CSI opened by `OPEN\e[3\r`, tainted by the `\r`, so the checkpoint reported `inexact [control_inside_open_sequence]`: exactly the CI actual. Whether the window opens depends on host speed, hence ~50% on a 4-vCPU runner and never on the 128-core host. The brief's hypothesis that fcfd5e5..6de1d0e moved the repaint snapshot point (replay.rs / vt read.rs) was checked and rejected: those hunks only add placeholder/index reads. The race was older; that window simply started losing it.

## Fix

`assembly/live/commit.rs` commits each chunk to the hub first, then to the waits; an exit is recorded on the hub, then on the waits. A chunk the hub refuses reaches no wait. This is an ordering fix, not serialization: no lock is added and `attach.repaint` is still served off the actor. Any reply after a completed `terminal.wait_output` (repaint, attach.open first paint, terminal.screen, terminal.output) now describes a hub state at or past the bytes the wait reported.

QA seam `serve --qa-output-commit-gap-ms N` (QA-fixture-root gated like the other seams; conformance `serve` step key `qa_output_commit_gap_ms`, refused by the protocol fixture) pauses N ms between the two commits so a test can hold the window open.

## Before

- `docs/evidence/task-P6/before-ci.log` (job 125073442270, run 41381346638, `scripts/check.sh (ubuntu)` on integration 6de1d0e): `FAIL ... suite::every_scenario_file_is_executed_or_a_listed_declaration` with `"failure": {"detail":"expectation reply[mid-repaint]/result/checkpoint/inexact did not hold",...,"step":"130"}` and `{"actual":["control_inside_open_sequence"],"check":"reply[mid-repaint]/result/checkpoint/inexact","expected":[],"passed":false}`.
- PR #85 run 41382441245 `test (ubuntu-latest)` failed the same way; fcfd5e5 run 41380553732 was green (the race was lost on 2 of 4 ubuntu suite runs of 6de1d0e-based trees).
- `docs/evidence/task-P6/before-loaded-loop.log`: 6de1d0e binaries under `taskset -c 0-1` + 8 busy loops, `conformance --scenario attach-input/adversarial --case all`: 5/5 runs fail at **step 54** (`overtaken` wait for 300000 flood bytes -> `timeout`). Under that load the debug binary never reaches step 130, so the window was opened on purpose with the QA gap seam instead (below).
- `docs/evidence/task-P6/regression.log`: mutant m1 = the 6de1d0e order (waits, gap, hub) on the new scenario `attach-input/repaint-after-wait`: **failed 3/3** at step 20 with `actual ["control_inside_open_sequence"]` and `checkpoint/output_offset actual "8"` (expected >= 14): the CI symptom, made deterministic.

## After

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-cli --lib -E 'test(commit::)'` | 0 | 2 tests run: 2 passed |
| `conformance --binary <branch>/ovis-terminal --scenario attach-input/repaint-after-wait --isolated --case all` x3 | 0 | passed 3/3, 32 assertions each (regression.log) |
| `conformance ... --scenario attach-input/adversarial --isolated --case all` (step 130 strengthened) | 0 | passed, 253 assertions |
| all/all x5, cpus 0-3 pinned + 4 busy loops, nice 10 (after-loaded-all.log) | 1,0,0,1,0 | attach-input/adversarial, /all, /repaint-after-wait passed **5/5**; runs 2,3,5: 98/98 cases, 9110 assertions; runs 1,4: only `part-a/task-30` step 145 = included `capacity/sixteen` 16x `input.send` replies `within_ms: 100` (wall clock). Same load, task-30 alone: 6de1d0e binaries 5/5, this branch 5/5 |
| `bash with-slot.sh bash scripts/check.sh` (check.log) | 0 | 1602 tests run: 1602 passed, 2 skipped; workspace-hygiene 0 violations |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | windows-check.log |
| `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` | 0 | windows-check.log |

Linux x86_64 (6.x kernel, 128-core host). macOS stress: see below.

### Tests

- `assembly::live::commit::tests::a_matched_wait_finds_its_bytes_already_in_the_hub_paint`: feeds `OPEN\e[3\r` then `1mDONE\r\n` through `commit::output`; asserts the hub paint is inexact after the first chunk, the armed wait resolved `Matched` after the second, and the hub paint is exact at offset 8+8.
- `assembly::live::commit::tests::a_chunk_the_hub_refuses_resolves_no_wait`: hub ended (as the kill path ends the hub alone); `commit::output` must error, the armed wait must still report `Timeout` at `Instant::now()` (no sleep), and the waits' cursor stays 0.
- Scenario `attach-input/repaint-after-wait` (new, registered in `runner_tests/suite.rs` ATTACH_INPUT): 500 ms QA gap; program `printf 'OPEN\033[3\r'; stty -echo; read -r _; printf '1mDONE\r\n'`; observer attaches (first_paint inexact [control_inside_open_sequence]); `terminal.wait_output` for `1mDONE` is **armed before** `input.send go`; then `attach.repaint` must report `checkpoint/inexact []` and `checkpoint/output_offset >= 14`.
- `attach-input/adversarial.json` step 130 strengthened (never loosened): adds `"/result/checkpoint/output_offset": {"$at_least": "14"}` next to `inexact []`.

### Mutants (mutants.log)

| Mutant | Killed by |
|---|---|
| m1 waits fed before the hub (6de1d0e order) | `a_chunk_the_hub_refuses_resolves_no_wait` FAIL; `repaint-after-wait` FAIL 3/3 (regression.log) |
| m2 waits never fed | `a_matched_wait_finds_its_bytes_already_in_the_hub_paint` FAIL |
| m3 hub refusal deferred until after `waits.ingest` | `a_chunk_the_hub_refuses_resolves_no_wait` FAIL |

The adversarial pass is in `adversarial.log`. Two masking paths were found and closed in the scenario. First, the tty echo of `go` contains `g`, a CSI final byte, which closes the sequence in an earlier chunk; the scenario now runs `stty -echo`. Second, a wait armed after the closing bytes matches from retained output on the actor, so it can't race; the scenario now arms the wait before the input. The first scenario draft passed on m1 for these reasons, which is recorded.

## Seams

Files outside the brief's write scope, and why:

- `crates/ovis-terminal-cli/src/assembly/live.rs`, `assembly/live/commit.rs` (+ `commit/tests.rs`): **the fix itself**. The brief listed server/streams, core/screen and vt/engine on the hypothesis that the snapshot point lived there; the wrong order is in the runtime assembly that feeds both screens, and none of the listed modules can see the waits.
- `crates/ovis-terminal-cli/src/assembly.rs` (`QaSeams::output_commit_gap_ms`), `assembly/actor.rs` (`Runtime::commit_gap`), `assembly/serve.rs` (seam gating + wiring), `invocation/args.rs` (`--qa-output-commit-gap-ms`, `ServeArgs::qa_seams`), `invocation/plan.rs` (uses `qa_seams()`; the move keeps plan.rs under the 250 pure-LOC limit): the QA delay seam the brief asked for ("like T31's --qa-holder-poll-ms").
- `conformance/src/scenario/serve.rs`, `conformance/src/engine/steps/serve.rs`, `conformance/src/engine/replay.rs`: runner `serve` key for the seam (in scope: conformance/src/**).
- `conformance/scenarios/attach-input/repaint-after-wait.json` + `conformance/src/runner_tests/suite.rs`: the regression scenario lives in its own file because the gap pauses the actor for every chunk; adding it to adversarial.json's single serve would slow the whole 134-step case.
- `probes/platform-qa/stress/repaint-after-wait.{json,sh}`: macOS stress lane (data-driven workflow, no workflow edit).

No schema/openrpc change: SDK not regenerated. Trial `git merge-tree` of this head with origin/integration e4cf46b and f71a8db: clean.

## Risks

- Exit ordering (hub.finish before waits.finish) follows the same invariant, but no test separates the orders: both screens refuse an exit identically.
- `capacity/sixteen`'s `within_ms: 100` admission bound misses on a heavily loaded 4-core slice (2/5 all/all runs here, independent of this change); a loaded CI runner could hit it.
- The adversarial scenario still cannot reach step 130 under the 2-core / 8-busy-loop recipe (step 54's 10 s flood wait); not changed here.

## DEFERRED VERIFICATION

None: the runner executes both scenarios (exit 0 on this branch, exit 1 on m1).
