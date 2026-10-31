# Task 73 investigation

Lane base: `bc73b938a6c21d0bda208c75b75b94a10ddea65e`.

The original failure is CI job 124348370980 on PR #12, head
`1e397d64736c551d4234e6f9ef993f0f3a09f5da`, macOS 15.8.2 aarch64.
The source log is
`/tmp/ulw-ovis-yCJzPH/evidence/g6/task-73-macos-happy-failure-job-124348370980.log`.
Lines 1095-1098 show:

```text
thread 'happy' (27172) panicked at crates/ovis-terminal-holder/../../probes/platform/tests/unix_happy.rs:45:5:
assertion `left == right` failed
  left: Grid { columns: 80, rows: 24 }
 right: Grid { columns: 132, rows: 40 }
```

## Hypotheses before reproduction

1. The shell's command substitution observes the slave before resize is visible.
   A fresh slave-side report differs from the already updated master.
2. A previous report is being mistaken for the new one. Distinct markers and
   complete frame consumption distinguish old buffered output from a new query.
   `OutputStream::read_until` drains through the marker and preserves the tail.
3. The backend returns before issuing the ioctl. Source inspection refutes this
   candidate: portable-pty 0.9.0 `PtyFd::resize` calls `TIOCSWINSZ` synchronously,
   checks its return value, then returns. `PtySession::resize` forwards that result.
   This does not prove when the shell observes or acts on the change.

## Artifact ledger

- Keep the manual-only stress workflow as requested.
- Keep reproduction and final verification evidence in this directory.
- No holder edits, dependency edits, debugger hooks or temporary source changes.
- LSP lookup was unavailable: `rust-analyzer` is not installed on the host.
  Compiler and Clippy verification will provide source diagnostics.

## Scheduling seam and dispatch blocker

At the user's direction, merged `origin/lane/t72-stall-probe-determinism` into
this lane as `32f12df`. Its failure-probe tests are part of the reproduction
environment, not task 73 implementation edits. The workflow runs both the
requested single-`happy` loop and a whole-`platform_probe` loop, with independent
fail-fast jobs and no serialization of tests.

The initial pre-fix harness is `ea6f1122951d6544b37061f3c997c23433af92e3`.
`bash scripts/check.sh` passed on Linux x86_64 before pushing it (191 tests
passed, 1 skipped; exit 0). Dispatch failed before creating a run:

```text
$ gh workflow run probe-stress-macos.yml --ref lane/t73-macos-resize-probe --repo devswha/ovis-terminal
HTTP 404: Not Found (https://api.github.com/repos/devswha/ovis-terminal/actions/workflows/probe-stress-macos.yml)
exit 1
```

GitHub does not register this newly added manual-only workflow from a branch
push. The default branch is `main`. Registration on that branch is outside
this lane's authorized scope; the user has been notified.

The user then authorized a lane-only push trigger to bootstrap and run stress
without changing protected `main`. The default-branch registration monitor
was stopped. Both dispatch attempts (CLI resolution and direct REST POST)
returned 404 before any run was created.

## Additional source-level candidate (not yet runtime-confirmed)

Apple's shipped Bash 3.2 readline has a read-then-write winsize sequence:
`lib/readline/rltty.c:set_winsize` reads `TIOCGWINSZ`, then writes that same
value with `TIOCSWINSZ`. `get_tty_settings` calls this during terminal
preparation. Source:
<https://github.com/apple-oss-distributions/bash/blob/main/bash-3.2/lib/readline/rltty.c>.

This introduces a different candidate from delayed XNU publication: after
the first `GRID` report has been printed, the interactive shell may be
preparing its next readline. If its read sees 80x24, the test resizes to
132x40, and readline then writes its saved 80x24, a fresh slave query will
correctly see the overwritten size. A prompt-ready barrier or a
noninteractive shell command waiting in a builtin read would avoid this
unowned write, whereas repeated slave queries alone could time out forever.
This is a source-level mechanism, not a confirmed explanation of the CI
failure. Runner reproduction and a distinguishing observation remain required.

After merging task 72, `bash scripts/check.sh` passed again on Linux x86_64:
193 tests passed, 1 skipped, exit 0.

## Runner setup correction

Run <https://github.com/devswha/ovis-terminal/actions/runs/41140148551>
failed before testing: `macos-latest` now selects `macos-26-arm64`,
image `20261013.0424.1`, and Xcode 16.4 does not exist there:

```text
xcode-select: error: invalid developer directory '/Applications/Xcode_16.4.app'
```

This is not a reproduction of the resize bug. The stress matrix now includes
both `macos-latest` (Xcode 26.6, from that image's published inventory) and
`macos-15` (Xcode 16.4, matching the original failure environment).
Each image runs both single-happy and full-binary 100-iteration jobs.

## Pre-fix reproduction

Run <https://github.com/devswha/ovis-terminal/actions/runs/41140283121>,
head `96f22b9884bc6c42ecedf6621d2c442a0b4612f1`, reproduced the original
failure in `stress (macos-latest, all)`, job 124352609444, iteration 2.

```text
2026-10-30T22:47:23.1677220Z ITERATION 2/100
2026-10-30T22:47:23.7453500Z         FAIL [   0.061s] (3/4) ovis-terminal-holder::platform_probe happy
2026-10-30T22:47:23.7459020Z     thread 'happy' (26122) panicked at crates/ovis-terminal-holder/../../probes/platform/tests/unix_happy.rs:45:5:
2026-10-30T22:47:23.7459500Z     assertion `left == right` failed
2026-10-30T22:47:23.7459810Z       left: Grid { columns: 80, rows: 24 }
2026-10-30T22:47:23.7460070Z      right: Grid { columns: 132, rows: 40 }
```

Next diagnostic change retains the original assertion and queries the master
only when that assertion fails. If it is also back at 80x24, the resize was
overwritten rather than merely invisible on the slave. No synchronization
fix is present in this diagnostic revision.

## Distinguishing runner evidence and fix

Diagnostic run <https://github.com/devswha/ovis-terminal/actions/runs/41140598357>,
head `7b0817d`, job 124353637950 (`macos-15, happy`), failed at the
**master assertion**, before the second slave query:

```text
2026-10-30T22:50:48.3503180Z     thread 'happy' (53260) panicked at crates/ovis-terminal-holder/../../probes/platform/tests/unix_happy.rs:44:5:
2026-10-30T22:50:48.3503840Z     assertion `left == right` failed
2026-10-30T22:50:48.3504140Z       left: Grid { columns: 80, rows: 24 }
2026-10-30T22:50:48.3505210Z      right: Grid { columns: 132, rows: 40 }
```

The synchronous resize result is therefore being overwritten; this is not
only delayed visibility on the slave or a buffered first report. Apple
readline's `set_winsize` provides the competing writer. The first `GRID`
line releases the test before readline has finished preparing the next
input. Its old-size read can precede the master resize and its old-size
write can follow it. Extra tests alter scheduling but share no terminal.

The fix gives the shell a recognizable prompt and consumes it after each
complete slave-side grid report. In Apple's readline, terminal preparation
(`rl_prep_term_function`) precedes `readline_internal_setup`, which prints
the prompt:
<https://github.com/apple-oss-distributions/bash/blob/main/bash-3.2/lib/readline/readline.c>.
Thus resize cannot race the previous command's next-input preparation.
The slave is still queried and must still report exactly 132x40 after
resize. No sleeps, retry-to-green, serialization, or session/holder changes.

The readiness wait has a bounded deadline and prints the last slave report
and current master size on failure. Its adversarial test holds a real shell
in a builtin read, changes the actual master size, and uses a zero deadline.
It catches only the readiness assertion so setup failures cannot pass it;
the shell is then released and joined.

## Local fixed-code verification

Linux x86_64, Rust 1.99.1, isolated Zig C/linker wrapper:

```text
$ cargo nextest run -p ovis-terminal-holder --test platform_probe shell_readiness_deadline --locked --nocapture
shell readiness failed: slave Grid { columns: 80, rows: 24 }, master Ok(Grid { columns: 132, rows: 40 }): Err(Timeout { waited: 0ns })
Summary [   0.014s] 1 test run: 1 passed, 4 skipped
exit 0
$ bash scripts/check.sh
Summary [   5.020s] 194 tests run: 194 passed, 1 skipped
check.sh: all steps passed
exit 0
```

The printed panic is the intentionally caught deadline failure, not an
uncaught failing test. All setup runs outside the catch.
Full logs are in the lane's ignored attempt directory pending final promotion.
The changed source is 185 pure lines; no new dependencies, public types,
wire/JSON fields, unsafe code, lint suppressions, or test serialization.

## Acceptance stress dispatches

After the lane-only push registered the workflow, manual dispatch became
available. Three consecutive `gh workflow run probe-stress-macos.yml
--ref lane/t73-macos-resize-probe --repo devswha/ovis-terminal` calls
targeted fixed head `89bb89b41c48cf221249a4be0cf9b40112840210`:

- <https://github.com/devswha/ovis-terminal/actions/runs/41141232863>
- <https://github.com/devswha/ovis-terminal/actions/runs/41141274758>
- <https://github.com/devswha/ovis-terminal/actions/runs/41141298345>

Run 41141232863 passed all four jobs, each with 100 `PASS n/100` lines,
100 successful nextest summaries, 100 actual `happy` passes and
`RESULT 100/100`. Its four complete logs are promoted here with names
`stress-41141232863-*.log`; their SHA-256 digests match the downloaded artifacts.
The other two runs also passed all four jobs at 100/100. Each run covers 100 happy-filter and 100
whole-binary iterations on each of macOS 15 and macOS 26 (`macos-latest`).
The redundant fixed-code push run 41141198738 was cancelled before testing;
the diagnostic run 41140598357 was cancelled after its failing job log
was preserved, to free native runner capacity. Neither is counted as green.
The original pre-fix run 41140283121 was left running to its natural result.

The temporary lane push trigger has now been removed. The retained workflow
is manual-only, as originally required.

The pre-fix run 41140283121 completed naturally with conclusion `failure`.
Both whole-binary jobs and the macOS 15 single-happy job failed. The
macos-latest single-happy job passed 100/100 on the same unfixed source.
This confirms that a successful single-test loop alone would not establish
determinism. All four artifacts and `gh run view 41140283121 --log-failed`
output were downloaded into the ignored attempt directory.

The exact requested `happy` filter now also matches the new deadline test's
`unix_happy::` module path: it runs two tests per iteration, including the
actual top-level `happy` test. The whole-binary variant runs all five tests
(happy, failure, the two task 72 tests, and the new deadline test), with
normal nextest concurrency. No selection is empty.

All 12 complete logs are committed as `stress-<run-id>-<image>-<variant>.log`,
with artifact-matching SHA-256 digests in `stress-sha256.txt`. Across the three
consecutive dispatches: 1,200 actual happy passes, 1,200 deadline-test passes,
600 failure-probe passes and 1,200 task 72 test passes (4,200 test executions).
Each log has exactly 100 successful iteration markers and nextest summaries,
100 passes of the actual top-level `happy` test, and `RESULT 100/100`.

Initial PR #14 CI passed at head `3a61e56`, run 41141524634, including
`test (macos-latest)` job 124356677607. Its platform-probes run 41141524652
also passed all three platforms. Windows smoke retains task 12's documented
ConPTY findings, which remain owned by Todos 53/54.

Before the final push, `origin/integration` advanced to `eb9827d81286df101a8a72644d44167c7cb210ee`
(task 72 landed). The required merge was committed as `15e5a4d`.
Its changes relative to this lane were only task 72 evidence. A diff of
`probes/`, `crates/`, `bench/`, `Cargo.toml` and `Cargo.lock` against the
stress-tested source `89bb89b` was empty after the merge.
`cargo metadata --locked --format-version 1 --no-deps` exited 0; no lockfile
conflict or regeneration was needed.

The full local gate was rerun after this integration merge:
`bash scripts/check.sh` exited 0, with 194 tests passed, 1 pre-existing skip,
and zero hygiene violations in 232 files. Complete output:
`check-after-integration.log`. All 12 promoted stress-log checksums also
passed `sha256sum -c stress-sha256.txt`.
