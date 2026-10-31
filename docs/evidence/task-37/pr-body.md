feat(conversation-records): expose typed conversation blocks and turn records (plan todo 37)

Closes GAP-098, GAP-100, GAP-103, GAP-105 and the contract side of GAP-104 (provider parsers stay Ovis-owned per the 2026-10-31 scope decision, EXCL-028/029). Delivers IDEAL-073, IDEAL-074, IDEAL-077, IDEAL-079. Enforces EXCL-008, EXCL-010, EXCL-019 and avoids V-4388 (closed enums, not free strings), V-4393 (no clock read in the turn fold; times come from boundary-stamped status reports), V-005 (typed `RecordsError` with kind/hint/source).

## Before

`absent`: no comparable baseline capability existed, and this is not an executed failing test. Gap baseline: GAP-098, GAP-100, GAP-103, GAP-105 (`docs/design/gap-list.md`). On integration 804e508, `agent.conversation` served only T36 transcript lines with untyped `blocks: Value[]`, did not accept `source`, and had no typed record ingestion, no tool-call linkage and no turn records.

## After

Native host: `Linux x86_64`. Binary `target/debug/ovis-terminal` sha256 `6166822728b7a3b2969264c7639d1d3b70cc2e56afaf26845c6f3c9b40f1fb41` (recorded in the evidence JSON).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal            # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/records --isolated --case happy --evidence "$ATTEMPT_DIR/task-37-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"agent/records","totals":{"assertions":"86","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/records --isolated --case failure --inject '{"method":"agent.conversation","params":{"source":"terminal_screen"}}' --evidence "$ATTEMPT_DIR/task-37-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"agent/records","totals":{"assertions":"44","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run ... --scenario agent/records --isolated --case all --evidence "$ATTEMPT_DIR/task-37-all.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"129","cases":"2","failed":"0","passed":"2","skipped":"0"}}
exit 0
```

Materialized commands: the runner starts `target/debug/conformance fixture --root /tmp/ovc-<pid>-0` (the real server crate: listener, connection, Dispatcher, SQLite Coordinator, Authority on a private Unix socket) and drives it through `target/debug/ovis-terminal connect` as the bridge. Raw replies from the failure-case wire transcript:

```
-> {"id":"inject-0","method":"agent.conversation","params":{"target":{"name":"agent"},"source":"terminal_screen"}}
<- {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32602,"message":"method parameters violate the closed schema","data":{"kind":"invalid_params","hint":"correct the method parameters","retryable":false,"field_path":"params.source"}}}
<- {"jsonrpc":"2.0","id":"unintegrated","error":{"code":-32000,"message":"operation failed","data":{"kind":"conversation_unavailable",...,"unavailable":{"reason":"not_reported","provenance":{"source":"records","agent_id":"agent:01a1944f-ece7-73c7-a932-1e0b15e091fb","session":{"source":"provider-x","kind":"id","value":"session-77"}}}}}}
<- {"jsonrpc":"2.0","id":"plain","error":{"code":-32000,"message":"operation failed","data":{"kind":"conversation_unavailable",...,"unavailable":{"reason":"no_agent","provenance":{"source":"records","agent_id":null,"session":null}}}}}
```

Happy observables (asserted in both the append result and the later read): records `u1, a1, u2, a2` in order; `u2` carries `[text, tool_result]` in provider order with `links[0] = {call_id: "c1", call_record: "a1"}` and `parent: "a1"`; `a1.phase = commentary`, `a2.phase = final`, `u1` has no phase; every record has `turn: "1"`. After `status.report idle`: `turns[0] = {state: "ended", outcome: "completed", ended_seq: "19", ended_ms: "1793489622798", model: "model-a", reasoning: null}`.

Raw evidence: `docs/evidence/task-37/task-37-ovis-terminal-v1.json` (happy and failure runs, argv, wire transcripts, every assertion's expected/actual), plus `happy.*`, `failure.*`, `all.*` stdout/stderr/exit companions.

## What changed

- **protocol** `domain::conversation_records`: `ConversationSource {transcript, records}`, `RecordRole {user, assistant, tool}`, `MessagePhase {commentary, final}`, `ContentBlock` (tagged `text | tool_call | tool_result`, closed fields), `RecordBlock` (typed, or a T36 transcript block kept verbatim), `ToolLink`, `RecordInput`, `TurnMetadata`, `AppendRecordsParams`, `ConversationTurn` + `TurnState {open, ended}`, `RecordsAppended` (one type for the event payload and the append result), `Unavailable {reason: no_agent|not_reported, provenance {source, agent_id, session}}`. New method `agent.append_records` (Mutation, owner token), new event `agent.records_appended`, new domain error kind `invalid_records`, new id `CallId`.
- **agent** `conversation::records`: `RecordsService::append` validates the batch against the stored log (unique record and call ids, every tool result answers a recorded call, role/block/phase rules, bounds of 64 records per append, 32 blocks, 64 KiB texts, 10,000 records or 8 MiB per agent), stamps each record with the status turn only while the status root is `working`, and commits the log row and event in one unit. `RecordsService::conversation` pages the log with history-bound cursors. A turn's outcome, `ended_seq` and boundary times come only from the T33 status row: `started_ms` and `ended_ms` are the acceptance times of the reports that started and ended the turn, recorded only while those reports are the status root and otherwise left null.
- **server** `lanes::conversation_records`: `AppendRecords` and the single `agent.conversation` route (`source` absent or `transcript` goes to T36 `BindingService`, `records` goes to `RecordsService`).
- **conformance**: executable `agent/records` scenario (happy, failure with a planned inject); the fixture registers the new lane.
- **workflow**: `conversation-records` option and job at the end of `probe-stress-macos.yml` (25 iterations of the lane tests and the scenario).

## Seams (public API changes in landed crates)

- `ovis-terminal-server`: `lanes::session_binding::Conversation` is removed; `agent.conversation` now lives in `lanes::conversation_records::Conversation`, which serves the transcript source unchanged. Updated callers: `lanes/session_binding/qa.rs` and `conformance/src/fixture/transcripts.rs`.
- `ovis-terminal-protocol`: `ConversationRecord.blocks` is `Vec<RecordBlock>` (was `Vec<Value>`; wire bytes are unchanged for transcript blocks, as the untagged fallback round-trips). `ConversationRecord` gains optional `phase`, `turn`, `at_ms`, `links`. `AgentConversationResult` and `ConversationPage` gain `turns` (skipped on the wire when empty). `AgentConversationParams` gains optional `source`. `ConversationRecord::new` takes `Vec<RecordBlock>`. `ErrorData` gains `unavailable` and `BoundaryError::with_unavailable`. `DomainErrorKind::InvalidRecords` and `ids::CallId` are new.
- `ovis-terminal-agent`: `AgentStatusEvent::RecordsAppended` is a new variant. T36 `binding::Read` gains crate-private `continued()`/`limit()` so both sources share one paging rule.
- `schema/ovis-terminal.openrpc.json` is regenerated. `openrpc compat schema/released.openrpc.json` returns `{"compatible_with":"schema/released.openrpc.json"}` (exit 0). `released.openrpc.json` is untouched, so no BASELINE-AMENDMENTS entry is needed.

## QA

- `cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets`: exit 0.
- `cargo clippy -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0.
- `bash scripts/check.sh`: exit 0, with nextest `1048 tests run: 1048 passed (1 slow), 1 skipped` and hygiene `0 violation(s)` (`docs/evidence/task-37/check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0 (`windows.log`).
- Lane tests (`lane-tests.log`): protocol 8, agent 13, server dispatch `conversation_records` 2 and `session_binding` 2, all passing.

What the tests assert:
- protocol: mixed `[text, tool_result, text]` blocks decode in order; an unknown role, an unknown phase, a numeric `at_ms`, an unknown block type and an unknown block field are each refused at their exact serde path; `source: terminal_screen` is refused at `source`; a provider block round-trips verbatim; `unavailable` error data round-trips; an open turn serializes every unknown fact as `null`; `u64::MAX` `turn`/`at_ms` round-trip as exact decimal strings; a proptest round-trips records and turns for any u64.
- agent (real Store, Coordinator, ops ledger, StatusService): the committed `agent.records_appended` event equals the append result and every read (ids, parents, links, phases, turn stamps); idle-time records carry no turn; a later append links to an earlier call; turn end outcome and time come from the ending report (`completed`/5000, `failed`/2000); end facts overwritten before any append stay null; a dangling result, duplicate record or call id, a role/block mismatch, a phase on a user record, empty blocks, an empty append and turn metadata while idle are each refused at their field and append nothing; a foreign token and an unregistered program are `not_owner`; a plain program gets `no_agent` and a silent agent gets `not_reported` with provenance; commentary in an open turn has no outcome, end or final phase; a proptest shows any split of the conversation into appends reads back identical records and turns; pages of 2 reassemble the log, and an older cursor is refused as `history_changed{change: append}`.
- server (real socket and dispatcher): `source: terminal_screen` → `invalid_params` at `params.source`; numeric `at_ms` → `invalid_params` at `params.records[0].at_ms`; an append replayed under one op_id returns the first result and commits once; `source: records` and the default transcript source each route to their own service.

## Adversarial

- Malformed input, a dangling tool-call link: `invalid_records` at `params.records[0].blocks[0].call_id`, and the next read is still `not_reported` (conformance failure case plus agent test).
- Misleading success: commentary-only records in a working turn read `state: open`, `outcome: null`, `ended_seq: null`, `ended_ms: null`, with no synthesized `final` (conformance failure case plus agent test). A turn whose end report is no longer the status root keeps `ended_ms: null` rather than borrowing a later time.
- Screen as a source: refused at decode, so no service runs (EXCL-008/019). A plain shell is an explicit `no_agent`, not `not_found` (EXCL-010).

## Contract checklist

1. Every u64 is a decimal string (`turn`, `at_ms`, `revision`, `started_seq`/`ended_seq`, `started_ms`/`ended_ms`). A numeric value is `invalid_params` on the wire, and `u64::MAX` round-trip tests are in place.
2. `RecordsError` is a `thiserror` enum with `kind()`, `hint()` and `#[source]` on its inner errors. Display does not repeat the kind token.
3. `CallId`, `RecordId`, `HistoryId`, `AgentId`, `TurnNumber`, `HistoryRevision` and `TimestampMs` are distinct newtypes. `Option` is used only where absence is a real state.
4. Tests assert behavior: the event equals the read, typed refusal kinds and paths, a proptest over every append split, and decimal round trips.
5. No `#[allow]`, no placeholder TODOs. Holder wire generation stays 2.
6. This PR states what the tests assert. The conformance scenario is executable, with nothing deferred.

## DEFERRED VERIFICATION

None: the runner (todo 21) has landed and `agent/records` runs as an executable scenario.

## Risks

- The record log is one SQLite resource per agent, rewritten on every append and bounded at 8 MiB/10,000 records; long sessions hit `too_large` instead of growing without limit.
- Turn end facts survive only if an append or read happens while the ending report is still the status root; otherwise they are null by design (reported as unknown, never estimated).
- `origin/integration` moved to b96f583 after branching (EXECUTABLE inventory became a slice); per the lane rule this branch does not merge it, and the orchestrator merges.
