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
