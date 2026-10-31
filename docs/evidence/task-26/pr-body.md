feat(geometry): order geometry transitions and unchanged grid repaint (plan todo 26)

Closes GAP-050, GAP-058, GAP-059. Delivers IDEAL-037, IDEAL-038, IDEAL-039.

## Before

RT-HC-05, GAP-050, GAP-058, GAP-059: `docs/evidence/baseline/smoke/hc-s09-framed-attach.log:15` (durable copy of `/tmp/ulw-ovis-yCJzPH/evidence/smoke/hc-s09-framed-attach.log:15`):

```
[stream ch1 DATA 672B] "\u001b[!p\u001b[?2026l\u001b[?9l\u001b[?1000l\u001b[?1001l\u001b[?1002l\u001b[?1003l\u001b[?1004l\u001b[?1005l\u001b[?1006l\u001b[?1015l ...
```

That line is a mode reset followed by a first paint. It is a keeper for first paint, but it proves nothing about resize. There is no geometry-change record before new-grid output, no redraw-complete signal, and no same-grid repaint. Ovis polls the size, parses synchronized-update ANSI and sends same-size SIGWINCH instead.

## What changed

`crates/ovis-terminal-core/src/geometry/` adds `GeometryController`, one per terminal incarnation:

- **Authority.** `request(GeometryAuthorization, &AttachResizeParams)` accepts only a `GeometryAuthorization`. Its sole constructor, `GeometryAuthorization::admit(&TerminalLeases, Caller, LeaseId)`, calls `TerminalLeases::authorize_resize` (todo 24) and requires the admitted lease's slot to be `Geometry`. A write authorization or any other token cannot be passed (round 3, B1; rounds 1-2 accepted any `leases::Authorization`). Observers get `forbidden`, non-owners `not_geometry_owner`. A grid below 2x1 gets `invalid_geometry`. A grid above u16 is refused when `attach.resize` decodes.
- **Order.**
  1. `Dispatch::commit(op, |unit| core.commit(unit))` hands the `geometry.changed` durable unit to the commit path and binds the receipt that call returned into a `Committed` token.
  2. `apply(&Committed)` requires that token, refuses a receipt not newer than the last applied change (`invalid_params`), resizes the VT (`Screen::resize`) and only then returns the holder `Resize` message.
  3. `acknowledged(epoch, Fence)` runs after the holder acknowledged delivery. The runtime records `geometry.checkpoint` (grid, offset, revision, `changed_seq`) and itself commits `geometry.input_ready`.
  
  No producer report and no output silence is consulted. Agent-declared readiness (todo 35) is untouched.
- **Readiness gate.** `input_ready()` and `repaint()` return `not_ready{pending}` from dispatch until the acknowledgement. Acknowledging an epoch that is not in flight is `stale_geometry`, and an unissued one is `invalid_params`.
- **Storms.** One transition runs at a time. Requests during it coalesce: the newest waits, and each older waiting request resolves as `Superseded{op, grid, by}`. On completion the waiting request is rechecked against the lease table (`TerminalLeases::recheck`). A released or taken-over owner's request resolves `Refused(lease_revoked)`, either via `lease_changed` or at resume.
- **Repaint.** `repaint()` returns a `FirstPaint` at the unchanged grid. It sends no holder message, so no shrink/restore and no injected keys.
- **Focus.** `focus(view, (&Session, &ClientId, LeaseId))` derives the owner from the authenticated session's principal and client label, never from a caller-supplied `Owner`, and records the focused lease (`no_lease` unless that owner holds it). It returns the geometry owner unchanged and never moves input or geometry (EXCL-004).
- **Errors.** `GeometryError` is one thiserror enum with `kind()`/`hint()` and `#[source]` on wrapped lease, VT and screen errors.
- **Wire form.** `GeometryEpoch` wraps `DecimalU64`.

## After

OS/arch: `Linux 7.0.0-38-generic x86_64`. Base `81cb6bf` (origin/integration at dispatch).

`cargo test -p ovis-terminal-core --test geometry --locked happy -- --exact --nocapture` exits 0 (`docs/evidence/task-26/geometry-happy.log`). It runs an unmodified `bash --norc --noprofile -i` on a real holder PTY:

1. Spawn at 100x30. The coordinator subscription is registered before the resize.
2. The operator resizes to 120x40 under its geometry lease.
3. The input gate is asserted `not_ready` between VT apply and holder acknowledgement.
4. `printf 'MARK_RESIZE_OK\n'` + Enter is admitted by the todo-25 sequencer under the interact lease.

Transcript (product JSON, printed by the test):

```json
{"events":[{"affected":[{"id":"terminal:9","kind":"terminal"}],"boot_id":"boot:1","data":{"changed":{"epoch":"1","from_offset":"16","grid":{"columns":120,"rows":40},"owner":"lease:2","previous":{"columns":100,"rows":30},"terminal":"terminal:9"}},"machine_id":"machine:00000000-0000-0000-0000-00000000001a","origin":null,"runtime_id":"runtime:1","schema_version":1,"seq":"1","type":"geometry.changed"},{"affected":[{"id":"terminal:9","kind":"terminal"}],"boot_id":"boot:1","data":{"checkpoint":{"changed_seq":"1","epoch":"1","grid":{"columns":120,"rows":40},"offset":"16","revision":"10","terminal":"terminal:9"}},"machine_id":"machine:00000000-0000-0000-0000-00000000001a","origin":null,"runtime_id":"runtime:1","schema_version":1,"seq":"2","type":"geometry.checkpoint"},{"affected":[{"id":"terminal:9","kind":"terminal"}],"boot_id":"boot:1","data":{"input_ready":{"epoch":"1","grid":{"columns":120,"rows":40},"revision":"10","terminal":"terminal:9"}},"machine_id":"machine:00000000-0000-0000-0000-00000000001a","origin":null,"runtime_id":"runtime:1","schema_version":1,"seq":"3","type":"geometry.input_ready"}],"holder_resize":{"geometry":[120,40],"geometry_epoch":"1","target":{"process_incarnation":"1","terminal_id":"terminal:9","terminal_incarnation":"1"}},"marker_ms_after_input_ready":"1","marker_offset":"65","osc_reports":"0","receipt":{"admitted_bytes":"26","agent_accepted":{"state":"unknown"},"delivery":{"output_offset":"29","state":"pty_written","written_bytes":"26"},"kind":"text","mode_revision":"1","op_id":"01a19375-39aa-713c-a54b-04b0946a105e","output_cursor":"16","queued_seq":"1","turn_ended":{"state":"unknown"},"turn_started":{"state":"unknown"}},"stty_size":"40 120"}
```

What the test asserts:

- Committed events arrive as `geometry.changed` (seq 1), `geometry.checkpoint` (seq 2, `changed_seq` 1), `geometry.input_ready` (seq 3).
- `from_offset` (16) is at or before the marker output offset (65).
- The holder message is `geometry (120,40)` at epoch `"1"`.
- The receipt is `pty_written`.
- The 120x40 screen paint contains `MARK_RESIZE_OK`, within 1 ms of input-ready (bound 10 s).
- `stty size` prints `40 120`.
- Bash emitted 0 OSC records, so there was no readiness or redraw report.

`cargo test -p ovis-terminal-core --test geometry --locked failure -- --exact --nocapture` exits 0 (`geometry-failure.log`). It asserts:

- Observer resize: `forbidden`.
- Input gate and repaint after commit, an acknowledgement before VT apply, and the input gate after apply: all `not_ready`.
- Reapply, or re-acknowledge of epoch 1: `stale_geometry`.
- Acknowledging epoch 7: `invalid_params`.
- Readiness stays at epoch 1.

Other runs, all exit 0:

- `cargo nextest run -p ovis-terminal-core --locked`: 254 passed (`core-nextest.log`).
- `cargo clippy -p ovis-terminal-core --all-targets --locked -- -D warnings` (`core-clippy.log`).
- `bash scripts/check.sh`: 827 passed (`check.log`).
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` (`windows-check.log`).

### DEFERRED VERIFICATION (runner lands in Todo 21)

The todo-21 runner exists, but `attach.resize/release_geometry/focus/repaint` are not routed to `ovis_terminal_core::geometry` by any served binary until todo 31, which this todo blocks. `conformance/scenarios/part-a/task-26.json` is therefore a declaration listed in `declared.json` (owner 26, reason names todo 31), with `core_tests` pointing at the commands above. Verbatim attempts (`conformance-happy.log`, `conformance-failure.log`):

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-26 --isolated --case happy --evidence "$ATTEMPT_DIR/task-26-ovis-terminal-v1.json"
{"case":"happy","error":{"detail":"part-a/task-26 is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed",...}
EXIT=1
```

The failure case gives the same `scenario_not_executable`, EXIT=1.

## QA

Tests in `crates/ovis-terminal-core/tests/geometry.rs`, run against the real authority, lease table, SQLite coordinator and screen:

- `happy` (Unix): unmodified bash, as described above.
- `failure`: as described above.
- `model::the_change_commits_before_the_grid_exists_and_ready_follows_the_acknowledgement`: the screen is still 80x24 after the change event commits. Events are seq-consecutive, and the checkpoint grid and revision match input_ready.
- `model::a_resize_storm_sends_two_grids_and_supersedes_every_other_request`: 100 requests during one transition give 99 `Superseded` (each naming its successor). Exactly two holder resizes are sent (100x30 then 209x35). The first completion emits only `geometry.checkpoint`, and input_ready comes after the second.
- `model::requesting_the_current_or_in_flight_grid_sends_nothing`.
- `model::any_storm_settles_on_the_newest_grid_with_at_most_two_dispatches`: proptest, 24 cases.
- `adversarial::non_owners_and_malformed_grids_are_refused_without_a_change`: `not_geometry_owner`; 0x30, 1x30 and 120x0 give `invalid_geometry`; 70000 columns does not decode.
- `adversarial::a_released_owners_waiting_resize_is_refused_and_the_grid_stays`: `lease_revoked`. Only the sent grid completes, and the geometry owner becomes none.
- `adversarial::focus_records_the_client_and_never_moves_geometry_or_input`: the lease view is unchanged; a borrowed lease gives `no_lease`.
- `bash::repaint_leaves_the_childs_grid_and_signal_count_unchanged` (Unix): bash traps WINCH. After two repaints it reports `winch=0 size=30 100`. Control: after a real resize it reports a nonzero count and `size=40 120`.
- `bash::the_holder_refuses_an_epoch_older_than_it_applied` (Unix): a holder `Resize` at epoch 0 after epoch 1 gives `stale_geometry`, and bash still reports `size=40 120`.
- `wire::*`: `GeometryEpoch` u64::MAX gives `"18446744073709551615"` and back; a numeric form is refused; proptest round-trips any u64. All 8 counters in committed events are strings and round-trip.

Mutation check (run locally, then reverted):

- Forcing `input_ready()` to `Ok` fails `failure`, `happy`, and two model tests.
- Disabling supersede fails the storm and unchanged tests.

macOS: `probe-stress-macos.yml` gains the option `geometry` and a job running `--test geometry` 100 times (appended at the end of both lists). Run: https://github.com/devswha/ovis-terminal/actions/runs/41235004014 (dispatched on this branch at bdad649): conclusion `success`, job log `RESULT 100/100` on macos-latest (bash 3.2).

## Adversarial

- **Stale state (epoch/grid).** A finished epoch acknowledged or applied again gives `stale_geometry`. An unissued epoch gives `invalid_params`. The holder's own old-epoch refusal is `stale_geometry`.
- **Malformed input.** Zero, 1-column and zero-row grids give `invalid_geometry`. Columns above u16 are rejected at decode. A numeric epoch is rejected.
- **Repeated interruptions.** A 100-request storm and a randomized storm settle on the newest grid with at most two dispatches and explicit superseded outcomes.
- **Misleading success.** Readiness is never quiet-ready: it stays `not_ready` after commit and after VT apply, and only the holder acknowledgement step asserts it. The happy case shows bash produced no OSC report.

## Contract checklist

1. Counters are decimal strings: epoch, from_offset, offset, revision and changed_seq. The u64::MAX round-trip test exists.
2. `GeometryError` derives thiserror, has `kind()`/`hint()`, and puts `#[source]` on Lease/VT/Screen. Display never repeats the kind.
3. Distinct newtypes: `GeometryEpoch`, `LeaseId`, `TerminalId`, `ContentRevision`, `OutputOffset`, `EventSeq`, `OpId`. `Option` is used only for real absence (waiting request, focus, next dispatch).
4. Tests assert behavior and typed kinds. Proptest covers the epoch wire and storm coalescing.
5. No `#[allow]`, no placeholder TODO. Hygiene: 0 violations.
6. Holder wire generation stays 2. No holder or protocol change.

## Seams

- New public API: `ovis_terminal_core::geometry::{GeometryController, GeometryAuthorization, Dispatch, Committed, GeometryEpoch, GeometryError, GeometryEvent, Fence, Outcome, Requested, Resolution, Resolved, Settled, Superseded, Repaint, Focused}`. No landed crate's public API changed.
- New core event types: `geometry.changed`, `geometry.checkpoint`, `geometry.input_ready`. They are not yet in the protocol `event_catalog` (which has `terminal.size`), and the `stale_geometry` kind is not in the protocol error catalog. Todo 31 maps them onto the wire and the stream `attach.control` checkpoint/input_ready records. Streams files are untouched.
- The runtime (todo 31) must call `input_ready()` before sequencer admission and feed the holder's resize result into `acknowledged`. The landed sequencer has no geometry readiness input of its own, and this lane does not touch `input/`.
- Tests reuse `src/input/sequencer/qa/fixture.rs` read-only via `#[path]`.

## Round 2: Windows open-file cleanup

CI `test (windows-latest)` failed: `geometry/qa/rig.rs:49` panicked with `geometry store cleanup: Err(Os { code: 32, .. being used by another process })` in `adversarial::a_released_owners_waiting_resize_is_refused_and_the_grid_stays`. This is the same class todo 50 fixed in `4e609d1`.

Cause: the rig only *dropped* its `Coordinator` and relied on field order to remove the store directory afterwards. It never called `Coordinator::close`, the API that tears the coordinator down (it drops sinks and pages, then `Store::close`) and returns the close error. Windows refuses `remove_dir_all` while any handle on the SQLite files remains, whereas Unix unlinks open files, so only Windows failed.

Fix (structural, all geometry rigs):

- `Rig` now owns the coordinator as `core: Option<Coordinator<..>>` and the directory as a plain `root: PathBuf`. `impl Drop for Rig` first `take()`s the coordinator and calls `close()`, asserting the result. Only then does it `remove_dir_all(root)`, also asserted. No retry, no sleep, no ignored error, no `#[cfg(windows)]`. Callers use `rig.core()?`.
- **Handle lifetimes:**
  - Every `Subscription` is a test-local declared after the rig, so it drops before the rig.
  - The only other holders of store handles are the coordinator's state (sinks, pages, store), which `close()` consumes.
  - The `Screen` and the lease and authority objects open no files.
- In the bash `Session` (the only other rig), fields drop in declaration order: `pty` first (its `Drop` runs `Holder::shutdown`, killing the program, closing the PTY and joining workers), then `rig` (store closed, then directory removed). The holder's cwd is the temp dir, never the store directory. This order is documented on the struct.
- **Verification:**
  - `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0 (`windows-check-r2.log`). This is a compile check only; the cleanup behaviour is proven at runtime by the Windows CI job.
  - `bash scripts/check.sh` exits 0, 840 passed (`check-r2.log`).
  - `cargo nextest run -p ovis-terminal-core --test geometry`: 14 passed, and no `ovis-geometry-*` directory is left in the temp dir.

## Round 3: authority bypass and binding (gate review B1, N1, N2, N3)

- **B1 (blocker).** In rounds 1-2 `request()` accepted any `leases::Authorization`. A typist holding only the interact lease could present `authorize_write`'s token and get a resize dispatched while `authorize_resize` said `not_geometry_owner`. Fixed at the type level inside the write scope, with no leases API change:
  - `request` takes `GeometryAuthorization`.
  - The only constructor, `admit`, calls `authorize_resize` and checks `slot == Geometry` (else `not_geometry_owner` naming the owner).
  - Test `authority::a_write_authorization_never_becomes_a_resize`: the operator keeps geometry; the typist holds interact and its write authorization is `Ok`. `admit` with the typist's interact lease and with the operator's grid lease both give `not_geometry_owner`. No event is committed (subscriber `try_recv` is empty), the grid stays 80x24 and readiness stays at epoch 0. Passing the write authorization to `request` does not compile.
- **N1.** `focus` derives `Owner` from the authenticated `Session` principal plus `ClientId`. Test `authority::focus_is_bound_to_the_authenticated_session_and_moves_nothing`: the phone's session claiming the operator's client label and interact lease gets `no_lease`, and the lease view is unchanged.
- **N2.** `apply` takes a `Committed`, which only `Dispatch::commit` builds from the receipt its own commit closure returned. A receipt not newer than the last applied change gets the typed `invalid_params` (`ForeignReceipt`). Test `authority::a_receipt_from_an_earlier_commit_does_not_release_the_holder_message`: the VT keeps 100x30 and the gate stays `not_ready`. Mutation check: disabling the receipt check fails exactly that test.
- **N3.** Risks below are refreshed.
- **Verification:**
  - `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` (`windows-check-r3.log`): exit 0.
  - `bash scripts/check.sh` (`check-r3.log`): exit 0, 854 passed.
  - `cargo nextest run -p ovis-terminal-core --test geometry`: 16 passed.
  - `happy` and `failure` rerun: `geometry-happy-r3.log`, `geometry-failure-r3.log`.

## Risks

- Merge status: CI ran green on `80785ab` (orchestrator merge of integration 436506e). The PR is CONFLICTING again against integration `1c21d63` only through the shared append-only workflow file. The orchestrator union-merges; this lane does not merge.
- After a control restart the controller starts at a caller-supplied epoch. Todo 48 must pass the holder's applied epoch, or the holder answers `stale_geometry`.
- If a dispatched transition is never committed or acknowledged (holder link lost), the gate stays `not_ready`. Recovery belongs to the link-loss path (todos 31/48).
