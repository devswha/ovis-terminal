# Task 71 investigation

Base: ea6fd5c (origin/integration), Linux x86_64.

The task-12 artifact is a saved proptest seed, not a full failure transcript:
`a6931b477c273c590c10953796b221df6a4a2646f0ab55b1ba3f11d9ad5f66dd`.
Its minimized tokens are DECAWM off, `aa aaa  a`, HT, and `中文`.

Hypotheses:
1. Pending-wrap cursor reconstruction prints a blank over a tab cell.
   Distinguishing evidence: paint loses the final HT while cursor/modes match.
2. Row painting wraps a rejected wide glyph onto the next row.
   Distinguishing evidence: restored rows contain a wide glyph absent live.
3. Checkpoint/parser continuation loses a partial UTF-8 character.
   Distinguishing evidence: checkpoint-only restoration differs at a UTF-8 split.

The regression tests checkpoint restoration and checkpoint-derived first paint
at every byte split, then compares visible lines, cursor and modes with live.
No temporary instrumentation or external artifacts are introduced.

Confirmed mechanism: `Vt::first_paint` draws the rows and flushes deferred tabs,
then `Painter::terminal_state` calls `Painter::cursor`. Reconstructing pending
wrap calls `cell`, which prints a space for HT and queues a tab after the final
tab flush. The right-margin tab is overwritten and never replayed. The live
engine instead rejects the wide glyph with DECAWM off, sets pending wrap, and
leaves the HT cell intact. `regression-before.log` and `regression-reverted.log`
both show exactly that missing HT at split 18; checkpoint-only restoration
matches live. This confirms hypothesis 1 and rules out 2 and 3 for the fixture.

The fix reproduces the non-writing wide-glyph rejection for a pending-wrap
cursor over a tab. It temporarily disables DECAWM, attempts the glyph at the
last column, then restores the painter's enabled DECAWM; terminal_state applies
the program's real mode afterwards. The fixed regression passes again after
the exact nine-line fix is restored. The three passing property rounds used
the same painter bytes as the final restored source.

The first formatting probe found one formatting difference in the new test.
The first check.sh attempt rejected four explicit slices under indexing_slicing.
Both issues were corrected (formatting and split_at); the final full gate
passed. No lint suppression or property weakening was used.

Rust-analyzer is absent. Compiler and workspace Clippy diagnostics ran through
check.sh instead. Runtime: rustc 1.99.1 (58cf8f9c3 2026-10-13),
cargo 1.99.1 (5f94df478 2026-08-27), Linux x86_64.

Before pushing, integration advanced to 61a6e5a (todo 12). It merged cleanly as
4702bfe, preserving the integrated Cargo.lock, manifests and Clippy list.
The initial merge command stopped because vim was absent; an explicit-message
merge commit completed it. Regression and check.sh were rerun on that merge,
both exit 0: 132 nextest tests passed, one existing skip, zero source-hygiene
violations. No manual edits outside task-71 scope were made.
