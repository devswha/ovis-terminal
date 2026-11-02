## Summary

Product-fix lane P4 (plan F4 "Ideal-state fidelity", pulled forward). The assembled daemon now publishes `layout.changed` and `selection.changed` to `events.subscribe` subscribers. Each event names the client `op_id` that caused it. This closes the named gap that T64 (#79) and T66 (#82) recorded, and it finishes GAP-043: every event the held-owner retry guidance names now arrives.

## Design: one committed record, one reader, an idempotent forward that replays on boot

T64 commits a lease transition to the lease log and then makes a second, separate write to the dispatch log. That pair is not atomic. This lane does not copy that pattern:

- **The layout log stays the only record of a layout change.** `layout.edit`, `selection.set` and the internal edits (`worktree.open` / `worktree.remove` workspaces) commit exactly as before. Nothing is written twice at the mutation site.
- **One reader** (`assembly/layout/feed.rs::forward`) reads the committed layout-log records after a watermark (`Snapshot::events_after`). It groups them by the store operation that committed them. Each group's first record is core's operation transition, which carries the client `op_id`. The reader commits each group to the dispatch log as one `commit_unit`.
- **Idempotent.** The same dispatch transaction moves the watermark (`layout_feed/forwarded` = last layout sequence forwarded) by compare-and-swap, and it is checked under the dispatch commit lock. A forward that already happened therefore fails as `Overtaken`, and the reader skips past it. The dispatch op id is deterministic (`layout-feed:<last layout seq>`), so the store's `duplicate_operation` is a second guard (mutant M2 shows it firing).
- **Replayed.** `Generation::open` runs a forward as soon as the boot's dispatch log is installed. Records committed while no boot served (a crash between the layout commit and the forward, or the restart window) reach the next boot's log.
- **When it runs.** The forward runs on the actor thread right after each layout commit, before the reply is written. A subscriber that subscribed before the mutation therefore gets the event without polling. A failed forward is reported on stderr as a typed `FeedError` (kind/hint). The layout reply is unaffected, and the next commit or boot retries from the watermark.

Why not "publish from the same committed record" literally? `events.subscribe`'s hub (`Subscriptions`) is bound to exactly one coordinator, the dispatch log. Moving layout rows into that log would change core's layout service, which is outside this write scope. The forward described above gives the same guarantees (exactly once per record, durable catch-up) without a dual write at the mutation site.

**Wire shape (additive, released-compatible).** The released document types `layout.changed` as `LayoutTree` and `selection.changed` as `Selection`. Neither has an `op_id` or change ids. The new data types are strict supersets:
- `LayoutChangedData` = `LayoutTree` (revision, workspaces, selections) + `op_id`, `created`, `removed`, `changed`.
- `SelectionChangedData` = `Selection` + `terminal_id` (of the selected pane), `op_id`, `revision`, `cause` (`requested` / `target_removed`).

`openrpc compat schema/released.openrpc.json` returns `{"compatible_with":"schema/released.openrpc.json"}`, so no BASELINE-AMENDMENTS entry is needed. `revision` is a decimal string. Selections are per client, not per terminal, so `selection.changed` names `client_id` and carries the selected pane's `terminal_id`.

## Before

`docs/evidence/task-P4/before.log` quotes the T64 gate review (orchestrator evidence `.omo/evidence/ovis-terminal-v1-task-64-gate-review.md`):

> `.omo/evidence/ovis-terminal-v1-task-64-gate-review.md:48`: My wire probe (`probe.ts event`, fixed binary) subscribed with `filter.types: [lease.revoked, lease.granted, layout.changed, selection.changed]`. Then: desk ready, phone `held` (...), desk `attach.close` OK, and a `layout.edit` OK. Result: `events_seen: []` after a 5 s window.
>
> `:155`: B1 wire probe, `layout.edit` plus `selection.set` with `layout.changed`/`selection.changed` subscribed | no events.

The same gap is named at `docs/evidence/task-64/pr-body.md:127` and `docs/evidence/task-66/pr-body.md:156`. At base `6de1d0e`, `ServedEvent` (`crates/ovis-terminal-cli/src/assembly/lanes.rs`) has no layout variant.

## After

Linux x86_64, debug binary built from this branch.

| Command | Exit | Result |
|---|---|---|
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary $CARGO_TARGET_DIR/debug/ovis-terminal --scenario events/layout-selection --isolated --case all --evidence <path>` | 0 | `result: passed`, 2 cases, 55 assertions, 0 failed (`docs/evidence/task-P4/after.log`, `events-layout-selection-all.json`) |
| same, `--case failure --inject '{"method":"events.subscribe","params":{"filter":{...,"types":"layout.changed",...}}}'` | 0 | `invalid_params` at `params.filter.types`, 8 assertions (`qa-inject-malformed-filter.log`) |
| `cargo test -p ovis-terminal-cli --lib --locked -- layout::feed` | 0 | 2 passed |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (memory scope, foreground) | 0 | `Summary [167.599s] 1602 tests run: 1602 passed (3 slow), 2 skipped`; `check.sh: all steps passed` (`check-tail.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | publish path is platform-neutral (no cfg) |
| `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` | 0 | |
| `openrpc write` + `openrpc compat schema/released.openrpc.json` | 0 | `{"compatible_with":"schema/released.openrpc.json"}` |
| `quality-matrix --mode contract` / `--mode verify` | 0 | `"findings": []`, `"verdict": "pass"` |
| `ovis-replay refresh` | 0 | `{"refreshed":12}` (manifest.json re-pinned; predicates.json unchanged) |
| `cd packages/client-ts && bun run generate && bun run generate:check && bun run typecheck && bun test` (binaries from this branch) | 0 | 31 files fresh; 51 pass, 0 fail |
| `evidence-audit --tree docs/evidence --exemptions probes/quality/evidence-exemptions.json` | 0 | `finding_count 0` after re-recording T66's schema-pinned evidence |

Events received in the happy case, subscribed before any mutation (from `after.log`):
- `layout.changed` op_id=`$op:ws`, revision `"4"` (= the `layout.edit` reply's `layout.revision`), `created[0].workspace_id = workspace:1`, `removed = []`
- `layout.changed` op_id=`$op:tab`, revision `"8"`, `created[0].tab_id = tab:2`
- `selection.changed` op_id=`$op:pick`, revision `"12"`, `client_id = client:desk`, `workspace_id = workspace:1`, `tab_id = tab:2`, `cause = requested`
- `selection.changed` op_id=`$op:close`, `client_id = client:desk`, `workspace_id = null`, `cause = target_removed`, then `layout.changed` op_id=`$op:close`, revision `"17"`, `removed[0].workspace_id = workspace:1`, `workspaces = []`. Both are in commit order; the close unit clears the selection first.

Each event step waits within the runner's 20 000 ms record bound. No step sleeps.

### What the tests assert

`crates/ovis-terminal-cli/src/assembly/layout/feed/tests.rs` uses real layout and dispatch SQLite stores, the real `Layouts` service and a real authority session:
1. `a_layout_edit_and_a_selection_reach_a_subscriber_with_their_op_ids`: a dispatch-log subscription is opened before the edit. A `create_workspace` under a minted op id is forwarded exactly once (`forward == 1`), and the delivered `layout.changed` has that `op_id`, the created workspace id, the head revision as a decimal string and the tree. A `selection.set` delivers `selection.changed` with its own op id, the client, the workspace and cause `requested`. Nothing else is delivered (`try_recv` is `Empty`). Each receive is bounded at 10 s, so a dropped publish fails instead of hanging.
2. `a_record_is_published_once_and_a_later_boot_forwards_what_no_boot_served`: a second forward publishes nothing (`0`, `Empty`). The first boot's dispatch log is then closed, and an edit is committed while no boot serves. A fresh boot on the same dispatch store forwards exactly that record and nothing older.

### Mutants (each must fail a shipped test; logs in `docs/evidence/task-P4/mutant-*.log`)

| Mutant | Result |
|---|---|
| M1: drop the publish (`commit` returns before `commit_unit`) | killed. Both unit tests fail with `Receive(Timeout)` after 10.03 s, and the scenario fails at step 7 with `a notification did not happen within 20000 ms` (elapsed 20 596 ms, `RUN_EXIT=1`). It fails within the bounds and does not hang. |
| M2: no watermark CAS write | killed: the second forward hits `DuplicateOperation{layout-feed:4}` (both tests) |
| M3: publish the store op id instead of the client op id | killed: `op_id` assertion left/right mismatch (both tests) |
| M4: start at the layout head (no replay) | killed: `forward` returns 0, expected 1 (both tests) |

## Seams (files outside the listed write scope, and why)

- `crates/ovis-terminal-protocol/src/schema/methods/layout.rs` (`mod events;` + re-export), the new `.../methods/layout/events.rs` and `schema/event_catalog.rs`: the published data types `LayoutChangedData`, `SelectionChangedData` and `SelectionChangeCause`, and the catalog entries that point at them. Without these, `op_id` and the change ids have no wire slot. The brief anticipates this ("If you touch the OpenRPC document (event catalog)").
- `schema/ovis-terminal.openrpc.json` (regenerated with `openrpc write`), `packages/client-ts/src/generated/*` (`bun run generate`; all 31 files change because each embeds the schema sha256), `conformance/ideal-map.json` (`quality-matrix --mode contract`), and `conformance/fixtures/ovis/manifest.json` (`ovis-replay refresh`). These are regenerated, not hand-edited.
- `docs/evidence/task-66/task-66-all.json` and `task-66-ovis-terminal-v1.json`: re-recorded with T66's own three literal commands (`cli/manual --case all`; `--case happy`; `--case failure --inject 'ovis-terminal agent get --unknown-flag --json' --append`; all exit 0, 122 / 71 / 29 assertions) on this branch's binary, because their `{"$equals_file":"schema/ovis-terminal.openrpc.json"}` step pinned the old schema and evidence-audit failed with 2 findings. This is the brief's mandated re-record step.
- `conformance/src/runner_tests/suite.rs`: `events/layout-selection.json` is added to `EXECUTABLE` in sorted position. It was not in `declared.json`.
- The scenario's `todo` is `64`. The traceability test admits only numeric plan todos (1..86), and GAP-043 is todo 64's gap. The GAP-043 note records that lane P4 closes it.
- No file under `crates/ovis-terminal-server/src/` changed. The publish path lives in the CLI assembly, where the layout log and the dispatch log are both owned, so the "server unit test" is the daemon-side unit test in `assembly/layout/feed/tests.rs`. `streams/` and `leases/` are untouched.

## Risks

- **Tree at forward time.** `layout.changed` carries the tree at the layout log head when the forward ran. Forwards run on the actor right after each commit, so head = that operation's revision. A record replayed at boot carries the head tree and head revision, which is truthful but coarser.
- **Trailing records without a published event** (an operation that only failed, or `layout.pane_bound`) cannot move the watermark alone, because a dispatch commit needs an event. They are re-read on the next forward until a later publishable operation moves the watermark past them. The cost is bounded by `PAGE` = 256 per read.
- **Retention.** If the layout log evicted records past the watermark (`EventPage::Expired`), the forward resumes at the floor. Those changes were never seen by the dispatch log either.
- **Schema regeneration.** Open lanes whose committed `$equals_file` evidence pins `schema/ovis-terminal.openrpc.json` will be stale on their merge ref once this lands (see notes from T84/T66).

## DEFERRED VERIFICATION

None: the runner executes `events/layout-selection` (exit 0 above).
