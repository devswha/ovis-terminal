# Task 74 investigation

Base: `c1dd6cf` (`origin/integration`). Scope: the Unix holder and its tests;
the holder protocol remains generation 2.

## Original failure

Source:
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-74-holder-happy-failure-job-124394856675.log:1212-1236`,
PR #15, job 124394856675, head 483fb88.

```text
FAIL [   0.011s] ( 74/245) ovis-terminal-holder::unix happy
Error: "no output may follow Exit"
Summary [   8.246s] 77/245 tests run: 76 passed, 1 failed, 1 skipped
error: test run failed
```

## Ordering investigation

Three possible sources were traced: a late output chunk, a queued or in-flight
writer acknowledgement, and socket dispatch reordering.

The waiter sends process evidence to the pump only after its drain barrier and
slave close. The pump publishes all output before calling `record_exit`.
`HolderState` serializes those publications under its mutex. The socket has one
FIFO outbox and one writer that writes an entire encoded frame before taking the
next message. Neither path can reorder an already-enqueued acknowledgement.

The input worker, however, writes outside the holder state mutex and acquires
that mutex again to acknowledge. `record_exit` closes its queue but does not wait
for the active acknowledgement or the queued entries that `InputQueue::pop`
continues to return after close. Consequently the pump can enqueue Exit before
the writer enqueues InputWritten. Todo 6's EOF barrier covers output, not input.
The independent shutdown join report must continue to come from actual worker
joins, not from any new completion state.

## Deterministic schedule

`exit_waits_for_gated_writer_to_acknowledge_closed_queue` uses the real bounded
input queue, holder state, outbox and `write_input` worker. A channel gate keeps
the worker from acknowledging two accepted entries until output and exit
evidence have been recorded. The closed queue must yield two zero-byte
acknowledgements before Exit. A second channel bounds worker completion; the
test joins the worker before examining all four messages. No sleeps, retries or
mock acknowledgement implementation are involved. `/dev/null` supplies the file
descriptor; closed entries must never write to it.

The real PTY and Unix socket surface is exercised separately by `happy` and
`failure`, including an actual gate-release write and partial blocked writes.

## Local tooling

Rust and Bash language servers are unavailable on this host. Compiler, Clippy,
rustfmt, nextest and repository hygiene provide the executable checks.
The first load harness attempt exposed Python's forkserver default: workers
could not resolve the inline `burn` function. The harness now explicitly uses
Linux's fork context and owns, terminates and joins all eight workers.
An initial reproduction loop was interrupted by a concurrently edited test's
compilation error; it is not counted as a socket ordering failure or stress
acceptance. Subsequent evidence uses a stable source tree during each run.

The first completed full-suite stress attempt failed on run 2 with
`AddrInUse` at the existing socket fixture's bind, not an ordering assertion.
That fixture leaves PID-named sockets in the shared temporary directory.
`stress-fixture-collision.log` preserves the failure. The stress driver now
gives every nextest run a fresh short `TMPDIR` and cleans that owned directory
afterward, avoiding stale sockets and PID reuse without changing or weakening
any test. The holder implementation did not change in response.

## Result

The forced schedule failed on the original production code with
`InputWritten followed Exit` (exit 101). After the completion-state fix it
passed, as did all 100 consecutive complete holder-suite runs under eight
CPU load workers: 42 tests per run, zero skips, 4,200 passing executions.
`stress.log` retains all 100 run IDs, per-test results and zero exit statuses.
The separate real-socket happy/failure invocation passed both tests.
