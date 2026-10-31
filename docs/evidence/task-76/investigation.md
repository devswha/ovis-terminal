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
