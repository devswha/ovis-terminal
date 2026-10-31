# feat(layout): implement deterministic thin workspace and layout service

Plan todo 18 (wave 8). Closes GAP-032, GAP-033, GAP-034, GAP-036, GAP-110, GAP-111, GAP-112, GAP-113, GAP-115, GAP-149, GAP-150. Delivers IDEAL-023, IDEAL-024, IDEAL-084, IDEAL-085, IDEAL-086, IDEAL-088.

## Before

absent (GAP-032, GAP-033, GAP-034, GAP-036, GAP-110, GAP-111, GAP-112, GAP-113, GAP-115, GAP-149, GAP-150; no comparable new-runtime capability is claimed as a legacy execution).

## After

What landed in `crates/ovis-terminal-core/src/layout/`:

- Records: workspace, tab, split node (`SplitId`, interior nodes only), pane (with optional bound `TerminalId`) and per-client `Selection`. They are stored as one CAS-versioned row per runtime and read back through the pinned WAL view. A persisted tree that is unreachable, shared or cyclic is rejected as `invalid_record`.
- Pure edits that name every resource by explicit ID: `create_workspace` (optionally `group_with`), `create_tab`, `split`, `close` (`target` | `group` scope), `move_tab`, `move_pane`, `swap`, `zoom`, `set_ratio`, `resize`. No edit reads or changes focus or any selection. Tabs and workspaces emptied by a move or close stay in place; nothing is closed implicitly.
- Geometry: integer rectangles in parts-per-million of the tab (`EXTENT = 1_000_000`), with the split rule `extent * permille / 1000`. A ratio outside 100..=900 permille fails with `invalid_geometry {reason: out_of_range, requested, min, max, step}`. A tab-relative `resize` whose result is not a whole permille share fails with `reason: unrepresentable` and the exact step. Neither is ever clamped or rounded. Unchanged requests return `no_op: already_applied` without a new revision.
- Transaction boundary (`Layouts::edit` / `select` / `get`): the todo-15 operation ledger (`accept` → `start` → `Operations::commit_unit`) commits the layout row, staged identity reservations, the op receipt and ordered events in one SQLite unit. Identical retries return the recorded result; a reused `op_id` returns `op_reused`; a rejected edit is recorded as a `failed` op carrying its error kind. Authority is checked at commit (`LayoutChange` per touched workspace, `Machine` for creating a workspace, every group member for group close).
- Events: `layout.changed {revision, created, removed, changed}` names every affected resource in the envelope. `selection.changed {client, workspace, tab, pane, cause: requested | target_removed}` names the client plus the IDs. `layout.pane_bound {pane, terminal}`. The revision is stamped at the commit sequence.
- Creation and launch: a creating edit with `launch` commits the created IDs first (`record_unit`, op still running), then calls the `Launcher`, then commits `LaunchOutcome` (`launched {terminal, spawn}` | `rejected {error}`) and binds the pane. A rejected launch never hides the created workspace/tab/pane. `Launcher` is implemented for the landed `terminals::Terminals` (unix).
- Close scope: closing a workspace linked to others with `scope: target` returns `group_scope_required` with the full affected ID list and changes nothing. Selections that named removed IDs are cleared with `cause: target_removed`, never retargeted.
- Session snapshot: `SessionProducer` captures `SnapshotSections { layout: Complete(Layout), terminals: Complete(Vec<terminals::Record>) }` plus the op ledger from one read transaction.

Real binary attempt (literal command, Linux 7.0.0-38-generic x86_64, rustc 1.99.1):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-18 --isolated --case happy --evidence "$ATTEMPT_DIR/task-18-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT 2
```

**DEFERRED VERIFICATION (runner lands in Todo 21)**: per plan line 275, part-A foundational todos land behind tests. `conformance/scenarios/part-a/task-18.json` records the happy/failure steps; the runner and the `layout.edit`/`selection.set` dispatch registration are not in this PR. No CLI/socket transcript is claimed. Transcript: `docs/evidence/task-18/conformance.log`.

## QA

| Command | Exit | What the test asserts |
|---|---|---|
| `cargo test -p ovis-terminal-core --test layout happy -- --exact --nocapture` | 0 (1 test ran) | Swap(p1,p3) then `move_tab` into another workspace: the tab's pane-ID set and split-ID set are unchanged; both clients' selections (`phone`→p1, `desk`→p3, the swapped panes) are byte-equal before/after; p1/p3 rectangles are exchanged; the source workspace remains with 0 tabs; revisions strictly increase. Exactly 6 `layout.changed` events arrive, each with `revision == event seq`, and the swap/move events name the panes, tab and both workspaces. (unix) `create_tab` with a real holder-backed `Terminals` launcher and a missing cwd returns the created tab+pane IDs, `launch.state = rejected` with kind `invalid_params` and an inspectable terminal `op_id`; the tab+pane are present in `layout.get` with no terminal. |
| `cargo test -p ovis-terminal-core --test layout failure -- --exact --nocapture` | 0 (1 test ran) | `set_ratio 950` → `invalid_geometry` (`out_of_range`, 950, 100, 900), layout byte-equal, and the op ledger shows `failed` with kind `invalid_geometry`. `resize +1` → `unrepresentable`, step 1000; `resize +450000` → `out_of_range`, max 400000. Closing a grouped workspace with `target` scope → `group_scope_required` naming both workspaces, the pane and the split, layout unchanged. A stale `expect_revision` → `revision_mismatch` with the current revision. A `group` close removes both workspaces and clears (not retargets) the `phone` selection. |
| `cargo nextest run -p ovis-terminal-core` | 0 | 103 passed (1 slow, pre-existing ops capacity test). |
| `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` | 0 | |
| `bash scripts/check.sh` | 0 | fmt, workspace clippy, 444 nextest passed / 1 skipped, doctests, hygiene (0 violations), manifests, json. Log: `docs/evidence/task-18/check.log`. |

Other tests in `tests/layout.rs`: `malformed_requests_are_rejected_before_any_effect`, `replay_converges_and_reuse_is_refused`, `session_snapshot_holds_layout_and_terminals_at_one_revision`, `launch_binds_terminal_to_created_pane` (unix, real holder), `layout_never_mutates_a_checkout` (unix). Unit/property tests in `src/layout/tests.rs`: proptest over random edit sequences. It asserts that ID sets stay unique, every tree validates, visible panes tile the tab exactly (sum of areas = EXTENT²), surviving splits keep their axis, a rejected edit leaves the layout byte-equal, and Layout JSON round-trips. There is also a proptest of `revision` as a canonical decimal string for every u64, plus a u64::MAX exact round trip with numeric rejection.

TDD note: the pure-layer property tests were written after the pure functions and passed on first run. Their ability to fail is shown by the mutation probes below, not by a recorded red run.

## Adversarial

- Malformed input (ratio/target): a misnamed `pane_id` field and a numeric `expect_revision` → `invalid_params` at `EditRequest::decode`. An unknown `pane:` target → `not_found` with `target` set. The layout is byte-equal afterwards, with no focus fallback (V-RT-HD-03).
- Stale state: `expect_revision` mismatch → `revision_mismatch`, nothing committed.
- Misleading success / partial creation: a launch rejection still commits and returns the created IDs, and the IDs remain readable. A successful launch binds the terminal to exactly the created pane in a separate `layout.pane_bound` commit.
- Dirty worktree: a disposable directory with an uncommitted file used as the launch cwd; after launch, split and close, the entries and bytes are identical.
- Replay: identical `op_id` + params → identical `Change`, no new revision. Same `op_id`, different params → `op_reused`.
- Mutation probes (`docs/evidence/task-18/mutation-probes.log`), each temporarily applied and reverted:
  - clamping `Ratio::new` → `failure` fails;
  - ignoring group scope → `failure` fails;
  - retargeting a selection on swap → `happy` fails (the first attempt survived because no client selected a swapped pane, so the test was strengthened);
  - turning a rejected launch into an error → `happy` fails.

## Contract checklist

1. u64 on the wire: `revision`, `seq` and op counters use the protocol `Revision`/`EventSeq` decimal-string newtypes. The numeric form is rejected (unit test plus `EditRequest::decode`), and u64::MAX round-trips exactly (unit test and proptest).
2. Boundary error: `LayoutError` is a `thiserror` enum with `kind()`/`hint()`; inner errors are `#[from]`/`#[source]`, and Display never repeats the kind token. `data()` builds the wire `ErrorData` through `BoundaryError::domain`.
3. Identities: the registry `WorkspaceId`/`TabId`/`PaneId`/`SplitId`/`TerminalId` newtypes and protocol `ClientId` are reused, and new IDs are staged through `IdentityReservation::stage` inside the same commit unit. `Option` is used only where absence is a state (empty tab root, unbound pane, unselected field).
4. Tests assert behavior (ID-set equality, rectangles, typed kinds, op-ledger state, event envelopes), not locally assigned literals. proptest covers edit sequences and revision round trips.
5. No `#[allow]`, no `#[expect]`, no TODOs; files are ≤250 pure LOC and functions have ≤3 parameters (hygiene: 0 violations).
6. Evidence states exactly what is asserted; the conformance attempt is recorded verbatim above as deferred.

Holder wire generation stays 2; no holder or protocol change.

## Seams

- New public API: `ovis_terminal_core::layout::*` (`Layouts`, `Call`, `EditRequest`, `SelectRequest`, `Edit`, `Target`, `CloseScope`, `Layout` records, `Change`, `LaunchOutcome`, `LayoutEvent`, `Launcher`, `Producer`/`LayoutSnapshot`, `SessionProducer`/`SessionSnapshot`, geometry types, `LayoutError`). `Launcher` is implemented for the landed `terminals::Terminals`. No landed crate's public API changed.
- The core edit vocabulary is a superset of the protocol's `LayoutEdit` (it adds `move_pane` and `resize`, and uses `group_with`). The released catalog's close-scope kind is `scope_required`, while this service returns `group_scope_required` as the todo specifies. Mapping onto `layout.edit`/`selection.set` and the additive schema/kind entries are left to the protocol/dispatch owners (todos 4/14/31). Dispatch registration is not in this PR.
- `.github/workflows/probe-stress-macos.yml`: new `layout` dispatch choice and job (100× `--test layout` on macos-latest). The `platform` job condition changed from `!= 'holder-shutdown'` to `== 'platform'`, which is equivalent for the existing choices.
- Worktree: the brief's path `lane-t76` already held another lane's branch (`lane/t24-input-geometry-leases`), so this work used `/home/devswha/workspace/ovis-terminal-wt/lane-t18` on `lane/t18-workspace-layout`.

## Risks

- Layout and terminals currently run on separate coordinators/stores, so the terminal binding is a layout-side record. Single-coordinator assembly (one event enum covering both) is todo 42's. `SessionProducer` is proven over one store at one cursor, with an empty terminal section in the test.
- Closing a pane or tab releases its terminals (listed in `released_terminals`) but does not stop them. Stopping is an explicit terminal operation.
- A replayed failed op returns the operation-ledger error `already_started` with the receipt (which carries the original kind), not the original layout error variant.
- macOS: the `layout` stress dispatch result is linked below once it finishes.
