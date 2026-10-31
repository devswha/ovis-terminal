# Task 76 investigation

Base: origin/integration, 8db5609. Holder wire generation stays 2.

## Hypotheses

1. The waiter reaps the target before shutdown signals its group. Distinguish
   with the wait result, PID/start-token lookup and process table on delivery
   failure. Fix: serialize reaping.
2. A group survives its leader. Distinguish with a process table containing
   PID, parent, group, state and ownership. Fix: verify incarnation.
3. A live target rejects delivery. Distinguish with a matching start token and
   live process table entry at failure. Preserve the typed delivery error.

## Source observations

`wait_for_exit` calls `Child::wait` outside the holder state mutex, then waits
for output drainage before the pump removes `Terminal::live`. `kill_all`
holds that mutex but signals every `live.pid` without checking the start
token or the waiter. Thus the output lifecycle is not process-liveness evidence.

The requested task-6 gate-review path does not exist in the main checkout or
the execution checkout. The landed `ShutdownReport`/`WorkersJoined` source
and task-74 evidence supply its actual join contract.

## Artifact ledger

- Temporary test-only diagnostics in `holder/control.rs` and
  `holder/threads.rs`: remove after native runner diagnosis.
- Keep the targeted manual workflow and final verification logs.
- No edits outside the task's permitted holder, workflow and evidence paths.
