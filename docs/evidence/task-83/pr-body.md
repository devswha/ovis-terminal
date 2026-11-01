# feat(streams): pass Kitty, iTerm2 and sixel graphics through attach unmodified with a whole-sequence size cap

Todo 83. Closes GAP-151, delivers IDEAL-118. The scenario also claims GAP-154 / IDEAL-121, because it exercises the todo-86 sequence cap on a live attach. OS/arch: Linux x86_64.

## Before

Base: integration `93d7fb7`. The only change was the new socket QA test (`crates/ovis-terminal-server/src/streams/graphics/qa.rs` and its registration in `tests/streams.rs`). The log is `docs/evidence/task-83/before.log`.

- `graphics_three_protocols_reach_a_live_viewer_byte_identical` **PASS**. The sequence bytes already reach an attached viewer, but no contract or test guards them. The filter released any held sequence longer than `HOLD_LIMIT` (`crates/ovis-terminal-server/src/streams/filter.rs:19`, 4096 bytes) unfiltered and in pieces.
- `graphics_a_sequence_at_the_cap_arrives_whole_past_the_stream_backlog` **FAIL** (`qa.rs:124`). An 8 MiB image (exactly `graphics_sequence_max_bytes`) passes the 4 MiB per-stream backlog (`streams/hub.rs:264` at `93d7fb7`). The viewer gets `gap` + `checkpoint` instead of the image, so the advertised cap is unreachable on a live attach. Todo 86's Risks flagged this.
- `graphics_an_over_cap_series_is_replaced_by_one_typed_notice` **FAIL** (`qa.rs:162`). `left: 0, right: 1`: no `graphics_dropped` record exists. The 8 MiB + 1 series is not bounded: its payload streams to the viewer (the log shows `QkJC…` = `BBB…` payload DATA frames up to offset 4194535), up to the backlog gap.

## After

How it works:
- The query filter's output passes through a new graphics gate (`crates/ovis-terminal-server/src/streams/graphics.rs`). The gate reuses the filter's escape-boundary tracker: `filter::{Scan, Step, step}` are now `pub(super)`, and their semantics are unchanged.
- The gate recognises three introducers, by the introducer bytes only:
  - `ESC _ G` (Kitty), including every following `m=1` chunk of the same transmission. Only the control header is read, for `m=`.
  - `ESC ] 1337;File=` (iTerm2).
  - `ESC P <params> q` (sixel). A DCS with an intermediate byte, such as DECRQSS `ESC P $ q`, is not graphics.
- The gate holds a unit until its last byte and releases it to the shared log as one piece, so no viewer is ever handed a prefix. Payloads are never decoded or inspected.
- A unit that passes `graphics_sequence_max_bytes` is dropped whole at the moment it does:
  - Its bytes become removed raw output, so log→raw offset mapping stays exact. Nothing more of it is buffered. Later `m=1`/`m=0` chunks of the series are discarded as they arrive.
  - A typed `attach.control` record `kind: "graphics_dropped"` stands exactly where the sequence began. It carries the todo-86 `GraphicsDroppedData` `{protocol, reason: too_large, output_offset, sequence_bytes, limit_bytes}`, built by `GraphicsCaps::admit`.
- A viewer's backlog bound is raised by the bytes of whole graphics sequences it still has to send, up to one sequence cap (`streams/graphics/backlog.rs`). The 8 MiB cap is therefore reachable. A viewer that still cannot keep up gets the existing typed `gap` + `checkpoint`.
- The VT is untouched. It ingests every raw byte as before, and alacritty keeps no image state.

Notice schema: `ControlKind::GraphicsDropped` and `AttachControl.graphics_dropped` (optional, `$ref GraphicsDroppedData`). Diff: `schema/ovis-terminal.openrpc.json` (+16 lines, additive). `openrpc compat schema/released.openrpc.json` → `{"compatible_with":"schema/released.openrpc.json"}`, exit 0.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-server -p ovis-terminal-vt --locked` | 0 | 289 run, 289 passed (`nextest.log`) |
| The same, graphics tests only (`... graphics`) | 0 | 17 passed: 13 gate unit tests + 3 socket QA tests (plus the log test). Kitty 3-chunk series, iTerm2 and sixel are each cut at **every** byte boundary (exhaustive) and over proptest random multi-cuts. Each run asserts: sha256(released) == sha256(written); the sequence lies whole inside one gate output; raw spans are contiguous. An over-cap sequence of each protocol, at any split, gives exactly 1 notice, at its start index, with 0 payload bytes and `sequence_bytes = cap+1`. A dropped series holds nothing after the drop. An answered query beside an image is still removed |
| `cargo nextest run -p ovis-terminal-conformance -p ovis-terminal-protocol --locked` | 0 | 289 run, 289 passed (`conformance-crate.log`); includes `all/all` and the traceability tests |
| `conformance --binary target/debug/ovis-terminal --scenario graphics/passthrough --isolated --case happy --evidence docs/evidence/task-83/qa-happy.json` | 0 | 1 case, 20 assertions. Real `ovis-terminal serve`. A shell writes a 3-chunk Kitty f=100 PNG and a sixel after an interact viewer attaches. DATA offsets are contiguous, and `graphics/contains[kitty_png_3_chunks]` and `[sixel]` match by SHA-256. Controls are exactly `checkpoint, input_ready` (`qa-happy.log/json`) |
| `... --case failure --inject '{"graphics":{"bytes":"over_cap"}}' --evidence docs/evidence/task-83/qa-failure.json` | 0 | 1 case, 30 assertions. The shell writes `before-mark`, a Kitty `m=1` chunk with 8388609 payload bytes, an `m=0` chunk, then `after-mark`. Controls are exactly `checkpoint, input_ready, graphics_dropped` with `reason too_large`, `protocol kitty`, `limit_bytes "8388608"` and `sequence_bytes "8388609"`. The transcript contains `before-markafter-mark`. `ESC _ G`, `AAAAAAAA` and the final chunk's `QQ` are all absent. `terminal.screen` lines contain `before-markafter-mark`, so the grid is unaffected (`qa-failure.log/json`) |
| `... --case all --evidence docs/evidence/task-83/conformance.json` | 0 | 2 cases, 49 assertions, failed 0, skipped 0 |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1445 run, 1445 passed, 2 skipped; hygiene 0 violations in 1457 files; 255 JSON files (`check.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy ... -- -D warnings` | 0 / 0 | `windows-check.log` |
| `cargo clippy -p ovis-terminal-server -p ovis-terminal-protocol -p ovis-terminal-conformance --all-targets --locked -- -D warnings` | 0 | `clippy.log` |
| `quality-matrix --mode contract` then `--mode verify` | 0 / 0 | verify: 0 findings, 121 ideals, 154 gaps, 104 methods |
| `cd packages/client-ts && bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | 47 pass, 0 fail (`sdk.log`). Every generated file changed, because each one embeds the schema sha256 |

Mutants (each applied alone, run against `cargo nextest run -p ovis-terminal-server --locked graphics`, then restored; logs committed):
- `mutant-kitty-series-never-continues.log`: `m=1` is never recognised. 6 tests fail.
- `mutant-drop-without-notice.log`: the unit is dropped but no notice is published. 5 tests fail.
- `mutant-no-graphics-backlog-bypass.log`: the backlog bound is not raised. 1 test fails (the 8 MiB socket test).

Adversarial classes and the test that guards each one are listed in `adversarial.log`.

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-protocol/src/schema/records/stream.rs`: `ControlKind::GraphicsDropped` and the optional `AttachControl.graphics_dropped`. The stream notice needs a wire slot. The data type is the existing todo-86 `domain/graphics.rs` `GraphicsDroppedData`, which this lane did not change.
- `schema/ovis-terminal.openrpc.json`, `packages/client-ts/src/generated/*`: regenerated (`openrpc write`, `bun run generate`), never hand-edited.
- `conformance/ideal-map.json`: regenerated by `quality-matrix --mode contract` for the schema change.
- `crates/ovis-terminal-server/src/streams/{hub,log,viewer,limits,open,resync}.rs`, `streams.rs`:
  - The gate sits between filter and log in `publish`/`finish`/`fence`.
  - The log carries per-chunk graphics bytes and the ordered notices.
  - The viewer sends a notice when its cursor reaches it.
  - `StreamLimits` carries the advertised `GraphicsCaps`.
  - Fresh paints skip older notices.
  - `mod graphics;`.
  - `filter.rs` changed visibility only (boundary reuse).
- `crates/ovis-terminal-server/tests/streams.rs`: registers the 3 socket QA tests.
- `conformance/src/scenario/model.rs`, `conformance/src/engine/steps.rs`, `conformance/src/fixture.rs`: register the new `graphics_transcript` step. Its implementation is `conformance/src/fixture/graphics.rs`.
- `conformance/src/runner_tests/suite.rs`: `ASSEMBLED += graphics/passthrough.json` (sorted). The scenario was not in `declared.json`.
- `docs/design/traceability.json`: GAP-154 `todos += "83"`. GAP-154 already listed `graphics/passthrough`, and the traceability test requires that scenario's owner to be listed.
- `probes/quality/evidence-exemptions.json`: exempts `docs/evidence/task-61/acceptance-evidence.json` and `task-61-ovis-terminal-v1.json`, with the existing precedent's reason. Their `$equals_file schema/ovis-terminal.openrpc.json` assertion pins the schema of that commit, and this schema growth makes it non-recomputable. task-43/59 have the same entries for the same cause.
- `probes/platform-qa/stress/graphics-passthrough.{json,sh}`: the macOS stress lane. Adding a lane edits nothing in the workflow.

## Risks

- **"Never split across frames" is held at the release boundary, not the wire frame.** A graphics unit enters the shared log as one piece, so a viewer never receives a prefix ahead of the rest being committed. It is then sent in credit-limited `attach.data` chunks of at most 16 KiB, because the 256 KiB / 1 MiB credit contract forbids one 8 MiB frame. The concatenated bytes are identical (tested by sha256).
- **Attach or resync during a transmission.** The unit stays held, and it follows the new viewer's checkpoint whole. That viewer gets the full image. The checkpoint's raw `output_offset` already includes those bytes, which draw no cells in the VT.
- **Kitty series interrupted by other output.** A Kitty series ends at the first non-chunk byte, and each contiguous run is measured against the cap. No text is ever held behind an image, and nothing is reordered.
- **Not recognised as units:** iTerm2 multipart (`MultipartFile`/`FilePart`) and 8-bit C1 introducers (the filter doesn't track them either).
- **Session backlog unchanged.** The 64 MiB aggregate backlog bound still applies: eight viewers each owing one 8 MiB image can still resync the largest. The typed `gap` + `checkpoint` reports it.
- **No `graphics.dropped` event.** The drop is reported only as the in-stream record. Committing the todo-86 `graphics.dropped` event to the event log is left to the store/replay todo (84).
- **The adversarial pass was executor-run** (`adversarial.log`). No separate reviewer model was available to this lane.

DEFERRED VERIFICATION: none. The runner executes `graphics/passthrough` against the real binary, both cases, exit 0, failed 0, skipped 0.
