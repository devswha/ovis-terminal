# Task 21 evidence

Host: Linux x86_64, toolchain 1.99.1. Branch lane/t21-conformance-runner, which includes origin/integration b205dca.

| File | What it is |
|---|---|
| `qa-summary.log` | Summary line and exit status of the two QA commands (`--case happy`, `--case failure`) |
| `qa-happy-transcript.log`, `qa-failure-transcript.log` | `--manual-transcript` output of those runs: every write and decoded record with its offset and size; the credential is redacted |
| `task-21-ovis-terminal-v1.happy.json`, `task-21-ovis-terminal-v1.failure.json` | Evidence files of those runs |
| `adversarial-probes.log` | Zero-assertion, unknown-key, wrong-id, fake-binary and missing-binary probes, each exit 1 with its kind |
| `check-summary.log` | `bash scripts/check.sh` on the merged head: exit 0, 555 tests passed |
| `mutants-r2.log` | Mutants M2, M4 and M5 re-run against the round-2 tests: each killed |
| `b1-probe.log` | The reviewer's B1 probe (misspelled marker, bad name) on the real scenario root: exit 1, both files invalid |
| `all-all-inventory.log` | `all/all` exit 0 (9 executed, 23 declared, 0 invalid/unlisted/stale), and `all/all --require-no-skips` exit 1 `declared_only_present` |
| `windows-check.log` | `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` |
| `pr-body.md` | The PR description |

macOS stress (`probe=conformance`, 25 iterations):
- https://github.com/devswha/ovis-terminal/actions/runs/41203118363: RESULT 25/25 on fcf71d3.
- https://github.com/devswha/ovis-terminal/actions/runs/41201714418: RESULT 25/25 on da3388c.

## Descriptor rule

Every key in a scenario or fixture file is in one of three classes:
- **Asserted.** It produces an assertion in the evidence. Wire `decodes`/`error` are checked by decoding the bytes before `send_wire` writes them.
- **Descriptor.** Scenario `title`, case `summary`, and a fixture's `descriptors` map. These are copied into the evidence and never asserted.
- **Rejected.** Anything else: `deny_unknown_fields` fails the load with `scenario_invalid` at its field path.

A file without `"format": "ovis-conformance/1"` is a declaration and is reported as `scenario_not_executable`; it is never run with skipped keys.

## Windows

`RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0. The Unix-only fixture modules share their caller's module-level `cfg(unix)`; off Unix, fixture mode is a typed `fixture_failed` ("bind a listener"). There are no lint attributes.

## Scenario inventory (round 2)

`all/all` classifies every file under `conformance/scenarios/` and reports it in `inventory`:
- **executed**: it carries `"format": "ovis-conformance/1"` and loads; there are 9 today.
- **declared_only**: a JSON file with no `format` key, which must be listed in `conformance/scenarios/declared.json` with `{file, owner_todo, reason}`; there are 23 today, `part-a/task-{1..20,24,25,32}.json`.
- **invalid**: a misspelled marker, a name outside the id grammar, a non-JSON file, or a parse or schema error. This gives `scenario_invalid` with the file path and fails the run.

A declaration missing from the list fails with `declaration_unlisted`. A listed file that became executable, vanished or changed owner fails with `declaration_stale`. `--require-no-skips` fails with `declared_only_present` while the list is non-empty; the F3 final wave empties it.
