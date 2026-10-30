Plan todo 71, follow-up to todo 7. Closes the GAP-048 regression and restores
the IDEAL-032 engine-level first-paint contract.

## Before

Source: `origin/lane/t12-platform-probes:docs/evidence/task-12/vt-paint-equivalence-proptest-failure.txt`
(PR #9), copied into [task-12-original.txt](task-12-original.txt). That file
contains a saved minimized proptest seed, rather than a full failure transcript:

```text
cc a6931b477c273c590c10953796b221df6a4a2646f0ab55b1ba3f11d9ad5f66dd # shrinks to tokens = [[27, 91, 63, 55, 108], [97, 97, 32, 97, 97, 97, 32, 32, 97], [9], [228, 184, 173, 230, 150, 135]]
```

The fixed regression was written before the painter change and executed on
integration `ea6fd5cb7df90ab0072d953775868216841cbe36`, exit 101.
[regression-before.log](regression-before.log) records the actual failure:

```text
assertion `left == right` failed: paint split 18
paint row 0: "aa aaa  a"
live row 0:  "aa aaa  a\t"
```

The summary above abbreviates the full six-row assertion in the log.
Baseline legacy first-paint equivalence evidence for this exact input: absent.

## After

`Painter::cursor` no longer overwrites a right-margin tab when rebuilding a
pending wrap. It attempts a wide glyph at the margin with DECAWM temporarily
off, reproducing the live emulator's rejection without changing the cell,
then restores the painting mode. The final terminal-state step still applies
the program's actual DECAWM mode.

All commands below ran on Linux x86_64, rustc/cargo 1.99.1, with the supplied
isolated Rust environment.

| Command | Exit | Observed result |
| --- | --- | --- |
| `cargo test -p ovis-terminal-vt --test paint_margin decawm_off_wide_char_after_tab_matches_live_at_every_split -- --exact --nocapture` | 0 | One test passed; printed `live vs restored screens equal at every byte split, including the right-margin tab`. [regression-after.log](regression-after.log) |
| `PROPTEST_CASES=4000 cargo test -p ovis-terminal-vt --lib paint_reproduces_arbitrary_streams` (round 1) | 0 | Unchanged equivalence property passed, 4000 generated cases plus saved seeds; 17.85 s. [proptest-round-1.log](proptest-round-1.log) |
| Same command (round 2, immediately following round 1) | 0 | 4000 generated cases plus saved seeds; 17.69 s. [proptest-round-2.log](proptest-round-2.log) |
| Same command (round 3, immediately following round 2) | 0 | 4000 generated cases plus saved seeds; 17.58 s. [proptest-round-3.log](proptest-round-3.log) |
| `bash scripts/check.sh` | 0 | Formatting, workspace Clippy with `-D warnings`, nextest (125 passed, one existing skip), doctests, source hygiene (zero violations), manifests, JSON. [check.log](check.log) |
| Same regression and `bash scripts/check.sh`, after merging integration `61a6e5a` (todo 12) | 0 | Regression passed; all gate steps passed, nextest 132 passed with one existing skip, source hygiene zero violations in 175 files. [check-after-integration.log](check-after-integration.log) |

The integration merge is `4702bfe`; no conflicts required lockfile regeneration
or Clippy-list edits. The PR diff remains confined to the task's authorized
paths. The three 4000-case rounds preceded the integration merge; todo 12 did
not change the VT engine or its equivalence property.

The original minimized seed is committed in the existing
`crates/ovis-terminal-vt/proptest-regressions/paint_equivalence.txt`.
No property comparison or generated input class changed.

### DEFERRED-BY-PLAN real-binary conformance

The assembly runner remains deferred until Todo 21 (plan line 275); the actual
attempt is recorded verbatim in [conformance-attempt.log](conformance-attempt.log):

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-7 --isolated --case happy --evidence docs/evidence/task-71/task-71-ovis-terminal-v1.json
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
conformance exit=2
```

This is not a passing socket/CLI conformance result. The runner did not produce
the requested JSON evidence file.

## QA

The new integration regression feeds `ESC[?7l`, `aa aaa  a`, HT, and `中文`
on the original 10-column, six-row grid. At every byte split, including inside
UTF-8 and CSI, it exports/imports a checkpoint, generates first paint from the
restored engine, and feeds the remaining bytes to live, restored, and attached
engines. It asserts exact paint fidelity, equal checkpoint-restored visible
lines, and equal painted visible lines, cursor, and structured modes.

The existing property additionally compares complete visible cells and flags,
active/inactive grids when alternate is active, cursor, saved cursor subject
to its existing explicit fidelity exception, modes, tab stops, scroll region,
title, keyboard state, charset, cursor style, and palette. It does not assert
history equivalence or all future output streams.

## Adversarial

- Reverting exactly the painter fix makes the new regression fail again with
  the same missing right-margin HT at split 18, exit 101:
  [regression-reverted.log](regression-reverted.log). Restoring it passes again.
- Three consecutive 4000-case rounds exercised the unchanged stream strategy
  and all persisted paint seeds, including the task-12 seed.
- The fixed fixture combines DECAWM off, HT at the right margin, rejected wide
  input, and every byte split; it uses real engine instances, not mocks.
- No JSON contract, boundary errors, or semantic identity types changed.
  Existing wire tests cover u64::MAX decimal-string round trips and rejection
  of numeric/noncanonical offsets and revisions; the full gate runs them.
- No `#[allow]`, blanket `#[expect]`, skipped input class, or weakened
  equivalence assertion was added.

## Risks

This uses alacritty_terminal 0.26.0's non-writing wide-character rejection at
the right margin to rebuild its pending-wrap state. Compatibility with other
terminal emulators is not proved by this engine-to-engine property.
The change is limited to pending-wrap cursors whose last cell is HT; existing
wide-spacer, wrap-row, saved-cursor, and mode restoration paths stay covered by
the unchanged property and package tests.

Real socket/CLI conformance is DEFERRED-BY-PLAN until Todo 21. Rust-analyzer was
unavailable; compiler and Clippy diagnostics passed. Initial new-test formatting
and explicit-slice lint failures were corrected before the final gate.
No other worktree or crate was edited.
