feat(vt): report image placeholders in screen reads and records (plan todo 85, wave 15)

Closes GAP-153 (screen reads lose images). Delivers IDEAL-120.

**Every** `terminal.screen` read now carries `images`: one `GraphicsPlaceholder` per stored image shown on the read screen, top row first, in this shape: `{image_id, format: kitty|iterm2|sixel, row, col, rows, cols, bytes}`. The list is empty when no image is shown.
- `image_id` is the image store key, which is the raw offset `graphics_replayed` names too.
- `rows`/`cols` are the covered cells still visible, clipped to the grid.
- `bytes` is the size of the store entry.

The option `include_images` (default false, a strict JSON boolean) adds `image_index`, the fuller placement index. It lists every image the store holds, in receipt order, with its placement `{buffer: primary|alternate, row, col, rows, cols}` (declared extent). The placement is `null` for a Kitty transmission that displays nothing itself.

Payload bytes are never copied into a read or a record. `lines` are unchanged: the cells an image covers read as blank text, exactly as before, and every other cell reads as a text-only stream shows it. The change is additive (`openrpc compat` exit 0).

## Before

Evidence: `docs/evidence/task-85/before.log` and `before-happy.json`, from the base `origin/integration` 1869b9e binary on Linux 7.0.0-38-generic x86_64. A shell prints `ab`, then a 3-chunk Kitty f=100 PNG declaring `c=4,r=2` at raw offset 4 (145 bytes), then `images-done`. The viewer receives the image byte-identical. The default screen read reports the 2x4 image region only as blank cells, and the read has no image fact:

```
{"jsonrpc":"2.0","id":"plain","result":{"cursor":{"col":11,"row":4,"visible":true},"lines":["","ab","","","images-done",...],"modes":{...},"output_offset":"169","revision":"27","title":null,"truncated":false}}
```

The base schema cannot ask for one either: `include_images` is an unknown field there (`invalid_params` at `params.include_images`). In code at 1869b9e, `crates/ovis-terminal-vt/src/engine/read.rs` `ScreenRead` (lines 83-108) has no image field, and `crates/ovis-terminal-protocol/src/schema/methods/terminal_read.rs:22-40` `TerminalScreenResult` is `{revision, output_offset, lines, cursor, modes, title, truncated}`.

## After

The same default read on the PR binary (`qa-happy.json`, `"id":"plain"`, no `include_images`):

```
{"jsonrpc":"2.0","id":"plain","result":{...,"images":[{"bytes":"145","col":2,"cols":4,"format":"kitty","image_id":"4","row":1,"rows":2}],"lines":["","ab","","","images-done",...],...}}
```

With `include_images: true`, the read also carries `"image_index":[{"bytes":"145","format":"kitty","image_id":"4","placement":{"buffer":"primary","col":2,"cols":4,"row":1,"rows":2}}]`. The schema diff is in `schema.diff`: new `GraphicsImageId`, `GraphicsPlaceholder`, `GraphicsIndexEntry`, `GraphicsIndexPlacement` and `GraphicsBuffer`; the optional param `include_images` (default false); the result fields `images` and `image_index`.

Every command ran on Linux x86_64 with `CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t85`, on the tree merged with integration 1c1527d.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-vt -p ovis-terminal-protocol -p ovis-terminal-core --locked` (heavy slot) | 0 | 733 tests run, 733 passed (`after.log`) |
| `conformance --binary $CARGO_TARGET_DIR/debug/ovis-terminal --scenario graphics/read --isolated --case happy --evidence docs/evidence/task-85/qa-happy.json` | 0 | 1 case, 34 assertions. The default read has exactly the one placeholder `{image_id "4", kitty, row 1, col 2, rows 2, cols 4, bytes "145"}` and no `image_index`. `lines[1]=="ab"`, `lines[2]==""`. With `include_images: true` the read also has `image_index` with the same id and its primary placement. |
| `… --case failure --inject '{"read":{"include_images":"yes"}}' --evidence docs/evidence/task-85/qa-failure.json` | 0 | 1 case, 35 assertions. `include_images` given as `"yes"`, `1` or `null` is `invalid_params` with `field_path: params.include_images` and no result. The next read with `true` returns the placeholder and `image_index[0].image_id == "4"`. |
| `… --case all --evidence docs/evidence/task-85/conformance.json` | 0 | 2 cases, 68 assertions, failed 0 |
| `openrpc check` / `openrpc compat schema/released.openrpc.json`; `quality-matrix --mode contract` / `--mode verify` | 0 / 0 / 0 / 0 | fresh; compatible; `"verdict": "pass"`, `"finding_count": "0"` (`openrpc.log`) |
| `bun run generate` → `cargo run --locked -p ovis-terminal-replay --bin ovis-replay -- refresh` | 0 / 0 | 31 SDK files regenerated; `{"refreshed":12}` pins |
| client-ts gate (`bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test`) | 0 | 51 pass, 0 fail (`sdk.log`) |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` | 0 | 1595 tests run, 1595 passed, 2 skipped; workspace-hygiene 0 violations in 1566 files; json.sh 300 files; source-check and evidence-audit pass (`check.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy … -- -D warnings` | 0 / 0 | `windows-check.log` |
| `evidence-audit --tree docs/evidence --exemptions probes/quality/evidence-exemptions.json` | 0 | `finding_count 0` (`evidence-audit.log`) |

New tests and what they assert:
- `crates/ovis-terminal-vt/tests/image_reads.rs` (7 tests):
  - A Kitty `c=4,r=2` image is reported at (1,2) as 2x4, and its lines equal a text-only engine's.
  - Partial scroll-off reports only the visible rows; full scroll-off reports none.
  - Printing over an image removes its placeholder.
  - A footprint is clipped at the right edge and the bottom.
  - Sixel (1x1), iTerm2 inline `width/height` and a Kitty put are reported; a bare `a=t` transmission is not.
  - A primary image is never reported on the alternate screen, even under an alternate image on the same cells.
  - A footprint that is no longer intact is not reported. Four damage cases are checked: one covered cell printed over, one cell erased (ECH), a line inserted between its rows (IL), and one covered row erased to its end (EL).
- `crates/ovis-terminal-core/src/terminals/images/placeholders/tests.rs` (7 tests):
  - Placeholder ids equal the store keys, and `bytes` equals the store entry's size.
  - An image the store evicted has no placeholder while the VT still shows it.
  - An image scrolled off and synced out of the store has no placeholder.
  - An image whose bytes never reached the store has no placeholder.
  - Encoded placeholders never contain the payload.
  - The index lists a bare transmission (`placement: null`) and an image on the inactive screen (`buffer: primary`, while no placeholder is shown).
  - The index omits what the store evicted, and it never contains the payload.
- `crates/ovis-terminal-protocol/src/domain/graphics/tests/placeholders.rs` (6 tests):
  - Decimal-string `image_id`/`bytes` round-trip at u64::MAX.
  - A numeric id or an extra payload field is refused, for both the placeholder and the index entry.
  - `include_images` defaults to false.
  - `"yes"`, `"true"`, `1`, `null` and `[true]` are each `invalid_params` at `params.include_images`.
  - `images` and `image_index` round-trip, and are omitted when absent.

Mutants were each applied alone, run, then restored; the logs are committed:
- `mutant-evicted-image-keeps-placeholder.log`: 2 core tests fail.
- `mutant-declared-rows-not-visible-rows.log`: 2 vt tests fail.
- `mutant-include-images-required.log`: 1 protocol test fails.
- `mutant-alternate-screen-not-filtered.log`: the buffer check, re-run on the round 2 expression; 1 vt test fails.
- `mutant-follow-keeps-erased.log`: follow never releases an erased footprint; 2 vt tests fail, including the new non-intact test.
- `mutant-read-ignores-still.log` (gate N4): it **survives 84/84, because it is equivalent**. Every grid mutation (`Vt::feed`, `Vt::resize`) ends in `follow_images()`. That releases any placement whose footprint is no longer whole on the active screen, and restore starts with no images. So the read-time `anchor.still(grid)` could never decide an outcome. I removed that redundant guard. Reads now state the invariant they rely on, and the guard that decides (`follow`) is the one mutated in `mutant-follow-keeps-erased.log`.

Adversarial classes and the test guarding each are in `adversarial.log`.

macOS stress: lane `graphics-read` (`probes/platform-qa/stress/graphics-read.{json,sh}`), 25 iterations of the placeholder/index tests plus `graphics/read --case all`. The round 1 code head `37c854f` passed: https://github.com/devswha/ovis-terminal/actions/runs/41374308872 (success). The final head: STRESS_FINAL.

### GAP-152 (cursor model): recorded as still open

Todo 85 does not move the authoritative VT cursor past a placement. The reasons are in `docs/evidence/task-85/gap-152.md`, and the GAP-152 note in `docs/design/traceability.json` is updated to match.

First, moving the cursor changes where every later character lands. This block forbids changing cell semantics for non-image cells.

Second, the advance is computable only for extents declared in cells. For a Kitty image without `c`/`r`, an iTerm2 image with `auto` size, or a sixel, it depends on pixel and cell-pixel sizes that the headless runtime never learns.

### Records (gate N2)

The join lives where its data does: `ImageStore::placeholders` and `ImageStore::index` in `crates/ovis-terminal-core/src/terminals/images/placeholders.rs`. `crates/ovis-terminal-core/src/terminals/records.rs` is unchanged from integration. No current terminal or agent record snapshots the screen: `Record`/`TerminalSnapshot` carry no screen content, and todo 37 forbids synthesizing conversation records from terminal bytes. A future screen-snapshot record would carry `ImageStore::placeholders` output.

## Round history

- **Round 1 (gate REJECT, B1):** placeholders appeared only with `include_images: true`, so the default read still dropped the image. **Fixed:** the default read carries them, and `include_images` controls only the fuller `image_index`. N4 and N2 are covered above. The DoneClaim is in the PR description.
- **Merge with integration 1c1527d (T66):** the only conflict was the generated pins in `conformance/fixtures/ovis/manifest.json`. I took integration's side and regenerated them.
  - T66's committed `cli/manual` evidence pins `{"$equals_file":"schema/ovis-terminal.openrpc.json"}`. This PR's additive schema made that pin stale on the merged tree: evidence-audit reported `assertion-forged` at `docs/evidence/task-66/task-66-all.json#0` and `task-66-ovis-terminal-v1.json#0` (`check-merged-attempt1-t66-evidence-stale.log`).
  - I re-recorded both files on this PR's binary with T66's own invocations, and all three runs exited 0 (`t66-rerecord-*.log`).
- **Earlier check.sh attempts:** `check-attempt1-contract-stale.log` and `check-attempt2-pins-stale.log` failed on a stale quality contract and stale fixture pins. Both were regenerated, not hand-edited.

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-vt/src/engine/images/{anchor,set}.rs`: the placement entries are private to the images module. `Images::on_screen` and two anchor accessors (visible rows, clipped columns) feed `read.rs`.
- `crates/ovis-terminal-vt/src/{engine.rs,lib.rs}`: export `ScreenImage`.
- `crates/ovis-terminal-vt/tests/image_reads.rs`: new vt behaviour tests.
- `crates/ovis-terminal-core/src/screen/{views,reads}.rs`: `ScreenView` carries the frozen read's `images`.
- `crates/ovis-terminal-core/src/terminals/images.rs` and the new `images/placeholders.rs` + `placeholders/tests.rs`: the store-side join (gate N2).
- `crates/ovis-terminal-protocol/src/domain/graphics/placeholder.rs`: the placeholder and index types, a child of `graphics.rs` to stay under 250 LOC.
- `crates/ovis-terminal-protocol/src/schema/methods/terminal_read.rs`: `include_images`, `images` and `image_index`.
- `crates/ovis-terminal-protocol/src/domain/graphics/tests{.rs,/placeholders.rs}`: protocol tests.
- `crates/ovis-terminal-server/src/streams/graphics/replay.rs`: `TerminalStreams::image_placeholders` / `image_index` under the hub lock.
- `crates/ovis-terminal-cli/src/assembly/reads.rs`: `terminal.screen` adds `images` always and `image_index` on request. It stays an unrecorded Query read.
- `schema/ovis-terminal.openrpc.json`, `packages/client-ts/src/generated/**`, `conformance/ideal-map.json`, `conformance/fixtures/ovis/{manifest,predicates}.json`: regenerated (`openrpc write`, `quality-matrix --mode contract`, `bun run generate`, `ovis-replay refresh`).
- `conformance/src/runner_tests/suite.rs`: `graphics/read.json` registered in `ASSEMBLED` in sorted position.
- `docs/design/traceability.json`: the GAP-152 note.
- `docs/evidence/task-66/task-66-all.json` and `task-66-ovis-terminal-v1.json`: re-recorded, because their schema pin follows this PR's schema.
- `probes/platform-qa/stress/graphics-read.{json,sh}`: macOS stress lane.

## Risks

- The screen snapshot and the image store are read under separate locks. A release landing between the two drops that placeholder, which is conservative: a placeholder is never reported without its store entry, but a just-released image can be missing from a read whose lines are one feed older.
- `rows`/`cols` are cell footprints (declared extent clipped to the grid, one cell when undeclared), not decoded pixel geometry.
- After this lands, other open lanes whose committed evidence pins `schema/ovis-terminal.openrpc.json` via `$equals_file` need re-recording on the merged ref.

DEFERRED VERIFICATION: none. The runner executes `graphics/read` today.

## DoneClaim

DONECLAIM_JSON
