# feat(session-binding): bind sessions and version conversation history cursors

Plan todo 36. Closes GAP-097, GAP-101 and GAP-102. Delivers IDEAL-072, IDEAL-075 and IDEAL-076. EXCL-008 and EXCL-019 are enforced: no screen text, no injected scroll input, and no session chosen by directory or recency.

## Before
`absent`. There was no comparable baseline capability: `agent.update_session` and `agent.conversation` were catalog entries with no route. This is not an executed failing test. Gap baseline: GAP-097, GAP-101, GAP-102.

## What changed
- **Explicit binding (`agent.update_session`).** Only the process holding the current program's token can bind, and only after `agent.register`. A replaced incarnation's token, or a successor that has not registered, gets `not_owner`.
- **Reasons.** The first binding needs no reason. Every later change must name `reason: resume | branch | replace`; otherwise the request is `invalid_session` at `params.reason`.
- **Branch selection.** `head` is accepted only with `branch`, must name an existing record, and must stay in the same session and history.
- **One commit unit.** Each change commits the history row, the agent row and an `agent.session_bound` event together. The event carries agent, incarnation, session, history `{history_id, revision, change}`, head, reason, the previous session and history, and the binding revision (the event's sequence).
- **Typed source references.**
  - `kind: path` must resolve, after canonicalization (symlinks and `..` included), to a file inside the store that `session.source` names.
  - `kind: id` is an opaque key with no readable store.
  - A history identity keeps the source it was first bound to. Rebinding it elsewhere is `invalid_session` at `params.history_id`.
- **Versioned history.** Every read observes the transcript from one open handle: file identity (dev/inode), length, and the SHA-256 of the recorded bytes.
  - An append, truncation, replacement (a new file under the same name) or in-place rewrite advances `HistoryRevision`, and so does an explicit branch selection. The same-size case is caught by the digest.
  - The observation is recorded even when the cursor is then refused.
- **Cursors.** `{history_id, revision, position, head?}` is accepted as `cursor` or as the opaque `page.after` token.
  - A cursor for another identity or revision gets `history_changed`, with `error.data.history = {history_id, revision, change}` and no records.
  - A position past the end of the chain, or a token this runtime did not issue, gets `cursor_expired`.
  - Pages never mix revisions.
- **Chains.** Each record names its parent; a line without one continues the previous line. The active chain is the newest record, or, once a head is selected, the newest record appended after that selection that continues it. Records off the active chain are listed in `off_branch` and can be read in full through `cursor.head`.
- **Unavailable, not synthesized.** A read with no binding, or with a session of `kind: id`, is `conversation_unavailable`. Nothing is built from terminal text.

## After
Native OS/arch: `Linux 7.0.0-38-generic x86_64`, rustc 1.99.1. Binary under test: `target/debug/ovis-terminal`, sha256 `ca1d47e12cc6db2a64384d7c1e40cd832b6013411d34e415f43144e262aab2b6`. Literal transcript: `docs/evidence/task-36/qa-transcript.log`. Raw evidence, with every request, reply, byte offset and digest: `docs/evidence/task-36/task-36-ovis-terminal-v1.json`.

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal
EXIT=0
$ cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets
EXIT=0   (230 passed, 0 failed; 22 lane tests, listed in lane-tests.log)
$ cargo clippy -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings
EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case happy --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","totals":{"assertions":"74","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/binding --isolated --case failure --inject '{"method":"agent.conversation","params":{"cursor":{"history_id":"h1","revision":"1","position":"0"}}}' --evidence "$ATTEMPT_DIR/task-36-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","totals":{"assertions":"78","cases":"1","failed":"0","passed":"1","skipped":"0"}}
EXIT=0
$ ... --case failure --inject '{"method":"agent.conversation","params":{"cursor":{"history_id":"h1","revision":1,"position":"0"}}}' ... --append
EXIT=0   (numeric revision -> invalid_params at params.cursor.revision)
$ ... --scenario agent/binding --isolated --case all ... --append
{"case":"all",...,"result":"passed","totals":{"assertions":"155","cases":"2","failed":"0","passed":"2","skipped":"0"}}
EXIT=0
$ ... --scenario all/all --isolated --case all --evidence "$ATTEMPT_DIR/all-all.json"
{"result":"passed","totals":{"assertions":"637","cases":"22","declared_only":"27","failed":"0","passed":"22","skipped":"0"}}
EXIT=0
```

Materialized bridge commands: each case runs `target/debug/ovis-terminal connect` as the stdin/stdout bridge to the fixture's real server, dispatcher and SQLite store on a private Unix socket. Exact argv and timing are in the evidence `processes` entries.

What the stale-cursor reply proves (from the evidence transcript). After the provider appends `r3`, the injected `{"cursor":{"history_id":"h1","revision":"1","position":"0"}}` returns `error.data.kind = "history_changed"` with `error.data.history = {"history_id":"h1","revision":"2","change":"append"}`, and `/result` is absent, so no records are mixed in. The next read at revision 2 returns `r1 r2 r3`.

Happy observable. The test binds h1, pages `r1 r2` (next cursor h1@1, position 2) and then `r3`. The provider forks `r4` from `r2`; the active chain becomes `r1 r2 r4` at revision 2 with `off_branch: ["r3"]`, and `cursor.head = "r3"` reads `r1 r2 r3` with `active: false`. `agent.update_session{reason: resume}` then binds h2, `agent.get` shows h2, h2 pages `s1`, and the old h1 cursor gets `history_changed` with `{history_id: h2, revision: "1", change: rebind}`. The `agent.session_bound` event payload is asserted in the agent crate test `a_first_binding_and_a_resume_each_commit_one_explicit_event`. The fixture has no event subscription step, so the scenario asserts the binding through `agent.get`.

## QA
- Happy and failure as above, both EXIT=0, with failed 0 and skipped 0.
- The separate malformed-type injection, a numeric revision `1`, returns `invalid_params` at `params.cursor.revision`. Only the schema-valid stale cursor is asserted to return `history_changed`.
- `bash scripts/check.sh` exits 0: fmt, workspace clippy `-D warnings`, nextest 715 passed, doctests, hygiene, manifests and json. Log: `docs/evidence/task-36/check.log`.
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0. Log: `windows.log`.

## Adversarial
- **Stale state, same-size rewrite.** In the scenario failure case, the rewrite step's reply asserts the same byte length as the original (`/result/bytes`); the next read is `history_changed` with change `rewrite`. Covered also by the agent test `truncation_replacement_and_a_same_size_rewrite_each_advance_the_revision` and the server wire test. Truncation and replacement each advance the revision (change `truncate` / `replace`). The proptest `appends_rewrites_and_truncations_are_told_apart` checks the classifier against arbitrary content and a single flipped byte.
- **Stale state, old process token.** After the program is replaced with incarnation 2, the old token gets `not_owner` on the wire (server test `a_replaced_programs_token_is_refused_and_a_replayed_branch_applies_once`). A successor that has not registered also gets `not_owner`, with a hint naming `agent.register`. Foreign and forged tokens are refused in the scenario.
- **Repeated interruptions, branch replay by op_id.** The same branch request with the same `op_id` is sent twice, as a client would after a lost reply. Both replies are identical, and the history moved exactly one revision. Checked in the server wire test and in scenario steps `branch-1`/`branch-2` (one `$op:branch` label, then a read pinned at revision 5).
- **Path escape.** An absolute path outside the store, a symlink inside the store pointing out, `../` traversal and an unlisted `session.source` are all `invalid_session`.

## Contract checklist
1. `HistoryRevision`, `RecordPosition`, binding `Revision` and `bytes` are canonical decimal strings. The proptest `cursors_round_trip_as_decimal_strings` covers any `u64`, including `u64::MAX`, for both the cursor and its token. `history_changed` carries `"18446744073709551615"` exactly in a unit test, and a numeric revision is `invalid_params`.
2. `BindingError` is a `thiserror` enum with `kind()` and `hint()`, and `#[source]` on the I/O and status sources. Its Display never repeats the kind token.
3. `HistoryId`, `RecordId`, `HistoryRevision`, `RecordPosition`, `ProcessToken`, `TranscriptPath`, `SessionKey` and the typed `SourceRef` are distinct types. `Option` is used only where absence is a real state (no reason on a first binding, no head on the active chain, no next page).
4. Tests assert behavior: revisions after real file writes, typed kinds and field paths, events from a real coordinator subscription, and wire replies. Proptests cover cursor round trips, chain structure and change classification.
5. No `#[allow]` and no placeholder TODO. There are two targeted `#[expect(clippy::exhaustive_structs, reason)]` on the cross-crate fact structs, following the worktrees precedent, and one `#[expect(clippy::unnecessary_wraps, reason)]` on the Unix file-identity function, which shares the fallible signature of its non-Unix counterpart.
6. Conformance is executed, not deferred: `agent/binding` is an executable scenario (format marker present) and is listed in `runner_tests/suite.rs` (`EXECUTABLE` now has 12 entries).

The holder wire generation stays 2.

## Seams (public API changes in landed crates)
- **protocol** (all additive; `openrpc compat schema/released.openrpc.json` reports compatible, and `schema/ovis-terminal.openrpc.json` is regenerated):
  - New `domain::session_binding` (`SessionChange`, `HistoryChange`, `ConversationCursor`, `HistoryHead`, `SessionBindingRecord`, `BindingFacts`, `ConversationPage`), plus `ConversationRecord::new` and `PageInfo::new`.
  - New id `RecordId`, scalars `HistoryRevision` and `RecordPosition`, and `PageToken::{new, as_str}`.
  - `AgentUpdateSessionParams` gains optional `reason` and `head`; `AgentConversationParams` gains optional `cursor`.
  - `ConversationRecord` gains `record_id` and `parent`. `AgentConversationResult` gains `revision`, `head`, `active`, `off_branch` and `next_cursor`. These are output-only additions; neither type had a producer before this PR.
  - New event `agent.session_bound`.
  - `ErrorData.history` (boxed, omitted when absent), with `BoundaryError::with_history` and `BoundaryError::with_field_path`.
- **agent:**
  - New `conversation::binding` (`BindingService`, `Update`, `Read`, `Stores`, `SourceRef`, `TranscriptPath`, `SessionKey`, `BindingError`).
  - `AgentStatusEvent::SessionBound`.
  - `status::row` is now a public module path (its items stay `pub(crate)`), and `StatusService` gains crate-internal `location`/`unit_event` access for the binding service.
- **server:**
  - New `dispatch::lanes::session_binding` (`SessionBindingLane`, `UpdateSession`, `Conversation`).
  - In `agent_status`, `terminal_access` and `terminal` are now `pub(super)` helpers shared by the two lanes.
  - `tests/dispatch.rs` gains one `#[path]` module line.
- **conformance fixture:**
  - Registers the two methods over the existing agent terminals and authorizes `<root>/transcripts` as the store `fixture-store`.
  - A fixture-only `fixture.transcript` method stands in for the agent writing its native store (write, append, or replace by rename). It is served by the fixture handler but not advertised in `hello`, since it is not a catalog method.
  - `agents::register` now returns the status service and directory.
- **CI:** `.github/workflows/probe-stress-macos.yml` gains only the `session-binding` option and job, both appended at the end.
- **Outside the literal write scope:** the scenario file is `conformance/scenarios/agent/binding.json`, because the runner's id grammar is `<family>/<name>.json` (same precedent as todo 45).

## Risks
- Transcripts are read whole, up to 8 MiB, on every read. Incremental reads are left to the provider adapters (todo 37+).
- On non-Unix targets, file identity is the creation time, so a replacement created within the same nanosecond is detected only through length or digest.
- `agent.conversation` runs as a recorded commit unit, because a read can record a newly observed revision. It is not an unrecorded read route.
- The binding and history rows persist in the store, but the token registry is in memory, as in todo 33.
- `origin/integration` moved to `b9be58f` (the t51 merge) after this lane was cut. Per the lane rules it was not merged in here. `git merge-tree` reports no conflicts (see the DoneClaim).
