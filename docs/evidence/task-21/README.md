# Task 21 evidence

Host: Linux x86_64, toolchain 1.99.1. Merged head da3388c (lane commit a853952 plus origin/integration ef1d8c1).

| File | What it is |
|---|---|
| `qa-summary.log` | Summary line and exit status of the two QA commands (`--case happy`, `--case failure`) |
| `qa-happy-transcript.log`, `qa-failure-transcript.log` | `--manual-transcript` output of those runs: every write and decoded record with its offset and size; the credential is redacted |
| `task-21-ovis-terminal-v1.happy.json`, `task-21-ovis-terminal-v1.failure.json` | Evidence files of those runs |
| `adversarial-probes.log` | Zero-assertion, unknown-key, wrong-id, fake-binary and missing-binary probes, each exit 1 with its kind |
| `check-summary.log` | `bash scripts/check.sh` on the merged head: exit 0, 521 tests passed |
| `pr-body.md` | The PR description |

macOS stress (`probe=conformance`, 25 iterations): https://github.com/devswha/ovis-terminal/actions/runs/41201714418
