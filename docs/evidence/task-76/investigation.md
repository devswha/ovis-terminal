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

## Deterministic Linux red proof

`shutdown_reports_already_exited_when_waiter_reaped_before_output_completion`
launches a real PTY child and calls the real waiter synchronously. The output
pump has not started, so the waiter's actual exit status arrives while the
PTY remains in `Terminal::live`. Shutdown therefore sees precisely the stale
live-PTY window, without sleeps or a mock signal/wait implementation.

Only the additive report field (default zero) was introduced before this run;
shutdown and waiter behaviour were unchanged. The command
`cargo test -p ovis-terminal-holder --locked --lib shutdown_reports_already_exited_when_waiter_reaped -- --nocapture`
exited 101 with `left: 0`, `right: 1`. Full transcript:
`forced-reap-before.log`. Linux's ESRCH was swallowed by the original code;
the test requires the typed already-exited outcome, not merely no error.

The diagnostic harness passed `bash scripts/check.sh`: 364 tests passed,
one inherited skip, exit 0. Native diagnostic dispatch:
<https://github.com/devswha/ovis-terminal/actions/runs/41173181430>,
head `882f2dc`. Its `holder-shutdown` job selects the original named test
and loops up to 200 times, stopping on the first failure.

Rust and YAML language servers are not installed. Rust compiler, Clippy,
rustfmt, repository hygiene and the native workflow validate the changes.

## Native diagnosis

Diagnostic run 41173181430, job 124455250742, failed at iteration 42.
`macos-before.log:1074-1076` records, in order:

```text
TASK76 waiter pid=6870 evidence=Status { code: None, signal: Some(9) }
TASK76 shutdown group=6870 error=Delivery { source: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" } } token=Err(ProcessGone { pid: 6870 })
```

The complete process table immediately afterwards contains neither PID 6870
nor any member of PGID 6870. No PID reuse or surviving group member was observed.
The syscall was a second SIGKILL, addressed to the negative process-group ID,
not a direct PID signal. The token/table diagnostics run after the failed
syscall, so this native trace alone does not order the reap against syscall
entry: it captures the exit/reap window, with the PID reaped by observation
time. The Linux syscall toggle below forces and proves the specific
reaped-before-shutdown schedule; the second deterministic test forces a
waitable zombie before the waiter reaps. The product handles both schedules.

Darwin's `killpg1` explains why this can be EPERM rather than ESRCH:
<https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c>.
It returns ESRCH when `pgrp_find` finds no group; when a group is found but the
iteration finds no eligible signal recipients, the POSIX path returns EPERM.
Its group filter explicitly excludes zombies (`p->p_stat != SZOMB`).
The kernel source is explanatory source evidence, not instrumentation of the
runner kernel's internals. The actual runner evidence proves the reaped PID,
group-addressed EPERM and subsequent absence from the process table.

## Product fix

The waiter first blocks in safe `rustix::process::waitid` with EXITED|NOWAIT.
This reserves the zombie/PID without holding holder state or reaping.
It then acquires the same holder state mutex used by signal delivery, calls
the nonblocking-in-this-state `Child::wait`, and records `WaiterState::Reaped`
before releasing the mutex. Output drainage and Exit publication retain their
separate existing barriers.

Before signalling, the shared delivery path checks waiter state, the retained
start token against the actual PID's token, and non-reaping kernel exit
readiness. The mutex prevents the waiter from reaping between these checks and
delivery, so PID reuse cannot cross that boundary. EPERM/ESRCH are reclassified
only if a fresh check proves the target already exited; live-target delivery
failure remains the existing sourced HolderError::Signal.

Shutdown counts already-exited retained targets separately from actual
worker-join outcomes. New process-observation failures have typed HolderError
variants with stable kinds, hints and sources. The safe wait wrapper reuses
rustix 1.1.6 already in the lockfile; no unsafe block or dependency version was
added, and holder wire generation remains 2.

## Gate round 1 regression and repair

AdversarialVerify round 1 rejected head `33f4d3b`. Its real-process probe ran
`trap '' HUP; sleep 777 & exit 0`: the leader exited and was reaped, but the
same-group `sleep` retained the PTY slave. The first fix returned
`AlreadyExited` before group delivery, so the pump never observed hang-up and
`join_outcome` waited without a bound. Base `8db5609` signalled the group and
returned in 599 microseconds.

`shutdown_kills_same_group_descendant_after_leader_is_reaped` reproduces that
schedule with the real holder workers. A condition variable observes the exact
waiter reap without polling or sleeping. The pre-fix run times out after two
seconds, signals the group only for cleanup, and exits 101; its transcript is
`round-2-pre-fix.log`.

The repair separates leader state (`Live`, `Exited`, `Replaced`) from group
delivery. A reaped or waitable leader still permits SIGKILL to the original
process group because its ID cannot be reused while members remain. A replaced
live PID prevents delivery. Successful group delivery after leader exit counts
the leader in `already_exited`; ESRCH and Darwin EPERM are reclassified only
through fresh evidence, while EPERM against a matching live leader remains a
sourced `signal_failed`.

`shutdown_stops_pump_when_reaped_leader_group_is_gone` enables shell job
control so the PTY-holding descendant moves to another group, proves the
original group is absent, then runs shutdown. The absent-group path drops a
Unix-stream pump-stop endpoint. The pump drains available bytes before
observing the stop, records Exit after output, closes the remaining master
descriptors and joins all workers within the two-second test budget.

### Round-2 hunk review (inherited uncommitted worktree)

A second executor inherited the uncommitted round-2 diff. Decision per hunk:

- `signalling.rs` `LeaderState` split and group delivery after leader exit:
  **keep**. It is the B1 fix: the leader's exit no longer suppresses SIGKILL
  to its group. The `Replaced` arm before the token comparison is correct,
  because once the waiter has reaped, any process now holding that PID is a
  different incarnation.
- `failed_delivery` ESRCH -> `AlreadyExited`, EPERM -> `AlreadyExited` only
  if fresh `leader_state` is not `Live`: **keep**, with the following scope.
  For a group, Darwin EPERM means the group exists but has no member this
  caller may signal (zombies are excluded). Linux EPERM means a live member
  with different credentials. After a fresh check proves the leader exited,
  both mean that no member remains that this holder can signal. That is
  what shutdown reports. It does not prove that every member is dead (see
  Risks).
- `#[cfg(test)] fn target_exited` in production code: **fixed**. Removed;
  the tests module calls `leader_state` directly.
- `kill_all` counting `Delivered { leader_exited: true }` in
  `already_exited`: **keep**. The leader is counted, and its group is still
  killed.
- `PumpStop`/`PumpWake` socketpair dropped on `AlreadyExited`: **keep**.
  Closing the holder's master would not wake the pump, which polls a
  duplicated descriptor. Closing a descriptor another thread is polling is
  also unsound. The stop endpoint is dropped only when the group is gone, so
  the same-group test can return within budget only if group SIGKILL closed
  the descendant's slave.
- `Shared::reaped` condvar: **keep**. It is the test's event-based wait for
  the real reap, with no sleeping or polling.
- Workflow `START_TOKEN_PS_LOOKUP_NS` timing probe: **redo**. Removed. The
  200x loop now also selects `shutdown_tests::`, so both round-2 regression
  tests run natively 200 times.

### Nextest parallel "failure"

The round-2 brief reported an unidentified nextest failure under
parallelism. `cargo nextest run --workspace --all-targets --locked
--no-fail-fast -j 8` on the round-2 source exited 0: 370 passed, 1 skipped,
1 slow. The slow test is
`ovis-terminal-core::ops bounds::capacity_accepts_1024_and_rejects_1025_without_eviction`,
which took about 110 s while CPU-bound in SQLite. Its process held only
`state.db*` descriptors and spawned no children. Core sources are
unchanged on this branch, and that test reaches no holder code. No nextest
timeout is configured, so a slow test is not a failure. The previous
executor's watch probably expired on this test and was misread as a
failure; that is an inference, not observed. No failing test was
reproduced, so no fix was made. Logs:
`round-2-nextest-parallel.log`, `round-2-check.log`.

## Syscall-level toggle proof

The original red-test executable was retained by Cargo under its pre-rustix
dependency hash, allowing a second before/after observation without editing
the source being stressed. Both commands used
`strace -f -e trace=kill,wait4,waitid -o <trace>` and the exact test name
`unix::holder::shutdown_tests::shutdown_reports_already_exited_when_waiter_reaped_before_output_completion`:

- Before: `target/debug/deps/ovis_terminal_holder-ce97f9393c8639b3`.
  `forced-reap-before.strace` shows `wait4` reaping PID 1551086, followed by
  `kill(-1551086, SIGKILL) = -1 ESRCH`. The test exited 101
  (`trace-before.log`).
- After: `target/debug/deps/ovis_terminal_holder-78b959f7c07c88e8`.
  `forced-reap-after.strace` shows WEXITED|WNOWAIT, then `wait4` reaping PID
  1551102, and **no kill syscall**. The test exited 0
  (`trace-after.log`).

The mismatch adversarial test changes retained start evidence while leaving a
real child live, requests a real kill through the production delivery path,
then restores the evidence and queries kernel exit readiness before cleanup.
The child must still be live. This exercises stale-incarnation protection;
it does not claim to force the operating system to reuse a particular PID.

The live-EPERM test supplies a typed delivery failure to the reclassification
function while real kernel/start-token checks prove the child remains live.
It proves error/source preservation, not that the OS generated a permissions
failure for an owned child. No mock replaces wait, start-token lookup or signal
delivery in the end-to-end signal tests.

## Linux source verification

`bash scripts/check.sh` passed on the fixed source: 368 tests passed,
one inherited store subprocess-fixture skip, zero hygiene violations
in 360 files, exit 0 (`check.log`). The direct holder suite passed all
46 tests with zero skips (`holder-after.log`), including real Unix-socket
happy/failure scenarios and actual worker-join assertions.

`cargo metadata --locked --no-deps --format-version 1` exited 0.
`source-sha256.txt` identifies the source, manifest, lockfile and workflow
used for the loaded Linux run and the fixed native dispatch.

The prescribed assembled-binary conformance attempt exited 2:
`conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)`.
The transcript is `conformance.log`; this is DEFERRED-BY-PLAN until Todo 21,
not a passing assembled-runtime scenario.

## Acceptance receipts

- Linux: `bash docs/evidence/task-74/stress.sh 50`, exit 0. All 50 complete
  holder suites passed all 46 tests with zero skips: 2,300 test executions
  under eight owned CPU load workers and nextest `-j 8`.
  `linux-stress.log` contains all per-run exit statuses.
- Native macOS: <https://github.com/devswha/ovis-terminal/actions/runs/41173999276>,
  job 124457661774, head `824ad606e13e725f3c0b2421a7c1aabdede97c31`,
  success/exit 0 on Darwin 25.6.0 arm64, Rust 1.99.1.
  `macos-stress.log` contains the initial 36/36 library pass and all 200
  consecutive required named-test passes, ending `RESULT 200/200`.
  The 35 filtered tests per targeted invocation were all executed in the
  initial complete library pass.
- The fixed source hashes matched `source-sha256.txt` before dispatch.
  `origin/integration` remained 8db5609 at the pre-push synchronization checks.
- Temporary diagnostic source statements were removed. No added unsafe code,
  test retries, sleeps, lint suppressions or changes outside the authorized
  source/workflow/dependency/evidence seams remain.
