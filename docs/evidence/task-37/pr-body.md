feat(conversation-records): expose typed conversation blocks and turn records (plan todo 37)

Closes GAP-098, GAP-100, GAP-103, GAP-105 and the contract side of GAP-104 (provider parsers stay Ovis-owned per the 2026-10-31 scope decision, EXCL-028/029). Delivers IDEAL-073, IDEAL-074, IDEAL-077, IDEAL-079. Enforces EXCL-008, EXCL-010, EXCL-019 and avoids V-4388 (closed enums, not free strings), V-4393 (no clock read in the turn fold; times come from boundary-stamped status reports), V-005 (typed `RecordsError` with kind/hint/source).

## Before

`absent`: no comparable baseline capability existed, and this is not an executed failing test. Gap baseline: GAP-098, GAP-100, GAP-103, GAP-105 (`docs/design/gap-list.md`). On integration 804e508, `agent.conversation` served only T36 transcript lines with untyped `blocks: Value[]`, did not accept `source`, and had no typed record ingestion, no tool-call linkage and no turn records.

## Round 2 (gate review B1, N2, N3)

- **B1, unreachable bounds.** Round 1 stored the whole agent log as one store value, capped at 1 MiB, and echoed every record body in the operation receipt, capped at 16 KiB. Neither cap is raised here. Instead:
  - Each record is now its own store entry. Small index entries record identities (`conversation_record_id`), calls (`conversation_call`) and turns (`conversation_turn`). The head holds the history id, revision, count, bytes and last id. No entry grows with the history.
  - `agent.append_records` returns, and publishes as `agent.records_appended`, identities, `first_position`, `total_records`, `total_bytes` and a `sha256:` digest of the appended records' stored encodings. It never returns record bodies. A full append (64 records with 128-byte ids, 256-byte model and reasoning labels) is tested to stay under 16 KiB.
  - Record bodies are read through the new `agent.records`. It is an unrecorded READ route (todo-50 precedent): answered from one committed snapshot, with no receipt and no event. Pages are bounded by count (default 50, max 200) and by about 4 MiB of record bytes.
  - `agent.conversation` stays T36's recorded transcript route, unchanged. T36 transcript reads must persist their observation to keep revisions honest, so that route cannot become a READ. It takes no `source`: `{"source":"terminal_screen"}` is refused as `invalid_params` at `params.source` by the closed schema before any service runs.
  - Bounds are the lane's own counters, each refused with a typed settled error at the bound:
    - 64 KiB per text, tool input or output (`invalid_records` at the block path)
    - 256 KiB per stored record, which is the default params bound of one request (`invalid_records` at `params.records[i]`)
    - 128-byte record ids, and 256-byte call ids, names and turn labels
    - 64 records per append
    - 10,000 records and 8 MiB per agent (`too_large`)
- **N2.** Only an append persists turn facts; reads compute turn views from the stored facts and the live status row without persisting anything. End facts survive only if an append happens while the ending report is still the status root. Otherwise they are null, never estimated.
- **N3.** The protocol fixture does not serve `events.subscribe`, and the product dispatcher does not serve it yet (assembly is todo 31), so I did not add a stand-in. The server test `conversation_records::a_replayed_append_commits_once_and_publishes_its_receipt_as_the_event` sends a real append over the socket, subscribes to the Coordinator before sending, and asserts that the committed `agent.records_appended` payload equals the wire `result` byte for byte, and that a replay under the same op_id returns the same result. The agent-level test asserts event == receipt and that the receipt's ids and digest match what `agent.records` returns.

## After

Native host: `Linux x86_64`. Binary `target/debug/ovis-terminal` sha256 `77533366ece7990fc4f37a449b80a1a0232258c4faa22e856ecc2407747822c2` (round 2 tree; recorded in the evidence JSON).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal            # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/records --isolated --case happy --evidence "$ATTEMPT_DIR/task-37-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"agent/records","totals":{"assertions":"87","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/records --isolated --case failure --inject '{"method":"agent.conversation","params":{"source":"terminal_screen"}}' --evidence "$ATTEMPT_DIR/task-37-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"agent/records","totals":{"assertions":"44","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run ... --scenario agent/records --isolated --case all --evidence "$ATTEMPT_DIR/task-37-all.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"130","cases":"2","failed":"0","passed":"2","skipped":"0"}}
exit 0
```

Materialized commands: the runner starts `target/debug/conformance fixture --root /tmp/ovc-<pid>-0` (the real server crate: listener, connection, Dispatcher, SQLite Coordinator, Authority on a private Unix socket) and drives it through `target/debug/ovis-terminal connect` as the bridge. Raw replies from the failure-case wire transcript:

```
-> {"id":"inject-0","method":"agent.conversation","params":{"target":{"name":"agent"},"source":"terminal_screen"}}
<- {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32602,"message":"method parameters violate the closed schema","data":{"kind":"invalid_params","hint":"correct the method parameters","retryable":false,"field_path":"params.source"}}}
<- {"jsonrpc":"2.0","id":"unintegrated","error":{"code":-32000,"message":"operation failed","data":{"kind":"conversation_unavailable",...,"unavailable":{"reason":"not_reported","provenance":{"source":"records","agent_id":"agent:01a19564-62e1-72e0-b70d-9a51497e352e","session":{"source":"provider-x","kind":"id","value":"session-77"}}}}}}
<- {"jsonrpc":"2.0","id":"plain","error":{"code":-32000,"message":"operation failed","data":{"kind":"conversation_unavailable",...,"unavailable":{"reason":"no_agent","provenance":{"source":"records","agent_id":null,"session":null}}}}}
```

Happy observables: the append receipt carries `record_ids [u1,a1,u2,a2]`, `first_position "0"`, `total_records "4"`, `total_bytes "765"` and a `sha256:` digest, with no record bodies. A 20 KiB tool result (`big`) is appended (`total_records "6"`) and read back whole through `agent.records`. The unrecorded `agent.records` read returns: records `u1, a1, u2, a2` in order; `u2` carries `[text, tool_result]` in provider order with `links[0] = {call_id: "c1", call_record: "a1"}` and `parent: "a1"`; `a1.phase = commentary`, `a2.phase = final`, `u1` has no phase; every record has `turn: "1"`. After `status.report idle`: `turns[0] = {state: "ended", outcome: "completed", ended_seq: "16", ended_ms: "1793507740307", model: "model-a", reasoning: null}`. The `unintegrated` and `plain` replies above come from `agent.records`; the provenance `source` is `records`. The evidence JSON holds 131 checks, all passed.

Raw evidence: `docs/evidence/task-37/task-37-ovis-terminal-v1.json` (happy and failure runs, argv, wire transcripts, every assertion's expected/actual), plus `happy.*`, `failure.*`, `all.*` stdout/stderr/exit companions.

## What changed

- **protocol** `domain::conversation_records`: `ConversationSource {transcript, records}`, `RecordRole {user, assistant, tool}`, `MessagePhase {commentary, final}`, `ContentBlock` (tagged `text | tool_call | tool_result`, closed fields), `RecordBlock` (typed, or a T36 transcript block kept verbatim), `ToolLink`, `RecordInput`, `TurnMetadata`, `AppendRecordsParams`, `ConversationTurn` + `TurnState {open, ended}`, `RecordsAppended` (one type for the event payload and the append result), `Unavailable {reason: no_agent|not_reported, provenance {source, agent_id, session}}`. New method `agent.append_records` (Mutation, owner token), new event `agent.records_appended`, new domain error kind `invalid_records`, new id `CallId`.
- **agent** `conversation::records`: `RecordsService::append` validates the batch against the stored entries (unique record and call ids through the identity and call index entries, every tool result answers a recorded call, role/block/phase rules, and the bounds listed under Round 2). It stamps each record with the status turn only while the status root is `working`, and commits record entries, index entries, turn facts, head and event in one unit. `RecordsService::read` serves pages from one snapshot with history-bound cursors and writes nothing. A turn's outcome, `ended_seq` and boundary times come only from the T33 status row: `started_ms` and `ended_ms` are the acceptance times of the reports that started and ended the turn, recorded only while those reports are the status root and otherwise left null.
- **server** `lanes::conversation_records`: `AppendRecords` (recorded mutation) and `Records` (`agent.records`, registered with `register_read`: unrecorded, Read-scope authz). `agent.conversation` stays in `lanes::session_binding`, byte-identical to integration.
- **conformance**: executable `agent/records` scenario (happy, failure with a planned inject); the fixture registers the new lane.
- **workflow**: `conversation-records` option and job at the end of `probe-stress-macos.yml` (25 iterations of the lane tests and the scenario).

## Seams (public API changes in landed crates)

- `ovis-terminal-server`: round 2 restores `lanes/session_binding.rs` and `lanes/session_binding/qa.rs` to their integration bytes, so `agent.conversation` is T36's route again. `conformance/src/fixture/transcripts.rs` registers the new lane (`agent.append_records`, read route `agent.records`) beside T36's.
- `ovis-terminal-protocol`: `ConversationRecord.blocks` is `Vec<RecordBlock>` (was `Vec<Value>`; wire bytes are unchanged for transcript blocks, as the untagged fallback round-trips). `ConversationRecord` gains optional `phase`, `turn`, `at_ms`, `links`. `AgentConversationResult` and `ConversationPage` gain `turns` (skipped on the wire when empty). `AgentConversationParams` is unchanged (no `source`). New `AgentRecordsParams` and method `agent.records`. `ConversationRecord::new` takes `Vec<RecordBlock>`. `ErrorData` gains `unavailable` and `BoundaryError::with_unavailable`. `DomainErrorKind::InvalidRecords` and `ids::CallId` are new.
- Authorized merge of `origin/integration` 7f6d268, made because PR #62 turned CONFLICTING after the round-2 push. The conflicts were append-only unions, and both sides were kept:
  - `domain.rs` and `lanes.rs` mod lines, and the `tests/dispatch.rs` qa modules.
  - `ErrorData.unavailable` beside `principal_epoch`, plus its error-catalog schema entry.
  - In `probe-stress-macos.yml`, the `conversation-records` option and job are appended last.
  - `schema/ovis-terminal.openrpc.json` is regenerated with `openrpc write`, and `openrpc compat schema/released.openrpc.json` returns `compatible_with` with exit 0.
- `packages/client-ts/src/generated` is regenerated by `bun run generate` (never hand-edited), because the new `agent.append_records`/`agent.records` methods, the `agent.records_appended` event and `invalid_records` change the schema. `bun run generate:check && bun run typecheck && bun test` gives 30 pass, 0 fail.
- `ovis-terminal-agent`: `AgentStatusEvent::RecordsAppended` is a new variant. T36 `binding::Read` gains crate-private `continued()`/`limit()` so both sources share one paging rule.
- `schema/ovis-terminal.openrpc.json` is regenerated. `openrpc compat schema/released.openrpc.json` returns `{"compatible_with":"schema/released.openrpc.json"}` (exit 0). `released.openrpc.json` is untouched, so no BASELINE-AMENDMENTS entry is needed.

## QA

- `cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets`: exit 0.
- `cargo clippy -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0.
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, with nextest `1070 tests run: 1070 passed (1 slow), 1 skipped`, hygiene `0 violation(s) in 1055 file(s) checked`, and `check.sh: all steps passed` (`docs/evidence/task-37/check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`: both exit 0 (`windows.log`).
- Lane tests (`lane-tests.log`): 438 passed and 0 failed across the three crates. Lane-relevant tests: protocol 8; agent `conversation::records` 18, including 5 bound tests (`qa::bounds`); server `conversation_records` 3, including the 20 KiB socket test and the receipt-equals-event test.

What the tests assert:
- protocol: mixed `[text, tool_result, text]` blocks decode in order; an unknown role, an unknown phase, a numeric `at_ms`, an unknown block type and an unknown block field are each refused at their exact serde path; `source: terminal_screen` is refused at `source`; a provider block round-trips verbatim; `unavailable` error data round-trips; an open turn serializes every unknown fact as `null`; `u64::MAX` `turn`/`at_ms` round-trip as exact decimal strings; a proptest round-trips records and turns for any u64.
- agent (real Store, Coordinator, ops ledger, StatusService): the committed `agent.records_appended` event equals the append result and every read (ids, parents, links, phases, turn stamps); idle-time records carry no turn; a later append links to an earlier call; turn end outcome and time come from the ending report (`completed`/5000, `failed`/2000); end facts overwritten before any append stay null; a dangling result, duplicate record or call id, a role/block mismatch, a phase on a user record, empty blocks, an empty append and turn metadata while idle are each refused at their field and append nothing; a foreign token and an unregistered program are `not_owner`; a plain program gets `no_agent` and a silent agent gets `not_reported` with provenance; commentary in an open turn has no outcome, end or final phase; a proptest shows any split of the conversation into appends reads back identical records and turns; pages of 2 reassemble the log, and an older cursor is refused as `history_changed{change: append}`.
- server (real socket and dispatcher): `source: terminal_screen` → `invalid_params` at `params.source`; numeric `at_ms` → `invalid_params` at `params.records[0].at_ms`; `agent.records` with `source: terminal_screen` is also `invalid_params` at `params.source`; an unreported agent read through `agent.records` is `conversation_unavailable`/`not_reported` and carries no `op_id` (unrecorded READ); an append replayed under one op_id returns the first result and commits once, and the `agent.records_appended` event the Coordinator subscriber receives (subscribed before the request was sent) equals the wire `result`; a 20 KiB tool result is appended and `agent.records` returns it whole with its call link.

Merged tree (after the authorized integration merge, local, Linux x86_64):
- `conformance ... --scenario agent/records --isolated --case all`: exit 0, 130 assertions, failed 0, skipped 0 (`merged-all.json`).
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: exit 0, `1197 tests run: 1197 passed (1 slow), 1 skipped`, hygiene `0 violation(s) in 1179 file(s) checked` (`merged-check.log`).
- Windows `cargo check` and `cargo clippy` for `x86_64-pc-windows-gnu` with `-D warnings`: both exit 0 (`merged-windows.log`).
- macOS stress dispatch (round 2 head 50c2269): https://github.com/devswha/ovis-terminal/actions/runs/41274503920

## Adversarial

- Malformed input, a dangling tool-call link: `invalid_records` at `params.records[0].blocks[0].call_id`, and the next read is still `not_reported` (conformance failure case plus agent test).
- Misleading success: commentary-only records in a working turn read `state: open`, `outcome: null`, `ended_seq: null`, `ended_ms: null`, with no synthesized `final` (conformance failure case plus agent test). A turn whose end report is no longer the status root keeps `ended_ms: null` rather than borrowing a later time.
- Round 2 mutants (`docs/evidence/task-37/mutants-r2.log`). Each was applied alone, ran `cargo test -p ovis-terminal-agent --lib conversation::records`, and was restored with a sha256-checked original:
  - M1 `count > RECORD_LIMIT + 1`: exit 101, killed by `the_10000th_record_is_accepted_and_the_10001st_is_refused_as_too_large`.
  - M2 text bound `> TEXT_LIMIT + 1`: exit 101, killed by `a_64_kib_text_is_accepted_and_one_byte_more_is_refused_at_its_path`.
  - M3 `bytes > BYTE_LIMIT + 300 KiB`: exit 101, killed by `records_beyond_8_mib_are_refused_as_too_large_with_counters_unchanged`.
  - M4 digest ignores record bytes: exit 101, killed by the 64 KiB bound test and `mixed_records_link_results_to_calls_and_the_event_matches_every_read`.
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

- Each agent's records are bounded at 10,000 records and 8 MiB (one store entry per record, so no single value grows). Long sessions hit a typed `too_large` instead of growing without limit; there is no rotation yet.
- Turn end facts are persisted only by an append made while the ending report is still the status root. Reads compute views and persist nothing, so otherwise the facts are null by design (reported as unknown, never estimated).
- Records read through `agent.records`; `agent.conversation` remains the T36 transcript route. Clients that expected round 1's `agent.conversation {source: records}` must switch.
- `origin/integration` moved to b96f583 after branching (EXECUTABLE inventory became a slice); per the lane rule this branch does not merge it, and the orchestrator merges.
