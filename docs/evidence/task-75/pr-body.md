## Before

Task 75 follows todo 13. The supplied regression labels are GAP-026 / IDEAL-053;
the behavior proved here is shutdown failure accounting.

PR #19, head b882b87, job 124433653620 failed at
`tests/connection/bound.rs:137` after the client received backpressure and EOF:

```text
assertion `left == right` failed
  left: None
 right: Some(1)
```

The durable excerpt is `docs/evidence/task-75/ci-before.log`; its source is
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-75-bound-closing-failure-job-124433653620.log:1155-1229`.
The two unforced bound tests passed ten consecutive pre-fix load runs
(`baseline-load.log`), so they alone did not reproduce the schedule reliably.

The new forced-race test did reproduce it before changing product behavior:

```sh
cargo nextest run -p ovis-terminal-server --locked \
  -E 'test(shutdown_joins_and_counts)' --nocapture
# exit 100; pre-fix-forced-race.log
```

```text
FORCED_RACE_REPORT=ServeReport { accepted: 1, failures: {"connection_io": 1}, panicked: 0, aborted: 0 }
assertion `left == right` failed
  left: None
 right: Some(1)
```

The initial missing-join hypothesis was not the cause. Shutdown already drains
and joins its accepted connections, including aborted tasks. The writer can
observe overflow and close independently while the driver has not yet observed
the same overflow. When shutdown wins the driver's biased select, a send into the
writer's now-closed queue replaces the backpressure cause with `connection_io`.
The joined task therefore supplies the wrong outcome to an otherwise correctly
drained report.

## After

`conn/cause.rs` owns one set-once first-terminal-cause cell and final writer/result
resolution. A budget refusal records overflow before publishing its signal; the
driver records another terminal path as soon as its select loop ends; and the writer
records genuine encode, write, or shutdown failures where they occur. Whichever
record wins stays authoritative.
The final driver result uses backpressure only when overflow won that ordering,
so later teardown errors cannot replace peer-visible backpressure and a later
escaped-notifier overflow cannot replace an earlier connection I/O failure. The
accept-loop join/grace behavior is unchanged. Extracting the ordering cell and
writer resolution leaves `driver.rs` at 237 pure lines, below the 250-line limit.

Local evidence is Linux x86_64 with the isolated Rust/linker environment from the
brief, after fast-forwarding to integration base
`8db5609986d000997b6344d230523c62d290e694`. `source-sha256.log` identifies the
nine Rust sources used for verification.

```sh
cargo nextest run -p ovis-terminal-server --locked \
  -E 'test(shutdown_joins_and_counts)' --nocapture
# exit 0; forced-race-pass.log
```

```text
FORCED_RACE_REPORT=ServeReport { accepted: 1, failures: {"backpressure": 1}, panicked: 0, aborted: 0 }
Summary [   3.173s] 1 test run: 1 passed, 33 skipped
FORCED_RACE_EXIT=0
```

AdversarialVerify round 1 found the inverse ordering missing. The new deterministic
test `io_before_overflow_keeps_its_terminal_cause` first makes the transport return
an I/O error, reads the actual `$/closing{reason:"connection_io"}`, then publishes
overflow through a retained real notifier while writer shutdown is gated.

```sh
cargo nextest run -p ovis-terminal-server --locked \
  -E 'test(io_before_overflow_keeps_its_terminal_cause)' --nocapture
# pre-fix exit 100; round-2-pre-fix.log
# EARLIER_IO_FINAL_ERROR=Backpressure { limit: 67108864, queued: 0 }

cargo nextest run -p ovis-terminal-server --locked \
  -E 'test(io_before_overflow_keeps_its_terminal_cause) |
      test(shutdown_joins_and_counts_an_overflow_before_the_driver_can_finish)' \
  --nocapture
# exit 0; round-2-ordering.log
# I/O first -> connection_io; overflow first -> backpressure

# round-2-ordering-stress.log
# ROUND_2_ORDERING_RESULT=50/50 PASS
# 50 runs x 2 ordering tests = 100 passing executions under eight CPU workers

bash scripts/check.sh
# exit 0; round-2-check.log
# Summary [ 119.909s] 368 tests run: 368 passed (1 slow), 1 skipped
# check.sh: all steps passed
# ROUND_2_CHECK_EXIT=0
```

```sh
# Executed in a persistent background shell with its log monitored:
bash docs/evidence/task-75/stress.sh
# The script owns and reaps eight sha256sum /dev/zero load workers.
# stress.log: STRESS_RESULT=100/100 PASS
# 100 runs x 34 tests = 3400 passing server test executions
bash scripts/check.sh
# exit 0; check.log
# Summary [ 116.047s] 365 tests run: 365 passed (1 slow), 1 skipped
# check.sh: all steps passed
# CHECK_EXIT=0
```

The one workspace skip is the pre-existing store `transactions::crash_child`
fixture, explicitly invoked by its parent crash tests; no test was skipped or
weakened here.
The first gate found two Clippy violations in the new selection expression.
They were fixed without suppressions; `check-initial-clippy.log` records that
failed attempt. The final complete gate passed. The interrupted earlier stress
attempt is retained separately as `stress-superseded.log`, not counted as proof.

DEFERRED VERIFICATION (runner lands in Todo 21):

```sh
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- \
  --binary target/debug/ovis-terminal --scenario part-a/task-75 --isolated \
  --case all --evidence "$ATTEMPT_DIR/task-75-ovis-terminal-v1.json"
# conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
# exit 2; conformance-attempt.log
```

This is the actual allowed deferred attempt, not a successful CLI conformance
claim. The real Unix socket component behavior was exercised as described below.

## QA

- `cargo nextest run -p ovis-terminal-server --locked -j 8 -E 'test(bound::)' --nocapture`
  exited 0 (`socket-qa.log`). Nextest serializes `--nocapture` runs; the hundred-run
  stress command does not use that flag and uses eight test workers.
- Round-2 `cargo nextest run -p ovis-terminal-server --locked` exited 0 with
  37 passed and zero skipped (`round-2-server.log`). The two real socket bound
  tests passed again with their peer-visible closing record (`round-2-socket-qa.log`).
- The record-boundary peer actually receives `$/closing` with reason
  `backpressure`, decimal-string limit `"67108864"` and queued bytes `"0"`, then
  EOF. Shutdown must report `backpressure: 1` and `aborted: 0`.
- The permanently unread peer reaches joined progress with `backpressure: 1`
  without server shutdown; another real authenticated connection still answers
  `doc.get`. Its final shutdown report retains the backpressure count.
- The forced-race test uses real bind/accept, OS credentials, authentication,
  dispatch, overflow, writer, and shutdown. A test-only gate pauses driver polling
  while the writer sends closing and EOF. Shutdown runs concurrently and is
  observed after signalling connections, yet remains unfinished until the gate
  is released. The resulting report counts one accepted connection, one
  backpressure failure, zero panics and zero forced aborts.
- The inverse-order test uses real authentication, dispatch, notifier budgeting,
  writer output and driver result selection over a duplex transport. It injects
  only the read error and writer-shutdown completion barrier. The peer receives
  `connection_io` before overflow is triggered, and the final error remains
  `connection_io`.

## Adversarial

- **Ordering:** one forced schedule publishes overflow before shutdown can resume
  the driver; the inverse publishes a transport I/O failure and peer-visible
  closing before an escaped notifier overflows during teardown. Both directions
  assert the first terminal cause.
- **Flaky tests:** final hundred-run result is recorded before submission.
  `load-workers.log` records eight `sha256sum /dev/zero` processes consuming
  approximately one CPU each. The final stress loop uses no retries.
- **Round-2 ordering stress:** both inverse orderings passed together 50/50 under
  eight independently observed CPU workers, with no retries or sleeps.
- No fixed sleeps, wire changes, fake connection outcomes, or loosened assertions.
  The only original-test changes add the full report to mismatch diagnostics.

## Contract checklist

1. **Unchanged:** no emitted or accepted JSON fields were added or changed, and
   no u64 wire representation was changed. Existing decimal-string and u64::MAX
   coverage remains in the passing suite; this PR adds no new round-trip claim.
2. **Unchanged:** no boundary error type or kind/hint/source/Display contract was
   added or changed. The existing typed `ConnectionError::Backpressure` is reused.
3. **Unchanged:** no identity type, primitive identity, optional identity, or
   generation/token/offset field was added or changed.
4. Tests assert real socket events, pending shutdown, and the final joined
   outcome. No protocol/wire/store round-trip implementation changed, so no new
   property test is applicable; the existing proptest dependency and tests remain.
5. No `#[allow]`, new `#[expect]`, placeholder, production unwrap/expect/panic,
   numeric cast, unsafe code, dependency, or prose-pinning test was added.
6. The before/after logs state what actually ran. CLI conformance is explicitly
   deferred with its literal exit-2 result. Direct programming/remove-ai-slops
   review is in `review.md`.

## Seams

No public API, wire schema, dependency, manifest, lockfile, or other crate changes.
The listener's poll gate and shutdown notification exist only under `cfg(test)`.
The private `conn/cause.rs` module owns the first-cause cell and writer/result
resolution. Budget, driver and writer record their existing terminal events
against it. No new externally configurable seam exists.

## Risks

- Local execution is Linux x86_64. Native macOS/Windows coverage belongs to the
  PR's CI lanes; no local cross-platform execution is claimed.
- The first cause is ordered by the successful set of one `OnceLock`: overflow
  publication, driver termination and writer failure compete exactly once. Later
  causes remain observable to their immediate caller but cannot rewrite the final
  connection outcome.
- The shutdown grace and classification of tasks that require forced abort
  remain unchanged. The regression asserts no forced abort in its scenario.
- Rust and shell LSP executables are unavailable locally. Compiler, Clippy,
  rustfmt, shell syntax checking, and repository hygiene were used instead.
