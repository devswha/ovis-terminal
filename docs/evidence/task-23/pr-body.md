feat(streams): serve typed attach outcomes and isolated output streams

Plan todo 23 (wave 11). Closes GAP-039, GAP-040, GAP-041, GAP-043, GAP-046, GAP-047, GAP-060, GAP-061 and GAP-065 at the server layer. Delivers IDEAL-028, IDEAL-032, IDEAL-033, IDEAL-040, IDEAL-041, IDEAL-043, IDEAL-045 and IDEAL-055 in `ovis_terminal_server::streams`. Holder wire generation stays 2.

## Before

RT-HD-07 / V-RT-HD-07 / EXCL-016: a second herdr attach is refused as terminal text while the server keeps running. Source: `docs/evidence/baseline/smoke/herdr-h08-attach.log:4` (original `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h08-attach.log:4`; the plan cites `docs/evidence/baseline/herdr-h08-attach.log:4`, but the durable copy lives under `baseline/smoke/`):

```
attach#2 (no takeover) exited=true exitCode=1 output="\u001b[?1016$p\u001b[0 qherdr: server shut down: terminal attach failed: terminal term_65f10d20ab0831 already has an attached client; retry with --takeover\r\n"
```

## After

Linux 7.0.0-38-generic x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13).

### Real socket: `attach.*` over the real server connection, authority and SQLite coordinator

`crates/ovis-terminal-server/tests/streams.rs` serves `AttachHandler` with `serve_unix` on a private socket. The other parts are real too: an `Authority` with issued per-principal credentials, a `TerminalLeases` table, a `Screen` (todo 17), and a SQLite `Coordinator` that commits every `lease.*` transition. The test feeds holder `Output`/`Exit` records where a holder would; no holder process runs. Full transcript: `docs/evidence/task-23/streams-socket.log`.

```
$ cargo nextest run -p ovis-terminal-server --test streams --locked --no-tests=fail --no-capture
        PASS [   0.050s] (1/3) ovis-terminal-server::streams failure
        PASS [   3.004s] (2/3) ovis-terminal-server::streams happy
        PASS [   2.849s] (3/3) ovis-terminal-server::streams stalled_viewers_close_without_pausing_others
     Summary [   5.904s] 3 tests run: 3 passed, 0 skipped
exit=0
```

Held names the owner and the lease-released retry boundary. The held client still paints, through an observe lease:

```
<- {"jsonrpc":"2.0","id":"open","result":{...,"first_paint":{"cursor":{"col":0,"row":3,"visible":true},"geometry":{"cols":40,"rows":8},"history_lines":"0","modes":{"enabled":["bracketed_paste"],"kitty_keyboard":0,"revision":"1"},"output_offset":"68","revision":"40","stream_offset":"0"},"holder":{"epoch":"1","holder":"tui","lease_id":"lease:1","role":"interact"},"lease":{"epoch":"1","holder":"bridge","lease_id":"lease:3","role":"observe"},"outcome":"held","reason":null,"retry":{"event":"lease.revoked","lease":{"epoch":"1","holder":"tui","lease_id":"lease:1","role":"interact"},"on":"lease_released"},"stream_id":"stream:01a19316-4acc-...","terminal":{"process_incarnation":"1","terminal_id":"terminal:7","terminal_incarnation":"1"}}}
```

Final bytes come before CLOSE `exited`. `final_offset` equals the bytes this viewer received, and `final_output_offset` equals the raw output length:

```
<- {"jsonrpc":"2.0","method":"attach.control","params":{"close":{"by":null,"exit_code":0,"final_offset":"348333","final_output_offset":"348124","reason":"exited","signal":null},"kind":"close",...,"stream_offset":"348333"}}
```

Typed failure records, never terminal text:

```
<- {"jsonrpc":"2.0","id":"open","result":{...,"first_paint":null,"lease":null,"outcome":"refused","reason":"forbidden","retry":null,"stream_id":null,...}}
<- {"jsonrpc":"2.0","id":"open","result":{...,"outcome":"recovering","retry":{"event":"server.recovery","lease":null,"on":"recovered"},"stream_id":null,...}}
<- {"jsonrpc":"2.0","id":"call-1","error":{"code":-32000,"message":"operation failed","data":{"kind":"invalid_credit","hint":"acknowledge at most the offset after the last byte received","retryable":false}}}
<- {"jsonrpc":"2.0","id":"call-3","error":{"code":-32602,...,"data":{"kind":"invalid_params",...,"field_path":"params.acknowledged"}}}
<- {"jsonrpc":"2.0","id":"call-1","error":{"code":-32000,...,"data":{"kind":"stream_not_found",...}}}
<- {"jsonrpc":"2.0","method":"attach.control","params":{"kind":"gap","skipped":{"from_output_offset":"261920","to_output_offset":"327682"},...,"stream_offset":"262144"}}
<- {"jsonrpc":"2.0","method":"attach.control","params":{"close":{...,"final_offset":"262144",...,"reason":"stream_stalled",...},"kind":"close",...}}
```

### Conformance: DEFERRED VERIFICATION (runner lands in Todo 21; `attach.*` is not routed by any served binary until todo 31)

The todo-21 runner exists, but its fixture registers no `attach.*` routes. No served binary feeds holder output into `TerminalStreams` until todo 31 assembles them, and the fixture is outside this todo's write scope. `conformance/scenarios/part-a/task-23.json` is therefore a declaration. It is listed in `conformance/scenarios/declared.json` with owner `23`, and the reason names todo 31. These are the exact plan invocations, run verbatim:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-23 --isolated --case happy --evidence "$ATTEMPT_DIR/task-23-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"part-a/task-23 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"part-a/task-23",...}
exit=1
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-23 --isolated --case failure --evidence "$ATTEMPT_DIR/task-23-ovis-terminal-v1.json"
{"case":"failure","error":{...,"kind":"scenario_not_executable",...},"result":"failed","scenario":"part-a/task-23",...}
exit=1
$ cargo run --locked --quiet --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario all/all --isolated --case all --evidence .../all-suite.json
{"case":"all","error":null,"result":"passed","scenario":"all/all","totals":{"assertions":"482","cases":"20","declared_only":"28","failed":"0","passed":"20","skipped":"0"}}
exit=0
```

Logs: `docs/evidence/task-23/conformance-{happy,failure,all}.log`.

## QA

What each test asserts, exactly:

- `streams::happy` (real socket):
  - The interact open is `ready`. It carries `terminal_id`/`terminal_incarnation`/`process_incarnation`, `credit_initial="262144"` and `first_paint.output_offset` equal to the raw length at attach, with `stream_offset="0"`.
  - The first `attach.control` is `checkpoint`, carrying the same checkpoint as the reply, and `input_ready` follows it.
  - A cursor query split across two reads (`ESC [ 6` | `n`) yields exactly one reply on the internal lane.
  - A late observer is `ready`, with a stream id different from the operator's.
  - The second interact client is `held`: `holder` equals the operator's lease record, `retry = {on: lease_released, event: lease.revoked, lease: <holder>}`, and its own lease is `observe`.
  - A viewer that never acknowledges receives exactly 262144 bytes of a 348 KiB flood. Its `attach.credit` for 262145 is `invalid_credit`.
  - `attach.close` delivers CLOSE `client_closed`. The `lease.revoked` for `tui` is committed through the coordinator, and a retried interact open is then `ready`.
  - On exit, both credited viewers get CLOSE `exited` with `exit_code=0`, `final_output_offset` = raw length and `final_offset` = their received byte count. Meanwhile the zero-credit viewer still holds exactly 262144 bytes. It catches up and closes the same way only after it acknowledges.
  - For every viewer, a VT fed only the received bytes equals a VT fed every raw byte in `lines`, `cursor`, `title` and `modes`.
  - Viewer bytes never contain `ESC [ 6 n`, while raw output does.
  - The text `herdr: server shut down: attach refused` printed by the program reaches viewers as ordinary data.
- `streams::failure` (real socket):
  - Credit beyond sent bytes, credit of `u64::MAX` as a string and a regressing acknowledgement are all `invalid_credit`.
  - A numeric `acknowledged` is `invalid_params` at `params.acknowledged`.
  - Afterwards, acknowledging exactly the sent offset leaves the full 262144 window: refused acks granted nothing.
  - Another principal's credit is `stream_not_found`, and credit for the caller's closed stream is `invalid_credit`.
  - Through the stream-channel table, upstream DATA is `direction_violation` and CREDIT on a retired channel is `invalid_credit`.
  - An observe-only credential asking for interact gets `refused`/`forbidden` with no stream.
  - The same `op_id` with other params is `op_reused`; with the same params it returns the identical result.
  - Dropping an interact connection commits `lease.revoked`, and the next interact open is `ready`.
  - Recovery sends `input_revoked` then CLOSE `lost` as the last two records. The next open is `recovering` with `retry.on=recovered` and no stream; after recovery it is `ready` with a new stream id.
- `streams::stalled_viewers_close_without_pausing_others` (real socket, 64 KiB backlog, 2 s stall bound):
  - A healthy viewer receives every byte of a 600 KiB flood with no gap and no close.
  - A viewer that never acknowledges gets `gap` with `from_output_offset < to_output_offset`, then `checkpoint`, then CLOSE `stream_stalled`.
  - An interact holder whose connection never reads has its `lease.revoked` committed, and the next interact open is `ready`.
- Unit and property tests (`streams::*`, 9 tests):
  - Query removal is independent of read boundaries (proptest).
  - With no answered queries, every byte is released in order (proptest).
  - Outstanding bytes never exceed the window, and a refused ack changes nothing (proptest).
  - Removed queries map log offsets back to raw offsets.
  - CREDIT decodes `u64::MAX` exactly from eight LE bytes, retired channels are never reused, DATA is `direction_violation`, and a short CREDIT is `invalid_credit`.

Gates:
- `bash scripts/check.sh` exit 0 (fmt, clippy `-D warnings` on the workspace, nextest 705/705, doc tests, hygiene 0 violations, manifests, json).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0.
- The streams socket suite was repeated 25 times locally with 25 passes.
- macOS stress: the `streams` option and job (50 iterations of `--test streams` plus `streams::` unit tests) were committed in 54c18a9 and dispatched from this branch: https://github.com/devswha/ovis-terminal/actions/runs/41226681019. The shared workflow file was then restored to its base content, because integration's edits to the same options line make the PR conflict and a conflicting PR gets no CI. The job block to union-append is in 54c18a9 (`git show 54c18a9 -- .github/workflows/probe-stress-macos.yml`).

## Adversarial

- Malformed input: numeric offset -> `invalid_params`; short CREDIT payload -> `invalid_credit`; upstream DATA -> `direction_violation` before any write.
- Stale state: credit for a closed stream or a retired channel -> `invalid_credit`, never applied to a newer stream; a recovered terminal issues fresh stream ids.
- Hung commands: a connection that stops reading is closed `stream_stalled` and loses its lease; healthy viewers are not paused.
- Prompt injection: application text that looks like a refusal stays terminal data; outcomes come only from the typed reply.
- Not stressed here: the session-wide backlog budget (`SessionBacklog`). It is enforced in `TerminalStreams::rebalance`, which resyncs the largest backlogs while the session is over budget, but no test drives the session cap over budget. Aggregate capacity stress is owned by todo 56.

## Contract checklist

1. Every u64 on the wire is a decimal string: offsets, credit, epochs, incarnations and history lines. A numeric `acknowledged` is `invalid_params` (tested), and CREDIT round-trips `u64::MAX` exactly (tested).
2. `StreamError` and `CreditRefusal` are `thiserror` enums with `kind()`/`hint()` and `#[source]` on inner errors; Display does not repeat the kind token.
3. `StreamId`, `LogOffset` (filtered log), `StreamOffset` (per-viewer delivery) and raw `OutputOffset` are distinct types. CREDIT acknowledges only `StreamOffset`.
4. Tests assert behavior over the real socket: outcomes, ordering, byte counts, VT equality and typed kinds.
5. No `#[allow]`, no `#[expect]`, no placeholder TODOs.
6. Holder wire generation is unchanged at 2.

## Seams

All changes outside the write scope are additive:
- `ovis-terminal-protocol`:
  - `AttachOpenResult` gains `terminal`, `retry`, `reason` and `final_output_offset`.
  - `Checkpoint` gains `history_lines`, the history cursor.
  - New records `RetryBoundary`/`RetryOn`, `AttachedTerminal`, `PaintedScreen`, `AttachControl`/`ControlKind`/`SkippedOutput`/`StreamClose` and `AttachData`.
  - `x-notifications` gains `attach.data` and `attach.control`.
  - Constructors added: `Geometry::new`, `LeaseRecord::new`, `Modes::new`, `ScreenCursor::new`, `AttachOpenResult::new`, `AttachStreamResult::new`, `Checkpoint::new` and `SkippedOutput::new`.
  - `schema/ovis-terminal.openrpc.json` was regenerated. `openrpc compat schema/released.openrpc.json` reports compatible: the change only adds outputs.
- `ovis-terminal-core`: `Screen::first_paint()` takes the VT paint under the ingestion lock.
- `ovis-terminal-server`:
  - `conn::Notifier::closed()` resolves when the connection's writer is gone.
  - New dependencies `ovis-terminal-holder`, `ovis-terminal-vt` and `base64`; workspace `base64 = "0.22"` was already in Cargo.lock.

## Risks

- Framed mode: `StreamChannels` implements the download-only channel contract: never-reused channels, eight-byte CREDIT and DATA as `direction_violation`. However, `conn/core.rs` (todo 13, outside this scope) still closes a connection on any non-control frame, and `attach.open{stream_mode: channel}` returns `needs_framed`. Routing stream frames through `StreamChannels` belongs to the assembly todo.
- Round 2 seam: `StreamLimits::with_credit_window(bytes)` (additive builder, used by the fence test).
- Explicit takeover inside `attach.open` (`takeover:{lease_epoch}`) goes through `TerminalLeases::take_over`. The displaced viewer gets CLOSE `taken_over` with `by`. No socket test in this PR exercises that path; todo 24's lease tests cover the table.
- `TerminalStreams::recovering`/`recovered` and `publish`/`finish` are driven by the test, not yet by a holder pump (todo 31).
- `integration` moved from 846a210 to 664ded7 during this lane. Per the lane rules it was not merged into the lane. Integration added `ovis-terminal-holder` to the server manifest on the same line this lane uses (see the `fix(streams)` commit), so the PR merges cleanly. A scratch merge of this head with 664ded7 (`git merge-tree`, built in a throwaway worktree outside the lane) passes `bash scripts/check.sh`: 757/757 tests and 0 hygiene violations.

## Round 2 (gate blocker B1: the first-paint fence was unprotected)

New real-socket tests in `crates/ovis-terminal-server/src/streams/qa/fence.rs`, run by `tests/streams.rs`:

- `paint_is_fenced_ahead_of_live_bytes_under_credit_pressure`:
  - A viewer opens with a 64-byte credit window. Its first paint is larger than 4 windows, and the test asserts that.
  - Exactly 64 bytes arrive. Live output is then published while the paint is still blocked on credit.
  - The test acknowledges in 64-byte steps and asserts the following. The bytes received first equal, byte for byte, the paint that an independent reference VT produces for the pre-attach output. Every remaining byte equals the published live output. The first control record is `checkpoint` at stream offset 0. The client VT equals a VT that saw every raw byte in lines, cursor and modes.
- `an_exited_terminal_answers_ended`:
  - After exit code 3, an interact `attach.open` returns `outcome=ended`, `reason=exited`, `final_output_offset` equal to the raw length, `lease=null`, and a first paint at the raw end.
  - The stream ends with CLOSE `exited` and `exit_code=3`, never sends `input_ready`, and its screen equals the continuous VT.

Mutant check (`docs/evidence/task-23/mutant-live-before-paint.log`). `Viewer::next` was mutated to serve live log bytes before the queued checkpoint/paint, and `cargo nextest run -p ovis-terminal-server --test streams --no-fail-fast` then reported 2 passed, 3 failed:
- `paint_is_fenced_ahead_of_live_bytes_under_credit_pressure` fails at `paint first`: live bytes follow the first 64-byte window.
- `an_exited_terminal_answers_ended` fails on screen parity.
- `failure` fails because `input_ready` overtakes the checkpoint.

The mutant was reverted, and `git diff` on `viewer.rs` is clean.

Round-2 gates: the streams socket suite passed 20 of 20 repeats (5 tests each); `bash scripts/check.sh` exit 0 (707/707, hygiene 0 violations); `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0.
