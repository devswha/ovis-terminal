Plan todo 65 (wave 19/28): document every Ovis adapter migration boundary.

Closes: GAP-004, GAP-021, GAP-039, GAP-060, GAP-080, GAP-098, GAP-106, GAP-112, GAP-139
Delivers: IDEAL-004, IDEAL-019, IDEAL-028, IDEAL-033, IDEAL-040, IDEAL-057, IDEAL-058, IDEAL-067, IDEAL-073, IDEAL-075, IDEAL-078, IDEAL-080, IDEAL-082, IDEAL-084, IDEAL-085, IDEAL-086, IDEAL-100, IDEAL-112, IDEAL-113

## Summary

- `docs/ovis-adapter.md`: one machine-read `ovis-migration` block per Ovis path of architect section 6 (38 paths plus the kept-policy row, 39 rows), each with today's behaviour cited by `file:line` in the read-only Ovis tree, the replacing runtime contract, and the fate (replace/delete/adapt/move/keep). Each fate is labelled a recommendation for a later Ovis PR only. The rows list served methods, unserved methods (honest gaps), events, `capabilities.*`/`limits.*`, identity types, fixtures and examples. Sections cover method-name resolution, the startup snapshot boundary and cursor reconnect, per-viewer streams and unknown delivery (with a Bun `OutcomeUnknown`/`Draft` snippet), prompt versions, status/turns, history identity, machine updates, layout receipts, terminal-content-is-data, the Windows capability contract, the kept Ovis policies, named gaps and the enforced exclusions (EXCL-001/003/004/008/011/012/013/016, V-RT-HD-01/07/09).
- 9 runnable `ovis-example` blocks (format `ovis-replay/1`), replayed by the runner against a fresh server in a fresh root through the binary's `connect` bridge.
- New conformance step `migration` (`conformance/src/scenarios/client_migration.rs` + child modules `guide`, `contract`, `policy`, `served`, `tests`) and scenario `client/migration`:
  - happy: `migration/coverage` (missing/extra/duplicate paths, wrong fate, undefined/uncited examples; all must be `[]`), `migration/coverage/paths` = 39, `migration/policy` (no guessed keys beside a prompt, no resend of a mutation under a fresh op id, no captured screen/output/record content sent back as params, and unknown delivery shown with `op.get`), `migration/packed-client/schema` (the schema `packages/client-ts/src/generated/openrpc.json` ships equals `schema/ovis-terminal.openrpc.json`), and `migration/contract`: every method is in the schema and in the `hello` method list of `ovis-terminal serve`, every unserved method is in the schema and not served, and every event, capability, identity type, `fixture:` case and executable `scenario:` exists. Then every example is replayed.
  - failure: planned injections rewrite one call of a guide example: a stale prompt version must get `version_conflict` and the card must stay `open` with `answer: null`; a stale layout `expect_revision` must get `revision_mismatch` and the layout revision and selections must be unchanged. `--inject` selects one; an unplanned one fails (`migration/inject/planned`).
  - `--guide <path>` reads another guide (default `docs/ovis-adapter.md`).

## Before

`absent` (no comparable baseline capability; not an executed failing test). `docs/evidence/task-65/before.log`:
```
$ git show fcfd5e5:docs/ovis-adapter.md
fatal: path 'docs/ovis-adapter.md' exists on disk, but not in 'fcfd5e5'
```
Gap baseline: GAP-004, GAP-021, GAP-039, GAP-060, GAP-080, GAP-098, GAP-106, GAP-112, GAP-139.

## After

Host: Linux x86_64 (native). Binary `/dev/shm/ovt-target-lane-t65/debug/ovis-terminal`, sha256 `fd1f80b113cf79a3a4b5c8fac206a9aa99c7facdfa78a8b369a59c5c276bba7a` (lane CARGO_TARGET_DIR on RAM; `target/debug/ovis-terminal` in the plan's command is this path). Raw log `docs/evidence/task-65/after.log`, evidence `docs/evidence/task-65/task-65-ovis-terminal-v1.json`.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                       exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary $B --scenario client/migration --isolated --case all --evidence "$ATTEMPT_DIR/task-65-ovis-terminal-v1.json"
{"result":"passed","totals":{"assertions":"207","cases":"2","failed":"0","passed":"2","skipped":"0"}}   exit=0
$ ... --case all --guide docs/ovis-adapter.md --evidence ...                             -> 207 assertions, failed 0, skipped 0, exit=0
$ ... --case happy --evidence "$ATTEMPT_DIR/task-65-ovis-terminal-v1.json"               -> 167 assertions, failed 0, exit=0
$ ... --case failure --inject '{"example":"prompt.answer","version":"stale"}' --evidence "$ATTEMPT_DIR/task-65-ovis-terminal-v1.json" --append   -> 24 assertions, failed 0, exit=0
```
Happy observable (`docs/evidence/task-65/qa.log`):
```
run: case=happy result=passed binary_sha256=fd1f80b113cf79a3a4b5c8fac206a9aa99c7facdfa78a8b369a59c5c276bba7a host={'arch': 'x86_64', 'family': 'unix', 'os': 'linux'} totals={'assertions': '167', 'cases': '1', 'declared_only': '0', 'failed': '0', 'passed': '1', 'skipped': '0'}
  [happy] descriptor scenarios/client/migration.json#source: {"lines": "161-183", "path": ".omo/ulw-loop/01a18d5f-ed46-704a-b988-38720320ebb6/waves/plan-advice/architect.md", "section": "6. Blast radius for the Ovis client", "sha256": "449750b7a3291c752b743823772a61b50a8640d75b91a75e47a4fe7fbd14326e"}
  [happy] migration/coverage passed=True actual={"duplicate_paths": [], "extra_paths": [], "missing_paths": [], "uncited_examples": [], "undefined_examples": [], "wrong_fate": []}
  [happy] migration/coverage/paths passed=True actual="39"
  [happy] migration/policy passed=True actual={"auto_resend": [], "content_as_command": [], "guessed_keys": [], "unknown_delivery_shown": true}
  [happy] migration/packed-client/schema passed=True actual=true
  [happy] migration/contract passed=True actual={"example_unknown_methods": [], "methods_not_served": [], "unknown_capabilities": [], "unknown_events": [], "unknown_fixtures": [], "unknown_identity": [], "unserved_now_served": []}
run: case=failure result=passed binary_sha256=fd1f80b113cf79a3a4b5c8fac206a9aa99c7facdfa78a8b369a59c5c276bba7a host={'arch': 'x86_64', 'family': 'unix', 'os': 'linux'} totals={'assertions': '24', 'cases': '1', 'declared_only': '0', 'failed': '0', 'passed': '1', 'skipped': '0'}
  [failure] descriptor scenarios/client/migration.json#source: {"lines": "161-183", "path": ".omo/ulw-loop/01a18d5f-ed46-704a-b988-38720320ebb6/waves/plan-advice/architect.md", "section": "6. Blast radius for the Ovis client", "sha256": "449750b7a3291c752b743823772a61b50a8640d75b91a75e47a4fe7fbd14326e"}
  [failure] migration/coverage passed=True actual={"duplicate_paths": [], "extra_paths": [], "missing_paths": [], "uncited_examples": [], "undefined_examples": [], "wrong_fate": []}
  [failure] migration/coverage/paths passed=True actual="39"
  [failure] migration/policy passed=True actual={"auto_resend": [], "content_as_command": [], "guessed_keys": [], "unknown_delivery_shown": true}
  [failure] migration/inject/planned passed=True actual={"example": "prompt.answer", "version": "stale"}
  [failure] replay/prompt.answer.injected/answer/error/data/kind passed=True actual="version_conflict"
  [failure] replay/prompt.answer.injected/still-open/result/answer passed=True actual=null
  [failure] replay/prompt.answer.injected/still-open/result/state passed=True actual="open"
  [failure] replay/prompt.answer.injected/still-open/result/version passed=True actual="1"
```
The executed example's materialized socket requests are in the evidence JSON connection transcripts (`prompt.answer.injected.ovis`): `prompt.get {"prompt_id":"p1"}` -> version `"1"`; `prompt.answer {"op_id":<uuidv7>,"prompt_id":"p1","version":"0","answer":{"choice":"yes"}}` -> `error.data.kind = "version_conflict"`, `retryable: false`, no result; `prompt.get` -> `state: "open"`, `answer: null`.

Packed client (round 2): `packages/client-ts/tests/guide.test.ts` extracts every `json ovis-example` block from `docs/ovis-adapter.md` and stages them as an `ovis-replay/1` fixture set, using the legacy map, server bindings and schema pins of `conformance/fixtures/ovis`, with each case pinned by its own bytes. It installs the packed tarball into a project outside the repository and runs T64's unchanged `tests/replay/{engine,replay}.ts` against the real binary and protocol fixture. `receipts.json` is not touched. CI's `client-ts` job runs it as part of `bun test`, so the guide and the client cannot drift apart unnoticed. `OVIS_GUIDE` replays another guide.
```
$ cd packages/client-ts && bun test ./tests/guide.test.ts
(pass) every example the guide cites replays through the packed client
(pass) no guide example has a failed expectation through the packed client
(pass) the staged examples are the guide's bytes and the schema pin is the published schema
(pass) the replay sources typecheck against the packed declarations
 4 pass  0 fail                                                              exit 0
$ bun run generate:check && bun run typecheck && bun test                   -> generated fresh; tsc exit 0; 55 pass, 0 fail
```
Mutant M6: changing the prompt.answer card's `/result/state` expectation from "open" to "answered-bogus" (`OVIS_GUIDE=<copy>`) fails "no guide example has a failed expectation through the packed client" (bun exit 1). Logs: `docs/evidence/task-65/{bun-guide.log,client-ts-gate.log,mutant-M6-bun-guide-expectation.log,mutants.log}`.

Section-6 provenance (gate note): the `migration` step records `source` = architect.md, section 6, lines 161-183, sha256 `449750b7a3291c752b743823772a61b50a8640d75b91a75e47a4fe7fbd14326e`, as an evidence descriptor that is recorded but never asserted, because the advisory is not in the repository.

Tests: `cargo nextest run --locked -p ovis-terminal-conformance -p ovis-terminal-replay` -> 56 tests run: 56 passed (includes the `all/all` suite with `client/migration` now executable). 7 new unit tests in `scenarios::client_migration::tests` assert coverage reports, policy verdicts, injection rewriting and the served/unserved contract.

Adversarial (each through the real runner with `--guide <mutated copy>`; exit 1 required, logs in `docs/evidence/task-65/adv-*.log`): dropped `server/mirror.ts` block -> `missing_paths: ["server/mirror.ts"]`; resend under a fresh op id -> `auto_resend: ["viewer-streams"]`; `input.keys` in place of `prompt.answer` -> `guessed_keys: ["prompt.answer"]`; screen line captured and sent as a search pattern (prompt injection) -> `content_as_command: ["terminal-content"]`; `machine.rm` listed as served -> `methods_not_served: ["machine.rm"]`; unplanned `--inject` -> `migration/inject/planned` actual null. All exit=1.

Mutants (each fails a shipped test; summary `docs/evidence/task-65/mutants.log`, logs `mutant-*.log`): M1 coverage `missing` filter inverted (2 failed), M2 resend detection requires equal op ids (1 failed), M3 content never tainted (1 failed), M4 injection keeps the original tail (1 failed), M5 served check compares against the schema instead of `hello` (1 failed), M6 one guide expectation changed, replayed through the packed client (bun test 1 fail).

Gates on the round-3 merge commit ( check.sh does not run bun, so the client-ts gate above was run separately):
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (foreground gate, memory-scoped) -> `check.sh: all steps passed`, exit 0; nextest `1629 tests run: 1629 passed (3 slow), 2 skipped`; `workspace-hygiene: 0 violation(s)`; source-check and evidence-audit passed (`docs/evidence/task-65/check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` -> exit 0; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` -> exit 0 (`docs/evidence/task-65/windows.log`).

## Seams

Round 3 merged origin/integration 4d0511d (P6, P4, t67, t85). Pins: integration's taken, then `ovis-replay refresh`. `suite.rs` merged clean (union). `served.rs` sets the new `Serve.qa_output_commit_gap_ms: None`. Because P4 now publishes `layout.changed`/`selection.changed`, the guide drops that gap, lists the events on the layout rows, and the layout-receipts example subscribes and awaits the split's `layout.changed` (matched by `data.op_id` and `data.revision`). No task-65 evidence pins the schema with `$equals_file`.


Files outside the write scope (`docs/ovis-adapter.md`, `conformance/scenarios/client/migration/`, `conformance/src/scenarios/client_migration.rs`) and why:
- `conformance/scenarios/client/migration.json`: the scenario file for id `client/migration`. The scenario id grammar is `<family>/<name>.json`, and the inventory classifies every file under the scenario root, so data files under a `client/migration/` directory would be invalid. The section-6 path list lives in this file's `migration` step and the examples live in the guide.
- `conformance/src/scenarios/client_migration/{guide,contract,policy,served,tests}.rs`: child modules of the named file, to keep it under the 250-pure-LOC limit.
- `conformance/src/scenarios.rs`, `conformance/src/lib.rs` (`pub mod scenarios;`): the parent module the named file requires.
- `conformance/src/scenario/model.rs` (`Step::Migration`), `conformance/src/engine/steps.rs` (dispatch arm; `connect` made `pub(crate)`): step registration.
- `conformance/src/engine/replay.rs`: `replay_case` split into `run_replay` (fresh root, serve, wire, teardown; returns the `CaseRun`) and the unchanged assertion building, so guide examples replay without fixture receipts. `assertion`, `replay_failure` and `fixture_dir` are `pub(crate)`. No behaviour change for `client/fixtures`.
- `conformance/src/cli.rs`, `conformance/src/invocation.rs`, `conformance/src/engine/session.rs`: the `--guide` flag and `Context.guide`.
- `conformance/src/runner_tests/suite.rs`: `client/migration.json` added to `EXECUTABLE` in sorted position (it was never in `declared.json`).
- `conformance/fixtures/ovis/{manifest,predicates}.json`: `ovis-replay refresh` after `client/migration` became executable (one predicate's scenario state changed).
- `probes/platform-qa/stress/client-migration.{json,sh}`: the macOS stress lane (10 iterations of `--case all --guide docs/ovis-adapter.md` plus the `--inject` failure run); adding a lane edits nothing else.
- `packages/client-ts/tests/guide.test.ts` (round 2; write scope widened by the gate): the packed-client replay of the guide's examples.
- `docs/evidence/task-65/`.

## Risks

- Named product gaps recorded in the guide, not fixed here: a `layout.edit` receipt `seq` differs from its `layout.changed` envelope `seq` (correlate by `data.op_id`/`data.revision`); `agent.list`, `agent.note`, `agent.release`, `agent.wait_children`, `doc.list`, `events.ack`, `machine.rm/login/exec/file`, `server.start`, `service.install/remove` and `terminal.history` unserved; `op.get` answers `not_found` for `input.*` operations (the unknown-delivery example asserts that answer); `events.wait` replies carry no terminal id; `schema.capabilities` reports `stream_modes: []` while plain attach works; attach across a control-server restart is uncovered; image placeholders (t85) landed after this base.
- `migration/contract` fails as soon as a listed-unserved method becomes served. That is intended, so the guide is updated with the product.
- The section-6 path list is copied into the scenario from the advisory in `.omo/` (not in the repo); a later advisory change needs the scenario updated.

macOS stress: `gh workflow run probe-stress-macos.yml -f probe=client-migration --ref lane/t65-document-every-ovis-adapter` -> https://github.com/devswha/ovis-terminal/actions/runs/41383839656

DEFERRED VERIFICATION: none; the runner executes `client/migration` (exit 0 above).
