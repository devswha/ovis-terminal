# feat(worktrees): manage explicit worktree identities and destructive scope

Plan todo 45. Closes GAP-114 and GAP-115; delivers IDEAL-087 and IDEAL-088. Enforces EXCL-005, EXCL-006, EXCL-018 and EXCL-022 (explicit targets, closed params, visible scope, identities not derived from paths) without the V-005, V-160, V-178 or V-179 defects (no `String` errors, at most 3 parameters, no six-parameter closure in the porcelain parser).

## Before
`absent`: no comparable baseline capability. This is not an executed failing test. Gap baseline: GAP-114, GAP-115.

## After
Native platform: Linux x86_64. Branch head includes the orchestrator's union merge of origin/integration 99e5c52 (2e38aa8). origin/integration has since moved to 473e7d5; this lane does not merge it.

The `worktree.*` lane is exercised over a real Unix socket. The test drives the real `Dispatcher`, Authority, SQLite `Coordinator` and Git 2.53 in a private repository: `cargo nextest run --locked -p ovis-terminal-server --test dispatch -E 'test(/worktrees::/)' --no-capture` exits 0 with 3 of 3 passed. Transcript: `docs/evidence/task-45/server-qa.log`. These are actual replies from that run:

```
<- {"id":"narrow","error":{"code":-32000,"data":{"kind":"scope_required","op_id":"01a19281-5410-...","affected":["checkout:01a19281-5456-...","workspace:1"]}}}
<- {"id":"op","result":{"state":"failed","method":"worktree.remove","error":{"kind":"scope_required","affected":[...]}, "updated_seq":"13"}}
<- {"id":"dirty","error":{"code":-32000,"data":{"kind":"checkout_dirty","op_id":"01a19281-534f-...","affected":["checkout:01a19281-5394-..."]}}}
<- {"id":"flat","error":{"code":-32602,"data":{"kind":"invalid_params","field_path":"params.force"}}}
<- {"id":"slow","error":{"code":-32000,"data":{"kind":"cancelled","op_id":"01a19281-5556-...","affected":["branch:partial","checkout:01a19281-55a1-..."]}}}
```

The happy test (`happy_create_open_and_enumerated_group_removal`) asserts the following:
- `worktree.create` from the primary checkout's explicit id returns `worktree.source` equal to that id and a decimal-string `seq`.
- `worktree.open` returns one `affected_workspaces` id, and the record lists it.
- `worktree.remove` with `{group:false, force:false}` returns `scope_required`, with `error.data.affected == [checkout, workspace]` and the request's `op_id`. The checkout directory still exists, and `op.get` reports `state: failed`.
- With `{group:true}`, the removal returns exactly that workspace in `affected_workspaces`, the directory is gone, the owner recorded one close, and `worktree.list` returns one checkout.

The failure test (`failure_dirty_checkout_is_refused_with_its_identity_and_kept`) asserts the following:
- The plan's flat payload `{"checkout_id":…,"force":false,"group":false}` is `invalid_params` (`params.force`) before any effect.
- `{scope:{group:false,force:false}}` on a checkout holding an untracked file returns `checkout_dirty` with `affected == [checkout]`.
- The file content is unchanged, `worktree.list` reports `dirty: true`, and no workspace was closed.

The cancel test (`op_cancel_kills_a_hung_create_and_reports_what_committed`) uses a Git wrapper that runs `worktree add`, reports on a FIFO, then blocks forever. After `op.cancel`, the create reply is `cancelled` with `affected == ["branch:partial", <checkout id>]`. `worktree.list` then shows that checkout id, and the branch exists.

Token note: the plan text names `dirty_worktree` / `group_scope_required`. The released catalog (todo 4, `schema/released.openrpc.json`) lists `worktree.remove` errors as `scope_required`, `checkout_dirty`, `checkout_locked` and `vcs_failed`. The lane returns the catalog tokens.

### DEFERRED VERIFICATION (runner lands in Todo 21)
The runner landed, but it cannot route this surface yet. Its only fixture kind (`protocol`) does not register service lanes, it has no `--inject`/`--append` flags, and the layout-backed `Workspaces` owner arrives with todo 42. `conformance/scenarios/services/worktrees.json` is therefore a declaration, allowlisted in `declared.json` (owner 45). These are the verbatim attempts with `ATTEMPT_DIR=.omo/evidence/ovis-terminal-v1/task-45`:

- `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/worktrees --isolated --case all --evidence "$ATTEMPT_DIR/task-45-ovis-terminal-v1.json"` → exit 1, `scenario_not_executable` ("services/worktrees is a declaration, not a registered executable scenario").
- The same command with `--case happy` → exit 1, `scenario_not_executable`.
- The same command with `--case failure --inject '{"method":"worktree.remove","params":{"checkout_id":"dirty-checkout","force":false,"group":false}}' ... --append` → exit 2, `error: unexpected argument '--inject' found`.
- `target/debug/conformance --binary target/debug/ovis-terminal --scenario all/all --isolated --case all` → exit 0: 16 cases passed, 345 assertions, 25 declared_only on the merged head (including services/worktrees), 0 failed, 0 skipped.

Raw output is in `docs/evidence/task-45/conformance-*.out|err` and `all-all.out`.

## QA
- `bash scripts/check.sh` → exit 0 (fmt, clippy `-D warnings`, nextest 575 run / 575 passed / 1 skipped, doc tests, hygiene, manifests, json). Log: `docs/evidence/task-45/check.log`.
- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` → exit 0.
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` → exit 0.
- The 13 worktree tests in the services and server crates ran 20 times under `timeout 120 cargo nextest run`: 20 of 20 runs, 13 of 13 passed each time.
- Services (`cargo nextest run -p ovis-terminal-services`): 10 tests against real Git. They cover:
  - create, open and group removal;
  - identity retirement after an external removal (the stale id is `not_found`);
  - dirty content kept until force, where group scope is not force;
  - a locked checkout needs force, and Git gets `--force --force`;
  - primary checkout, `-x`, `a..b` and an existing branch are refused with nothing created;
  - cancellation of a hung `worktree add` lists the committed branch and checkout;
  - a Git that never answers is killed at its bound (`vcs_failed`, retryable);
  - a proptest porcelain round trip;
  - an exhaustive 32-case scope table.
- Protocol: 7 tests. They cover `u64::MAX` decimal-string round trips for `revision` and `seq`, numeric `revision` rejected, flat or partial scope rejected (`deny_unknown_fields`), branch names refused as options or control characters, `affected` identities distinct per family, and `ErrorData.affected` round trip (omitted when empty).

## Adversarial
- Dirty worktree and locked checkout: refusals happen before the commit boundary and before any Git mutation. Tests assert the file content and the directory survive. Group scope never satisfies force, and force never satisfies group.
- Hung commands: Git runs with `kill_on_drop`. It is killed when the operation's cancellation flag flips (`op.cancel` or dispatcher shutdown) or at a per-command bound. Both cases are tested with real blocking processes, and every test wait is bounded.
- Misleading success: `worktree.create` reports only the created checkout. Opening a workspace is a separate `worktree.open` with its own `affected_workspaces`, and no program is launched. Cancellation reports what was committed (`branch:…`, `checkout:…`) instead of a plain failure.
- Misnamed target (EXCL-005/006): unknown or flat fields are `invalid_params`. Unknown repository or checkout ids are `not_found`, never a focus fallback.
- Identity reuse (EXCL-022): see Round 3 below. Identity is an ovis marker in the worktree's Git admin directory, never the path.
- Argv injection: Git runs by argv with `-C <dir>`. Inherited `GIT_*` variables are stripped, prompts are disabled, branch names cannot start with `-`, and `--` precedes paths.

## Seams
Public API changes, each disclosed:
- `ovis-terminal-protocol`: `ErrorData` gains an additive field, `affected: Box<[TargetId]>` (default empty, omitted when empty). It also gains `BoundaryError::with_affected` and the matching schema property. `schema/ovis-terminal.openrpc.json` is regenerated, and `openrpc compat schema/released.openrpc.json` stays compatible. A boxed slice keeps `RpcReply` under clippy's large-variant bound.
- `ovis-terminal-protocol`: a new `domain::worktrees` module (`WorktreeFacts`, `BranchName`, `AffectedResource`), plus constructors `WorktreeChange::new`, `WorktreeListResult::new` and `RemoveScope::new` for the `#[non_exhaustive]` catalog types.
- `ovis-terminal-server`: `Execution::cancellation() -> watch::Receiver<bool>` lets an effect inside `commit` stop cooperatively. `DispatchError::Worktree(WorktreeError)` maps kind, hint, retryable and `affected` onto the wire. A new `dispatch::lanes::worktrees::Lane` registers the four methods.
- `ovis-terminal-services`: a new `worktrees` module (`Worktrees`, `Config`, `Repository`, `Git`, `Cancel`, `Call`, `Change`, `WorktreeEvent`, the `Workspaces` port, `WorktreeError`).
- Manifests: services gains `serde`, `serde_json`, `thiserror`, `tokio`, `uuid` and dev `proptest`; server gains dev `uuid`. `tests/dispatch.rs` gains two `#[path]` module lines.
- Outside the literal write scope: the scenario file is `conformance/scenarios/services/worktrees.json`, because the runner's id grammar is `<family>/<name>.json`. The plan's `services/worktrees/` directory would be an invalid id.
- The holder wire generation stays 2.

## Contract checklist
1. Every u64 on this surface (`revision`, `seq`, `updated_seq`) is a decimal string. A `u64::MAX` round trip and a numeric-rejection test are included.
2. `WorktreeError` and `BranchNameError` are `thiserror` enums with `kind()` and `hint()`, and `#[source]` where an inner error exists. Display text never repeats the kind token.
3. `RepositoryId`, `CheckoutId`, the core `WorkspaceId`, `BranchName`, `OpId` and `TargetId` are distinct newtypes. `Option` is used only for real absence (branch for a detached checkout, source for a checkout not created here).
4. Tests assert behavior (filesystem state, Git refs, typed kinds, affected lists, op state). They include a proptest for porcelain and record round trips.
5. There is no `#[allow]` and no placeholder TODO. The only lint expectations are targeted `#[expect(clippy::exhaustive_structs, reason=…)]` on four structs that other crates construct, following the holder crate's precedent.
6. This body states what each test asserts. The conformance attempts are recorded verbatim above.

## Round 2 (Darwin)
On 380aff6, `test (macos-latest)` failed both hung-create tests: the services test hit "Elapsed" after 60 s, and the server test reported "git never reported that worktree add ran" after 30 s. Linux and Windows were green.

The cause is in the test fixture. It waited for the fake Git's `ready` line through `tokio::net::unix::pipe::Receiver`. On macOS that receiver does not report the FIFO's data or end-of-file; todo 43 hit the same problem in stress run 41213460834. The product was unchanged and is not the blocking party.

Fix: the shared `signalled(fifo)` helper in both suites now uses a blocking `read_to_string` on `spawn_blocking`, which is plain `read(2)` and behaves the same on both kernels. It is bounded at 30 s. On timeout it opens the writer end, so the blocked reader returns and runtime shutdown cannot hang. The services test sends the cancellation even when the signal fails, so the hung create always ends.

Verification after the fix:
- The 13 worktree tests ran 20 times under `timeout 120 cargo nextest run`: 20 of 20 runs passed.
- `bash scripts/check.sh` → exit 0 (575 passed).
- The `x86_64-pc-windows-gnu` pre-check → exit 0.
- Not reproduced locally on Darwin. This round's CI macOS job is the Darwin check.

## Round 3 (gate review B1 and authz)
B1 (EXCL-022): `State::identify` matched `known.path == path`. A checkout removed and re-created at the same path between two listings therefore kept the old `checkout_id` and its stale `source`. A client holding the old id could retarget or force-remove the new checkout. The `no_retain` mutant also survived.

Fix: checkout identity lives in an ovis marker, `ovis-terminal-checkout.json` (`{checkout_id, source}`, written atomically by temp file plus rename). The marker sits in the worktree's Git admin directory:
- for a linked checkout, `<git-common-dir>/worktrees/<name>`, mapped from each admin entry's `gitdir` file;
- for the primary checkout, the common directory.

Git deletes the admin directory with the worktree, so a re-created checkout has no marker and gets a new UUIDv7 id with no source. A `git worktree move` keeps the admin directory, so a moved checkout keeps its id and source; this also covers gate note N3. The path is reported but is never a key. Because the source relationship is persisted in the marker, it also survives a control restart. The marker is runtime metadata in Git's private admin area, not in checkout content, and the first listing of an existing checkout writes it.

New tests:
- `worktrees::tests::identity::a_checkout_recreated_at_the_same_path_never_inherits_the_old_identity`: remove plus re-add at the same path between listings gives a new id and `source: null`. The old id is `not_found` for both remove and open, and the new checkout survives.
- `worktrees::tests::identity::a_moved_checkout_keeps_its_identity_and_source`.
- `worktrees::state::tests::identities_missing_from_a_listing_are_retired_with_their_links`: this kills the `no_retain` mutant. An unlisted identity is dropped with its workspace links, and another repository's identities are untouched.

Authz: `worktree.remove` now needs `Action::Administer` (the admin permission). It deletes files and closes workspaces, so the layout-change grant is not enough. Create and open stay on `LayoutChange`. The new test is `worktree_authority::removal_needs_the_admin_grant_beyond_layout_change`: a principal holding observe and interact over the whole machine can create a checkout, but `worktree.remove` with `{group:true, force:true}` returns `forbidden`. The directory still exists, and the checkout is still listed.

Verification:
- The worktree tests (7 protocol, 13 services, 4 server) ran 20 times under `timeout 120`: 20 of 20 runs, 24 of 24 passed each time.
- clippy `-D warnings` and workspace-hygiene are clean.
- The Windows `x86_64-pc-windows-gnu` pre-check exits 0.
- `bash scripts/check.sh` → exit 0 (nextest 604 run / 604 passed / 1 skipped). Log: `docs/evidence/task-45/check.log`.

## Risks
- Todos 43 and 46 add `DispatchError::Domain` and the `domain`/`lanes` module roots too. This PR adds `DispatchError::Worktree` and inline `pub mod domain { … }` / `pub mod lanes { … }` blocks. Expect a union merge.
- Checkout ids and source relationships are durable in the Git admin markers. Workspace links and revisions stay in memory (recovery is todo 49).
- `Workspaces` is a port: no production implementation exists until todo 42 assembles the layout owner on the shared coordinator. Until then, `worktree.open` needs an assembly-supplied owner.
- `worktree.list` runs `git status` per checkout under the service lock. That is fine for tens of checkouts, but not tuned for hundreds.
- Git stdout is read whole and stderr is discarded, so Git's diagnostic text is not surfaced.
- No macOS manual stress job was added. macOS runs these tests in the regular CI matrix.
