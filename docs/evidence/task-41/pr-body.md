## Summary

Todo 41 (wave 13): parent/child links bound to the parent's **program incarnation**, declared `notify` / `die_with_parent` policy, durable parent-death facts with per-machine acknowledgement, and the `agent.children` read.

- `crates/ovis-terminal-protocol/src/domain/agent_tree.rs` (+ `agent_tree/orphan.rs`): `ChildRef`, `ChildLink` (state `live|ending|ended`, `ChildEnd{reason exited|with_parent, seq}`), `ParentIdentity` (parent agent/session/turn captured at link time), `ChildNotice` + `NoticeAck`, `ParentDeath`, `RemoteDelivery`, `OrphanNotice`, `OrphanAck`; constructors for the released `ParentRef`, `ChildPolicy`, `ChildRecord`, `AgentChildrenResult`. Every incarnation/sequence is a decimal-string newtype.
- `crates/ovis-terminal-agent/src/tree/`: `TreeService`
  - `admit` refuses a parent **before anything is launched or linked**: `parent_finished` (that incarnation's finish is recorded), `stale_target` (the terminal runs another incarnation), `parent_not_found`.
  - `link` writes the link in the launch's commit unit and keeps the parent's identity; child status never writes the parent's status row.
  - `exited(coordinator, owner)` is ONE commit unit: records the finish once per incarnation (highest-finished-incarnation marker per terminal), orders the end of **only** live `die_with_parent` children of **that** incarnation (state `ending`, returned as `end_orders`), queues the death in each holding machine's outbox, ends the program's own link (`ended`, reason `with_parent` only if an end was ordered) with a `ChildNotice` when the parent asked for news, and expires the program's open prompts. A replayed finish is a no-op (`replayed: true`, no orders).
  - `orphaned` applies a death received from the parent's machine (exact incarnation only; replay is a no-op); `link_to_remote`, `link_remote`, `outbox`, `orphan_acked` (idempotent), `acknowledge` (parent acknowledges notices through a seq; replay covers nothing new); `children` (current incarnation only).
- `crates/ovis-terminal-server/src/dispatch/lanes/agent_tree.rs`: `agent.children` registered as an unrecorded READ (Read authz), result is the released `AgentChildrenResult`.
- Conformance: executable `conformance/scenarios/agent/tree.json` (happy + failure) and `conformance/src/fixture/tree.rs`.

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline: GAP-094, GAP-119.

## After

Native OS/arch: `Linux x86_64`. Binary `target/debug/ovis-terminal` sha256 `523b9e9a4127aea44bf3bffd559a0e085806be34df4a3e7674122ad8cc47e661` (also recorded in the evidence JSON `binary` field).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                      # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/tree --isolated --case happy --evidence "$ATTEMPT_DIR/task-41-ovis-terminal-v1.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"agent/tree","totals":{"assertions":"77","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/tree --isolated --case failure --inject '{"method":"terminal.run","params":{"parent":{"terminal_id":"parent","incarnation":"retired"}}}' --evidence "$ATTEMPT_DIR/task-41-ovis-terminal-v1.json" --append
{"case":"failure","error":null,...,"result":"passed","scenario":"agent/tree","totals":{"assertions":"57","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit 0
$ cargo run ... --scenario agent/tree --isolated --case all --evidence "$ATTEMPT_DIR/task-41-all.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"136","cases":"2","failed":"0","passed":"2","skipped":"0"}}
exit 0
```

The runner materializes each request as a JSON-RPC line through `ovis-terminal` as the bridge client to the fixture's real server (dispatcher, SQLite coordinator, authority) on a private Unix socket; every request and its actual reply are in the evidence JSON (`docs/evidence/task-41/task-41-ovis-terminal-v1.json`, `task-41-all.json`, `qa-*.stdout|stderr`).

**Happy observable (asserted):** parent incarnation 1 registers session `orchestrator-1` and reports working; `child-a` (`notify`, `die_with_parent`) opens prompt `child-a-question` and works; `child-b` (independent) works. `fixture.exit parent` → `{"finished":"recorded","end_orders":["terminal:7"],"ended":["terminal:7"]}`; `fixture.program child-a` → `kills "1", live false`; `child-b` → `kills "0", live true`; `prompt.get child-a-question` → `state "expired"`; `status.get child-b` → still `working`; `agent.children parent` → child-a `unseen_news 1`, child-b `unseen_news 0` and still working; `status.get parent` → session `orchestrator-1`, `process_incarnation "1"` unchanged.

**Failure observable (asserted):** parent 1 exits (ending child-a), the terminal is relaunched as incarnation `"2"`, child-c links to it. `terminal.run` under `{"terminal_id":"terminal:6","incarnation":"1"}` → `parent_finished`; under incarnation `"7"` → `stale_target`; `fixture.program child-b` → `incarnation "0", live false` (nothing launched, nothing linked); `agent.children parent` lists only `terminal:9`. Replaying the death of incarnation 1 (`fixture.exit {"incarnation":"1"}`) → `{"finished":"replayed","end_orders":[],"ended":[]}` and child-c stays `live true, kills "0"`: the reused address cannot kill the new child.

**Deviation, stated plainly:** the plan's injected literal `"incarnation":"retired"` is not a decimal string, so the strict schema answers `invalid_params` with `field_path "params.parent.incarnation"` (asserted; decimal-string contract, checklist item 1). It cannot produce `parent_finished`/`stale_target`. With `--inject` only that planned request runs, but the other steps of the failure case still run, and they are what prove `parent_finished` and `stale_target` before linking. A second planned injection with a well-formed retired incarnation (`"terminal:6"`, `"1"`) asserts `parent_finished`; it runs in the `--case all` run.

`terminal.run` and `fixture.exit`/`fixture.program` in the fixture are **stand-ins** (todo 31 assembles the real launcher; nothing spawns a real process here). `terminal.run` is served but **not advertised** in hello (protocol/discovery asserts it is absent). The stand-in calls the product `TreeService::admit`/`link` inside its commit unit, and `fixture.exit` calls the product `StatusService::exited` + `TreeService::exited` and carries out end orders with an incarnation-checked kill (a stale order kills nothing).

## QA

- `cargo test -p ovis-terminal-agent -p ovis-terminal-protocol -p ovis-terminal-server --all-targets`: covered by `bash scripts/check.sh` nextest (see `check.log`). Lane-relevant tests: protocol `domain::agent_tree::tests` (6, incl. 2 proptests), agent `tree::qa::{links,exits,remote}` (9), server `tests/dispatch.rs agent_tree::` (1, real socket: `agent.children` decimal strings, `not_found`, unknown field → `invalid_params`, reused address lists no children).
- `cargo clippy -p ... --all-targets -- -D warnings`: in check.sh, exit 0.
- `bash scripts/check.sh`: exit 0 (fmt, workspace clippy, nextest, doctests, hygiene 0 violations, manifests, json).
- Windows pre-check `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0 (`windows.log`).
- Runner suite: `EXECUTABLE` now 17 entries incl. `agent/tree.json`; `all/all` passes 32/32 cases.

## Adversarial

- **Stale state: parent address reuse.** Unit test `a_replayed_death_of_a_reused_address_never_ends_the_new_incarnations_child` and `a_finished_replaced_or_unknown_parent_is_refused_before_anything_is_linked`; failure scenario (above).
- **Repeated interruptions: replayed death notification.** Local replay (`exited` twice → `replayed`, no orders, no second outbox entry), remote replay (`orphaned` twice → second `replayed`; another incarnation's notice ends nothing), replayed `orphan_acked` → `false`, replayed `acknowledge` → empty. Scenario step `replayed-death`.
- **Misleading success: wait for child exit evidence** (V-RT-HC-03). An ordered end leaves the child `ending` and its prompt open; only its observed exit makes it `ended` (reason `with_parent`) and expires the prompt (`only_the_opted_in_child_is_ordered_to_end_and_ends_only_on_its_own_exit`, `an_ended_child_expires_its_open_prompts_in_the_same_unit`). The fixture reports `ended` only after the kill was delivered to the matching live incarnation.
- **Mutation probe (run, then reverted):** dropping the `die_with_parent` check from `ChildLink::order_end` makes `agent/tree --case happy` exit 1 (`end_orders` became `["terminal:7","terminal:8"]`).
- EXCL-011/EXCL-012/EXCL-021: no toast/sound/banner; news is a durable `ChildNotice` with acknowledgement, failures are typed and queryable. V-004/V-4388: identities are newtypes (`ChildRef`, `ParentRef`, `Incarnation`, `EventSeq`), states are closed enums.

## Seams

- **Public API additions** (additive): `ovis_terminal_protocol::domain::agent_tree` (new module); `ParentRef::new`, `ChildPolicy::new`, `ChildRecord::new`, `AgentChildrenResult::new`; `ovis_terminal_agent::tree` (new module); `AgentStatusEvent` gains variants `ParentFinished`, `ChildEnded`, `NoticesAcknowledged`, `OrphanAcknowledged` with event types `agent.parent_finished`, `agent.child_ended`, `agent.notices_acknowledged`, `agent.orphan_acknowledged`. These event types are **not yet in the protocol event catalog** (same precedent as todo 26's geometry events; catalog registration is left to todo 31/42).
- `ovis_terminal_server::dispatch::lanes::agent_tree::{AgentTreeLane, ListChildren}` (new).
- Outside the listed write scope, minimal and necessary: `agent/src/prompts/expire.rs` (crate-private `PromptService::expire_program`, so an ended program's prompts expire in the same unit) + `mod expire;` and two `pub(super)` widenings in `prompts/store.rs` (`Versioned::delete`, `Index::closed`); `status/event.rs` (the four variants); slot lines in `agent/src/lib.rs`, `protocol/src/domain.rs`, `server/src/dispatch/lanes.rs`, `server/tests/dispatch.rs`; conformance fixture wiring (`fixture.rs`, `fixture/agents.rs` adds terminals 6-9 to the directory, `fixture/handler.rs`, `fixture/serve.rs`, `fixture/tree.rs`) and `runner_tests/suite.rs` (EXECUTABLE 16→17).
- Integration points for later todos: todo 31/42 call `TreeService::admit`+`link` inside `terminal.run`'s unit and `StatusService::exited`+`TreeService::exited` on every observed exit, then carry out `Exit::end_orders` through the incarnation-checked kill path; todo 44 transports `Exit::deliveries`/`outbox` as `OrphanNotice`, applies received ones with `orphaned`, and answers `OrphanAck` → `orphan_acked`.

## Contract checklist

1. Decimal strings: every incarnation/seq/revision in emitted and accepted JSON is a decimal string; numeric → `invalid_params` (protocol test with `u64::MAX` round-trip; scenario `numeric-incarnation`). `ChildRecord.unseen_news` is the released `u32` field.
2. `TreeError` is a `thiserror` enum with `kind()`/`hint()`, `#[source]` on `Status`/`Prompt`, `#[from]` on `Event`/`Encode`; Display never repeats the kind token.
3. Distinct newtypes for identities; `Option` only for real absence (`ended`, `acknowledged_seq`, `parent_identity` for a remote parent).
4. Tests assert transitions, typed kinds and round trips (proptest); no read-back of local literals.
5. No `#[allow]`/`#[expect]`, no TODO placeholders. Holder wire generation stays 2.
6. Evidence states exactly what is asserted; conformance runs for real (runner landed in todo 21).

## DEFERRED VERIFICATION

- macOS stress (`probe-stress-macos.yml`): the `agent-tree` option and job block are appended, but this lane's copy of the workflow (dispatch base 0d3022c) does not parse (`yaml.scanner.ScannerError ... line 821`: the pre-existing union-merge damage repaired on integration at a74fba7). Per the orchestrator, it was not dispatched; the block parses standalone and is ready for the structural union.

## Risks

- Prompt expiry, finish markers and links live in the store; links/notices are bounded by 64 running children per parent incarnation. Finish markers are one row per terminal and outboxes are removed per acknowledgement, but an unacknowledged outbox grows until its peer acknowledges.
- Real launch/exit wiring (todo 31/42) and remote transport (todo 44) are not in this PR; the conformance path uses fixture stand-ins for those two seams only.
