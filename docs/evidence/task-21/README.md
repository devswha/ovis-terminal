# Task 21 evidence

Host: Linux x86_64, toolchain 1.99.1. Branch lane/t21-conformance-runner, which includes origin/integration ef1d8c1.

| File | What it is |
|---|---|
| `qa-summary.log` | Summary line and exit status of the two QA commands (`--case happy`, `--case failure`) |
| `qa-happy-transcript.log`, `qa-failure-transcript.log` | `--manual-transcript` output of those runs: every write and decoded record with its offset and size; the credential is redacted |
| `task-21-ovis-terminal-v1.happy.json`, `task-21-ovis-terminal-v1.failure.json` | Evidence files of those runs |
| `adversarial-probes.log` | Zero-assertion, unknown-key, wrong-id, fake-binary and missing-binary probes, each exit 1 with its kind |
| `check-summary.log` | `bash scripts/check.sh` on the merged head: exit 0, 523 tests passed |
| `pr-body.md` | The PR description |

macOS stress (`probe=conformance`, 25 iterations): https://github.com/devswha/ovis-terminal/actions/runs/41201714418

## Descriptor rule

Every key in a scenario or fixture file is in one of three classes:
- **Asserted.** It produces an assertion in the evidence. Wire `decodes`/`error` are checked by decoding the bytes before `send_wire` writes them.
- **Descriptor.** Scenario `title`, case `summary`, and a fixture's `descriptors` map. These are copied into the evidence and never asserted.
- **Rejected.** Anything else: `deny_unknown_fields` fails the load with `scenario_invalid` at its field path.

A file without `"format": "ovis-conformance/1"` is a declaration and is reported as `scenario_not_executable`; it is never run with skipped keys.

## Windows

`RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0. The Unix-only fixture modules share their caller's module-level `cfg(unix)`; off Unix, fixture mode is a typed `fixture_failed` ("bind a listener"). There are no lint attributes.
