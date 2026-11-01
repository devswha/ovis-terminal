# feat(protocol): advertise graphics caps and the daemon no-decode contract

Todo 86. Closes GAP-154, delivers IDEAL-121.

## Before

On integration `da5b456`, hello limits carry no graphics cap. The hello reply recorded at `docs/evidence/task-79/conformance.json:856` has the following `result.limits`. That file is unchanged at `da5b456`, and the server builds hello limits from `Limits::v1()` (`crates/ovis-terminal-server/src/conn/identity.rs:45`). The object has 23 keys:

```json
{"connection_unwritten_bytes":"67108864","credit_initial_bytes":"262144","credit_max_bytes":"1048576","doc_max_bytes":"1048576","event_replay_bytes":"67108864","event_replay_events":"10000","heartbeat_ms":"15000","history_lines":"100000","input_queue_bytes":"16777216","max_frame_bytes":"16777216","max_in_flight_requests":"64","op_retention_ms":"86400000","output_ring_bytes":"67108864","queued_operations":"32","rpc_default_deadline_ms":"10000","session_backlog_bytes":"67108864","session_event_queue_bytes":"67108864","session_input_bytes":"67108864","session_retained_bytes":"4294967296","session_rss_bytes":"1073741824","session_vt_bytes":"134217728","stream_backlog_bytes":"4194304","stream_chunk_bytes":"16384"}
```

It is identical to `x-limits.defaults` in `schema/ovis-terminal.openrpc.json` at `da5b456` (`docs/evidence/task-86/before.log`). `crates/ovis-terminal-protocol/src/schema/limits.rs:62-63` (at da5b456) ends `Limits` at `pub session_vt_bytes: ByteCount,`. No graphics notices existed in `x-events`, and there was no stated decode policy (GAP-154).

## After

- `Limits::v1` gains `graphics_sequence_max_bytes = 8 MiB` and `graphics_terminal_store_max_bytes = 32 MiB`. hello advertises them as `"8388608"` and `"33554432"`.
- Two new events in `x-events`: `graphics.dropped` (`GraphicsDroppedData`, reason `too_large`) and `graphics.evicted` (`GraphicsEvictedData`, reason `store_full`). Both reasons are open enums. In round 2 both data objects became closed (`#[serde(deny_unknown_fields)]`, which emits `additionalProperties: false`): exactly their five typed fields, all required, so a notice cannot carry an extra payload field. Their typed constructors are in the new `crates/ovis-terminal-protocol/src/domain/graphics.rs` (`GraphicsCaps::advertised`, `GraphicsCaps::admit`, `GraphicsEvictedData::store_full`).
- hello now carries both caps. The recorded reply at `docs/evidence/task-86/qa-happy.json:60` has 25 `result.limits` keys, including `"graphics_sequence_max_bytes":"8388608"` and `"graphics_terminal_store_max_bytes":"33554432"`.
- Design doc: [`docs/design/graphics.md`](https://github.com/devswha/ovis-terminal/blob/lane/t86-define-graphics-caps-and-the/docs/design/graphics.md). It covers the no-decode contract (header-only detection, keep or drop whole, viewers render), the cap table, the notice table and the layer budget.
- Regenerated schema diff: [`schema/ovis-terminal.openrpc.json` in the PR diff](https://github.com/devswha/ovis-terminal/pull/68/files) (round 1 +137 lines, round 2 +2 lines `additionalProperties: false`; two `Limits` properties and their defaults, two events, five component schemas). The TS SDK `packages/client-ts/src/generated` was regenerated with `bun run generate`. Every generated file changed because each one embeds the schema sha256 in its header.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-core -p ovis-terminal-protocol --locked` | 0 | round 2: 554 tests run, 554 passed (`r2-nextest.log`); round 1: 551 passed (`nextest.log`) |
| `cargo clippy -p ovis-terminal-protocol --all-targets --all-features --locked -- -D warnings` | 0 | `r2-clippy.log` |
| `cargo nextest run -p ovis-terminal-protocol --locked graphics` | 0 | 9 passed. Round 2 added three. (a) `the_caps_are_read_from_the_limits_a_hello_reply_carries`: a hello limits object advertising `"7"`/`"11"`, decoded as a client would, yields caps 7/11; 7 bytes are admitted, 8 are dropped with `limit_bytes` 7, and `store_full` reports `limit_bytes` 11. (b) `notices::notice_schemas_are_closed_objects_of_their_typed_fields_only`: both notice schemas have `additionalProperties: false`, and their properties and `required` lists are exactly the five typed fields. (c) `notices::a_notice_carrying_any_extra_field_is_refused`: both notices round-trip, and adding a `payload` field makes decoding fail. The six round-1 tests assert: hello carries both caps as `"8388608"`/`"33554432"`; for each cap `"18446744073709551615"` round-trips to `u64::MAX` and a JSON number is refused; exactly 8 MiB is admitted and 8388609 bytes are dropped with the full `too_large` notice JSON; the `store_full` notice JSON; `x-events` maps `graphics.dropped`/`graphics.evicted` to their schemas and the reason enums are `[too_large]`/`[store_full]`; plain-mode base64 of a sequence fits `max_frame_bytes`, and the store fits the sequence, the output ring and the session budget |
| `cargo run -p ovis-terminal-protocol --bin openrpc -- compat schema/released.openrpc.json` | 0 | `{"compatible_with":"schema/released.openrpc.json"}` (`r2-schema-compat.log`, `schema-compat.log`); additive, no BASELINE-AMENDMENTS entry; `openrpc check` reports the committed schema fresh |
| `cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-86 --isolated --case happy` | 0 | 1 case, 4 assertions, incl. `/result/limits/graphics_sequence_max_bytes == "8388608"` (`qa-happy.log/json`) |
| `... --case failure --inject '{"method":"hello","params":{"limits":{"graphics_sequence_max_bytes":8388608}}}'` | 0 | 1 case, 5 assertions: `invalid_params`, `field_path` `params.limits`, no result (`qa-failure.log/json`) |
| `... --case all --evidence docs/evidence/task-86/conformance.json` | 0 | 2 cases, 8 assertions |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | round 2, before the merge (ea21abb tree): 1177 passed, 1 skipped, hygiene 0 violations in 1164 files (`r2-check.log`). Round 1: 1174 passed, 1 skipped (`check.log`). The post-merge run is under Merge below |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy ... -- -D warnings` | 0 / 0 | round 2 before the merge: `r2-windows-check.log`; round 1: `windows-check.log`; post-merge under Merge below |
| `cd packages/client-ts && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | round 2: 30 pass, 0 fail (`r2-sdk.log`, attempt 2). Attempt 1 had 1 failure: the real-socket test ran the prebuilt `target/debug/conformance` left over from round 1 (`test/support/fixture.ts:11`), whose served document lacked the new `additionalProperties: false`. After `cargo build --locked -p ovis-terminal-cli -p ovis-terminal-conformance` (CI does the same, `ci.yml:233`), the single gate run passed. Round 1: 30 pass (`sdk.log`) |

OS/arch: Linux x86_64. Correction from round 1: these conformance runs do not exercise the `ovis-terminal` binary. The scenario's `serve` step spawns the runner's protocol fixture (`target/debug/conformance fixture`, which is the real server crate, dispatcher and store; see `processes` in `qa-happy.json`), and the hello travels `via: socket` directly to it. `--binary target/debug/ovis-terminal` is recorded in the evidence, but that binary is not on this path. The repository's scenario notes say real-binary serving lands with todo 31.

Mutants (each applied alone, run, then restored; logs committed). Round 2 ran these against the full protocol suite on the shipped head:
- `mutant-advertised-sequence-hardcoded.log`: `GraphicsCaps::advertised` returns a literal 8 MiB instead of `limits.graphics_sequence_max_bytes`. 225 run, 224 passed, 1 failed (`the_caps_are_read_from_the_limits_a_hello_reply_carries`). This mutant survived 222/222 in the review.
- `mutant-advertised-store-hardcoded.log`: a literal 32 MiB instead of `limits.graphics_terminal_store_max_bytes`. 225 run, 224 passed, 1 failed (the same test). It also survived in the review.
- `mutant-dropped-notice-open-object.log`: `#[serde(deny_unknown_fields)]` removed from `GraphicsDroppedData`. 225 run, 222 passed, 3 failed: the closed-schema test, the extra-field test, and the committed-schema freshness test `schema happy`.

Round 1 (recorded at a952c49):
- `mutant-admit-off-by-one.log`: `<=` changed to `<` in `admit`. `the_cap_is_admitted_and_one_byte_more_is_dropped_whole` fails.
- `mutant-sequence-default.log`: the default changed to 9 MiB. The hello caps test and the admit test fail.
- `mutant-evicted-dropped-from-catalog.log`: the event renamed. `the_event_catalog_publishes_both_graphics_notices` fails.

Note on the QA failure case: hello params are a closed schema with no `limits` field, so the injected request is refused `invalid_params` at `params.limits`. The refusal of the numeric cap form itself is proven by `graphics_caps_round_trip_u64_max_and_refuse_numbers`, not by this inject.

## Merge: integration 8ac8489 (round 2)

origin/integration moved to `8ac8489` (T37 conversation records, with regenerated schema and SDK), and this PR conflicted. As authorized, I merged it in `05651a7` (parents `ea21abb` and `8ac8489`); nothing was rebased or force-pushed.
- `crates/ovis-terminal-protocol/src/schema/event_catalog.rs`: union of the two import lines (`conversation_records::RecordsAppended` and `graphics::{GraphicsDroppedData, GraphicsEvictedData}`). The catalog has `agent.records_appended` and both graphics events.
- `.github/workflows/probe-stress-macos.yml`: integration's file, with `graphics-caps` appended last, after `conversation-records`, both to the options and as the final job.
- `schema/ovis-terminal.openrpc.json`: regenerated with `openrpc write` rather than resolved by hand. `openrpc check` reports it fresh, and `openrpc compat schema/released.openrpc.json` exits 0 (`r2-merge-schema-compat.log`).
- `packages/client-ts/src/generated`: integration's files taken as the base, regenerated with `bun run generate`, and committed with the merge.
- `Cargo.lock`: no conflict and identical to integration's; `cargo metadata --locked` exits 0.

Post-merge gates on `05651a7`:

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-core -p ovis-terminal-protocol --locked` | 0 | 562 tests run, 562 passed (`r2-merge-nextest.log`) |
| `cargo clippy -p ovis-terminal-protocol --all-targets --all-features --locked -- -D warnings` | 0 | `r2-merge-clippy.log` |
| `cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-86 --isolated --case all` | 0 | 2 cases, 8 assertions (`r2-merge-conformance.log/json`) |
| `cd packages/client-ts && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | generate changed 0 files, so the committed SDK is fresh; 30 pass, 0 fail, run once on a freshly built fixture (`r2-merge-sdk.log`) |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1207 tests run: 1207 passed, 1 skipped; hygiene 0 violations in 1182 files; 143 JSON files checked (`r2-merge-check.log`). This run started before the 2026-11-01 RAM-target host rule (`env.sh` changed at 15:59 +0900) and used the on-disk target |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy ... -- -D warnings` | 0 / 0 | `r2-merge-windows-check.log`; ran under the RAM-target rule (`CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t86`) |

## Seams

- Write scope named `crates/ovis-terminal-core/src/limits.rs`, but that file does not exist. The runtime `Limits::v1` lives in `crates/ovis-terminal-protocol/src/schema/limits.rs`, which core already reads (`docs/service.rs`, `ops/params.rs`, `subscriptions/limits.rs`). The two fields were added there, additively.
- `crates/ovis-terminal-protocol/src/domain.rs`: `pub mod graphics;` registration.
- Round 2: `crates/ovis-terminal-protocol/src/domain/graphics/tests/notices.rs` is a child test module of the new `domain/graphics.rs` scope. The notice-shape tests moved there so `tests.rs` stays under the 250 pure-LOC hygiene limit (143 and 126 lines).
- `schema/error_kinds.rs` is unchanged. The notices are events with their own open reason enums, and no new error kind was needed.
- `conformance/scenarios/part-a/task-86.json` (new executable scenario) and `conformance/src/runner_tests/suite.rs`: `EXECUTABLE += part-a/task-86.json`, in sorted position. It was never in `declared.json`.
- `docs/design/traceability.json`: GAP-154 `scenarios += part-a/task-86`. The conformance traceability test requires every scenario to be listed under each id it closes. `graphics/passthrough` is kept for todo 83.
- `.github/workflows/probe-stress-macos.yml`: option `graphics-caps` and its job appended last.
- `packages/client-ts/src/generated/*`: regenerated, not hand-edited.

## Risks

- **The 8 MiB sequence cap is not yet reachable on a live attach.** `stream_backlog_bytes` is 4 MiB (`streams/hub.rs:264` gaps a viewer above it), and `streams/filter.rs` `HOLD_LIMIT` is 4096. This lane adds no stream code. `docs/design/graphics.md` "Layer budget" makes it a binding requirement for todo 83 to make the cap reachable before enforcing it: either deliver an admitted sequence without charging it to the backlog as one burst, or raise the backlog to at least the cap plus one chunk.
- The independent gate review (`.omo/evidence/ovis-terminal-v1-task-86-adversarial.md` in the main checkout, REJECT) raised three blockers. Round 2 closes the two plumbing and closure gaps the orchestrator scoped and quotes the real hello limits in Before. Two blockers are still open by scope:
  - Store-path proof: this lane adds no store code, and an image store with entries sized to the cap belongs to todos 83 and 84. `docs/design/graphics.md` states the rule, but no implementation proves it yet.
  - Exact task-13 QA: `part-a/task-13` is a declared-only descriptor the runner cannot execute (`conformance/scenarios/declared.json:25`). The hello assertions live in the executable `part-a/task-86`, served by the conformance fixture (see the correction under After).
- The real hello in the server is built wholesale from `Limits::v1()` (`conn/identity.rs:45`). Its graphics values are asserted against a live fixture server by `part-a/task-86`. A test that the server advertises a non-default `Limits` would need configurable server limits, which this lane does not add.
- The notice objects are closed. Adding a field later is therefore a deliberate protocol change: strict decoders of the current schema would refuse it.
- Round 1's adversarial pass was executor-run (`adversarial.log`). The requested terra reviewer model id did not resolve, and subagent nesting is capped at depth 1.

DEFERRED VERIFICATION: none. The runner executes `part-a/task-86` with exit 0, both cases.
