# feat(vt): keep a bounded placement-keyed image store and replay on-screen images on reattach

Todo 84. Closes GAP-152 (reattach loses images), delivers IDEAL-119. OS/arch: Linux x86_64. Base: integration `070871b`, merged with `1abcced` before the push, as the orchestrator instructed.

## Before

**alacritty keeps no image placements.** Each line below is quoted in `docs/evidence/task-84/before.log`:
- `grep -rniE 'sixel|graphic|image' alacritty_terminal-0.26.0/src` finds a single hit: `grid/mod.rs:41: /// Currently configured graphic character sets.` There is no image type, store or placement anywhere in the emulator.
- `vte-0.15.0/src/lib.rs:377`: `0x5E..=0x5F => self.state = State::SosPmApcString`, and `lib.rs:182`: `State::SosPmApcString => self.anywhere(performer, byte)`. A Kitty APC body never reaches a `Perform` callback.
- `vte-0.15.0/src/ansi.rs:1311-1326`: `hook`/`put`/`unhook` only `debug!("[unhandled hook] …")`. A sixel `DCS q` is dropped.
- `vte-0.15.0/src/ansi.rs:1523`: `_ => unhandled(params)`. Any OSC that alacritty does not consume, including `1337;File=`, is dropped.
- `alacritty_terminal-0.26.0/src/term/cell.rs:12-35`: a `Cell` has no image field. Bit 15 of `Flags` is undefined; this PR uses it as the anchor mark.
- `crates/ovis-terminal-vt/src/engine.rs:124` (base): the engine enables only `kitty_keyboard: true`. It has no graphics state.

**Reattach transcript with no images.** The new socket test, run on unchanged base `070871b`: `cargo nextest run -p ovis-terminal-server --locked --test streams replay_an_on_screen` gives FAIL with `Error: "kitty image not replayed"`. The late viewer's whole paint is 233 bytes, decoded in `before.log`. It contains the text `top` and `shown` but no `ESC _ G` and no `DCS … q`. The images were on screen and herdr-core had sent them, but the reattached viewer gets none.

## After

How it works:
- **VT** (`crates/ovis-terminal-vt/src/engine/images/`):
  - The tracker reads each raw byte with the attach filter's boundary rules (`images/scan.rs`). It reads headers only: Kitty keys `a d i I r c m` up to the gate's 256-byte limit, iTerm2 `inline/width/height`, and sixel when `q` follows fewer than 64 parameter bytes. A graphics sequence is keyed by the raw offset of its ESC. For a Kitty series this is the offset of its first chunk, even when other output interrupts the series. That is the same key the gate uses for the unit it releases.
  - When the sequence completes, the engine places it on the cursor cell at receipt. It marks that cell with flag bit 15 and follows the row by the address of its cell buffer, which moves with the row through scrolling and region swaps. The row and column covered come from the header (Kitty `r=`/`c=`, iTerm2 `width/height` in cells); without them the image covers one placeholder cell.
  - A placement is released `scrolled_out` when its row leaves the visible screen, and `erased` when its anchor cell is printed over, erased or recycled (alacritty resets the cell and its mark), or when its alternate screen is left.
  - A placement is also released `deleted` by Kitty `a=d` (id, number, or all; lower case keeps `a=t` data, upper case removes it), `replaced` when a transmission reuses its id, `reset` on RIS, and `limit` past 1024 images.
  - `FirstPaint.images` names where each image still shown is re-sent: after its screen's rows and before the cursor and modes are restored, with a CUP to its anchor. Primary images under an active alternate screen are re-sent inside the primary part of the paint. Payloads are never read.
- **Core** (`crates/ovis-terminal-core/src/terminals/images/`):
  - `ImageStore` keeps the bytes of each released unit that the engine still retains, as its own entries sized by their bytes against `graphics_terminal_store_max_bytes` (32 MiB). It evicts oldest first.
  - Each eviction returns the todo-86 typed `GraphicsEvictedData` (`store_full`, `evicted_sequences`, `evicted_bytes`, `retained_bytes`, `limit_bytes`).
  - Nothing goes to the ops ledger or SQLite, and nothing is decoded.
- **Server** (`crates/ovis-terminal-server/src/streams/graphics/replay.rs`):
  - The gate now reports every unit it releases. The hub offers each unit to the store, places a `graphics_evicted` control record after the sequence that caused an eviction, and syncs the store with the engine.
  - On attach and on resync, the paint is sent as pieces that share the store's `Arc` bytes, byte-identical and in receipt order. One `graphics_replayed` control record follows, with `replayed` (raw offsets) and `released` (the last 64, each with its reason).
  - Replayed bytes are bounded by the store and are not charged to the 4 MiB stream backlog or the session budget, so a large replay cannot trigger a gap and checkpoint storm.
- **Schema:** `ControlKind::{GraphicsEvicted, GraphicsReplayed}`, `AttachControl.{graphics_evicted, graphics_replayed}` (optional), and `GraphicsReplayedData`, `GraphicsReleased` and `GraphicsReleaseReason` (closed objects, open reason enum). The change is additive: `openrpc compat schema/released.openrpc.json` returns `{"compatible_with":"schema/released.openrpc.json"}`. The schema diff is +122 lines. The TS SDK was regenerated with `bun run generate`; every generated file changed because each embeds the schema sha256.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-vt -p ovis-terminal-core -p ovis-terminal-server --locked` | 0 | 687 tests run, 687 passed (`nextest.log`). 51 of them are new or touched graphics, replay and log tests. |
| (acceptance) image received then scrolled off, not replayed | 0 | `replay_a_scrolled_off_image_is_not_replayed`: no `ESC _ G` in the late paint; `graphics_replayed.replayed == []`; `released == [("0","scrolled_out")]`. VT: `the_anchor_follows_scrolling_and_leaves_with_the_visible_screen` (row 2→1→0, then `ScrolledOut`). |
| (acceptance) image on screen, replayed byte-identical after a reattach | 0 | `replay_an_on_screen_image_is_replayed_byte_identical_after_a_reattach`: a 2-chunk Kitty PNG and a sixel are found byte-identical, Kitty before sixel and both before live bytes. They are preceded by `ESC[2;1H` / `ESC[3;1H`. `replayed` lists both offsets and `released` is empty. The late viewer's grid and cursor equal those of a VT fed every raw byte. |
| (acceptance) clear screen, none replayed | 0 | `replay_a_cleared_screen_replays_no_image`: neither image is re-sent. Reasons are `["scrolled_out","erased"]`: alacritty's ED 2 moves the top row into scrollback (`Grid::clear_viewport`) and resets the rest. |
| (acceptance) Kitty `a=d`, that id gone | 0 | `replay_a_kitty_delete_removes_only_the_named_image`: after `a=d,d=I,i=1`, image 2 is re-sent, image 1 is absent, and `released == [(offset_of(1),"deleted")]`. |
| (acceptance) store at cap, oldest evicted, typed count in the stream notice | 0 | `replay_a_full_store_evicts_the_oldest_with_typed_counts`: the live viewer gets exactly one `graphics_evicted` `{reason:"store_full", evicted_sequences:"1", evicted_bytes:"<len>", retained_bytes:"<2·len>", limit_bytes:"<cap>"}`. A late viewer gets images 2 and 3, and image 1 is listed `store_full`. Core: 7 store tests (oldest first, a unit over the cap is never retained, a growing series is evicted whole, runs join their first run, sync reasons, the 64-entry release list). |
| `cargo nextest run -p ovis-terminal-vt --locked` (graphics) | 0 | 17 image tests: header extent and placement; byte-at-a-time chunking equals whole; an interrupted series keeps one key; region scroll moves a surviving anchor and releases a recycled one; ED/EL/ECH/print erase only the anchor; delete by id, number and case; id reuse; RIS; lookalikes (iTerm2 `inline=0`, `SetUserVar`, DECRQSS, `a=q`, 64-parameter sixel); alternate screen; the 1024 limit; paint splices rebuild the same placements and screen in a fresh engine. |
| `conformance --binary $CARGO_TARGET_DIR/debug/ovis-terminal --scenario graphics/replay --isolated --case happy --evidence docs/evidence/task-84/qa-happy.json` | 0 | 1 case, 40 assertions, real `ovis-terminal serve`. A shell draws a 3-chunk Kitty f=100 PNG and a sixel while viewer 1 watches. Viewer 1 sends `attach.close`. Viewer 2 attaches interact, and its transcript contains both sequences by SHA-256 (`graphics/in_order` true). Controls are exactly `checkpoint, graphics_replayed, input_ready`, with `replayed == ["2","142"]` and `released == []`. |
| `… --case failure --inject '{"graphics":{"scroll_rows":"200"}}' --evidence docs/evidence/task-84/qa-failure.json` | 0 | 1 case, 42 assertions. After 200 lines, viewer 2's transcript contains no `ESC _ G` and no sixel. `graphics_replayed.replayed == []` and `released == [{"output_offset":"2","reason":"scrolled_out"},{"output_offset":"142","reason":"scrolled_out"}]`. |
| `… --case all --evidence docs/evidence/task-84/conformance.json` | 0 | 2 cases, 80 assertions, failed 0, skipped 0 |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (merged tree `53f1e17`) | 0 | 1524 tests run, 1524 passed, 2 skipped; hygiene 0 violations in 1525 files; json.sh 270 files; source-check and evidence-audit pass (`check.log`). Attempt 1 failed after nextest passed: the doctest build hit `Disk quota exceeded (os error 122)` on the shared `/dev/shm` user quota (`check-attempt1-quota.log`). This lane's 11G RAM target was wiped, and attempt 2 passed from cold. |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` / `cargo clippy … -- -D warnings` | 0 / 0 | `windows-check.log` |
| `cargo clippy -p ovis-terminal-vt -p ovis-terminal-core -p ovis-terminal-server -p ovis-terminal-protocol -p ovis-terminal-conformance --all-targets --all-features --locked -- -D warnings` | 0 | `clippy.log` |
| `openrpc write/check schema/ovis-terminal.openrpc.json`; `openrpc compat schema/released.openrpc.json`; `quality-matrix --mode contract` then `--mode verify` | 0 | fresh; compatible; verify `"findings": []`, `"verdict": "pass"` (pre- and post-merge) |
| `cd packages/client-ts && bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | 47 pass, 0 fail (`sdk.log`, post-merge; the merge needed no regeneration) |

Mutants: each was applied alone, run, then restored; the logs are committed. All 8 fail a shipped test:
- `mutant-scrolled-out-never-released.log`: 4 fail, including both scrolled-off tests and the cleared-screen test.
- `mutant-store-evicts-newest-first.log`: 2 fail (core cap test, socket eviction test).
- `mutant-kitty-delete-ignores-id.log`: 2 fail.
- `mutant-series-not-carried.log`: the aggregate-cap test fails.
- `mutant-replay-not-spliced.log`: 4 fail.
- The three T83 survivors named in that PR's seams now fail: `mutant-t83-osc1337-any-is-iterm2.log`, `mutant-t83-kitty-header-32.log` and `mutant-t83-sixel-params-8.log`, each 1 failure.

Adversarial classes and the test guarding each: `adversarial.log` (executor-run).

Owned seams from T83:
- (a) The aggregate cap now spans a Kitty series that same-terminal text interrupts. The gate carries the series start and its byte tally to the next Kitty unit, so one notice stands at the series start and the dropped series leaves the store. Test: `an_interrupted_kitty_series_is_one_key_measured_against_the_cap_together`.
- (b) With no viewer attached, `OutputLog::trim` drops notices at `<=`. Test: `a_notice_at_the_end_is_trimmed_only_when_no_viewer_is_attached`.
- (c) The three mutants listed above.

macOS stress: lane `graphics-replay` (`probes/platform-qa/stress/graphics-replay.{json,sh}`, 25 iterations of the image/replay tests plus `graphics/replay --case all`). The run URL is in the PR comment and the DoneClaim.

## Round 2: CI fix for `graphics/replay` failure inside `all/all`

**What failed.** On PR #81 at `fe06883`, ubuntu `scripts/check.sh` failed in `suite::every_scenario_file_is_executed_or_a_listed_declaration` (`all/all`). The failure was `graphics/replay` case `failure`, step 20: `expectation graphics/controls did not hold`. Step 20 is the second viewer's transcript, which expects exactly `checkpoint, graphics_replayed, input_ready`.

**Cause, reproduced deterministically.** The store admitted a released unit only while the VT engine still retained its image. When the PTY hands over the image and the lines that scroll it off in one read (which a loaded runner does), the engine has already released the image by the time the gate releases its bytes. The store never saw it, so nothing was listed as released and no `graphics_replayed` record was sent. What a reattach reported therefore depended on how output was split into reads. `replay_a_scrolled_off_image_is_not_replayed` now publishes the image and the 12 lines in one read. Before the fix it fails with `Error: "no graphics_replayed record"` (`r2-before.log`).

**Fix (structural, no waits).** `ImageStore::admit` records, once per key, the engine's release reason for a unit whose image the engine already released. A sequence the engine never retained (for example Kitty `a=q`) is still not listed. New core test: `an_image_released_before_its_bytes_arrive_is_still_reported_once`.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-core -p ovis-terminal-server --locked --no-fail-fast images replay` | 0 | 31 run, 31 passed (`r2-after.log`) |
| 8 `nice -n 10` busy loops (pids captured, killed after), then 10 × `conformance --scenario graphics/replay --isolated --case failure --inject '{"graphics":{"scroll_rows":"200"}}'` | 0 | 10/10 passed |
| under the same load: `with-slot.sh cargo nextest run -p ovis-terminal-conformance --locked suite::every_scenario_file_is_executed_or_a_listed_declaration` | 0 | 1 passed, 95.7 s (`r2-suite-under-load.log`) |
| `with-slot.sh bash scripts/check.sh` (attempt 2) | 0 | 1525 run, 1525 passed, 2 skipped; hygiene 0 violations in 1525 files (`r2-check.log`). Attempt 1 failed only on the unrelated known flake below |
| `RUSTFLAGS=-Dwarnings cargo check … x86_64-pc-windows-gnu` / `cargo clippy …` | 0 | `r2-windows-check.log` |

Unrelated known flake: round 2 check.sh attempt 1 failed only in `recovery/e2e` happy, step 36 (`reply[child-on-peer]/error`: `internal`, "operation failed"). In that same run `graphics/replay` passed both cases, and 83 of 84 cases passed (`r2-check-attempt1-recovery-flake.log`). Step 36 is the runner fixture's `fixture.remote_child`, the two-clock-read race that F2a root-caused and fixed in PR #80 (`conformance/src/fixture/machines/remote.rs` `stamped()`, merge pending). It is not a task-84 defect.

## Round 2 addendum: wire-contract test and integration 60076f7

This adds tests and changes no behaviour. The binding contract requires a u64::MAX decimal round-trip test for every u64 offset on the wire. The new `GraphicsReplayedData.replayed` and `GraphicsReleased.output_offset` had none.
- New file `crates/ovis-terminal-protocol/src/domain/graphics/replay/tests.rs`:
  - `offsets_round_trip_u64_max_as_decimal_strings`: both fields serialize as `"18446744073709551615"` and decode back equal.
  - `numeric_offsets_and_extra_fields_are_refused`: a numeric `replayed` or `output_offset`, or an extra field on either object, fails to decode.
  - `a_release_reason_added_later_decodes_as_unknown`: the open enum keeps `"evicted_by_policy"` as `Unknown`.
- `mutant-replayed-data-open-object.log`: with `#[serde(deny_unknown_fields)]` removed from `GraphicsReplayedData`, 14 run, 13 passed, 1 failed (`numeric_offsets_and_extra_fields_are_refused`).
- Merge `ba5e396` of integration `60076f7` (the F2a remote-fixture clock-race fix): no conflicts. `cargo metadata --locked` is OK. The schema is unchanged: `openrpc check` reports it fresh, and quality-matrix verify passes. SDK `generate:check` reports 31 files fresh, and typecheck is OK.

| Command | Exit | Result |
|---|---|---|
| `cargo nextest run -p ovis-terminal-protocol --locked --no-fail-fast graphics` | 0 | 12 run, 12 passed, including the 3 new tests (`r2-addendum-protocol.log`) |
| `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` (merged tree) | 0 | 1529 run, 1529 passed, 2 skipped; hygiene 0 violations in 1527 files (`r2-addendum-check.log`) |
| `RUSTFLAGS=-Dwarnings cargo check ... x86_64-pc-windows-gnu` / `cargo clippy ... -- -D warnings` | 0 / 0 | `r2-addendum-windows-check.log` |

macOS stress (lane `graphics-replay`):
- Run https://github.com/devswha/ovis-terminal/actions/runs/41357513948 at `fe06883` (the pre-fix head): iterations 1-21 passed, and iteration 22 failed 1 of 2 `graphics/replay` cases (78 of 80 assertions recorded). The artifact uploads only the lane log, so the failing step cannot be read from it. It is consistent with the round-2 coalesced-read race, but that is not proven.
- Re-dispatched once on the fixed head: https://github.com/devswha/ovis-terminal/actions/runs/41361113759 at `2c5e277`, `RESULT 25/25`.

Independent review: the orchestrator ran the terra (gpt-6.1) adversarial review on `2c5e277`. Its two findings in the VT anchor model are with the gate for a ruling, and both were already disclosed under Risks:
1. Writes inside an image's footprint but outside its anchor cell do not expire it.
2. An image is dropped when its anchor row scrolls off, even if a lower row is still visible.

Deviation: the block says to expire an entry "when its rows scroll out of the scrollback window". This PR expires it when its anchor row leaves the visible screen. The attach paint re-sends visible rows only, so an image whose anchor is in scrollback has no position a paint can place it at. This is stricter than the block and is the same question as finding 2.

## Seams

Files outside the write scope, and why:
- `crates/ovis-terminal-vt/src/engine.rs`: `mod images;`, the `images` field, applying image cuts in `feed` and following after `feed`/`resize`, and re-exports. `crates/ovis-terminal-vt/src/lib.rs`: re-exports. `engine/tracker.rs`: the scanner sees every raw byte and adds a cut where a graphics sequence completes; the loop body moved into `consume` without changing its behaviour. `engine/paint.rs`: the `FirstPaint.images` splice points, `#[serde(skip)]`.
- `crates/ovis-terminal-core/src/terminals.rs`: `pub mod images;`. `crates/ovis-terminal-core/src/screen/reads.rs`: `Screen::images()`, an unrecorded read under the screen lock.
- `crates/ovis-terminal-protocol/src/domain/graphics.rs` (+ new child `graphics/replay.rs` and its `replay/tests.rs`) and `schema/records/stream.rs`: the wire slots for the two records. `schema/ovis-terminal.openrpc.json`, `packages/client-ts/src/generated/*` and `conformance/ideal-map.json` were regenerated (`openrpc write`, `bun run generate`, `quality-matrix --mode contract`), not edited by hand.
- `crates/ovis-terminal-server/src/streams/{graphics.rs, graphics/{held,unit,backlog}.rs}`: the released-unit report and the carried Kitty series (seam a); the replay bytes are not charged to the backlog. `streams/{hub,log,viewer,open,resync,ops}.rs`:
  - The store lives in `HubState`.
  - Log notices are dropped or evicted, and trim is `unwatched` (seam b).
  - Viewer paints go out as pieces, with `replay_pending`/`charged_backlog`.
  - The replay record is queued after the paint on attach and resync.
  - The `trim` call site in `ops.rs` changed.
- `crates/ovis-terminal-server/tests/streams.rs`: registers the 5 socket replay tests in `graphics/replay/qa.rs`.
- `conformance/src/fixture/graphics.rs`: the `graphics_transcript` step gains `in_order` and `replayed`. `conformance/src/runner_tests/suite.rs`: `ASSEMBLED += graphics/replay.json` (sorted; it was never in `declared.json`). `docs/design/traceability.json` already mapped GAP-152 and IDEAL-119 to `graphics/replay` and todo 84, so it is unchanged.
- `probes/platform-qa/stress/graphics-replay.{json,sh}`: the macOS stress lane. The workflow resolves lanes by file, so it is not edited.
- Merge `53f1e17` of integration `1abcced` (T70 soak harness): no conflicts. Cargo.lock is integration's and `cargo metadata --locked` is OK. The schema regenerated byte-identically, the SDK was fresh, and the exemptions came in from integration unchanged.

## Risks

- **The replay is at the VT's anchor, not the client's cursor model.** alacritty does not move the cursor for an image, while xterm.js/kitty do. Two images printed back to back therefore share an anchor in the VT, and the replay draws the second at the first one's cell, where the live client drew it below. Text after an image is placed by the VT's model both live and on replay, so this inherits the existing divergence between the VT grid and the client. A sixel or iTerm2 image re-sent at the bottom rows may also make the client scroll the painted grid.
- **Only the anchor cell decides liveness.** The footprint from the header is reported in `ImagePlace` (rows and columns). A clear that covers the footprint but not the anchor keeps the image, and text printed into the footprint keeps it too. This was chosen because the VT cursor stays inside the image after a Kitty `a=T`, so marking the footprint would release almost every image on the next prompt.
- **Images whose anchor row scrolled off but whose lower rows are still visible are released.** A paint can only position from visible rows.
- **Kitty commands are re-sent byte-identical, so `i=` without `q=` makes a client answer again.** An interact client could forward that answer as input. The bytes are not rewritten, because the contract says byte-identical.
- **The store is in memory only.** A planned control restart or a recovery from checkpoint starts with an empty store and replays nothing; a restored engine has no placements. Persisting would need its own store entries, and is left out.
- **No `graphics.evicted` / `graphics.dropped` event is committed to the event log.** Both are reported only as in-stream records, as T83 left them.
- **Flag bit 15 is undefined in alacritty 0.26** (`the_mark_is_a_flag_bit_alacritty_never_defines` guards it). An alacritty upgrade that defines it would fail that test.
- This lane's own adversarial pass was executor-run (`adversarial.log`, 9 mutants). The orchestrator ran the independent terra review (see Round 2 addendum).

DEFERRED VERIFICATION: none. The runner executes `graphics/replay` against the real binary, both cases, exit 0, failed 0, skipped 0.
