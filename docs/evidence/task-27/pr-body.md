## Before

RT-HC-02, GAP-076, GAP-077, GAP-086: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s06-wait-output.log:11`, durable copy `docs/evidence/baseline/smoke/hc-s06-wait-output.log:11`:

```text
{"children":[],"ended":false,"exit_code":null,"reason":"timeout","signal":null,"status":"idle","terminal":"t1","turn":1}
```

`hc wait --output READY_MARK` timed out although READY_MARK had been printed during the preceding blocking send: the wait watched only bytes after registration (GAP-076, EXCL-025). `hc interrupt` returned `{"key":"esc","outcome":"stopped"}` while `sleep 30` kept running (RT-HC-03, GAP-077, EXCL-026). herdr/hc turn waits are not bound to a send or turn and have no typed timeout (GAP-086).

## After

`crates/ovis-terminal-core/src/waits/` adds one `Waits` hub per terminal incarnation. It owns the todo-17 `Screen` and is the path holder `Output`/`Exit`, holder-link loss and accepted declared status reports take into the core:

- **Atomic evaluate-then-subscribe (IDEAL-026).** `Waits::{output,state,turn,stop}(condition, &CancelScope) -> Ticket<Outcome>` evaluates the condition under the same mutex every feed (`ingest`, `finish`, `lost`, `report`) takes, and inserts the observer only if it is not already decided, so no byte, exit or report can fall between the check and the subscription. A satisfied state wait answers `already_satisfied` without arming anything.
- **Cursor output waits (IDEAL-053).** `OutputWait::since(&Receipt, pattern)` starts at the receipt's `output_cursor`. Todo 25 captures that cursor at admission, before the first input byte. Retained output since that boundary is scanned before arming, and later chunks are scanned incrementally, including matches split across chunks. A cursor behind retention is rejected as `WaitError::OutputGap` (kind `output_gap`, with `requested/floor/head`), never answered with a timeout. A cursor past the head is `invalid_params`. Outcomes: `matched{at,cursor} | exited{exit,cursor} | lost{cursor} | gap | timeout{cursor} | cancelled{cursor}`; every one names where a later wait may resume.
- **Turn waits bound to a send or turn (IDEAL-063).** `TurnBinding::send(&Receipt)` binds to the first turn that started after the program had received that input entry; `TurnBinding::Turn(n)` binds to that turn number. Outcomes: `completed | blocked | failed{turn,ended_seq}`, `exited{turn?,exit}`, `lost{turn?}`, `cancelled{turn?}`, `timeout{turn?}`. The deadline yields `timeout`, never completion. A turn number that passed unretained is `unknown_turn`. Turn starts and ends come only from declared reports (`StatusFact`: report seq, `AgentState`, `TurnPosition{turn, input}`). Stale sequences and reused turn numbers are `stale_seq`; a report after exit is `finished`.
- **Evidenced interrupt outcomes (IDEAL-054).** `StopWatch` is armed before the interrupt key is admitted. `InterruptReport::new(&receipt, observed)` keeps `delivered` (holder write evidence: `pty_written`/`partial` with more than 0 bytes) apart from `observed{end, process: running|exited|unknown, turn, status_seq}`. `stopped` is true only for `WatchEnd::Stopped`, whose only sources are the holder `Exit` of this incarnation or a declared report that left working with a sequence above the one at arming. Screen text and a successful Escape write are never evidence.
- **Cancellation without polling.** `CancelScope::cancel()` resolves every wait armed under it as `cancelled` and removes its observer. A wait armed under an already cancelled scope is never armed. Dropping a `Ticket` detaches its observer. `Ticket::wait(deadline)` blocks on a `Condvar` that feeds signal; when the deadline passes the observer is removed and the typed timeout is filled, unless a feed won the race first.

Not wired to RPC yet: `terminal.wait_output`, `turn.wait`, `events.wait` and `input.interrupt` route to this seam with todos 28-31. Holder wire generation stays 2; no landed crate's public API changed.

Real-PTY transcript on Linux 7.0.0-38-generic x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13). Each test drives a real `ovis-terminal-holder` PTY through the todo-25 `Sequencer`, with the real authority and lease table from the sequencer QA fixture (`docs/evidence/task-27/qa-happy.log`, `qa-failure.log`):

```text
$ cargo nextest run -p ovis-terminal-core --test waits --locked --no-capture happy
receipt {"op_id":"send-ready-mark","queued_seq":"1","kind":"text","admitted_bytes":"11","mode_revision":"0","output_cursor":"5","delivery":{"state":"pty_written","written_bytes":"11","output_offset":"5"},"agent_accepted":{"state":"unknown"},"turn_started":{"state":"unknown"},"turn_ended":{"state":"unknown"}}
wait_from_receipt_cursor {"outcome":"matched","at":"5","cursor":"15"}
wait_armed_after_send {"outcome":"timeout","cursor":"16"}
     Summary [   0.026s] 1 test run: 1 passed, 18 skipped
EXIT=0
$ cargo nextest run -p ovis-terminal-core --test waits --locked --no-capture failure
interrupt {"op_id":"interrupt-esc","queued_seq":"1","delivered":true,"delivery":{"state":"pty_written","written_bytes":"1","output_offset":"5"},"observed":{"end":{"kind":"timeout"},"process":"running","turn":"unknown","status_seq":null},"stopped":false}
     Summary [   0.436s] 1 test run: 1 passed, 18 skipped
EXIT=0
```

In `happy`, `cat` (raw mode) echoes `READY_MARK\r` after the send. The wait armed afterwards from the receipt cursor matches at byte 5, the receipt's cursor. A wait armed at the post-send head (byte 16) times out, which is the herdr behavior this replaces. In `failure`, Escape to `sleep 30` is delivered (1 byte written) and observed for 300 ms: `process: running`, `stopped: false`, and `ps -p <pid>` still lists the program (independent oracle).

DEFERRED VERIFICATION (runner lands in Todo 21)

The runner is present, but the task-27 RPC surface is not routed, so `conformance/scenarios/part-a/task-27.json` is a declaration (no format marker) listed in `conformance/scenarios/declared.json` with owner 27. It is blocked on todos 28-31 wiring `ovis_terminal_core::waits` to `terminal.wait_output`/`turn.wait`/`events.wait`/`input.interrupt`. After `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` (exit 0), `ATTEMPT_DIR=<lane>/.omo/evidence/ovis-terminal-v1/task-27` (`docs/evidence/task-27/conformance.log`, evidence file `conformance-evidence.json`):

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-27 --isolated --case happy --evidence "$ATTEMPT_DIR/task-27-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"part-a/task-27 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"<lane>/.omo/evidence/ovis-terminal-v1/task-27/task-27-ovis-terminal-v1.json","evidence_written":null,"result":"failed","scenario":"part-a/task-27","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
CONFORMANCE_EXIT=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-27 --isolated --case failure --evidence "$ATTEMPT_DIR/task-27-ovis-terminal-v1.json"
{"case":"failure","error":{"detail":"part-a/task-27 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},"evidence":"<lane>/.omo/evidence/ovis-terminal-v1/task-27/task-27-ovis-terminal-v1.json","evidence_written":null,"result":"failed","scenario":"part-a/task-27","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
CONFORMANCE_EXIT=1
```

## QA

- `cargo nextest run -p ovis-terminal-core --test waits --locked`: 19 tests run, 19 passed, exit 0 (`waits-nextest.log`).
- `cargo clippy -p ovis-terminal-core --all-targets --locked -- -D warnings` and `cargo nextest run -p ovis-terminal-core --locked` (220 passed, 1 slow): exit 0 (`core.log`).
- `bash scripts/check.sh`: `check.sh: all steps passed`; workspace nextest 712 passed, 1 skipped (`check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked`: exit 0 (`windows.log`).

What the tests assert:

| test | asserts |
|---|---|
| `happy` (PTY) | a receipt-cursor wait armed after the echo matches `at == output_cursor`; a wait armed at the post-send head times out |
| `failure` (PTY) | Escape is `delivered: true`, `observed.end: timeout`, `process: running`, `stopped: false`; `ps -p` still lists the pid |
| `interrupt_that_ends_the_program_is_stopped_by_exit_evidence` (PTY) | ctrl+c to `sleep 30` gives `stopped: true` by `StopEvidence::Exit` with signal 2; `ps -p` no longer lists the pid |
| `marker_printed_before_arming_is_matched_from_the_cursor` | match at the exact offset from an earlier cursor; output before the cursor never satisfies a wait |
| `armed_wait_wakes_on_a_marker_split_across_chunks` | a wait blocked on another thread wakes on the chunk completing the marker, with the exact offset |
| `marker_is_found_under_every_split` (proptest) | any split of the output and any cursor at or before the marker find it at its offset |
| `expired_cursor_is_rejected_as_output_gap` | `output_gap` with the retained floor; `invalid_params` for a cursor ahead; `invalid_pattern` for an empty pattern |
| `exit_and_cancel_end_output_waits_distinctly` | `cancelled`, `exited{code 3}`; a wait under a cancelled scope is not armed |
| `outcomes_round_trip_decimal_offsets` | `u64::MAX` offsets serialize as decimal strings and round-trip; a numeric offset is rejected |
| `already_satisfied_state_returns_immediately` | `already_satisfied` for a current idle report and for a past exit, with a deadline of now |
| `armed_state_wait_ends_by_report_exit_loss_or_timeout` | `matched`, `timeout`, `lost` |
| `stale_and_late_reports_are_rejected` | `stale_seq` leaves the state unchanged; a report after exit is `finished`; an empty condition is `invalid_params` |
| `cancel_scope_ends_and_detaches_its_waits` | the scope's waits are `cancelled` while another scope's wait matches |
| `only_the_turn_after_the_send_completes_its_wait` | turn 1 (started before the send) ends first, yet the send's wait resolves only on turn 2 (`blocked`) |
| `timeout_exit_and_cancel_never_complete_a_turn` | `timeout{turn: none}`, `timeout{turn: 1}`, `cancelled`, `exited{code 130}` |
| `unknown_and_stale_turns_are_rejected` | `unknown_turn`, reused turn `stale_seq`, `u64::MAX` decimal round-trip |
| `declared_stop_after_arming_is_stop_evidence` | working then idle after arming is `Stopped{Status{seq 2}}` |
| `text_and_earlier_status_never_claim_a_stop` | "stopped"/"^C" text and an idle report with no working report since arming yield `timeout`, `stopped: false` |
| `exit_stops_and_cancel_does_not` | `cancelled` is not stopped; exit is `Stopped{Exit}` |

## Adversarial

- cancel/resume at the arming boundary: `CancelScope::enlist` re-checks cancellation after arming, and a scope cancelled before `arm` never arms (`exit_and_cancel_end_output_waits_distinctly`, `cancel_scope_ends_and_detaches_its_waits`).
- stale state (turn/cursor): a turn already running at send time cannot satisfy the send's wait, stale report sequences and reused turns are rejected, and evicted or future cursors are typed rejections.
- hung commands: every `Ticket::wait` takes a deadline. The `failure` case observes a running `sleep 30` for a bounded 300 ms, and its result is `timeout`, never `completed`/`stopped`.
- misleading success: `stopped` is checked against an independent `ps -p <pid>` oracle in both PTY interrupt tests. Screen text saying "stopped" is ignored.

## Contract checklist

1. Every offset, report sequence and turn number in emitted JSON is a decimal string (`OutputOffset`, `ReportSeq`, `TurnNumber`, `InputSeq`); `u64::MAX` round-trip tests are included and a numeric offset is rejected.
2. `WaitError` is one `thiserror` enum with `kind()`/`hint()` and `#[source]` on the screen error; Display never repeats the kind token.
3. Identities reuse existing newtypes (`OutputOffset`, `InputSeq`, `ReportSeq`, `TurnNumber`, `OpId`); `Option` appears only where absence is a state (`turn` before a bound turn starts, `status_seq` before any report).
4. Tests assert behavior on real PTYs and typed kinds, plus a proptest over splits; nothing is mocked.
5. No `#[allow]`/`#[expect]`, no placeholder TODO; every file is at most 250 pure LOC; every function takes at most 3 parameters.
6. Conformance is a declaration with the exit attempt recorded verbatim above. Holder wire generation is unchanged at 2.

## Seams

New public module `ovis_terminal_core::waits` (`Waits`, `CancelScope`, `Ticket`, `OutputWait`/`OutputOutcome`, `StateWait`/`StateOutcome`/`Satisfied`, `TurnBinding`/`TurnOutcome`, `StopWatch`/`Observed`/`WatchEnd`/`StopEvidence`/`ProcessObservation`/`InterruptReport`, `StatusFact`/`TurnPosition`, `ProcessState`/`ExitEvidence`, `WaitError`). No landed crate's public API changed. `conformance/scenarios/declared.json` gains the task-27 entry in sorted position.

## Risks

- Integration must route each terminal's holder `Output`/`Exit` through `Waits::ingest`/`finish` instead of feeding `Screen` directly (todos 23/31). It must also supply `StatusFact.input` as the highest input sequence acknowledged written when a report is parsed, because the send binding depends on it.
- Turn history keeps 64 turns. A send whose turn was trimmed binds to the next retained turn that started after it.
- `origin/integration` moved to b9be58f during this lane; no merge was run here. `.github/workflows/probe-stress-macos.yml` is deliberately unchanged because the option list line conflicts with the t51 union. The `waits` job block to union-merge is in `docs/evidence/task-27/stress-job.yml`, so no macOS stress run was triggered from this branch. CI's macOS matrix still runs the `waits` tests.
