feat(machine-actor): isolate remote actors and replay orphan acknowledgements

Plan todo 44. Closes GAP-108, GAP-109, GAP-119, GAP-120. Delivers IDEAL-080, IDEAL-082, IDEAL-083, IDEAL-092.

## Before

`absent` - no comparable baseline capability (not an executed failing test). Gap baseline: GAP-108 (a lost line silently omits its resources), GAP-109 (no per-machine deadlines or failure isolation), GAP-119 (orphan notices not acknowledged or replayed by parent incarnation), GAP-120 (no fresh boot proof; todo 43 landed the proof, this lane keys every actor by that proven machine identity).

## After

Native host: `Linux x86_64`. Binary: `target/debug/ovis-terminal`, sha256 `ad0362ba24c36414332cc770cd517d6675ab732ff7d870f97e314b5c9602f0ee` (also recorded as `binary` in the evidence JSON).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                                   # exit 0
$ cargo test --locked -p ovis-terminal-remote -p ovis-terminal-protocol -p ovis-terminal-server --all-targets   # exit 0, 398 passed
$ cargo clippy --locked -p ovis-terminal-remote -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings   # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/isolation --isolated --case all --evidence "$ATTEMPT_DIR/task-44-all.json"   # exit 0
{"case":"all",...,"result":"passed","scenario":"remote/isolation","totals":{"assertions":"111","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/isolation --isolated --case happy --evidence "$ATTEMPT_DIR/task-44-ovis-terminal-v1.json"   # exit 0
{"case":"happy",...,"result":"passed","scenario":"remote/isolation","totals":{"assertions":"67","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario remote/isolation --isolated --case failure --inject '{"method":"$/orphanedAck","params":{"parent_incarnation":"reused-old","notification_id":"unknown"}}' --evidence "$ATTEMPT_DIR/task-44-ovis-terminal-v1.json" --append   # exit 0
{"case":"failure",...,"result":"passed","scenario":"remote/isolation","totals":{"assertions":"36","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
```

Materialized requests and replies, copied from `docs/evidence/task-44/task-44-ovis-terminal-v1.json` (the runner drives the real `ovis-terminal connect` bridge to the fixture server):

Failure case - the injected acknowledgement is refused, the outbox is retained, the unreachable machine stays listed:
```
sent     {"id":"inject-0","jsonrpc":"2.0","method":"$/orphanedAck","params":{"notification_id":"unknown","parent_incarnation":"reused-old"}}
received {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"not_found","hint":"acknowledge an id a pending orphan notice carried","retryable":false}}}
sent     {"id":"outbox-retained","jsonrpc":"2.0","method":"fixture.machine","params":{"action":"inspect","machine":"bravo"}}
received {..."result":{"health":{..."state":"unreachable"...},"outbox":["machine:0191f1a2-0000-7000-8000-000000000021/terminal:6/1>machine:0191f1a2-0000-7000-8000-0000000000b2"],"remote":{"applied":"0",...}}}
sent     {"id":"no-local-fallback","jsonrpc":"2.0","method":"fixture.remote","params":{"deadline_ms":"5000","machine":"bravo","method":"server.status"}}
received {"jsonrpc":"2.0","id":"no-local-fallback","error":{..."data":{"kind":"machine_unreachable","hint":"reconnect the machine; nothing ran locally instead","retryable":true}}}
sent     {"id":"roster-keeps-bravo","jsonrpc":"2.0","method":"machine.list","params":{}}
received {..."result":{"machines":[{..."name":"alpha",..."state":"connected"...},{..."name":"bravo",..."revision":"3","state":"unreachable"...}]}}
```
Without `--inject` (the `--case all` run) the same inject step also asserts: the real notice id under `reused-old` and under a later parent incarnation (`.../terminal:6/2`) are `stale_target`, and a numeric `parent_incarnation` is `invalid_params`.

Happy case observable: bravo is frozen; a call to it ends `deadline_exceeded` while a pipelined call to alpha answers with alpha's id and `machine.list` answers with both machines connected. The parent exits; delivering its orphan notice to frozen bravo stops at its deadline with `remaining: "1"` and the notice in the outbox. bravo's line is cut and `machine.list` lists it `unreachable`. Reconnecting replays the notice once (`applied: "1"`, `remaining: "0"`, outbox `[]`, remote `applied: "1"`, `line_epoch: "2"`, state `connected`), and a repeated `$/orphanedAck` of that id answers `outcome: "duplicate"`.

## What changed

- `crates/ovis-terminal-remote/src/actors/` - one actor task per machine: its own bounded mailbox (a full queue is refused at once as `backpressure`, never awaited), per-call deadlines (`deadline_exceeded` plus a `$/cancelRequest` to the peer), cancellation when the caller leaves, a write bounded by the call's deadline (a hung write loses only that line), and failure state published on a `watch` channel (`MachineHealth`: state, line epoch, revision, last failure kind and time, deadline count, calls in flight, last seen remote seq, dropped notifications). Roster reads (`Actors::list`) never message an actor. A lost line fails calls in flight as `machine_unreachable` and keeps the machine listed. A `std` mutex is held only to find a machine, never across an await. Dropping `Actors` aborts its tasks (no detached loops, V-4076).
- `crates/ovis-terminal-remote/src/mirror/` - remote `event` and `heartbeat` notifications become local commits through the core coordinator. The local seq comes from commit order, and the remote machine, lifetime and seq ride as `RemoteOrigin` (the landed `Commit::mirrored_from` seam) and as `MirrorSource`. Clock skew (`SkewMs`, signed decimal string) is metadata only. Explicit `scan`, `gap`, `lifetime_changed`, `line_lost` and `resumed` boundaries. Replayed remote seqs yield nothing. On a lost line the machine's terminals stay listed, unavailable, at their last-known revision; only a remote removal forgets one.
- `crates/ovis-terminal-remote/src/orphans/` - replay of a machine's pending notices oldest first, one at a time, each acknowledged before the next, so todo 41's "older incarnation after newer" drop rule always sees them in order. Replays to one machine never overlap. `Inbox` applies a notice at most once per notification id and answers a replay as `duplicate`, so remote work never ends or starts twice. `Acknowledgements::acknowledge` answers `$/orphanedAck` with `not_found` or `stale_target` without touching the outbox. The `Outbox` trait is implemented over todo 41's durable agent-tree outbox.
- `crates/ovis-terminal-protocol/src/domain/machine_actor.rs` (+ `orphan.rs`, `skew.rs`, `tests.rs`) - wire records: `MachineHealth`, `LineFailure`, `RemoteEvent`, `RemoteScan`, `RemoteTerminal`, `MirrorSource`, `MirroredEvent`, `MirrorBoundary`, `OrphanedParams`/`OrphanedResult`/`OrphanedAckParams`/`OrphanedAckResult`, opaque `NotificationId` and `ParentIncarnation` tokens (1..512 visible ASCII, minted by the sender, echoed unchanged), and constructors `MachineRecord::observed`, `MachineListResult::new`.
- `crates/ovis-terminal-server/src/dispatch/lanes/machine_actor.rs` (+ `outbox.rs`, `pump.rs`, `qa/`) - `MachineActorLane`: `machine.list` served as an unrecorded read (todo-19 Read authz) from actor state; `attach` (line plus outbox replay); `deliver`; `pending`; `acknowledge` (`$/orphanedAck`); `orphaned` (receiving side over `TreeService::orphaned`, returning end orders once); `pump` (mirror into a coordinator); `TreeOutbox` (todo 41 outbox, store work on `spawn_blocking`).
- Conformance: executable `conformance/scenarios/remote/isolation.json` (happy + failure) and fixture `conformance/src/fixture/machines.rs` (+ `machines/remote.rs`).

## Seams

- New public modules `ovis_terminal_remote::{actors, mirror, orphans}` and `ovis_terminal_protocol::domain::machine_actor`. A test-only `ovis_terminal_remote::qa_support` (`#[cfg(test)] pub mod`) holds shared pipe helpers.
- New server lane `ovis_terminal_server::dispatch::lanes::machine_actor` (child modules `outbox`, `pump`, `qa` beside the scoped `machine_actor.rs`, like `machine_identity/`).
- Additive constructors on protocol types from a domain module: `MachineRecord::observed`, `MachineListResult::new`. No catalog method, OpenRPC document or released schema changed. `$/orphaned` and `$/orphanedAck` are peer messages served beside the dispatcher, as `fixture.*` and `$/cancelRequest` are.
- Manifests: `ovis-terminal-remote` gains `ovis-terminal-store` and `serde` (the `DEPENDS_ON` layering record updated). The conformance crate gains `ovis-terminal-remote`. Cargo.lock: two edge lines.
- Outside the named write scope (disclosed): `conformance/src/fixture.rs`, `fixture/handler.rs`, `fixture/serve.rs` (registration and routing of the new fixture module, the same shape todo 41 used), `conformance/src/runner_tests/suite.rs` (EXECUTABLE 19 -> 20, sorted), and slot `mod` lines in `domain.rs`, `lanes.rs` and remote `lib.rs`. `.github/workflows/probe-stress-macos.yml`: only the `machine-actor` option and job appended at the end.
- Fixture stand-ins (stated plainly): the two remote machines are in-process stand-in runtimes over byte pipes. They answer `server.status` and receive `$/orphaned` with the real `Inbox`. `fixture.remote` stands in for remote dispatch (todo 52 owns `dispatch/remote.rs`), and `fixture.remote_child` records a remote child as a remote launch would. Everything between the bridge and those pipes is the real lane, actors, outbox (todo 41 store) and authz-checked dispatcher (`machine.list`). The mirror is not wired into the fixture, because the fixture's event type (`AgentStatusEvent`) carries no mirror variant. It is proven by `mirror::qa` and `lanes::machine_actor::qa::pump` against a real coordinator and store, and todo 52 assembles it.

## QA

What the tests assert (all over real byte streams; peers are driven record by record):
- `actors::qa::isolation` - a frozen bravo holds neither alpha's answered call nor the roster (bravo's call still pending when alpha's returns; bravo `in_flight` = 1); an expired call is `deadline_exceeded` with the requested deadline, counted in health, and the peer receives `$/cancelRequest` with that id; a caller that leaves gets its call cancelled on the peer; a write hung on a full pipe fills only that machine's queue (the next call is `backpressure` with capacity 1), ends `deadline_exceeded`, loses the line, and the queued call becomes `machine_unreachable`; unknown machine `not_found`, zero/over-bound deadlines `invalid_params`, duplicate add `machine_exists`, all before anything is sent.
- `actors::qa::loss` - a lost line keeps the machine listed `unreachable` with `last_seen_seq` 5 and a later revision; calls fail `machine_unreachable` carrying that seq; reattach gives epoch 2, connected, and calls work; inbound order is Attached, Notification, Lost, Attached. Remote error objects pass through unchanged and peer requests are refused with `method_not_found`.
- `mirror::qa` - over a real `Coordinator` and SQLite store: local seq follows arrival (bravo's remote seq 900 with +600000 ms skew is committed before alpha's seq 3), the origin keeps machine, lifetime and seq, and skew is reported as `600000`. Also asserted: gap, scan, line-lost, resumed and lifetime-changed boundaries; replayed seqs yield nothing; terminals stay unavailable at revision 4 after loss; only a remote removal forgets one; a non-identity lifetime is refused (`remote_incompatible`) without moving the cursor.
- `orphans::qa` - notices replay in incarnation order 1 then 2, one at a time; an ack lost with the line is replayed after reconnect as a `duplicate` with remote applications staying 1; a frozen machine keeps its notice pending; `not_found` / `stale_target` (reused-old token, older incarnation token) leave the outbox untouched; acknowledged then `duplicate`; an inbox refuses a notice addressed to another machine without applying it.
- `lanes::machine_actor::qa` - over todo 41's real `TreeService`, `Coordinator` and store: a parent's death for a frozen bravo stays in the durable outbox through `deadline_exceeded`, `not_found`, `stale_target` and `invalid_params` (numeric incarnation); `machine.list` lists bravo `unreachable` after the cut; reattach replays it once and empties the outbox; a repeated ack is `duplicate`. On the receiving side a death ends child terminal 7 exactly once, and a replay returns no end orders; a notice addressed elsewhere is `stale_target`. The `pump` mirrors Resumed, event 41 (origin seq 41) and LineLost with increasing local seqs, and refuses one numeric-seq event.
- `domain::machine_actor::tests` - `SkewMs` round-trips every `i64` (proptest) and equals clamped remote-minus-local for every `u64` pair; numeric or non-canonical skew is refused; `u64::MAX` `finished_seq`/incarnation round-trips as decimal strings and a numeric `finished_seq` is refused; numeric `seq`/`ts_ms` in `RemoteEvent` are refused; token bounds and unknown fields are refused; notification ids are stable per (notice, target) and differ per target and incarnation.

Full gate: `bash scripts/check.sh` exit 0 (fmt, workspace clippy `-D warnings`, nextest `1067 tests run: 1067 passed (1 slow), 1 skipped`, doctests, hygiene `0 violation(s) in 1073 file(s)`, manifests, json) - `docs/evidence/task-44/check.log`. Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0 - `docs/evidence/task-44/windows.log`.

## Adversarial

- Hung commands and cancel/resume: per-machine deadline without a global lock - `a_stalled_machine_holds_neither_another_machine_nor_the_roster`, `a_hung_write_fills_only_that_machines_queue_and_then_loses_its_line`, `a_caller_that_leaves_cancels_its_call_on_the_peer`, plus the happy scenario's pipelined frozen/alpha/roster batch.
- Repeated interruptions, ack loss: `a_lost_acknowledgement_is_replayed_without_applying_twice` (applied then line dropped before the reply; replay counts as duplicate, remote work ran once).
- Stale state, reused parent: `$/orphanedAck` with `reused-old` and with a later parent incarnation is `stale_target` (crate test, lane test, scenario). Renamed machine: actors, outbox ids and notices are keyed by the proven `MachineId`, never by name; todo 43's alias tests cover rename resolution. This lane adds no rename path.
- Timing: the deadline tests use deadlines as the behavior under test and wait on events otherwise. The recipe `taskset -c 0-1` with 8 busy loops, 25 iterations of `cargo test -p ovis-terminal-remote` + `-p ovis-terminal-protocol --lib machine_actor` + `-p ovis-terminal-server --lib machine_actor`, passed `RESULT 25/25` (`docs/evidence/task-44/stress.log`).

## Contract checklist

1. Every counter on the wire is a decimal string (`Count`, `EventSeq`, `Revision`, `TimestampMs`, `DurationMs`, signed `SkewMs`). Numeric forms are refused (tests above), and the `u64::MAX` round trip is tested. No new u64 wire field without decimal strings.
2. Boundary errors are `thiserror` enums with `kind()`/`hint()` and `#[source]`: `ActorError`, `MirrorError`, `OrphanError`, `OutboxError`, `TokenError`, `SkewError`. Display never repeats the kind token.
3. Distinct newtypes: `NotificationId`, `ParentIncarnation`, `SkewMs`. `MachineId`, `EventSeq` and `Incarnation` are reused; `Option` appears only for real absence (`last_failure`, `last_seen_seq`, `through`).
4. Tests assert behavior (transitions, typed kinds, round trips, ordering); proptest is used for the skew and token codecs.
5. No `#[allow]`/`#[expect]`, no placeholder TODOs, no prose-pinning tests.
6. The conformance scenario is executable (not deferred): `remote/isolation` runs `--case all` with 111 assertions, failed 0, skipped 0.

Holder wire generation stays 2; the checklist items are unchanged.

## Risks

- `origin/integration` moved to 8a436e1 (todo 29) after this lane's base f835935. It touches union-merge files this lane also edits (`probe-stress-macos.yml`, conformance `fixture.rs`/`handler.rs`/`serve.rs`/`runner_tests/suite.rs`, `Cargo.lock`). Per the lane's hard process rule, nothing was merged here; the orchestrator union-merges.
- Fixture stand-ins (see Seams). Two real runtimes joined over SSH/stdio arrive with todo 52.
- In-memory dedupe windows: `Inbox` remembers 4096 applied ids and `Acknowledgements` 1024 settled ids. Older replays fall back to todo 41's durable idempotency (`TreeService::orphaned` returns `replayed`); an ack older than the window answers `not_found`, which is typed and changes nothing.
- A caller that leaves is detected at the actor's next activity or at its deadline (bounded by the deadline), not instantly.
- Remote notifications the local consumer has no room for are counted in `MachineHealth.inbound_dropped`; the mirror reports the skipped seqs as a gap.
- macOS stress: `gh workflow run probe-stress-macos.yml --ref lane/t44-isolate-remote-actors-and-re -f probe=machine-actor` -> https://github.com/devswha/ovis-terminal/actions/runs/41256769132 conclusion `success`, `RESULT 25/25` (remote actors/mirror/orphans, protocol machine_actor, server lane machine_actor, and `remote/isolation --case all` at 111 assertions each iteration).
- PR CI: this PR is `CONFLICTING` against `origin/integration` 8a436e1, so GitHub created no `ci` run. The local `check.sh`, the Windows pre-check and the macOS stress run above are the executed gates.


## Addendum - merge round (integration 8a436e1)

Merge commit `aceba79` merges `origin/integration` 8a436e1 (T29 + T57).
- **Conflicts resolved:**
  - `conformance/src/fixture/handler.rs`: kept both T57's `fixture.deliver` and this lane's `fixture.remote`, `fixture.machine`, `fixture.remote_child` and `$/orphanedAck` routes. T57's `Revalidated` wrapper, writer principal and the T49/T29 parts in `serve.rs`/`fixture.rs` auto-merged, alongside the `machines` module.
  - `conformance/src/runner_tests/suite.rs`: integration's slice form, sorted, with both `remote/isolation.json` and `security/rotation.json`.
  - `probe-stress-macos.yml`: integration's file plus the `machine-actor` option and job last; it parses with 29 jobs and 29 options.
- **Cargo.lock:** `git checkout --theirs` + `cargo generate-lockfile --offline`; `cargo metadata --locked` passes; the diff against integration is +3 lines (this lane's edges).
- **OpenRPC:** `openrpc write schema/ovis-terminal.openrpc.json` is byte-identical (no catalog change); `openrpc compat schema/released.openrpc.json` exits 0. Pre-existing on integration, not touched here: `packages/client-ts/src/generated/*` is stale against the schema (T57's credential methods are missing; `bun scripts/generate.ts --check` reports `stale_output`).
- **Gates on the merged tree** (Linux x86_64, binary sha256 `adf9228658d5ce7f26bf1012d14aa80a99be91acbcdf9cb35cb48fabb6a6c84d`):
  - `cargo +nightly fmt --all`.
  - Windows `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0, and `cargo clippy` for that target `-D warnings` exit 0.
  - Hygiene: `0 violation(s) in 1114 file(s)`.
  - `bash scripts/check.sh` exit 0 (`1110 tests run: 1110 passed (1 slow), 1 skipped`).
  - `remote/isolation --case all` exit 0 (111 assertions, failed 0, skipped 0).
  - `all/all` exit 0 (42 passed, 37 declared-only, failed 0, skipped 0).
  - Logs are in `docs/evidence/task-44/m-*`.
- **CI:** none. While the gate ran, `origin/integration` moved again to 8cab061 (T55, PR #47). That merge conflicts with this lane in `conformance/src/fixture/handler.rs` and `conformance/src/fixture/serve.rs`, so the PR is `CONFLICTING` again and GitHub created no `ci` run for aceba79 (checked once). No second merge was made, because this round named 8a436e1.

## Addendum - merge round 2 (integration 8cab061 + dda248a)

Merge commits `45a401e` (8cab061: the handler.rs/serve.rs union with T55's `processes::register` and shutdown-report helper alongside this lane's `machines` module; suite slice sorted; Cargo.lock regenerated offline) and `70183e1` (dda248a: `machine-actor` stress option/job kept last). The gates were re-run after a host reboot, on Linux x86_64 with binary sha256 `d1c4992325f89cf4add6c0fe342b81ffa39d35b80a34e67031178eb861cd36aa`. `with-slot.sh bash scripts/check.sh` exit 0 (`1136 tests run: 1136 passed (1 slow), 1 skipped`; hygiene `0 violation(s) in 1132 file(s)`). The Windows `cargo check`/`clippy` for `x86_64-pc-windows-gnu` with `-D warnings` exit 0. Lane `cargo test`/`clippy -D warnings` for remote/protocol/server exit 0. `remote/isolation --case all` exit 0 (111 assertions, failed 0, skipped 0). `all/all` exit 0 (44 passed, 1823 assertions, 37 declared-only, failed 0, skipped 0). Logs: `docs/evidence/task-44/r2-*`.

## Addendum - CI round 3 (deadline count race)

- **750584d `rustdoc -D warnings`:** an unresolved [`Inbox`] link in `conformance/src/fixture/machines.rs` is now the full path `ovis_terminal_remote::orphans::Inbox`. `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps -p ovis-terminal-conformance -p ovis-terminal-remote` exits 0.
- **750584d `scripts/check.sh (ubuntu)`:** `remote/isolation` happy step 16 read `health.deadline_exceeded` = 1 where 2 was expected.
  - Root cause (product, not scenario): `Actor::expire` (`crates/ovis-terminal-remote/src/actors/actor/expiry.rs`) answered the caller with `deadline_exceeded`, then awaited the `$/cancelRequest` writes, and only then counted the expiry in health. A caller that read health as soon as it heard of its deadline (the `deliver` replay, which stops on `deadline_exceeded`) could see the old count. How often that happened depended on scheduling.
  - Fix: `expire` takes the ended calls out of `pending`, records them in health, and only then answers the callers and writes the cancels.
  - The scenario's `2` is now the guaranteed invariant: step 12's expiry is counted before `bravo-stalls` gets its error, and the replay's expiry is counted before `deliver` sees `stopped: deadline_exceeded`. There are no sleeps or retries, and the scenario is unchanged.
- **Regression test:** `actors::qa::isolation::an_expiry_is_counted_before_its_caller_hears_of_it`. It uses a 16-byte pipe, so the cancel write is backpressured and the old ordering is parked inside the 1 s control write when the caller reads health. On the old `expiry.rs` it fails deterministically (`r3-mutant-old-order.log`, nextest exit 100). On the fix, `cargo nextest run -p ovis-terminal-remote --lib` passes 41/41 (`r3-remote-tests.log`).
- **Stress (through the heavy gate):** `remote/isolation --case all`, 25 runs pinned with `taskset -c 0-1` beside 8 busy loops on the same cores: `RESULT 25/25` (`r3-stress25.log`).
- **Gates on the fixed tree** (Linux x86_64):
  - `with-slot.sh bash scripts/check.sh` exit 0 (`1137 tests run: 1137 passed (1 slow), 1 skipped`; hygiene `0 violation(s) in 1132 file(s)`).
  - Windows `cargo check`/`clippy -D warnings` exit 0.
  - `remote/isolation --case all` exit 0 (111 assertions, failed 0, skipped 0).
  - `all/all` exit 0 (44 passed, 1823 assertions, failed 0, skipped 0).

## Addendum - merge round 3 (integration 76a173b, then 1d2f0af)

- **`4b3ce12`** merges 76a173b (T47).
  - `conformance/src/fixture/handler.rs` keeps T47's `fixture.update` route next to this lane's `fixture.remote`, `fixture.machine`, `fixture.remote_child` and `$/orphanedAck` routes.
  - `probe-stress-macos.yml` is integration's file plus `machine-actor` as the last option and the last job. It parses with 35 options and 35 jobs; the `platform` option vs `stress` job naming is pre-existing on integration.
  - The union pushed `fixture::serve::serve` to 102 lines (clippy `too_many_lines`). It was fixed structurally by moving the store/coordinator open into `open_core`, with no behaviour change.
- **`48954ad`** merges 1d2f0af. It includes 7183f34, which maps IDEAL-118..121 and GAP-151..154 in the traceability ledger. On 76a173b alone, `traceability::every_ledger_id_is_mapped_exactly_once_to_known_todos` failed on integration's side; that is fixed upstream and passes 2/2 here (`r5-traceability.log`).
- **OpenRPC:** `openrpc write` is byte-identical and `openrpc compat schema/released.openrpc.json` exits 0.
- **client-ts:** `bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test` exits 0 (30 pass, 0 fail). `src/generated` is unchanged because this lane adds no OpenRPC methods, so there is no SDK commit (`r5-client-ts.log`).
- **Gates on 48954ad** (Linux x86_64, binary sha256 `f17f967b53fb343a8fb716d714f32b1e66899e8ea617551e5786ebec1498ae9d`):
  - `with-slot.sh bash scripts/check.sh` exit 0 (`1168 tests run: 1168 passed (1 slow), 1 skipped`; hygiene `0 violation(s) in 1161 file(s)`).
  - `remote/isolation --case all` exit 0 (111 assertions, failed 0, skipped 0).
  - `all/all` exit 0 (46 passed, 1960 assertions, 37 declared-only, failed 0, skipped 0).
  - The Windows `cargo check`/`clippy -D warnings` exit 0 was run on 4b3ce12, the code merge; 48954ad adds only docs, traceability and client-ts.
- **Timing fix (restated):** step 16 (`deliver-frozen`) reads `health.deadline_exceeded` only after the replay has returned `stopped: deadline_exceeded`. The actor now counts every expiry in health before it answers the expired caller, so both deadlines (step 12's and the replay's) are counted before that reply is built. Proof: 25/25 pinned to `taskset -c 0-1` with 8 busy loops on those cores (`r3-stress25.log`), and the mutant with the old order fails `an_expiry_is_counted_before_its_caller_hears_of_it` (`r3-mutant-old-order.log`).
