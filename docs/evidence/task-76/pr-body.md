## Before

Task 76 closes the GAP-022 / IDEAL-031 shutdown regression. Baseline: absent
from the original herdr/hc transcripts; this is an ovis-terminal native
regression following todos 6 and 74.

PR #20, head `bd85265`, failed in macos-latest job 124452993931:
https://github.com/devswha/ovis-terminal/actions/runs/41172415356/job/124452993931

```text
FAIL ovis-terminal-holder unix::holder_tests::newer_fence_revokes_queued_input_behind_a_blocked_write
holder_tests.rs:61:23:
joined: Signal { source: Delivery { source: Os { code: 1, kind: PermissionDenied, message: "Operation not permitted" } } }
exit: 100
```

Original excerpt: `docs/evidence/task-76/ci-before.log`, sourced from
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-76-macos-holder-shutdown-eperm-job-124452993931.log:904-949`.

The native diagnostic harness reproduced this at iteration 42:
https://github.com/devswha/ovis-terminal/actions/runs/41173181430
(head `882f2dc`, Darwin 25.6.0 arm64, Rust 1.99.1).
`macos-before.log:1074-1076` contains the waiter's actual SIGKILL exit status,
shutdown's group-addressed EPERM, and a post-failure ProcessGone lookup for
PID 6870. The following complete process table has neither that PID nor a
member of PGID 6870. These post-syscall diagnostics capture the exit/reap
window; they do not alone order reaping against syscall entry.

The original code used the retained live PTY as process-liveness evidence.
The waiter reaped outside the holder mutex, but `Terminal::live` remained
until output completion. Shutdown could therefore signal an exited/reaped
process group. Darwin's `killpg1` excludes zombies from eligible recipients
and returns EPERM on its POSIX path if a found group has no eligible
recipients, whereas a missing group returns ESRCH:
https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/kern_sig.c.
This source explains the error semantics; it is not kernel instrumentation.

Deterministic Linux red proof, before changing shutdown/waiter behavior:

```text
$ cargo test -p ovis-terminal-holder --locked --lib shutdown_reports_already_exited_when_waiter_reaped -- --nocapture
assertion `left == right` failed
  left: 0
 right: 1
test result: FAILED. 0 passed; 1 failed
exit: 101
```

The additive report field initially defaulted to zero; only the implementation
of the requested outcome was absent. `forced-reap-before.log` preserves the
run. Tracing the retained pre-fix executable also shows a successful reap
followed by `kill(-1551086, SIGKILL) = -1 ESRCH`
(`forced-reap-before.strace`, `trace-before.log`, exit 101).

## After

The waiter blocks in safe `rustix::process::waitid` with WEXITED|WNOWAIT,
leaving the PID reserved. It then takes the holder state mutex, reaps through
`Child::wait`, and records `WaiterState::Reaped` before unlocking. The existing
output/input completion barriers and actual worker joins remain separate.

The shared signal path holds that same mutex through PID/start-token and
non-reaping exit-readiness checks and delivery. A previously reaped target
cannot cross that boundary. A target that exits during delivery is rechecked
only for EPERM/ESRCH; an error becomes `AlreadyExited` only with actual exit
or changed-incarnation evidence. Live-target EPERM remains a sourced
`HolderError::Signal`. Shutdown returns the non-error count in
`ShutdownReport::already_exited`.

Fixed product commit: `824ad606e13e725f3c0b2421a7c1aabdede97c31`.
`source-sha256.txt` identifies the exact source used for Linux verification
and the native dispatch. Later evidence-only changes do not change it.

Linux x86_64, Rust 1.99.1, isolated Cargo/rustup environment and local-only
Zig linker wrapper:

```text
$ cargo test -p ovis-terminal-holder --locked --lib shutdown_reports_already_exited_when_waiter_reaped -- --nocapture
test result: ok. 1 passed; 0 failed
exit: 0

$ cargo nextest run -p ovis-terminal-holder --locked --no-tests=fail
Summary: 46 tests run: 46 passed, 0 skipped
exit: 0

$ bash docs/evidence/task-74/stress.sh 50
CPU load workers: 8; command: cargo nextest run -p ovis-terminal-holder --locked -j 8 --no-tests=fail
RUN 50 exit: 0
PASS 50/50
exit: 0

$ bash scripts/check.sh
Summary: 368 tests run: 368 passed (1 slow), 1 skipped
workspace-hygiene: 0 violation(s) in 360 file(s) checked
check.sh: all steps passed
exit: 0

$ cargo metadata --locked --no-deps --format-version 1
exit: 0
```

Every loaded Linux run passed all 46 tests without retries or skips:
2,300 passing test executions. The inherited task-74 driver starts, terminates
and joins eight independent CPU workers and gives each invocation a fresh
short TMPDIR to avoid stale PID-named sockets. `linux-stress.log` contains
all 50 run exit statuses and summaries.

Logs: `forced-reap-after.log`, `holder-after.log`, `holder-lint.log`,
`check.log`, and `linux-stress.log` under `docs/evidence/task-76/`.
The inherited repository-wide skip is the store `crash_child` subprocess
fixture exercised by its parent tests; the holder suite has no skips.

The fixed syscall trace executes the same exact forced-reap test using
`strace -f -e trace=kill,wait4,waitid`. It shows WEXITED|WNOWAIT, successful
reaping, and **no kill syscall** (`forced-reap-after.strace`,
`trace-after.log`, exit 0). This proves signal omission, not merely error
swallowing.

Native acceptance was dispatched with:

```sh
gh workflow run probe-stress-macos.yml --repo devswha/ovis-terminal \
  --ref lane/t76-holder-shutdown-reaped-target -f probe=holder-shutdown
```

Run: https://github.com/devswha/ovis-terminal/actions/runs/41173999276,
job 124457661774, **success**, head
`824ad606e13e725f3c0b2421a7c1aabdede97c31`.
Native Darwin 25.6.0 arm64, Rust 1.99.1:

```text
$ cargo nextest run -p ovis-terminal-holder --locked --lib --no-tests=fail
Summary: 36 tests run: 36 passed, 0 skipped
exit: 0

# Each of the 200 fail-fast loop iterations:
$ cargo nextest run -p ovis-terminal-holder --locked --lib newer_fence_revokes_queued_input_behind_a_blocked_write --no-tests=fail --success-output immediate
Summary: 1 test run: 1 passed, 35 skipped
PASS 200/200
RESULT 200/200
job exit: 0
```

`macos-stress.log` contains all 200 consecutive ordered `PASS n/200` receipts
and 200 successful one-test summaries, plus the initial 36-test library
pass. The 35 filtered tests per targeted invocation are intentional selection,
not disabled tests; they all ran in the initial library suite. All four
new regression/adversarial tests passed natively there as well.

### DEFERRED VERIFICATION (runner lands in Todo 21)

DEFERRED-BY-PLAN. The prescribed assembled-binary attempt is retained
verbatim in `conformance.log`:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-76 --isolated --case all --evidence "$ATTEMPT_DIR/task-76-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit: 2
```

`ATTEMPT_DIR` is
`/home/devswha/workspace/ovis-terminal-wt/lane-t76/.omo/evidence/ovis-terminal-v1/task-76/`.
The real holder socket/PTY tests are not a claim of assembled-runtime
conformance.

## QA

- The forced-reap fixture launches a real PTY child and runs the real waiter
  before starting any output pump. It observes the actual zero exit status,
  verifies the live PTY remains retained without an Exit record, and calls
  shutdown. No worker handles were registered in this fixture; actual joins
  are covered separately by the existing worker-lifetime test.
- The zombie fixture uses native WNOWAIT to force the opposite ordering:
  shutdown sees a waitable child before the waiter reaps it. It then runs the
  real waiter to clean up. Both schedules must report one already-exited
  target.
- Real Unix-socket `happy` and `failure` scenarios pass in every complete
  holder run, including PTY input, output/Exit ordering, blocked writes,
  bounded queue rejection, control delivery and worker shutdown.
- The manual macOS selection runs the whole holder library once and then
  the originally failing named test exactly 200 times, stopping on failure.
  The default platform-stress selection is retained.

## Adversarial

- **pid_reuse:** the start-token mismatch test leaves a real target live,
  replaces only retained incarnation evidence, and requests a real kill.
  Restoring the evidence and querying native exit readiness must show the
  target survived. This simulates stale identity evidence; it does not
  claim to force numeric PID reuse.
- **ordering:** deterministic reaped-before-output-completion and
  zombie-before-reap schedules cover both sides of waiter/shutdown ordering.
  The syscall traces prove that the pre-fix reap was followed by a signal
  and the fixed reap was not.
- **live EPERM:** a typed EPERM delivery result is supplied directly to the
  reclassification function while real kernel/start-token checks establish
  a live target. Its kind and original OS source must survive. This tests
  classification, not an OS permissions denial for an owned child.
- **flaky_tests:** all 50 complete Linux runs use eight load workers and
  nextest `-j 8`, fail on the first nonzero status, and retry no tests.
  No sleeps, polling delays or test serialization were added.

## Contract checklist

1. **Unchanged:** holder wire generation stays **2**. No JSON/wire counter or
   record changed. Existing canonical-decimal-string/property tests,
   large-counter message coverage and
   `maximum_counters_round_trip_exactly_through_the_socket` cover exact
   `u64::MAX` behavior; they run in the complete holder suites.
2. The existing `#[derive(thiserror::Error, Debug)]` holder boundary gains
   sourced process-start/process-wait observation variants, stable kinds and
   hints. Existing sourced signal errors are preserved for live EPERM.
   No handwritten Error implementation or stringly failure was introduced.
3. Existing PID, terminal/process incarnation and start-token newtypes are
   reused. The only new lifecycle state is a private closed waiter enum.
4. Tests assert native process state, real signal omission/survival, typed
   outcomes and error-source preservation. Existing protocol/wire/store
   proptest dependencies and round-trip coverage are unchanged.
5. No allow/expect lint suppressions, numeric casts, placeholder code,
   generic EPERM swallowing, sleeps/retries or prose-pinning tests.
   Strict Clippy, formatting and the 250-pure-LOC/three-parameter gate pass.
6. Evidence distinguishes real native/syscall/socket execution from the
   typed-error classification input and the deferred conformance scaffold.
   No platform or permissions result is claimed beyond what ran.

## Seams

The holder's non-wire public API adds `ShutdownReport::already_exited` and
two `HolderError` observation variants; tests are updated in this same PR.
Worker join outcomes retain their existing meaning. Holder wire generation,
wire schema, JSON representation, and settled-contract checklist items are
unchanged.

The authorized manifest seam adds the already-locked rustix 1.1.6 as a direct
Unix dependency with its `process` feature. The generated Cargo.lock change
is one dependency-list entry; no dependency version or new unsafe block is
introduced. No other crate's implementation or API changes.

## Risks

- macOS's existing `ps` start token has one-second granularity; Todo 55 owns
  stronger cross-platform consistency. The new reap/delivery mutex and
  non-reaping wait prevent reuse inside this holder's delivery boundary.
- A descendant retaining a PTY slave can still delay output completion.
  Shutdown does not signal a recycled/unauthenticated group merely to force
  EOF after the original target is gone.
- Rust/YAML LSP servers are unavailable locally. Compiler, Clippy, rustfmt,
  repository hygiene and actual workflow execution supply validation.
- The requested task-6 gate-review file was unavailable at its specified
  path and in the searched workspaces. The landed join-report API/tests and
  task-74 evidence were read instead.
