# feat(protocol): advertise graphics caps and the daemon no-decode contract

Todo 86. Closes GAP-154, delivers IDEAL-121.

## Before

`docs/evidence/task-86/before.log`: on integration `da5b456`, hello `limits` (`schema/ovis-terminal.openrpc.json` `x-limits.defaults`) has 23 keys, and none of them is a graphics cap:

> graphics keys present: 0 of 23

`crates/ovis-terminal-protocol/src/schema/limits.rs:62-63` (at da5b456) ends `Limits` at `pub session_vt_bytes: ByteCount,`. No graphics notices existed in `x-events`, and there was no stated decode policy (GAP-154).

## After

- `Limits::v1` gains `graphics_sequence_max_bytes = 8 MiB` and `graphics_terminal_store_max_bytes = 32 MiB`. hello advertises them as `"8388608"` and `"33554432"`.
- Two new events in `x-events`: `graphics.dropped` (`GraphicsDroppedData`, reason `too_large`) and `graphics.evicted` (`GraphicsEvictedData`, reason `store_full`). Both reasons are open enums. Their typed constructors are in the new `crates/ovis-terminal-protocol/src/domain/graphics.rs` (`GraphicsCaps::advertised`, `GraphicsCaps::admit`, `GraphicsEvictedData::store_full`).
- Design doc: [`docs/design/graphics.md`](docs/design/graphics.md). It covers the no-decode contract (header-only detection, keep or drop whole, viewers render), the cap table, the notice table and the layer budget.
- Regenerated schema diff: `schema/ovis-terminal.openrpc.json` (+137 lines: two `Limits` properties and their defaults, two events, five component schemas). The TS SDK `packages/client-ts/src/generated` was regenerated with `bun run generate`. Every generated file changed because each one embeds the schema sha256 in its header.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-core -p ovis-terminal-protocol --locked` | 0 | 551 tests run, 551 passed (`nextest.log`) |
| `cargo nextest run -p ovis-terminal-protocol --locked graphics` | 0 | 6 passed. They assert: hello carries both caps as `"8388608"`/`"33554432"`; for each cap `"18446744073709551615"` round-trips to `u64::MAX` and a JSON number is refused; exactly 8 MiB is admitted and 8388609 bytes are dropped with the full `too_large` notice JSON; the `store_full` notice JSON; `x-events` maps `graphics.dropped`/`graphics.evicted` to their schemas and the reason enums are `[too_large]`/`[store_full]`; plain-mode base64 of a sequence fits `max_frame_bytes`, and the store fits the sequence, the output ring and the session budget |
| `cargo run -p ovis-terminal-protocol --bin openrpc -- compat schema/released.openrpc.json` | 0 | `{"compatible_with":"schema/released.openrpc.json"}` (`schema-compat.log`); additive, no BASELINE-AMENDMENTS entry |
| `cargo run -p ovis-terminal-conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-86 --isolated --case happy` | 0 | 1 case, 4 assertions, incl. `/result/limits/graphics_sequence_max_bytes == "8388608"` (`qa-happy.log/json`) |
| `... --case failure --inject '{"method":"hello","params":{"limits":{"graphics_sequence_max_bytes":8388608}}}'` | 0 | 1 case, 5 assertions: `invalid_params`, `field_path` `params.limits`, no result (`qa-failure.log/json`) |
| `... --case all --evidence docs/evidence/task-86/conformance.json` | 0 | 2 cases, 8 assertions |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1174 tests run: 1174 passed, 1 skipped; hygiene 0 violations (`check.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy ... -- -D warnings` | 0 / 0 | `windows-check.log` |
| `cd packages/client-ts && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | 30 pass, 0 fail (`sdk.log`) |

OS/arch: Linux x86_64. The conformance runs drive the real `target/debug/ovis-terminal` binary.

Mutants (each applied alone, run, then restored; logs committed):
- `mutant-admit-off-by-one.log`: `<=` changed to `<` in `admit`. `the_cap_is_admitted_and_one_byte_more_is_dropped_whole` fails.
- `mutant-sequence-default.log`: the default changed to 9 MiB. The hello caps test and the admit test fail.
- `mutant-evicted-dropped-from-catalog.log`: the event renamed. `the_event_catalog_publishes_both_graphics_notices` fails.

Note on the QA failure case: hello params are a closed schema with no `limits` field, so the injected request is refused `invalid_params` at `params.limits`. The refusal of the numeric cap form itself is proven by `graphics_caps_round_trip_u64_max_and_refuse_numbers`, not by this inject.

## Seams

- Write scope named `crates/ovis-terminal-core/src/limits.rs`, but that file does not exist. The runtime `Limits::v1` lives in `crates/ovis-terminal-protocol/src/schema/limits.rs`, which core already reads (`docs/service.rs`, `ops/params.rs`, `subscriptions/limits.rs`). The two fields were added there, additively.
- `crates/ovis-terminal-protocol/src/domain.rs`: `pub mod graphics;` registration.
- `schema/error_kinds.rs` is unchanged. The notices are events with their own open reason enums, and no new error kind was needed.
- `conformance/scenarios/part-a/task-86.json` (new executable scenario) and `conformance/src/runner_tests/suite.rs`: `EXECUTABLE += part-a/task-86.json`, in sorted position. It was never in `declared.json`.
- `docs/design/traceability.json`: GAP-154 `scenarios += part-a/task-86`. The conformance traceability test requires every scenario to be listed under each id it closes. `graphics/passthrough` is kept for todo 83.
- `.github/workflows/probe-stress-macos.yml`: option `graphics-caps` and its job appended last.
- `packages/client-ts/src/generated/*`: regenerated, not hand-edited.

## Risks

- **The 8 MiB sequence cap is not yet reachable on a live attach.** `stream_backlog_bytes` is 4 MiB (`streams/hub.rs:264` gaps a viewer above it), and `streams/filter.rs` `HOLD_LIMIT` is 4096. This lane adds no stream code. `docs/design/graphics.md` "Layer budget" makes it a binding requirement for todo 83 to make the cap reachable before enforcing it: either deliver an admitted sequence without charging it to the backlog as one burst, or raise the backlog to at least the cap plus one chunk.
- The adversarial review was done by the executor (`adversarial.log`). The requested terra reviewer model id did not resolve, and subagent nesting is capped at depth 1.

DEFERRED VERIFICATION: none. The runner executes `part-a/task-86` with exit 0, both cases.
