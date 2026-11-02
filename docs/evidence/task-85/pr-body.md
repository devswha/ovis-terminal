feat(vt): report image placeholders in screen reads and records (plan todo 85, wave 15)

Closes GAP-153 (screen reads lose images). Delivers IDEAL-120.

`terminal.screen` takes a new optional `include_images` flag. Absent means false. When it is true, the read adds a placement index to its result. That index lists one `GraphicsPlaceholder` `{image_id, format: kitty|iterm2|sixel, row, col, rows, cols, bytes}` for every stored image shown on the active screen. `image_id` is the image store key, which is the raw offset `graphics_replayed` also names. `bytes` is the size of that store entry. Payload bytes are never copied into a read or a record. Text `lines` are unchanged: covered cells still read as blank text, and non-image cells read exactly as a text-only stream would show them.

## Before

`docs/evidence/task-85/before.log` and `before-happy.json` come from the base `origin/integration` 1869b9e binary, on Linux 7.0.0-38-generic x86_64. A shell prints `ab`, then a 3-chunk Kitty f=100 PNG declaring `c=4,r=2` at raw offset 4 (145 bytes), then `images-done`. The viewer receives the image byte-identical (`graphics/contains[kitty_png_3_chunks]` passed). The screen read reports the 2x4 image region only as blank cells:

```
{"jsonrpc":"2.0","id":"plain","result":{"cursor":{"col":11,"row":4,"visible":true},"lines":["","ab","","","images-done",...],"output_offset":"169","revision":"27","title":null,"truncated":false}}
{"jsonrpc":"2.0","id":"with-images","error":{"code":-32602,...,"data":{"kind":"invalid_params",...,"field_path":"params.include_images"}}}
```

The base read has no image fact, and the base schema cannot ask for one. In code, before this PR, `crates/ovis-terminal-vt/src/engine/read.rs` `ScreenRead` (lines 83-108 at 1869b9e) had no image field. `crates/ovis-terminal-protocol/src/schema/methods/terminal_read.rs:15-20` `TerminalScreenParams` was `{target, format}` only. The conformance run against the base binary exited 1 at step 13 (`reply[with-images]/error`).

## After

Every command ran on Linux x86_64 (`uname -srm`: Linux 7.0.0-38-generic x86_64), with `CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t85`.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-vt -p ovis-terminal-protocol -p ovis-terminal-core --locked` (heavy slot) | 0 | 729 tests run, 729 passed (`after.log`) |
| `conformance --binary $CARGO_TARGET_DIR/debug/ovis-terminal --scenario graphics/read --isolated --case happy --evidence docs/evidence/task-85/qa-happy.json` | 0 | 1 case, 31 assertions. `terminal.screen` with `include_images: true` returns `"images":[{"bytes":"145","col":2,"cols":4,"format":"kitty","image_id":"4","row":1,"rows":2}]` at the drawn position. `lines[1]=="ab"`. The same read without the flag has no `images` field. |
| `… --case failure --inject '{"read":{"include_images":"yes"}}' --evidence docs/evidence/task-85/qa-failure.json` | 0 | 1 case, 34 assertions. `include_images` given as `"yes"`, `1` or `null` is `invalid_params` with `field_path: params.include_images` and no result. The next read with `true` still returns the one placeholder. |
| `… --case all --evidence docs/evidence/task-85/conformance.json` | 0 | 2 cases, 64 assertions, failed 0 |
| `cargo run -p ovis-terminal-protocol --bin openrpc -- check schema/ovis-terminal.openrpc.json` / `… compat schema/released.openrpc.json` | 0 / 0 | fresh; `{"compatible_with":"schema/released.openrpc.json"}`, additive (`openrpc.log`, schema diff `schema.diff`: new `GraphicsImageId`, `GraphicsPlaceholder`, optional param `include_images` (default false), optional result field `images`) |
| `cd packages/client-ts && bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | 51 pass, 0 fail (`sdk.log`). SDK regenerated; every generated file's header carries the new schema sha256. |
| `cargo run --locked --manifest-path probes/Cargo.toml --bin quality-matrix -- --mode contract` then `--mode verify` | 0 / 0 | `"findings": []`, `"verdict": "pass"`, methods 104 (`quality-contract.log`) |
| `cargo run --locked -p ovis-terminal-replay --bin ovis-replay -- refresh` | 0 | `{"refreshed":12}` fixture pins (schema and ideal-map digests) |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1571 tests run, 1571 passed, 2 skipped; workspace-hygiene 0 violations in 1550 files; json.sh 296 files; source-check and evidence-audit pass (`check.log`). Attempt 1 (`check-attempt1-contract-stale.log`) failed because the quality contract was stale (`quality-matrix --mode contract` had not been run after the schema change). Attempt 2 (`check-attempt2-pins-stale.log`) failed because the fixture pins of `conformance/ideal-map.json` were stale (`ovis-replay refresh` after the contract). Both were regenerated, not hand-edited. |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy … -- -D warnings` | 0 / 0 | `windows-check.log` |

New tests and what they assert:
- `crates/ovis-terminal-vt/tests/image_reads.rs` (6 tests):
  - A Kitty `c=4,r=2` image is reported at (1,2) as 2x4. Its lines equal a text-only engine's.
  - Partial scroll-off reports only the visible rows; full scroll-off reports none.
  - Printing over an image removes its placeholder.
  - A footprint is clipped at the right edge and the bottom.
  - Sixel (1x1), iTerm2 inline `width/height` and a Kitty put are reported; a bare `a=t` transmission is not.
  - A primary image is never reported on the alternate screen, even under an alternate image on the same cells, and is reported again after leaving.
- `crates/ovis-terminal-core/src/terminals/records/tests.rs` (5 tests):
  - Placeholder ids equal the store keys, and `bytes` equals the store entry's size.
  - An image the store evicted has no placeholder while the VT still shows it.
  - An image scrolled off and synced out of the store has no placeholder.
  - An image whose bytes never reached the store has no placeholder.
  - The encoded placeholders never contain the payload.
- `crates/ovis-terminal-protocol/src/domain/graphics/tests/placeholders.rs` (5 tests):
  - Decimal-string `image_id`/`bytes` round-trip at u64::MAX.
  - A numeric id or an extra payload field is refused.
  - `include_images` defaults to false.
  - `"yes"`, `"true"`, `1`, `null` and `[true]` are each `invalid_params` at `params.include_images`.
  - A result without images omits the field, and one with images round-trips.

Mutants were each applied alone, run, then restored; the logs are committed. All 4 fail a shipped test:
- `mutant-evicted-image-keeps-placeholder.log`: 2 core tests fail.
- `mutant-declared-rows-not-visible-rows.log`: 2 vt tests fail.
- `mutant-include-images-required.log`: 1 protocol test fails.
- `mutant-alternate-screen-not-filtered.log`: this one survived round 1 (`still()` already rejected unmarked alternate cells). The test now draws an alternate image on the same cells, and 1 test fails.

Adversarial classes and the test guarding each are listed in `adversarial.log` (executor-run).

macOS stress: lane `graphics-read` (`probes/platform-qa/stress/graphics-read.{json,sh}`). It runs 25 iterations of the placeholder tests plus `graphics/read --case all`. Run on `37c854f` (pre-merge code head): https://github.com/devswha/ovis-terminal/actions/runs/41374308872. The merge touched `crates/` (T66 CLI), so it was dispatched once more on the final head: STRESS2_URL

### GAP-152 (cursor model): recorded as still open

Todo 85 does not move the authoritative VT cursor past a placement. The reasons are in `docs/evidence/task-85/gap-152.md`, and the GAP-152 note in `docs/design/traceability.json` is updated to match.

First, moving the cursor changes where every later character lands. This block forbids changing cell semantics for non-image cells.

Second, the advance is computable only for extents declared in cells. For a Kitty image without `c`/`r`, an iTerm2 image with `auto` size, or a sixel, it depends on pixel and cell-pixel sizes that the headless runtime never learns. A partial advance would trade one cursor mismatch for another.

What a read reports in that case is consistent: text printed over an image releases it, and the read shows no placeholder.

### Records

`crates/ovis-terminal-core/src/terminals/records.rs` gains the placeholder record form `image_placeholders(shown, store)`. It joins the VT placements with the image store and is used by `terminal.screen`. No current terminal or agent record snapshots the screen: `Record`/`TerminalSnapshot` carry no screen content, and todo 37 forbids synthesizing conversation records from terminal bytes. So no other record changes shape. Any future snapshot record must call the same function.

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-vt/src/engine/images/{anchor,set}.rs`: the placement entries are private to the images module. `Images::on_screen` plus two anchor accessors (visible rows, clipped columns) expose the visible footprint to `read.rs`.
- `crates/ovis-terminal-vt/src/{engine.rs,lib.rs}`: export `ScreenImage`.
- `crates/ovis-terminal-vt/tests/image_reads.rs`: new vt behaviour tests.
- `crates/ovis-terminal-core/src/screen/{views,reads}.rs`: `ScreenView` carries the frozen read's `images`.
- `crates/ovis-terminal-core/src/terminals.rs`: re-export `image_placeholders`.
- `crates/ovis-terminal-core/src/terminals/records/tests.rs`: tests of the placeholder form.
- `crates/ovis-terminal-protocol/src/schema/methods/terminal_read.rs`: the wire option `include_images` and the result field `images`.
- `crates/ovis-terminal-protocol/src/domain/graphics/tests{.rs,/placeholders.rs}`: test registration and the tests themselves.
- `crates/ovis-terminal-server/src/streams/graphics/replay.rs`: `TerminalStreams::image_placeholders` reads the hub's image store under its lock.
- `crates/ovis-terminal-cli/src/assembly/reads.rs`: `terminal.screen` adds `images` when requested. It stays an unrecorded Query read.
- `schema/ovis-terminal.openrpc.json`, `packages/client-ts/src/generated/**`, `conformance/ideal-map.json`, `conformance/fixtures/ovis/{manifest,predicates}.json`: regenerated (`openrpc write`, `bun run generate`, `quality-matrix --mode contract`, `ovis-replay refresh`).
- `conformance/src/runner_tests/suite.rs`: `graphics/read.json` registered in `ASSEMBLED` in sorted position. `declared.json` never listed it.
- `docs/design/traceability.json`: GAP-152 note records the todo 85 decision.
- `probes/platform-qa/stress/graphics-read.{json,sh}`: macOS stress lane.

## Risks

- The screen snapshot and the image store are read under separate locks. A release landing between the two drops that placeholder, which is conservative: a placeholder is never reported without its store entry, but a just-released image can be missing from a read whose lines are one feed older.
- `rows`/`cols` are the declared footprint clipped to the grid (one cell when undeclared), not decoded pixel geometry.
- After this lands, other open lanes whose committed evidence pins `schema/ovis-terminal.openrpc.json` via `$equals_file` will need re-recording on the merged ref (schema-landing lesson).

## Round 2: merged origin/integration 1c1527d (T66) at the orchestrator's request

The PR conflicted only in `conformance/fixtures/ovis/manifest.json`, which holds generated fixture pins. I took integration's side and regenerated. The schema did not change in the merge (`openrpc write` reproduced 656804 bytes; `bun run generate` changed 0 files). The steps were `openrpc write` + `compat` (exit 0), `quality-matrix --mode contract` (verdict pass), `bun run generate`, then `ovis-replay refresh` (`{"refreshed":12}`). The merge commit is `0f663a5`.

On the merged tree, check.sh attempt 1 failed in `quality_evidence` (`check-merged-attempt1-t66-evidence-stale.log`). The evidence-audit reported `assertion-forged` at `docs/evidence/task-66/task-66-all.json#0` and `task-66-ovis-terminal-v1.json#0`. T66's committed `cli/manual` happy step pins `{"$equals_file":"schema/ovis-terminal.openrpc.json"}`, and this PR's additive schema change makes that pin stale on the merged tree. This is the schema-landing case. I re-recorded those two files on the merged binary with T66's own invocations (all; happy, then failure `--inject 'ovis-terminal agent get --unknown-flag --json' --append`). All three runs exited 0 (`t66-rerecord-*.log`), and evidence-audit then reported `finding_count 0` (`evidence-audit.log`).

| Command (merged tree) | Exit | Result |
|---|---|---|
| `conformance … --scenario graphics/read --case happy` / `failure --inject '{"read":{"include_images":"yes"}}'` / `all` | 0 / 0 / 0 | re-recorded `qa-happy.json`, `qa-failure.json`, `conformance.json` (all: 2 cases, 64 assertions) |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1591 tests run, 1591 passed, 2 skipped; hygiene 0 violations in 1564 files; json.sh 300 files (`check.log`); the pre-merge pass is kept as `check-pre-merge.log` |
| Windows `cargo check` / `cargo clippy` (`x86_64-pc-windows-gnu`, `-D warnings`) | 0 / 0 | `windows-check.log` |
| client-ts gate | 0 | 51 pass, 0 fail (`sdk.log`) |

Added seam: `docs/evidence/task-66/task-66-all.json` and `task-66-ovis-terminal-v1.json` were re-recorded, not edited by hand, because their schema pin follows this PR's schema.

DEFERRED VERIFICATION: none. The runner executes `graphics/read` today (happy, failure and all exit 0, as above).

## DoneClaim

DONECLAIM
