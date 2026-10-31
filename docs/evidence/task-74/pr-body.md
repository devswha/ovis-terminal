## Before

Task 74 fixes the Exit-last-message regression reported by PR #15, job
124394856675, head `483fb88`. The original excerpt is copied verbatim to
`docs/evidence/task-74/ci-before.log` from
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-74-holder-happy-failure-job-124394856675.log:1212-1236`:

```text
FAIL [   0.011s] ( 74/245) ovis-terminal-holder::unix happy
Error: "no output may follow Exit"
```

The deterministic regression ran against the original production code:

```text
$ cargo test -p ovis-terminal-holder --locked --lib exit_waits_for_gated_writer -- --nocapture
InputWritten followed Exit
test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 31 filtered out
exit: 101
```

Full output: `docs/evidence/task-74/race-before.log`. The pre-fix `happy` test
also ran 100 times with eight CPU load workers and nextest `-j 8`; all 100
passed (`pre-fix-stress.log`). That intermittent reproduction did not expose
the bug locally; the channel-controlled schedule above did.

Root cause: the output pump serialized output before Exit, but closing the
input queue did not finish the input worker. An active write or entries still
returned by `pop()` after close could publish InputWritten after Exit.
The socket's single FIFO writer preserved that incorrect publication order.

## After

At output completion, the holder closes input admission and retains the
process exit evidence. Its internal completion enum publishes Exit only when
both output and input have ended. The input worker marks completion after its
last acknowledgement, under the same holder mutex used by output and exit.
InputRejected is already published synchronously under that mutex, so it
cannot cross the terminal's exit boundary. The FIFO socket writer writes
every earlier complete frame before writing the Exit frame.

The existing waiter-to-pump EOF/drain barrier remains intact, including the
waiter fallback if the pump is lost. Unexpected output errors are retained
when observed, before deferral. Shutdown's per-worker join report still comes
only from actual `JoinHandle::join` results.

Commands below ran on Linux x86_64, rustc 1.99.1
(`58cf8f9c3`, LLVM 23.1.1), with the isolated Cargo/rustup environment documented
in `AGENTS.md` and the local zig linker wrapper. No wrapper is committed.

```text
$ cargo test -p ovis-terminal-holder --locked --lib exit_waits_for_gated_writer -- --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 31 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --locked --test unix -- --nocapture
test happy ... ok
test failure ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
exit: 0

$ bash scripts/check.sh
Summary: 233 tests run: 233 passed, 1 skipped
workspace-hygiene: 0 violation(s) in 278 file(s) checked
check.sh: all steps passed
exit: 0

$ cargo metadata --locked --no-deps --format-version 1
exit: 0

$ bash docs/evidence/task-74/stress.sh 100
CPU load workers: 8; command: cargo nextest run -p ovis-terminal-holder --locked -j 8 --no-tests=fail
RUN 100 exit: 0
PASS 100/100
exit: 0
```

All 100 consecutive suite invocations passed all 42 holder tests, with zero
skips: 4,200 test executions, including 100 `happy`, 100 `failure` and 100
forced-order regression executions. The driver owns and joins the eight load
workers and uses a fresh TMPDIR for each invocation. Every invocation's full
output and exit code is retained in `stress.log`; there were no test retries.

`bash scripts/check.sh` also passed after the complete stress loop; its final
transcript is `check-final.log`.

Logs: `race-after.log`, `qa-socket.log`, `stress.log`, `check.log`, and `check-final.log` in
`docs/evidence/task-74/`. The one repository-wide skip is the inherited
store `crash_child` subprocess fixture, invoked explicitly by its parent
tests; no test was disabled or changed to pass. The holder stress selection
has no skips.

### DEFERRED VERIFICATION (runner lands in Todo 21)

DEFERRED-BY-PLAN: the assembled binary conformance runner is still a scaffold.
The exact attempt is retained in `docs/evidence/task-74/conformance.log`:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-74 --isolated --case all --evidence "$ATTEMPT_DIR/task-74-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit: 2
```

`ATTEMPT_DIR` was
`/home/devswha/workspace/ovis-terminal-wt/lane-t74/.omo/evidence/ovis-terminal-v1/task-74/`.
The passing socket tests exercise the real holder service and PTYs; they do
not claim assembled `ovis-terminal` binary conformance.

## QA

- `happy` drives a real `/bin/sh` PTY through the holder Unix socket,
  disconnects mid-UTF-8 character, re-adopts the same process, writes the
  gate-release input, and checks contiguous final output before Exit. Its
  subsequent Tail request must receive Tail, not any late terminal event.
  The failure diagnostic now names the unexpected `MessageKind`.
- `failure` drives a real non-reading PTY, overflows the bounded input queue,
  checks typed InputRejected bounds, applies resize and kill independently,
  and verifies actual partial/unwritten acknowledgements and exit evidence.
- The deterministic test gates the real input worker before it can process
  two accepted entries. It records final output and exit evidence, then
  releases the worker and waits for bounded completion. The real queue,
  `write_input`, holder state and outbox must produce output, both zero-byte
  acknowledgements, then Exit with final offset 5. It uses `/dev/null` as a
  descriptor for entries that must never be written. No mock replaces the
  ordering behavior; the test does not claim to run a PTY or socket itself.

## Adversarial

- Ordering: the deterministic pre-fix failure names `InputWritten`; the same
  test passes after the fix. Gate and completion channels, not sleeps, force
  the schedule. Both the existing input-first typed-error test and the new
  output-first test exercise the completion boundary.
- Flaky tests: the loaded stress driver fails immediately on any nonzero
  nextest result; it does not retry individual tests. It starts exactly eight
  independent CPU workers and uses nextest `-j 8`.
- Fixture isolation: the first full-suite attempt hit an existing PID-named
  stale socket (`AddrInUse`) on run 2. Its complete failure is preserved in
  `stress-fixture-collision.log`. The driver now gives each run a fresh short
  TMPDIR and removes only that owned directory afterward. No assertion or
  production behavior was changed for this collision.
- Existing property tests for queue fencing/bounds, retained tails, protocol
  fragmentation and adoption transitions are included in the full holder
  suite. Actual shutdown worker-join assertions are retained.

## Contract checklist

1. **Unchanged by this todo:** no JSON/wire fields, counters or protocol
   generation changes. Generation remains **2**. Existing canonical-string
   property tests and `every_message_variant_encodes_large_counters_as_exact_json_strings`
   run in the holder suite.
   `maximum_counters_round_trip_exactly_through_the_socket` asserts exact
   `u64::MAX` sequence, grant epoch and fence epoch decoding after a real
   socket write and acknowledgement.
2. **Unchanged by this todo:** no boundary error types or error serialization
   changes. Existing thiserror enums, kinds, hints and sources are reused.
3. **Unchanged by this todo:** no identity representation or public identity
   API changes. Existing terminal/process newtypes are reused. The only new
   state is a private completion enum, not a wire identity or counter.
4. The new deterministic regression asserts real publication order and
   acknowledgement counts. Existing proptest dependencies and protocol,
   store and wire round-trip tests remain intact; no new serialization path
   was introduced.
5. No new allow/expect lint suppression, sleeps, protocol changes, retries,
   placeholder implementation or prose-pinning tests. Compiler, strict
   Clippy, rustfmt and the repository hygiene gate pass. Touched files stay
   within 250 pure LOC and three parameters. `ending.rs` is 248 pure LOC;
   the existing socket test file is at its 250-line limit.
6. The evidence distinguishes deterministic worker/state testing, real
   holder socket/PTY testing, and the deferred assembled-binary scenario.

## Seams

No landed crate's public API, wire schema, dependency or lockfile changed.
Edits are confined to the task's allowed holder paths and evidence directory.

## Risks

- Local runtime evidence is Linux x86_64. Native macOS behavior remains a CI
  result, not a Linux cross-compilation claim.
- As in todo 6, a descendant retaining the PTY delays EOF and therefore Exit.
  Exit additionally waits for already accepted input acknowledgements.
- The shared PID-named temporary-socket fixture remains unchanged; the stress
  harness isolates its own runs to avoid stale-socket reuse.
- Rust and Bash language servers are unavailable locally; compiler, Clippy,
  rustfmt, shell syntax and repository hygiene checks provide validation.
