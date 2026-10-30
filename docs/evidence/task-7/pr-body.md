# feat(checkpoint): build the pure rust vt state engine

Plan todo 7 (wave 2). Closes GAP-066. Delivers the engine-level slice of IDEAL-032, IDEAL-035, IDEAL-036, IDEAL-046 and IDEAL-047; end-to-end acceptance waits for the attach and read lanes and the Todo 21 runner.

`ovis-terminal-vt` wraps `alacritty_terminal` 0.26 (no ghostty-vt, no Zig). It provides:

- `Vt::feed`: stream-located query replies (`TerminalQuery`) and bounded OSC records (`OscRecord`).
- Structured, revisioned modes (`TerminalModes`, `ModeRevision`): bracketed paste, mouse tracking and encoding, DECCKM/DECKPAM, kitty flags (per-screen stacks), modifyOtherKeys, DEC 9001 win32 input, alternate screen, DECOM/DECAWM/IRM/LNM, 2026.
- Offset-fenced checkpoints (`export_checkpoint` / `Checkpoint::{to_bytes, from_bytes}` / `Vt::from_checkpoint`). These carry both screen buffers with history and both cursors, plus the state alacritty keeps private: tab stops, scroll region, title stack, kitty stacks, charset, REP character. They also carry the bytes of a sequence still open at the offset, or a typed `InexactReason` when those bytes cannot be replayed.
- `first_paint` for late attach.
- Non-mutating reads (`screen`, `history`, revision-guarded `search` / `selection_text`).

## Before

- RT-HC-05 / GAP-066: `docs/evidence/baseline/smoke/hc-s09-framed-attach.log:15`, `[stream ch1 DATA 672B] "\u001b[!p\u001b[?2026l ... \u001b[?2004h ..."`. hc's first paint is "keeper, exactness must be proved", and it forces `ESC[?2004h` (bracketed paste) on every attach regardless of the program's mode. That line's 300 logged bytes are committed as `probes/vt/hc-s09-first-paint-prefix.json` (source path, line and sha256 included). `tests/modes.rs::herdr_attach_preamble_shows_the_bracketed_paste_it_forces` replays it and asserts the forced `PasteMode::Bracketed`. The same test asserts that our first paint of the same screen keeps `PasteMode::Plain`.
- Original source of that transcript: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s09-framed-attach.log:15`.

## After

All commands ran on Linux x86_64 with rustc/cargo 1.99.1 (local isolated toolchain).

| Command | Exit | Result |
|---|---|---|
| `bash scripts/check.sh` | 0 | fmt, workspace clippy `-D warnings`, nextest (44 tests, 44 passed), doctests, hygiene (0 violations in 81 files), manifests, json ([check.log](check.log)) |
| `cargo nextest run -p ovis-terminal-vt --locked` | 0 | 33 tests run, 33 passed ([nextest-vt.log](nextest-vt.log)) |
| `cargo clippy -p ovis-terminal-vt --all-targets --locked -- -D warnings` | 0 | [clippy-vt.log](clippy-vt.log) |
| `RUSTDOCFLAGS="-D warnings" cargo doc --no-deps --all-features --workspace --locked` | 0 | |
| `PROPTEST_CASES=4000 cargo test -p ovis-terminal-vt` | 0 | every binary passes; the paint property was stress-run several times at 4000 cases while hardening |
| `cargo test -p ovis-terminal-vt --test checkpoint` before any implementation (TDD red) | 101 | unresolved imports ([tdd-red.log](tdd-red.log)) |
| `target/debug/ovis-terminal --version` | 0 | `ovis-terminal 0.1.0` |

Test binaries:

| Binary | Tests |
|---|---|
| unit `paint_equivalence` | 5 |
| `adversarial` | 7 |
| `checkpoint` | 2 |
| `modes` | 3 |
| `properties` | 3 |
| `queries` | 5 |
| `reads` | 4 |
| `wire` | 4 |

### DEFERRED VERIFICATION (runner lands in Todo 21)

The plan's real-binary command was run as written; the output is copied verbatim from [conformance-attempt.log](conformance-attempt.log):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-7 --isolated --case happy --evidence "$ATTEMPT_DIR/task-7-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit=2
$ uname -sm
Linux x86_64
```

Conformance execution is deferred by plan (plan line 275) for part-A foundational todos until Todo 21 lands. The binary has no VT socket surface yet: attach and first paint arrive with Todos 17, 23 and 31. `conformance/scenarios/part-a/task-7.json` records the scenario with the shell runner used by task-1, so it currently runs the crate's QA tests. [task-7-ovis-terminal-v1.json](task-7-ovis-terminal-v1.json) is produced by this lane, not by the runner. It holds the exit-2 attempt plus the parsed JSON emitted by both QA tests.

## QA

### happy

Command: `cargo test -p ovis-terminal-vt --test checkpoint happy -- --exact --nocapture`, exit 0 ([qa-happy.log](qa-happy.log)).

- **Before the checkpoint:** a live engine is fed 857 bytes in 3-byte chunks. They contain a title, bracketed paste, kitty push, modifyOtherKeys 2, DEC 9001, mouse 1002+1006, custom tab stops, 24 lines of colored wide and combining text, DECSTBM, DECSC and 1049h with content, and they end inside `ESC [ 3`.
- **Checkpoint and import:** the test asserts the checkpoint is `Fidelity::Exact` with pending bytes `ESC [ 3`. It round-trips the checkpoint through `to_bytes`/`from_bytes` and imports it.
- **After the checkpoint:** 63 more bytes are fed: 2-byte chunks to the live engine, one chunk to the restored one. They complete the CSI, leave the alternate screen, issue DECRC and a `CSI 6n` query, include a tab, OSC 7501 and a split UTF-8 character.
- **Asserted:**
  - `restored.export_checkpoint() == live.export_checkpoint()`: full logical grid, history, both cursors, mode bits, palette and private state.
  - Equal cursor, `modes()`, content revision, mode revision and offset.
  - Identical query replies at identical offsets (exactly one, `"887"` → `ESC[1;6R`).
  - The visible rows prove DECRC and completion of the split UTF-8 character.

Parsed output (abridged): `"checkpoint_offset":"857"`, `"offset":"920"`, `"revision":"456"`, `"mode_revision":"8"`, `"pending_bytes":"\u001b[3"`, `"paste":"bracketed"`, `"kitty_flags":1`, `"modify_other_keys":"all"`, `"win32_input":true`, `"restored_equals_live":true`.

### failure

Command: `cargo test -p ovis-terminal-vt --test checkpoint failure -- --exact --nocapture`, exit 0 ([qa-failure.log](qa-failure.log)).

- `Status: working` / `Approve? [y/n]` stay screen lines and produce no record.
- `OSC 75x1;state=working` becomes `OscContent::Malformed { fault: invalid_number }`, with no fields kept.
- A 5023-byte OSC 7501 becomes `OscContent::Oversize { number: 7501, bytes: 5023 }`.
- An unterminated `OSC 7501;state=working` yields no record and is carried as exact pending bytes.
- A split `E4 B8` UTF-8 sequence is carried as pending bytes.
- Checkpoint `version: 0` and a foreign `engine` are rejected with `kind() == "checkpoint_incompatible"`; the hint reads "discard the checkpoint and rebuild the screen from retained output in a fresh engine".
- Truncated JSON is rejected with `checkpoint_malformed`.

Additional behavior tests:

- **Chunking independence:** `properties::chunking_never_changes_state` (proptest over control-heavy streams and every chunk size 1-8).
- **Checkpoint exactness at an arbitrary byte boundary:** `properties::checkpoint_at_any_boundary_is_exact_or_says_why`. Either the checkpoint is exact and the restored engine equals the live one, or it reports `ControlInsideOpenSequence` / `PrimarySavedCursorUnrepresentable` and the restored engine keeps that fidelity.
- **First-paint equivalence:** `paint_equivalence::*`. A paint fed to a fresh engine reproduces the visible rows of both screens cell for cell, plus cursor, saved cursor, mode bits, structured modes, tabs, region, title, kitty stack, charset, cursor style and palette. Covered: an arbitrary-stream proptest, alt screen, resize, and a declared `SavedCursorNotPaintable` loss.
- **Query ownership:** `queries::replayed_queries_are_not_answered_twice` (ownership fence suppresses replayed queries). The other `queries::*` tests cover a split query answered once at its final byte, and client-only facts (`NeedsClientFact`) that are never invented.
- **Mutation checks during development:** dropping tab-stop restore failed `checkpoint_at_any_boundary_is_exact_or_says_why`, and dropping pending-byte replay failed `happy`.

## Adversarial

| Class | Observable |
|---|---|
| malformed input: truncated OSC / UTF-8 | Unterminated OSC: no record, exact pending bytes. Split UTF-8: pending bytes, completed after import. Malformed or oversize OSC: typed `Malformed` / `Oversize` (`checkpoint::failure`, `adversarial::osc_payload_at_the_limit_is_kept_and_one_byte_more_is_oversize`). |
| prompt injection: fake working/approval text | Stays `ScreenLine` text. No OSC record, no query, no mode change (`checkpoint::failure`, `adversarial::fake_status_and_paste_text_are_only_screen_text`). The VT crate has no status concept at all; OSC 7501 is forwarded as raw fields for the Todo 33 owner. |
| stale state: checkpoint version | `checkpoint_incompatible` for version 0 and for a foreign engine. A geometry contradicting the grid gives `checkpoint_inconsistent`. One column, or a numeric u64, gives `checkpoint_malformed` (`adversarial`, `wire`). |
| resource exhaustion | 5000 kitty pushes are refused past depth 4096, with no panic. Open-sequence bytes are capped at 64 KiB, beyond which the reason is `OpenSequenceTooLong`. OSC records are capped at 4096 bytes, beyond which they are `Oversize`. |
| clipboard exfiltration | OSC 52 is disabled in the emulator, and emulator-owned OSC numbers never become records (`adversarial::emulator_owned_osc_numbers_never_leak_as_records`). |

## Contract checklist

1. **u64 as decimal strings.** `OutputOffset`, `ContentRevision` and `ModeRevision` serialize through `engine/decimal.rs` as canonical decimal strings. A JSON number, sign, leading zero, empty string or overflow is rejected. Tests: `wire::u64_max_round_trips_exactly_as_a_string` (`"18446744073709551615"`), `wire::numeric_and_non_canonical_forms_are_rejected`, the proptest `wire::every_u64_round_trips`, and `wire::checkpoint_with_a_numeric_offset_is_malformed`. A numeric form fails here as `checkpoint_malformed`; mapping that to the wire's `invalid_params` belongs to the protocol layer. Other u64-sized values were narrowed to JSON-safe `u16`/`u32`: palette indexes, and the `Oversize` byte count, which saturates at `u32::MAX`. Exception: the embedded alacritty grid serialization (inside `Checkpoint`, gated by the `engine` identity) keeps alacritty's own integer encoding, which this crate does not control.
2. **Errors.** `VtError` is a single `#[derive(Debug, thiserror::Error)]` `#[non_exhaustive]` enum with `kind()` and `hint()` returning stable `&'static str`, and `#[source]` on both serde-error variants. Display text never repeats the kind token. There are no hand-written `Error` impls.
3. **Distinct newtypes.** Offsets, content revisions, mode revisions, geometry and history limit are distinct newtypes. `RevisionMismatch` carries `ContentRevision`, not u64. `Option` appears only where absence is real: the restored-from offset, the title, an unparsed OSC number, and preceding characters.
4. **Tests assert behavior.** Equality is checked between live and restored engines, paint-replayed state, typed error kinds and offsets derived from the input bytes (`queries::end_of`), not locally assigned literals. proptest covers checkpoint round trips under every split, chunking, the u64 encoding and first paint. Minimized regressions are committed.
5. **No suppression or placeholders.** There are no `#[allow]` attributes, no crate-level `#[expect]`, no TODOs and no prose-pinning tests. The one scoped `#[expect(clippy::trivially_copy_pass_by_ref)]` sits on `decimal::serialize`, whose `&u64` signature serde's `with` attribute fixes. `redundant_pub_crate` is satisfied structurally: every module is a child of the private `engine` module, so `pub(super)` items are engine-visible only, and public items are re-exported by name.
6. **Evidence matches the tests.** This body states what each test asserts. Conformance is reported as deferred, with the exit-2 attempt recorded verbatim.

Bounded queues and buffers:

- The event queue is drained per parsed segment.
- `FeedOutcome` vectors are bounded by the chunk the caller fed.
- Open-sequence bytes are capped at 64 KiB, beyond which the typed reason is `OpenSequenceTooLong`.
- OSC payloads are capped at 4096 bytes, beyond which the typed outcome is `Oversize`.
- Title and kitty stacks are capped at depth 4096, beyond which kitty pushes are refused.

## Upstream findings (alacritty_terminal 0.26)

1. `push_keyboard_mode` past depth 4096 evicts from the **title** stack (`title_stack.remove(0)`), which panics when that stack is empty. Guard: `recorder.rs` refuses pushes at the depth limit. Test: `adversarial::kitty_stack_overflow_is_refused_instead_of_crashing_the_engine`.
2. Printing a wide character into a one-column grid panics (`grid/mod.rs:439`). Guard: `Geometry::new` and `Geometry` deserialization require `MIN_COLUMNS = 2` and return the typed `invalid_geometry` / `checkpoint_malformed`. Tests: `adversarial::degenerate_geometry_is_a_typed_error`, `adversarial::narrowest_grid_survives_wide_characters`.
3. `PartialEq for Grid` asserts an unrotated ring buffer (`storage.rs:58`) and panics on any grid that has scrolled. Guard: checkpoints compare grids logically, row by row with matching dimensions, history and offset, and exported grids are `truncate()`d to a canonical compact layout (a 200-line checkpoint went from 3.7 MB to 131 KB).

## Out-of-scope edit (orchestrator-granted)

`clippy.toml` gains exactly one line: `allowed-duplicate-crates = ["windows-sys"]`. The plan mandates `alacritty_terminal` 0.26, which depends on `windows-sys` 0.59 directly, while its own dependencies need 0.61:

- `miow` 0.6.1, required by `alacritty_terminal`
- `polling` 3.11, required by `alacritty_terminal`
- `errno` 0.3.14, through `rustix` 1.1, required by `alacritty_terminal`, `polling` and `rustix-openpty`
- `home` 0.5.12, required by `alacritty_terminal`
- `signal-hook-registry` 1.4.8, through `signal-hook` 0.4, required by `alacritty_terminal`

No lockfile pin unifies them; `miow` 0.6.0 would pull in `windows-sys` 0.48 instead. `clippy::multiple_crate_versions` therefore failed workspace clippy in every crate depending on `ovis-terminal-vt` (core, probes, bench and others). `Cargo.lock` changes because adding the dependency requires it.

## Risks

- **Checkpoint size and format.** Checkpoints are JSON. A 200-line history is about 131 KB; 10,000+ lines will be MB-scale. Compact encoding and storage cadence belong to the recovery and store lanes (Todos 8 and 49). The format is tied to alacritty's grid serialization by `CHECKPOINT_ENGINE`, so an alacritty bump requires bumping that constant, which makes old checkpoints `checkpoint_incompatible`, never misread.
- **Trust in checkpoint input.** Import validates geometry, cursor ranges, stack depths, palette indexes, pending size and history bounds. alacritty's internal ring-buffer fields cannot be validated through its public API, so checkpoints must come from the core's own store (CRC-guarded per plan), not from untrusted peers.
- **Unbounded vte buffering.** vte (std) buffers an unterminated OSC/DCS without bound inside the emulator's parser. This crate caps what it keeps and reports, but bounding the parser's own memory needs the scheduling and budget work of Todo 56.
- **Synchronized update.** Mode 2026 is applied immediately (state is never held back) and only reported as a mode. Attach streams still carry the raw bytes.
- **Paint inexactness.** A cursor outside the DECOM region, or a pending wrap on a wrapped row, is parked with DECSC. That spends the program's saved cursor and is declared as `SavedCursorNotPaintable` unless the saved and current cursors are equal. When the alternate screen is active, the primary pen and saved cursor are approximated: alacritty itself overwrites the primary saved cursor on entry.
- **Byte-at-a-time tracker.** The boundary tracker parses every byte a second time, one byte at a time. Throughput budgets belong to Todo 56.
- **No socket surface yet.** The real-binary After stays deferred until Todos 21, 23 and 31.
