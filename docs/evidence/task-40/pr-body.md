## Summary

Todo 40 (closes GAP-085; delivers IDEAL-062, IDEAL-105, IDEAL-106). A program whose prompt widget takes no terminal keys (pi-ask style) opens it with `mechanism: extension`. Each accepted answer reaches the program's in-process extension once, as a typed `ExtensionEvent`, and the extension acknowledges it at the version the answer committed. No terminal byte is written.

This adds no second prompt state machine. Versions, answers and receipts stay with todo 34's `PromptService`, and every event and delivery record is derived from the committed prompt record and its answer receipt (`ExtensionEvent::from_answered`).

- `crates/ovis-terminal-protocol/src/domain/prompt_bridge.rs`: `ExtensionEvent`, `DeliveryState` (`pending|delivered|acknowledged|outcome_unknown`), `DeliveryRecord`, `BridgeAttachment`, and `ProgramCredential` (the credential view read before the closed decoder). Every counter is a decimal-string newtype.
- `crates/ovis-terminal-agent/src/bridge/`: `PromptBridge` with:
  - process-token-authenticated `attach`, which mints a `BridgeToken` session capability (digest-only storage, redacted `Debug`; one session per program incarnation; re-attach supersedes);
  - `next`, delivering in closing-sequence order behind a per-program watermark, so an answer is handed out at most once, within a bounded queue of `NonZeroU8` unacknowledged events that answers `limit_exceeded` with `bound`;
  - `ack`, tied to the delivered version and rechecked against the committed receipt;
  - `detach`, after which unacknowledged deliveries become `outcome_unknown` and are never redelivered;
  - `delivery` reads.
  
  Errors use one thiserror enum, `BridgeError`, with `kind()` / `hint()` / `error_data()`.
- `crates/ovis-terminal-server/src/dispatch/lanes/prompt_bridge.rs`: `PromptBridgeGate` plus the `Gated<H, T>` `RequestHandler` decorator. For `prompt.open` / `prompt.close`, the gate reads only the credential fields. It refuses as `forbidden` a token naming no current program, or a target or routed prompt on another program's terminal, before the closed parameter type is decoded or any operation is admitted. Everything else passes unchanged. The prompt lane still rechecks the token inside its commit unit.
- `conformance/scenarios/agent/bridge.json` (executable) plus the fixture extension `conformance/src/fixture/bridge.rs`, driven by the unadvertised fixture-only `fixture.bridge` method (same pattern as `fixture.pty`).

## Before

`absent`. There was no comparable baseline capability, and this is not an executed failing test. Gap baseline: GAP-085. Before this PR, an extension-mechanism answer committed with `delivery: extension` and nothing ever delivered it. A `prompt.open` presenting a foreign token with a partial prompt was refused as `invalid_params` by the strict decoder, which leaked the request shape, instead of `forbidden`.

## After

Native OS/arch: `Linux 7.0.0-38-generic x86_64`. Binary: `target/debug/ovis-terminal`, sha256 `b66a3c7555c533047eafb14b85e4b94146a3a1cc05cd50b5a4755cc503da4f8b` (151978440 bytes; also recorded in the evidence `binary` field).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                       # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case all --evidence "$ATTEMPT_DIR/task-40-all.json"
{"case":"all",...,"result":"passed","scenario":"agent/bridge","totals":{"assertions":"167","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
ALL_EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case happy --evidence "$ATTEMPT_DIR/task-40-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","scenario":"agent/bridge","totals":{"assertions":"54","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
HAPPY_EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/bridge --isolated --case failure --inject '{"method":"prompt.open","params":{"process_token":"other-process","prompt":{"kind":"form","mechanism":"extension"}}}' --evidence "$ATTEMPT_DIR/task-40-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","scenario":"agent/bridge","totals":{"assertions":"111","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
FAILURE_EXIT=0
$ cargo run ... --scenario all/all --isolated --case all --evidence "$ATTEMPT_DIR/task-40-suite.json"
{"case":"all",...,"result":"passed","scenario":"all/all","totals":{"assertions":"1169","cases":"32","declared_only":"33","failed":"0","passed":"32","skipped":"0"}}
SUITE_EXIT=0
```

The happy and failure runs used `--manual-transcript`. The materialized wire traffic, sent through the real `ovis-terminal` binary acting as the bridge client to the real server crate fixture on a private Unix socket, is in `happy.stderr` / `failure.stderr`. Decisive lines (op ids elided):

Happy. The fixture extension opens a form, the answer receives an in-process ack, and the byte-counting PTY records zero injected keys:
```
<< {"id":"blocked-send","error":{..."data":{"kind":"blocked",...,"prompt_ids":["form-1"]}}}
<< {"id":"pty-open","result":{"bytes":"0"}}
<< {"id":"answer","result":{...,"answer":{"answer":{"fields":{"branch":"main"}},"answered_seq":"11","delivery":"extension",...},"mechanism":"extension","state":"answered","version":"2"}}
<< {"id":"take","result":{"acked":[{...,"prompt_id":"form-1","state":"acknowledged","version":"2"}],"received":[{"answer":{"fields":{"branch":"main"}},...,"kind":"form","process_incarnation":"1","prompt_id":"form-1","version":"2"}]}}
<< {"id":"state","result":{"delivery":{...,"state":"acknowledged","version":"2"},"received":"1"}}
<< {"id":"receipt","result":{...,"method":"prompt.answer",...,"state":"committed"}}
<< {"id":"pty-after","result":{"bytes":"0"}}
```

Failure. The binary returns forbidden, and the owner's prompt and conversation are unchanged:
```
>> {"id":"inject-0","method":"prompt.open","params":{"op_id":"...","process_token":"other-process","prompt":{"kind":"form","mechanism":"extension"},"prompt_id":"form-1","target":{"name":"agent"}}}
<< {"id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"forbidden","hint":"present the token handed to the terminal's current program, or attach again","retryable":false}}}
<< {"id":"owner-prompt","result":{"prompts":[{...,"prompt_id":"form-1","session":{"kind":"path","source":"fixture-store","value":"one.jsonl"},"state":"open","text":"Release details","version":"1"}]}}
conversation-before == conversation-after: history h1, revision "1", records r1,r2 of total 3
```

## QA

- Happy: the scenario asserts the extension `attach` (terminal `terminal:4`, incarnation `"1"`, `queue_limit` `"2"`), and the extension form opened at v1. While it is open, the ordinary `input.send` is `blocked` naming `form-1`. The answer with declared fields commits v2 with `delivery: extension`. The extension receives exactly one event carrying v2, the answer, the kind and the answer's op id, and acks it at v2. The delivery reads `acknowledged`, received `"1"`. `op.get` on the answer is `committed`. `fixture.pty` reads `"0"` before and after.
- Failure (`--inject`, the plan's exact payload): `forbidden`, with `/result` absent. The scenario then asserts that `prompt.list` still shows only `form-1` at v1, open, with the same text, incarnation and session, and that `agent.conversation` returns the same history, revision and records before and after the injection. Without `--inject`, the planned `prompt.close` with `other-process` is also asserted `forbidden`. A forged `fixture.bridge` attach with `other-process` is `forbidden`.

## Adversarial

- **Malformed input (answer outside schema).** The answer `{"fields":{"undeclared":"x"}}` gets `answer_invalid` at `params.answer.fields`, and `{"text":"main"}` on the form gets `answer_invalid` at `params.answer`. The extension then receives nothing and the prompt is still open at v1 (scenario, failure case). Agent test: `only_the_programs_own_extension_answers_reach_its_session`.
- **Ack tied to version.** An ack naming v1 of an answer committed at v2 gets `version_conflict` with `current_version` `"2"` (scenario). In the agent test, a second ack gets `not_delivered`.
- **Cancel/resume (bridge drops before ack).** The extension takes the event without acking and then drops its session, so the delivery reads `outcome_unknown`. A late ack with the dropped session is `forbidden`. `op.get` on the answer still settles `committed`. Retrying `prompt.answer` with the same op id returns the retained receipt (v2, same op id). A resumed session receives `[]`, so the extension's receive count stays `"1"` (no double delivery), and the PTY stays at `"0"`. Agent test: `a_session_dropped_before_its_ack_is_never_redelivered`, which also covers supersession by re-attach and the `pending` state before any attach.
- **Program replacement.** Rebinding the terminal to a new incarnation ends the old session (`forbidden`). The retired token cannot attach, and the successor attaches at incarnation 2 (`a_session_needs_the_token_of_a_current_program`).
- **Bounded queue.** With a limit of 2 and three answers given in the order e3, e1, e2, `next` returns `[e3, e1]` in answer order. The next call is `limit_exceeded` with bound value `"2"`, and after one ack `next` returns `[e2]` (`the_queue_holds_at_most_its_limit_in_answer_order`).

## Tests (exactly what they assert)

- `cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets`: exit 0, 318 passed, 0 failed (`test-results.txt`). Lane-relevant: 12 (`bridge-tests.txt`).
  - Protocol (4): only an answered extension-mechanism record with an extension receipt yields an event (typed, rpc, dismissed and open records yield none). The credential view decodes from the plan's injected payload and from `prompt.close` params, and refuses tokenless or numeric tokens. `u64::MAX` versions round-trip exactly as decimal strings, and a numeric `version` is refused. A proptest round-trips arbitrary version, sequence and incarnation counters as decimal strings.
  - Agent (5, over a real SQLite store, coordinator, operation ledger and `PromptService`): the five behaviors listed under Adversarial plus the happy delivery, and acknowledgement at the version.
  - Server (3): the gate refuses `other-process` on `prompt.open` and `prompt.close` with `forbidden` and the bridge hint; refuses a shell token targeting the agent terminal and passes the agent's own token; passes ungated methods, tokenless requests and numeric tokens through to the strict decoder.
- `cargo clippy -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings`: exit 0.
- `cargo test --locked --manifest-path conformance/Cargo.toml`: exit 0 (the suite inventory lists `agent/bridge.json` as executable, count 17).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0.
- `bash scripts/check.sh`: exit 0 (`check.log`, "check.sh: all steps passed"). The first gate pass in `gates.out` shows `check exit=101` from a clippy `arithmetic_side_effects` finding in the fixture counter. That was fixed with `saturating_add`, and check.sh was re-run to exit 0.

## Contract checklist

1. Every counter on the wire is a decimal-string newtype (`RecordVersion`, `EventSeq`, `Incarnation`, `Count`). Numeric forms are refused, and the `u64::MAX` round trip is tested. The fixture's received count is also a decimal string.
2. Boundary errors use `BridgeError`, a `#[derive(thiserror::Error, Debug)]` enum with `kind()`, `hint()` and `#[source]` on the `PromptError` / `StatusError` wrappers. `Display` never repeats the kind. There are no hand-written Error impls and no string errors (V-005 not reproduced).
3. Identities are distinct newtypes: `BridgeToken` (redacted `Debug`, digest-only storage), `ProcessToken`, `PromptId`, `OpId`, `RecordVersion`. `Option` is used only where absence is real.
4. Tests assert behavior: state transitions, typed kinds, and proptest round trips. No locally assigned literal is read back.
5. No `#[allow]` / `#[expect]` and no placeholder TODOs. Docs are public API rustdoc only.
6. The plan's QA commands were actually executed (not deferred). Results are above, with raw evidence in `docs/evidence/task-40/`. Holder wire generation stays 2.

Exclusions enforced:
- EXCL-023: the extension form blocks ordinary `input.send` (`blocked`, zero bytes), and the bridge offers no force path.
- EXCL-017: nothing is discarded silently. A full queue is a typed `limit_exceeded` with the answer still pending, and a dropped session is a typed `outcome_unknown`.
- V-3908: each bridge request resolves its session and acts under one lock acquisition, and the session is re-validated against the current program inside that acquisition (no resolve, release, relock).

## Seams

- New public modules: `ovis_terminal_protocol::domain::prompt_bridge`, `ovis_terminal_agent::bridge`, `ovis_terminal_server::dispatch::lanes::prompt_bridge` (plus `mod` slot lines in `domain.rs`, `lib.rs` and `lanes.rs`). No landed public API changed, and no catalog, OpenRPC or holder wire change.
- Production assembly must wrap the server's `RequestHandler` in `Gated::new(PromptBridgeGate::new(..), handler)` and construct `PromptBridge` beside `PromptService`. That is todo 42 (blocked by this todo); only the conformance fixture does it today.
- Files outside the literal write scope, all conformance seams:
  - `conformance/src/fixture/bridge.rs` (new fixture extension);
  - `conformance/src/fixture/{handler.rs,serve.rs}` (`Probes` struct replaces the `(gate, directory)` tuple; the handler is served behind `Gated`);
  - `conformance/src/fixture.rs` (mod line);
  - `conformance/src/runner_tests/suite.rs` (`EXECUTABLE` 16 to 17, sorted).
  
  The scenario file is `conformance/scenarios/agent/bridge.json`, because the runner maps `agent/bridge` to that path rather than to the `agent/bridge/` directory named in the scope.
- Test child modules: `domain/prompt_bridge/tests.rs` and `lanes/prompt_bridge/qa.rs`.

## Risks

- The extension's wire surface is fixture-only. The catalog has no `prompt.bridge.*` method, so an out-of-process extension (a real pi extension) cannot attach over RPC yet. Delivery and ack are an in-process Rust API (`PromptBridge`), and the fixture drives it through the unadvertised `fixture.bridge` method. Adding catalog methods needs a schema/OpenRPC change outside this write scope.
- Delivery state (watermark, in-flight set, the last 64 settled deliveries per program) is in memory. After a control-server restart, retained extension answers of a still-running incarnation could be offered again. Persisting the watermark belongs with restart work (todo 48/49).
- The gate refuses foreign tokens as `forbidden` before decoding. A token that becomes stale between the gate and the commit unit still gets todo 34's `not_owner` from the commit unit.
- macOS stress: `.github/workflows/probe-stress-macos.yml` is not valid YAML at the base or at current `origin/integration`: `yaml.scanner.ScannerError ... line 821, column 13` (a duplicated `options:` line and a malformed `prompts` job from a union). It cannot be dispatched, and fixing it is outside this scope. The job block for the union is in `docs/evidence/task-40/stress-job.yml` (option `prompt-bridge`), and no run URL exists.
- `origin/integration` moved to 15432bb (T28 WebSocket) after the lane base 0d3022c. None of this PR's files changed upstream. I did not merge (lane rule); the orchestrator merges.
