# Todo 64: replayable Ovis consumer conformance fixtures

Ships a versioned, hash-pinned fixture set (`conformance/fixtures/ovis`). It comes with a Rust transport-neutral replay adapter (`conformance/replay`, crate `ovis-terminal-replay`) and a runner scenario `client/fixtures`. The packed Bun client replays the same files through `bun run test:fixtures`. Both clients must reproduce the committed `receipts.json`, so their normalized receipts are equal. No Ovis file is touched.

Closes GAP-008, GAP-009, GAP-020, GAP-021, GAP-036, GAP-037, GAP-039, GAP-043, GAP-048, GAP-055, GAP-068, GAP-069, GAP-070, GAP-080, GAP-098, GAP-099, GAP-106, GAP-112, GAP-139. Delivers IDEAL-008, 009, 015, 018, 023, 024, 025, 028, 034, 035, 036, 047, 048, 057, 058, 073, 078, 080, 084, 085, 086 and 112. Enforces EXCL-005, 006, 008, 014, 015, 016, 022 and 023 as negatives.

## Before

`docs/evidence/baseline/smoke/attempt2-herdr-h05-process-info-trap.log:2` (RT-HD-03), from the original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/attempt2-herdr-h05-process-info-trap.log:2`:

```
$ pane.split {"pane_id":"w2:p1","direction":"right"}
<- {"id":"r12","result":{"type":"pane_info","pane":{"pane_id":"w1:p2",...,"workspace_id":"w1",...}}}
```

herdr ignored `pane_id` and split the pane in another, focused workspace.

## After

Host: `Linux 7.0.0-38-generic x86_64`. Binary under test: `target/debug/ovis-terminal`, sha256 `10eea71508bec2dbc1d8ffcfa739a1f59fe4afbd2464d0e15ef49a50bd863038` (recorded by the runner in the evidence).

| command | exit | result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `build.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario client/fixtures --isolated --case all --evidence "$ATTEMPT_DIR/task-64-all.json"` | 0 | 2 cases, failed 0, skipped 0, 2759 assertions |
| same with `--fixtures conformance/fixtures/ovis` (`task-64-fixtures.json`) | 0 | 2 cases, failed 0, skipped 0, 2759 assertions |
| `bun run --cwd packages/client-ts test:fixtures -- --fixtures ../../conformance/fixtures/ovis` | 0 | 4 pass: pins equal the manifest (same 15 hashes the runner checks), 8/8 cases replay with no failed check, Bun receipts equal `receipts.json`, replay sources typecheck against the packed declarations |
| QA happy: `... --scenario client/fixtures --isolated --case happy --evidence "$ATTEMPT_DIR/task-64-ovis-terminal-v1.json"` | 0 | 1450 assertions, failed 0 |
| QA failure: `... --case failure --inject '{"method":"pane.split","params":{"pane_id_typo":"other-pane","direction":"right"}}' --evidence "$ATTEMPT_DIR/task-64-ovis-terminal-v1.json" --append` | 0 | 1283 assertions, failed 0 |

The materialized socket exchange of the failure case (`after.log`, through `ovis-terminal connect`):

```
sent:     {"id":"select","method":"selection.set","params":{"client_id":"desk","pane_id":"pane:1","tab_id":"tab:1","workspace_id":"workspace:1",...}}
received: {"result":{"layout":{"revision":"8","selections":[{"client_id":"desk","pane_id":"pane:1",...}],...}}}
sent:     {"id":"split-typo","method":"layout.edit","params":{"edit":{"axis":"horizontal","op":"split_pane"},"op_id":"01a1990d-...","pane_id_typo":"other-pane"}}
received: {"id":"split-typo","error":{"code":-32602,"data":{"kind":"invalid_params","retryable":false,"field_path":"params.pane_id_typo"}}}
sent:     {"id":"unchanged","method":"layout.get","params":{}}
received: {"result":{"revision":"8","selections":[{"client_id":"desk","pane_id":"pane:1","tab_id":"tab:1","workspace_id":"workspace:1"}],...}}
```

The legacy `pane.split` intent is translated by `legacy.json` into an explicit-ID `layout.edit`. Unknown legacy keys reach the binary unchanged, and no target is ever filled in. The binary refuses with `invalid_params` at `params.pane_id_typo`. The layout revision (8) and the client's selection are unchanged.

What the replay cases assert (8 cases, 5 against `ovis-terminal serve`, 3 against the protocol fixture):

- `layout-explicit-ids` (IDEAL-023/024/084/085/086; GAP-036/069/112; EXCL-022):
  - Fresh root at revision 0.
  - `create_workspace` returns all created IDs, and a repeated `op_id` returns the same IDs, revision and seq.
  - Per-client `selection.set` for two labelled clients; a split by another client keeps both selections.
  - Stale `expect_revision` gets `revision_mismatch`, an unknown pane `not_found`, and ratio 1500 `invalid_geometry` with no clamping. None of them changes the revision or selections.
  - Swap keeps the IDs, and `session.snapshot` shows the same revision.
- `strict-targeting` (IDEAL-008/009; GAP-008/009; EXCL-005/006): the inject above; `terminal.process_info {pane_typo}` refused at `params.pane_typo`; an explicit legacy split succeeds without moving the selection.
- `attach-held-retry` (IDEAL-028/034/048; GAP-039/043/048/070; EXCL-016/023):
  - A second interact attach is the typed `held` outcome. It names the holder lease and the retry boundary `{on: lease_released, event: lease.revoked, lease}`.
  - Input without a lease, or with the released lease, gets `no_lease`.
  - After release, the retry gets `ready`, and input with the new lease is admitted.
- `subscriptions` (IDEAL-015/018/024/112; GAP-020/021/139; EXCL-014/015):
  - A second subscribe on the same connection keeps both deliveries; both receive the same seq.
  - `events.update` is acknowledged, and the next event belongs only to the unaffected subscription.
  - `rpc.discover` reports the pinned schema's version.
- `terminal-reads` (IDEAL-035/036/047/080; GAP-068/106; EXCL-008):
  - The screen's `output_offset` equals the raw output's `next_offset` (authoritative live bottom), and reads leave the revision unchanged.
  - A never-printed pattern ends at its bound with `outcome: timeout`.
  - Prompt-like bytes create no prompt and no reported status.
  - A rename does not redirect: the old name is `not_found`, and the ID still resolves on the same machine.
- `agent-records` (IDEAL-025/073; GAP-037/098): typed records with tool-call linkage, kept verbatim even when the text is instruction-like. The turn ends with `finish_generation 1, last_turn.turn 1, outcome completed`, recoverable from `agent.get`.
- `prompt-cards` (IDEAL-057/058; GAP-080; EXCL-023): a versioned prompt; a stale answer and a second answer each get `version_conflict`; the current-version answer is accepted.
- `history-store` (IDEAL-078; GAP-055/099):
  - An unlisted store gets `invalid_session` at `params.session.source` (authorized provenance), and a climbing path is refused.
  - A session bound by id makes `agent.conversation` a non-retryable `conversation_unavailable`.
  - `agent.history` with `store: "../"` gets `forbidden`.

Set audit (runner, every run):

- Schema, ideal-map, ideal-state and gap-list pins, plus the 11 fixture-file pins.
- `generation` is canonical.
- No unlisted case file.
- All nine adversarial classes are covered.
- The receipts list every case.
- `predicates.json` has 121 IDEAL rows and 29 EXCL rows with predicate digests equal to the ideal map, scenario lists equal to the ideal map, and every scenario resolving to a file or a named pending owner. Every `client/fixtures` row has a replay case, with links both ways.

Unit and integration tests:

- `ovis-terminal-replay`: 9 unit tests and 3 integration tests (scripted transport: capture, stop on failure, inject selection, legacy translation, committed-set audit).
- `schema_params::misspelled_target_is_named_instead_of_the_field_it_left_absent`.
- `binding::qa::observe::a_transcript_replaced_by_an_outside_link_after_binding_is_refused`.

Gates:
- `bash scripts/check.sh` (heavy gate) exit 0: `1488 tests run: 1488 passed (3 slow), 2 skipped`, hygiene `0 violation(s) in 1493 file(s)` (`check.log`).
- Windows: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0 and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`windows.log`).
- client-ts: `bun run generate:check` fresh (31 files), `bun run typecheck` 0, `bun test` 51 pass 0 fail.

Adversarial pass (logs `adv-*.log`, all exit 1 as required):
- Unplanned `--inject` (`pane_id` instead of the typo) fails `replay/failure/cases`.
- One byte changed in a case file fails `replay/pin/cases/strict-targeting.json`, and the Bun run on the same copy fails too.
- An extra unlisted case file fails `replay/cases/listed`.

Mutants (logs `mutant-*.log`, each fails a shipped test with exit 101):
- M1: decode never reports the unknown key; fails `misspelled_target_is_named_instead_of_the_field_it_left_absent`.
- M2: read-time containment accepts either check alone; fails `a_transcript_replaced_by_an_outside_link_after_binding_is_refused`.
- M3: legacy unknown keys go inside `edit`; fails `known_fields_are_renamed_and_unknown_fields_reach_the_server_unchanged`.
- M4: replay continues after a failed step; fails `captures_bind_later_requests_and_a_failed_step_stops_the_case`.

## Seams

Files outside the write scope (`conformance/fixtures/ovis/`, `conformance/replay/`, `conformance/scenarios/client/fixtures/`):
- `conformance/scenarios/client/fixtures.json`: the scenario itself. Scenario ids resolve to `<family>/<name>.json`, so `client/fixtures` must be this file.
- `conformance/src/{cli.rs,invocation.rs,engine.rs,engine/session.rs,engine/steps.rs,engine/steps/serve.rs,scenario/model.rs}`:
  - The `--fixtures` flag and the `replay` step.
  - `connect` and `serve` widened to `pub(super)` and `pub(crate)` for the replay transport.
- New `conformance/src/engine/replay.rs` and `engine/replay/wire.rs`: the runner's replay step and its bridge transport.
- `conformance/src/runner_tests/suite.rs`: `client/fixtures.json` in sorted `EXECUTABLE`. It was not in `declared.json`.
- `Cargo.toml`, `Cargo.lock`, `conformance/Cargo.toml`: the new workspace member `conformance/replay` and the runner's dependency on it. No new external crate.
- `crates/ovis-terminal-protocol/src/schema/decode.rs` (+ `tests/schema_params.rs`): when params are refused and a top-level key is unknown to the method, that key is the field path. Without this, the binary reported `params.edit.pane_id` (the field the typo left absent) instead of `params.pane_id_typo` (IDEAL-008/GAP-008). Only the reported path of an already-invalid request changes. No schema change.
- `crates/ovis-terminal-agent/src/conversation/binding/observe.rs` (+ `qa/observe.rs`): the seam assigned to 64. Containment was checked only at bind, so replacing the bound transcript (or a directory above it) with a link to an outside file made reads follow it. Every read now re-resolves the path and compares the opened file's identity; a mismatch is `OutsideStores` (`invalid_session`).
- `packages/client-ts/{package.json,tsconfig.json}`, new `scripts/fixtures.ts`, `tests/fixtures.test.ts`, `tests/replay/*`: the `test:fixtures` entry point. The replay sources run in an isolated project that installs the packed tarball, so they are excluded from the repository typecheck, as `tests/consumer` is.
- `probes/platform-qa/stress/client-fixtures.{json,sh}`: the data-driven macOS stress lane (10 iterations of the runner `--case all` and the Bun `test:fixtures`). Dispatched once at c80a4f7: https://github.com/devswha/ovis-terminal/actions/runs/41354470413.
- Merged `origin/integration` 070871b (T83); conflicts in `steps.rs`, `steps/serve.rs` and `model.rs` were resolved by keeping both step kinds. Re-pinned with `cargo run -p ovis-terminal-replay --bin ovis-replay -- refresh`. `graphics/passthrough` now exists, so it left `pending`. `openrpc check` reports fresh, `quality-matrix --mode verify` passes, and `bun run generate:check` reports fresh.

## Risks

- **Pins are strict by design.** Any change to the schema, `conformance/ideal-map.json`, `ideal-state.md`, `gap-list.md` or a fixture file fails `client/fixtures` (and therefore `all/all`) until `cargo run --locked -p ovis-terminal-replay --bin ovis-replay -- refresh` re-pins. After a union merge with a schema-changing lane, run refresh and commit the two JSON files.
- **Lease and layout events are not published by the assembled daemon** (probed). `attach.open` returns the typed `held` outcome naming `lease.revoked` as the retry event. But a subscription with `types: ["lease.revoked", "layout.changed", "selection.changed"]` receives none of them, because the daemon commits them to its separate layout/lease log only. GAP-043 is proven here through the typed held outcome and the bounded retry after release, not through event delivery. Publishing those events belongs to the assembly owner (todo 66).
- Predicates for IDEAL rows not delivered here are checked as bound mappings: digest, scenario list, and that each scenario resolves to a file or a named pending owner. Their behaviour is proven by the owning lane scenarios that `all/all` runs, not replayed again here.
- `../outside.jsonl` (non-existent) is refused as `conversation_unavailable` (missing source), not `invalid_session`. The provenance refusal is proven by the unlisted store.
- Windows: the replay step needs a Unix endpoint, as every `serve` step does, and the scenario declares `linux` and `macos`.
