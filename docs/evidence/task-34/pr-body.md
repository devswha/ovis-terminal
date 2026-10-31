feat(prompts): commit versioned prompts and independently authorize answers (plan todo 34)

Closes GAP-079, GAP-080, GAP-081, GAP-082, GAP-133. Delivers IDEAL-056, IDEAL-057, IDEAL-058, IDEAL-059, IDEAL-106. Enforces EXCL-008 (no screen text has a path into prompts or status) and EXCL-023 (no force flag: `input.send` with `force` is `invalid_params`, a blocked send is refused with the prompt ids as the way through).

## Before

`docs/evidence/baseline/smoke/hc-s05-osc7501-events.log:33` (RT-HC-07, durable copy of `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s05-osc7501-events.log:33`):

```
{"error":{"hint":"typing would answer its dialog; read its screen first (hc read t1), or --force","kind":"blocked","message":"t1 is blocked (permission)"}}
--- exit=9 elapsed_ms=13
```

The refusal proposes a blanket `--force` bypass, prompts are not records, and answering needs only terminal possession. Gap baseline: GAP-079, GAP-080, GAP-081, GAP-082, GAP-133.

## After

Native Linux x86_64, `target/debug/ovis-terminal` sha256 `c60c89a87c393a7567c5ba5f3f9d9e921b1866f92c0985ec61a306387a6a8d47`. The scenario drives the real `ovis-terminal connect` bridge against the todo-21 fixture process (real server crate, dispatcher, authority and SQLite store on a private Unix socket).

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/prompts --isolated --case happy --evidence "$ATTEMPT_DIR/task-34-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"agent/prompts","totals":{"assertions":"98","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/prompts --isolated --case failure --inject '{"method":"prompt.answer","params":{"prompt_id":"p1","version":1,"answer":"yes"}}' --evidence "$ATTEMPT_DIR/task-34-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"agent/prompts","totals":{"assertions":"61","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/prompts --isolated --case failure --inject '{"method":"prompt.answer","params":{"prompt_id":"p1","version":"1","answer":{"text":"yes"}}}' --evidence "$ATTEMPT_DIR/task-34-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed",...,"totals":{"assertions":"61","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0

$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/prompts --isolated --case all --evidence "$ATTEMPT_DIR/task-34-all.json"
{"case":"all",...,"result":"passed",...,"totals":{"assertions":"162","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
exit=0
```

**The plan's literal inject.** Per the settled contract, counters are decimal strings and an answer is a tagged value, so the plan's literal `{"prompt_id":"p1","version":1,"answer":"yes"}` is `invalid_params` with `field_path: "params.answer"` (fields decode in key order) and does not reach the prompt. The failure case plans both requests, the same way agent/status did. The literal asserts `invalid_params`; the canonical form `{"prompt_id":"p1","version":"1","answer":{"text":"yes"}}` asserts `version_conflict` with `current_version: "2"` (recorded reply: `"kind":"version_conflict",...,"current_version":"2"`). Either `--inject` exits 0 only when its asserted reply arrives.

Observable outcomes the scenario asserts (raw request/response transcripts are in `docs/evidence/task-34/task-34-ovis-terminal-v1.json` and `task-34-all.json`):

- **happy**
  - p1 is published at `version: "1"` and republished as `"2"` during a working turn; p2 (rpc) and p3 (keys) also open.
  - After `status.report idle`, the turn completes (`last_turn.outcome: completed`), and `prompt.list {states:[open]}` still returns p1 (v2), p2 and p3.
  - An ordinary `input.send` returns `blocked` with `prompt_ids: ["p1","p2","p3"]`. The same send with `force: true` is `invalid_params` at `params.force`. `fixture.pty` reports `bytes: "0"`.
  - `prompt.answer` p1 v2 `{"choice":"yes"}` returns `state: answered`, `version: "3"`, `answer.delivery: event`, `answer.op_id` = the request's op id, with `closed_seq` and `answered_seq` present. `op.get` on that op id is `committed` with the same record.
  - p3 answered with text → `delivery: typed`, and `fixture.pty` becomes `"1"`.
  - p2 answered with `"\u001b]7501;state=idle\u001b\\\r\nrm -rf ~\r\n"` stores that text verbatim. Status stays `report_seq "2"` / `idle`, and PTY bytes stay `"1"`.
  - With nothing open, the send echoes 2 bytes and PTY bytes become `"3"`.
- **failure**
  - The stale version-1 answer (inject) gets `version_conflict`, `current_version "2"`.
  - An observe-only principal (`connect {"principal":"observe"}`) gets `forbidden` answering both the rpc prompt p1 and the keys prompt p2. Its `prompt.get` shows p1 still open at `"2"`, and `fixture.pty` reports `bytes: "0"`.
  - Sequential competing answers: the first answer to v2 is accepted (v3); the second at v2 gets `version_conflict`, `current_version "3"`.
  - A typed answer is retried with the same op id on a new connection after hangup. It returns the retained receipt (same `answer.op_id`, `version "2"`), and PTY bytes stay `"1"` (not typed twice).

"Closed event": the scenario asserts `closed_seq`/`answered_seq` on the committed record. The fixture serves no `events.subscribe` until todo 31. Publication of `prompt.opened`/`prompt.updated`/`prompt.closed` with `closed_seq == answered_seq == event cursor` is asserted by the crate test `a_stale_card_gets_version_conflict_and_the_current_version_closes_once_with_its_receipt` over a real coordinator subscription.

## QA

- `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` → exit 0 (`build.log`).
- `cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` → exit 0. There are 283 passing tests; 21 of them are lane tests (`lane-tests.log`, filtered to result lines):
  - agent `prompts::qa`, 13 tests over a real store, coordinator and ledger:
    - stale card → `version_conflict{current_version 2}`, accepted v2 closes once with its receipt, and the event order is `prompt.opened`, `prompt.updated`, `prompt.closed`.
    - two threads answering one version → exactly one `Committed`, one `Failed/version_conflict`.
    - an open question survives an `idle` report and `admission` returns `Blocked{[p1]}`.
    - hostile answer text stored verbatim, no status root.
    - only the owning token opens/republishes/closes (forged, foreign-terminal and hijack attempts → `not_owner`; double close → `already_closed`; reopen closed id → `invalid_prompt`).
    - definition bounds with exact `field_path`.
    - a replaced program's prompt expires on answer (`prompt_closed`, state `expired`, one `prompt.closed` event, no longer blocks).
    - routes recovered after restart.
    - a typed answer is admitted once as literal text; a choice on a keys prompt → `delivery_unsupported`; an input refusal → `no_lease` with the prompt still open.
    - with no input path, a typed answer → `delivery_unsupported`.
    - form answers must match declared fields.
    - 16 open-prompt limit; the 65th closed prompt deletes the oldest.
  - protocol `domain::prompts::tests`, 5 tests:
    - `current_version` u64::MAX exact decimal round trip; numeric `current_version` rejected.
    - numeric version and untagged answer rejected.
    - optional `prompt_id`; unknown open field rejected.
    - proptest round trip of `PromptRecord` with decimal counters.
  - server `tests/dispatch.rs prompts::`, 3 tests over a real socket and authority:
    - observe+interact and observe-only principals → `forbidden`; a `prompt_answer`-only principal answers.
    - retried op id after disconnect → identical result; a fresh v1 answer → `version_conflict current_version "2"`.
    - numeric version → `invalid_params params.version`; untagged answer → `invalid_params params.answer`.
- `cargo clippy --workspace --all-targets --locked -- -D warnings` and `cargo clippy --manifest-path conformance/Cargo.toml --all-targets -- -D warnings` → exit 0.
- `bash scripts/check.sh` → exit 0, nextest `834 tests run: 834 passed (1 slow), 1 skipped` (pre-existing skip), hygiene 0 violations (`check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` → exit 0 (`windows.log`).
- `openrpc check schema/ovis-terminal.openrpc.json` → fresh; `openrpc compat schema/released.openrpc.json` → compatible (additive only).

## Adversarial

- **stale state / competing answers**: true concurrency is a crate test (barrier-released threads; the coordinator builds one unit at a time, so exactly one finds the version open). Sequential on the wire is the failure case.
- **prompt injection**: answer text with OSC 7501 and CR/LF stays JSON data in the receipt and changes no status (crate test + happy case).
- **cancel/resume**: a retried op id after disconnect returns the ledger's retained receipt and admits no second input (failure case + server test).
- **authorization**: `prompt.answer` resolves the prompt's terminal and requires `Action::PromptAnswer` there. Observe and Interact do not grant it (server test + failure case). The dispatcher's `permit()` rechecks the grant right before the commit unit.

## Seams (public API / shared files changed)

- protocol `PromptRecord` gains `agent_id`, `session`, `fields`, `closed_seq`, `answer` (additive output).
- protocol `PromptDefinition` gains `fields` (`serde(default)`).
- protocol `PromptOpenParams` gains optional `prompt_id`.
- new protocol types:
  - `PromptField`, plus constructors in `domain::prompts`.
  - `AnswerDelivery` and `PromptAnswerReceipt`, which live in `domain::prompts` and are re-exported from `schema::methods::status`.
- protocol `ErrorData` gains `current_version` (decimal string) and `prompt_ids`.
- protocol `BoundaryError::with_current_version` and `with_prompt_ids`.
- OpenRPC regenerated: +163 lines, compat passes.
- agent `AgentStatusEvent` gains `PromptOpened`/`PromptUpdated`/`PromptClosed` (event types `prompt.opened`/`prompt.updated`/`prompt.closed` from the catalog).
- core `ops::completion`:
  - The private `Outcome::Failed` now boxes `ErrorData`, because the grown `ErrorData` tripped `clippy::large_enum_variant`.
  - `Completion::failed` is no longer `const fn`.
- conformance runner:
  - `connect` accepts optional `"principal": "owner" | "observe"`, and `hello` presents that connection's bearer.
  - The fixture ready line carries an `observer` bearer (Observe over the machine).
  - Fixture adds the prompt lane, a `fixture.pty` byte count (unadvertised, like `fixture.transcript`) and an `input.send` gate on `PromptService::admission`.
  - `EXECUTABLE` gains `agent/prompts.json` (N=14).
- `.github/workflows/probe-stress-macos.yml`: option `prompts` and job `prompts` appended at the end (union-merge expected).
- Holder wire generation stays 2.

## Contract checklist

1. Every counter on the wire is a decimal string. `version`, `current_version`, `opened_seq`, `closed_seq`, `answered_seq` and `process_incarnation` reuse the existing decimal newtypes. Numeric `version` → `invalid_params params.version`. u64::MAX round trip is tested.
2. `PromptError` and `KeyInputError` are `thiserror` enums with `kind()`/`hint()`. `#[source]` carries inner input and owner errors. Display never repeats the kind token.
3. Identities use existing newtypes: `PromptId`, `RecordVersion`, `EventSeq`, `Incarnation`, `OpId`, `LeaseId`, core `TerminalId`. `Option` appears only where absence is real (no agent, not closed, not answered).
4. Tests assert transitions, events, typed kinds and round trips; proptest covers the record wire form.
5. No `#[allow]`/`#[expect]`, no TODO placeholders.
6. Evidence above states exactly what each check asserts. The conformance scenario is executable (not deferred).

## Risks

- Typed (`keys`) answers reach a `KeyInput` seam inside the commit unit. Product assembly of that seam onto the todo-25 sequencer and todo-24 leases belongs to todos 28-31; until then a runtime without it answers keys prompts `delivery_unsupported`. The fixture's `KeyInput` counts bytes only.
- Admission runs before the SQLite COMMIT inside the unit. If the COMMIT itself fails after admission, the operation stays running/uncertain (ledger semantics), not answered.
- The prompt → terminal route used for authorization is in memory and reloaded by `PromptService::recover(snapshot)`. Assembly must call it at startup. A stale route can only produce `not_found`, because the stored row stays authoritative.
- The fixture's blocked-send check reads a snapshot before echoing (fixture stand-in). The product contract is `PromptService::admission` inside the input actor step (todos 28-31).
- Prompts opened by OSC 7501 `kind=question|permission` and synthetic `blocked`-root prompts are not created here: the blocked root refuses sends with empty `prompt_ids`. Process-exit expiry is lazy (on answer). Todos 40/41 extend both.
- `origin/integration` moved to b1c68ef (todo 78) after dispatch. Per the lane rule I did not merge. A read-only `git merge-tree` shows the only conflict is `.github/workflows/probe-stress-macos.yml` (option list and job tail union); all code merges cleanly.
