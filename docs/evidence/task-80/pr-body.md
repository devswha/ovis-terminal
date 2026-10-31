fix(server): make the connection ordering test observe the product's real ordering signal

Todo 80 (wave 11, follow-up). Closes GAP-REGRESSION (test flake, no GAP id). Delivers IDEAL-050: the ordering guarantees stay testable.

## Before

The three sightings of `ovis-terminal-server::connection happy` failing:

1. **CI, PR #35, head `7c33202`.** Job `scripts/check.sh (ubuntu)`, run 41219136858, job 124589845804 (https://github.com/devswha/ovis-terminal/actions/runs/41219136858/job/124589845804). Log lines 1415-1416:
   ```
   2026-10-31T16:22:10.6341265Z     thread 'happy' (17651) panicked at crates/ovis-terminal-server/tests/connection.rs:98:5:
   2026-10-31T16:22:10.6341913Z     a waited for b: ["m", "a", "b"]
   ```
2. **Todo 16 round-2 local gate.** Ledger finding: "connection happy failed once in the T16 round-2 local gate (crate untouched by the lane), then passed on re-run". No output was captured.
3. **Todo 13 round-3 verifier.** Ledger `flake_disclosure`: "one full-suite run showed happy failing at setup (finished in 0.00s), output not captured". No output was captured.

Only sighting 1 carries an assertion text. Sightings 2 and 3 cannot be attributed to this assertion from evidence, and this PR does not claim they were.

### Reproduction on the unmodified base `846a210`

| Command | Exit | Evidence |
|---|---|---|
| `bash docs/evidence/task-80/stress.sh .`: 200 x `cargo nextest run -p ovis-terminal-server --test connection --locked` under 8 busy loops, 128 CPUs | 0 | `before-stress-unpinned.log`: `PASSED 200/200` |
| `taskset -c 0-3 bash docs/evidence/task-80/stress.sh .`: the same loop pinned to 4 CPUs, like the 4-vCPU CI runner | 0 | `before-stress.log`: `PASSED 200/200` |
| `bash docs/evidence/task-80/sample.sh target/debug/deps/connection-<hash> 64 200 <dir>`: 64 parallel loops x 200 runs of `happy --exact` | 25 failing runs | `before-sample.log`: `samples 12800 failures 25`. 22 runs show `a waited for b: ["m", "a", "b"]`, the CI text, and 3 show `a waited for b: ["a", "b", "m"]` |

The plan's failure QA (the nextest loop under 8 busy loops) does **not** fail on this host within 200 runs, pinned or unpinned. The race was reproduced with the oversubscribed sampler above, which produces the identical CI assertion.

## Root cause

The test assumed a reply order that the product does not promise.

- The connection driver spawns each request into a `JoinSet` and writes each reply when its task settles: `crates/ovis-terminal-server/src/conn/driver.rs`, `join_next_with_id`, then `settle`, then `reply`. In tokio 1.54.1, `IdleNotifiedSet` pushes woken tasks with `push_front` and `pop_notified` takes them with `pop_back`, so replies go out in task-completion order.
- In the test handler, `b` calls `Notify::notify_one()` and returns, while `a` awaits `notified()`. Once `b` wakes `a`, `a` can run on another worker and complete before `b`'s own task finishes. In that case `a`'s reply is written first: `["m", "a", "b"]`.
- Todo 77 records the same contract: requests pipelined on one connection run concurrently and settle in either order. The product is correct, and the reply-stream position was never an ordering signal.

## After

- The test `Recorder` (`tests/connection/handler.rs`) now keeps a journal. `b` records `"b released a"` before it notifies, and `a` records `"a resumed"` after `notified()` returns.
- `happy` asserts `recorder.journal() == ["b released a", "a resumed"]` with the message `a waited for b`. This is the causal order the handler observes, and it is independent of which reply the driver writes first.
- The concurrency check is unchanged. `a` can settle only after `b` has run, so if dispatch were serial, `a`'s reply would never arrive and `expect_next` would fail at its `WAIT` bound.
- No product code changed. Nothing is serialized and the test has no retries, sleeps or flaky marker.

| Command | Exit | Evidence |
|---|---|---|
| `bash docs/evidence/task-80/stress.sh .`: 200 x `cargo nextest run -p ovis-terminal-server --test connection --locked` under 8 busy loops | 0 | `stress-200.log`: `PASSED 200/200` |
| `bash docs/evidence/task-80/sample.sh ... 64 200 ...` on the fixed tree | 0 failing runs | `after-sample.log`: `samples 12800 failures 0` |
| `bash scripts/check.sh` | 0 | `check.log`: fmt, clippy `-D warnings`, nextest `693 tests run: 693 passed (1 slow), 1 skipped`, doctests, hygiene |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | `windows-check.log` |

All commands ran on Linux x86_64. The macOS manual stress job `probe-stress-macos.yml` (option `connection`, 200 iterations) was dispatched on this branch: MACOS_RUN.

DEFERRED VERIFICATION (runner lands in Todo 21): not applicable. This todo changes only a server integration test and adds no RPC surface, so no conformance scenario is added or declared.

## QA

- happy: `for i in $(seq 200); do cargo nextest run -p ovis-terminal-server --test connection --locked || exit 1; done`, run under `for i in $(seq 8); do (while :; do :; done) & done`. The exit status was 0, recorded in `stress-200.log`. Only the 8 busy-loop PIDs the script started were killed, each asserted > 1.
- failure: the pre-fix tree under the same loop passed 200/200 on this host (`before-stress.log`, `before-stress-unpinned.log`). The race needs oversubscription to show here, so the failure was reproduced with `sample.sh`: 25 of 12,800 runs failed (`before-sample.log`).

## Adversarial

- **Ordering.** The 3 sampled `["a", "b", "m"]` failures show that `m` can also settle after both reads. Event delivery is still asserted (`events.len() == 1`, `seq == "1"`) independent of reply position.
- **Serial dispatch regression.** If the driver awaited each handler in turn, `a` would never be released. `expect_next` would then fail at the `WAIT` bound, so the test still detects it.

## Contract checklist

- Wire encoding, error enums, identities: unchanged. No product code or public API changed.
- Holder wire generation stays 2.
- No `#[allow]`/`#[expect]`, no placeholder TODO, no sleeps or retries.

## Seams

None.

## Risks

- The reproduction on this host needs oversubscription: the plan's 8-busy-loop nextest loop did not fail here within 200 runs.
- Sightings 2 and 3 had no captured output. If either was a different failure (sighting 3 failed at setup in 0.00s), it is not addressed here.
