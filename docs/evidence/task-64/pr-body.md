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
| Loaded host: `taskset -c 0-1` with 8 busy loops, `conformance ... --scenario client/fixtures --isolated --case all` x10 | 10 × 0 | 10/10 after the `attach.close` fix; 5/10 before, each at `attach-held-retry/send-released-lease` (`load-stress.log`) |
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

Product defect found by the replay under load (`attach.close` answered before the lease was revoked):
- What the loaded loop showed: before the fix, 5/10 runs failed at `attach-held-retry/send-released-lease`. desk's `attach.close` had replied, then `input.send` with desk's released lease was expected to get `no_lease`. Instead it got a result: `{"receipt": {"admitted_bytes": "1", ...}}`, a write admitted on a released lease (IDEAL-048 / GAP-070).
- Why it is the product, not the fixture: the fixture sends `input.send` only after the close reply arrives. `TerminalStreams::close` only revoked input on the viewer and marked the close. The owner's leases were disconnected later, by the pump task's `detach`, so the reply could precede the revocation.
- Fix (`crates/ovis-terminal-server/src/streams/ops.rs`, `fn close`):
  - Before `Ok(at)`, `close` calls the existing `self.release(stream, detach_op())`. It is the same release the pump's detach performs, under the same "another stream of this owner keeps the leases" rule. The pump's later detach finds nothing to revoke, and an empty transition journals nothing.
  - Because `close` now commits to the store, `attach.close` runs it through `spawn_blocking` in `streams/handler.rs`, as `attach.open` and `attach.takeover` already do. Running it on the executor thread panicked the server in the first attempt.
  - `attach.repaint` is unchanged.
  - `attach.close` carries no op id (its params are `{stream_id}` only), so the release is journaled under an op id minted by the pump's existing `detach_op()` (`pump.rs`, now `pub(super)`).
- Regression test: `streams::ops::tests::close_revokes_the_interact_lease_before_it_returns` drives the hub directly with no pump task: `open` interact, `close`, then the interact lease must be gone. Deterministic: there is no task scheduling between `close` and the check.

Gates:
- `bash scripts/check.sh` (heavy gate) exit 0: `1504 tests run: 1504 passed (3 slow), 2 skipped`, hygiene `0 violation(s) in 1529 file(s)` (`check.log`).
- `RUSTDOCFLAGS='-D warnings' cargo doc --workspace --no-deps --locked` exit 0 (`rustdoc.log`).
- `cargo nextest run --manifest-path conformance/Cargo.toml -E 'test(suite::)'`: 4 passed, including all/all (`suite.log`).
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
- M5: `close` without the release (the old ordering); fails `close_revokes_the_interact_lease_before_it_returns` with `left: Some(LeaseId(1)), right: None`.


## Round 3 (gate r2 B1-B3, N4)

**B1. GAP-043: `lease.revoked` is now delivered, and the fixture awaits it.**
- Product: `BootJournal::record` commits a lease transition to the lease log (authoritative, unchanged). It then commits the same events into the serving boot's dispatch log, which `events.subscribe` reads (`crates/ovis-terminal-cli/src/assembly/journal.rs`).
  - `ServedEvent` gains `Lease(LeaseEvent)` (`lanes.rs`).
  - Its wire data is the published `LeaseEventData` `{lease, displaced_by, reason}`, built with `lease_record` and validated against that type before it leaves the runtime (new `lease_feed.rs`).
  - Each generation points the journal at its own log, and `end()` detaches it before the log is closed (`generation.rs`). A restart therefore never commits into a closed log.
  - `lease.acquired` and `lease.geometry` are published by the same path.
- Not published here: `layout.changed` and `selection.changed`. They are committed to the separate layout log in `assembly/layout*`, a different site with its own record shape. They stay a named gap: `layout.changed` and `selection.changed` are not delivered to `events.subscribe` by the assembled daemon.
- Fixture `attach-held-retry`: phone subscribes `{types: [lease.revoked]}` before its open is held. The held reply's `/result/retry/event` is captured. After desk's `attach.close`, an event step awaits `/params/type == $value:retry_event`, desk's `lease:1`, `holder: desk`, `reason: disconnected`, on the captured subscription and terminal. Both the Rust and Bun replays receive it.

**B2. GAP-070: only open streams keep an owner's leases.**
- `TerminalStreams::release` (`streams/ops.rs`) now counts other viewers of the owner only when `!other.is_closed()`. An owner closing its interact and observe streams loses the interact lease at the second `attach.close`, not at a later pump detach.
- Test `an_owner_closing_its_last_two_streams_loses_the_lease_at_the_second_close`, hub-level with no pump: the lease is kept after the first close (observe still open) and is `None` after the second.

**B3. GAP-069 / IDEAL-047: the content-revision guard is replayed.**
- The assembled binary now serves `terminal.search` (`assembly/reads.rs`, `routes.rs`, `handler.rs`). It previously answered `method_not_found` and was in the CLI test's unimplemented list.
  - It runs `Screen::search` against the revision the caller names; a snapshot `terminal.screen` pinned answers unchanged.
  - A revision that is not pinned is the typed `stale_revision` (`retryable: false`).
  - Only literal text is supported; a regex pattern is refused `format_unsupported`.
  - The method was already published; there is no schema change.
- New case `content-revision-guard`. The program prints `READY`, then waits for input, then prints 3000 lines and `DONE-MARK`. The case:
  1. pins with `terminal.screen`;
  2. sends `go` with the interact lease;
  3. searches at the pinned revision while output flows (answered at that revision);
  4. searches at revision `1` (`stale_revision`, not retryable);
  5. waits for `DONE-MARK`;
  6. searches at the pinned revision again (still answered, after the live revision moved);
  7. searches at the live revision.

**N4.** `manifest::tests::a_pin_holds_only_for_the_digest_of_the_bytes_on_disk` (a changed or missing file fails its pin) and `expect::tests::an_exact_object_matches_only_an_equal_object`.

**Replay harness fixes found by the round-3 mutants:**
- The runner's event wait now has one deadline (`conformance/src/engine/replay/wire.rs`). Before, heartbeats and stream data extended it to the 300 s case bound.
- A transport failure is reported as the runner's own typed error, a `timeout` for example, instead of `scenario_inconsistent` (`engine/replay.rs`).

**Round-3 mutants** (logs `mutant-M6..M10-*.log`), each killed by a shipped test:
- M6: journal does not publish to the feed. `client/fixtures` happy fails at the awaited event with `timeout` ("a replay notification did not happen within 20000 ms") after 21 s.
- M7: closed streams counted again. Fails `an_owner_closing_its_last_two_streams_loses_the_lease_at_the_second_close` (`left: Some(LeaseId(1)), right: None`).
- M8: search ignores the requested revision. Fails `content-revision-guard/pinned-while-flowing/result/revision` (live revision 11 answered).
- M9: pin compared to itself. Fails `a_pin_holds_only_for_the_digest_of_the_bytes_on_disk`.
- M10: an exact matcher passes on any present value. Fails `an_exact_object_matches_only_an_equal_object` and `every_operator_holds_and_fails_on_the_values_it_names`.

**Round-3 After** (Linux 7.0.0-38-generic x86_64):
- Acceptance `--case all` and `--case all --fixtures conformance/fixtures/ovis`: exit 0 each, 2 cases, failed 0, skipped 0, 2794 assertions.
- `bun run test:fixtures`: exit 0, 4 pass (9 cases, receipts equal, pins equal).
- QA happy and QA failure inject: exit 0.
- Loaded host (`taskset -c 0-1`, 8 busy loops) `--case all` x10: 10/10 (`load-stress.log`).
- Merged `origin/integration` 60076f7 (F2a, conformance fixture only); clean merge.
- `bash scripts/check.sh` (heavy gate) exit 0: `1508 tests run: 1508 passed (3 slow), 2 skipped`, hygiene `0 violation(s) in 1531 file(s)`. Windows `cargo check` and `cargo clippy` (`x86_64-pc-windows-gnu`, `-D warnings`) exit 0 (`windows.log`).

## Seams

Seams outside the write scope:
- Round 3: `crates/ovis-terminal-cli/src/assembly/{journal.rs,lanes.rs,lease_feed.rs (new),generation.rs,assembly.rs}` (lease events published to subscribers, B1); `assembly/{reads.rs,routes.rs,handler.rs}` + `crates/ovis-terminal-cli/tests/assembly.rs` (`terminal.search` served and removed from the unimplemented list, B3); `crates/ovis-terminal-server/src/streams/ops.rs` + `ops/tests.rs` (open-stream release rule, B2); `conformance/src/engine/replay.rs`, `engine/replay/wire.rs` (event deadline, typed transport failures).
- `crates/ovis-terminal-server/src/streams/ops.rs` (`fn close`, + `ops/tests.rs`), `streams/handler.rs` (`stream_call` runs `close` off the executor), `streams/pump.rs` (`detach_op` is `pub(super)`): the `attach.close` lease-revocation defect above. The fix is the smallest one: other callers keep their behaviour, and only `attach.close` reaches `close`.
- `conformance/src/engine/replay.rs`: each replay connection is half-closed and decoded to end of stream before `finish_conn` asserts `bytes_received = bytes_decoded`. This fixes the CI failure in all/all, where a notification still in flight left a partial line received but not decoded. It is the same contract as the `hangup` and `closed` steps, bounded by the record wait, with no sleep.

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
- Merged `origin/integration` 1abcced (T70 soak) after 070871b; clean merge. Re-pinned (`performance/soak` now declared, so it left `pending`); openrpc check fresh, quality-matrix verify pass, SDK `generate:check` fresh. Earlier: merged `origin/integration` 070871b (T83); conflicts in `steps.rs`, `steps/serve.rs` and `model.rs` were resolved by keeping both step kinds. Re-pinned with `cargo run -p ovis-terminal-replay --bin ovis-replay -- refresh`. `graphics/passthrough` now exists, so it left `pending`. `openrpc check` reports fresh, `quality-matrix --mode verify` passes, and `bun run generate:check` reports fresh.

## Risks

- **Pins are strict by design.** Any change to the schema, `conformance/ideal-map.json`, `ideal-state.md`, `gap-list.md` or a fixture file fails `client/fixtures` (and therefore `all/all`) until `cargo run --locked -p ovis-terminal-replay --bin ovis-replay -- refresh` re-pins. After a union merge with a schema-changing lane, run refresh and commit the two JSON files.
- Fixed in round 3 for `lease.*`; `layout.changed` and `selection.changed` remain unpublished (named gap). Earlier note: **Lease and layout events were not published by the assembled daemon** (probed). `attach.open` returns the typed `held` outcome naming `lease.revoked` as the retry event. But a subscription with `types: ["lease.revoked", "layout.changed", "selection.changed"]` receives none of them, because the daemon commits them to its separate layout/lease log only. GAP-043 is proven here through the typed held outcome and the bounded retry after release, not through event delivery. Publishing those events belongs to the assembly owner (todo 66).
- Predicates for IDEAL rows not delivered here are checked as bound mappings: digest, scenario list, and that each scenario resolves to a file or a named pending owner. Their behaviour is proven by the owning lane scenarios that `all/all` runs, not replayed again here.
- `../outside.jsonl` (non-existent) is refused as `conversation_unavailable` (missing source), not `invalid_session`. The provenance refusal is proven by the unlisted store.
- Windows: the replay step needs a Unix endpoint, as every `serve` step does, and the scenario declares `linux` and `macos`.
