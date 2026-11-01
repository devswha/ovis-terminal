feat(attach): assemble the binary and plain interactive attach client (plan todo 31)

## Before

RT-HD-07, GAP-039, GAP-040, GAP-041, GAP-042, GAP-044, GAP-045, GAP-056, GAP-057, GAP-137, GAP-145:
`docs/evidence/baseline/smoke/herdr-h08-attach.log:4`. A second herdr attach exited 1 and printed a refusal as terminal text:

```
attach#2 (no takeover) exited=true exitCode=1 output="\u001b[?1016$p\u001b[0 qherdr: server shut down: terminal attach failed: terminal term_65f10d20ab0831 already has an attached client; retry with --takeover\r\n"
```

Before this PR, `ovis-terminal` had no `serve`, `hold` or `terminal attach`. Every scenario's server was the runner's fixture process, and no binary fed holder output into attach streams.

## After

Linux 7.0.0-38-generic x86_64. Real `ovis-terminal` CLI/socket transcript of an attach (`docs/evidence/task-31/after-transcript.txt`; bearers redacted):

```
$ ovis-terminal serve --root <tmp> --stdin-lifetime
{"ready": {"socket": "/tmp/t31-t00rkjnk/run/s.sock", "bearer": "<redacted>", "observer": "<redacted>"}}
$ ovis-terminal --json call terminal.run --params '{"program":{"argv":["/bin/sh"]},"name":"shell",...}'  -> exit 0
{"spawn": "spawned", "terminal": {"terminal_id": "terminal:1", "address": "t1", "state": "live", "process": {"argv": ["/bin/sh"], "cwd": "/tmp", "incarnation": "1", "pid": 1672057, "start_token": "linux:44213f41-39db-4eed-a62b-f5e5992ca358:34616068", "started_ms": "1793485899840"}}}
$ ovis-terminal terminal attach --target t1   (stdin/stdout = outer PTY slave, 100x30, controlling terminal)
typed: 'echo mark-$((30+1))\r'  -> outer PTY shows 'mark-31': True
$ ovis-terminal --json call attach.leases (while attached) -> {"geometry_owner":{"epoch":"1","holder":"cli","lease_id":"lease:2","role":"interact"},"interact":{"epoch":"1","holder":"cli","lease_id":"lease:1","role":"interact"},"observers":[]}
outer PTY resized to 120x40; inner 'stty size' reports '40 120': True
typed: Ctrl-\ (0x1c) -> client exit 0; stderr: {"detached":{"reason":"detach_key","stream_closed":true,"stream_id":"stream:01a19417-0853-75a9-8bb0-72773d50472c"}}
outer PTY termios after detach == before attach: True
$ ovis-terminal --json call attach.leases (after detach) -> {"geometry_owner":null,"interact":null,"observers":[]}
serve report: {"report":{"accepted":"4","failures":{},"panicked":"0"}}  exit 0
```

Conformance with the real binary as the server (`"fixture": "assembled"` starts `ovis-terminal serve`), at the round-4 head; evidence in `docs/evidence/task-31/r4-*.json`:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-31 --isolated --case happy --evidence "$ATTEMPT_DIR/task-31-ovis-terminal-v1-happy.json"
{"case":"happy",...,"result":"passed","scenario":"part-a/task-31","totals":{"assertions":"23",...,"failed":"0",...}}   EXIT=0
$ ... --scenario part-a/task-31 --isolated --case failure ...
{"case":"failure",...,"result":"passed","scenario":"part-a/task-31","totals":{"assertions":"89",...,"failed":"0",...}}   EXIT=0
$ ... --scenario recovery/restart --isolated --case happy ...
{"case":"happy",...,"result":"passed","scenario":"recovery/restart","totals":{"assertions":"37",...,"failed":"0",...}}   EXIT=0
$ ... --scenario recovery/restart --isolated --case failure ...
{"case":"failure",...,"result":"passed","scenario":"recovery/restart","totals":{"assertions":"25",...,"failed":"0",...}}   EXIT=0
```

What the scenarios assert:
- **task-31 happy:** hello advertises the attach, input and terminal routes, and `layout.get` answers `method_not_found`. `terminal.run` spawns `/bin/sh` as `t1`. The real outer-PTY client `terminal attach --target t1` makes the shell print `mark-31`; `attach.leases` names `cli` as interact and geometry owner; after the outer PTY is resized to 120x40 the inner `stty size` prints `40 120`; Ctrl-\ exits 0 with `detached.reason=detach_key` and `stream_closed=true`; the outer terminal settings equal the originals; `attach.leases` is empty afterwards and the serve report shows `panicked: "0"`.
- **task-31 failure:** an RPC interact attach is ready under its own lease; an observe-only principal is refused for its permission (`forbidden` from the boundary policy's precheck for `input.raw` and `input.send`) and the owner naming the writer's lease gets `no_lease`. An operator held behind the writer takes the interact lease over with `attach.takeover`; the writer's stream is told `input_revoked` and closes `taken_over` naming the operator, and the writer's revoked lease then admits nothing (`no_lease`). The operator's `attach.close` is confirmed by its CLOSE, after which one `attach.leases` read shows no holder. A PTY holder then attaches; a PTY observer follows output and its typed bytes are never evaluated (absent after a fence); a second interact attach exits 11 `lease_held` naming `cli`; `t9` exits 3 `not_found`; interact without a TTY exits 2 `not_a_tty`. Stopping the server sends `$/closing` first, and both PTY clients exit 4 `connection_lost` naming `closing.reason=server_shutdown`, with their outer terminal settings restored.
- **recovery/restart happy:** `server.restart` answers both boots, no ended and one preserved incarnation; the old connection gets `$/closing{server_restart, incarnations_preserved:true}` then end of stream; the server announces the successor boot on stdout; a new connection sees the same `runtime_lifetime`, the new `server_boot_id` (in hello and `server.status`) and the same program pid and start token. **failure:** `server.stop` with an unknown scope is `invalid_params` at `params.scope`, and the program keeps its pid and start token under the same boot.

Gate: `bash scripts/check.sh` exits 0 at the round-4 head (`1330 tests run: 1330 passed (2 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1313 file(s) checked`), and the Windows pre-check (`cargo check` and `cargo clippy` for `x86_64-pc-windows-gnu` with `-D warnings`) exits 0; details under Verification below.

## What ships

- **`ovis-terminal serve [--root DIR] [--stdin-lifetime] [--websocket-port P]`** (`crates/ovis-terminal-cli/src/assembly/`) assembles the landed crates: the Unix endpoint and optional loopback WebSocket listener, authenticated connections and bearers (owner, observe-only, writer), the SQLite stores, the shared holder under the planned-restart `Control`, and per spawned incarnation `Screen`, `TerminalStreams`/`AttachLane`, `TerminalLeases`, the input `Sequencer` and the `GeometryController`. One actor thread applies holder messages and request jobs in one order.
- **Handler stack:** `Revalidated(Measured(BoundaryPolicy(Gated(Assembled))))`. Credentials are revalidated per call; every settled call is counted in `server.metrics`; the todo-58 policy audits each call, prechecks its permission and scrubs secrets from errors; the todo-40 prompt bridge gate (from T42's `AgentAssembly`) refuses a `prompt.open`/`prompt.close` whose process token does not own its terminal; then the attach methods, the runtime routes and one dispatcher of hosted lanes answer.
- **Agent domain:** T42's `AgentAssembly` is built once per server lifetime over the terminal directory and registered in every generation's dispatcher (its 14 methods), with open prompts recovered from that boot's log; the runtime binds spawn tokens in the assembly's status service, so tokens and prompts survive planned restarts.
- **hello advertises 68 of the 104 published methods**, and `tests/assembly.rs` diffs the live hello against `schema/ovis-terminal.openrpc.json`: the difference must equal a named set of 36, and each of those answers `method_not_found`. Every advertised method reaches a landed implementation; there are no stub results.
- **`ovis-terminal terminal attach`** is the plain client (raw mode with RAII restore, held typed bytes until `input_ready`, SIGWINCH resize under the geometry lease, Ctrl-\ detach confirmed by the stream's CLOSE, Ctrl-C to the program, observe mode without a TTY). It treats the server's `$/closing` as readiness revocation and names it in its `connection_lost` record.
- **`ovis-terminal hold`** runs the session holder in the foreground.

## Seams of rounds 1-3 (public API changes outside the write scope, disclosed)

- `ovis-terminal-core`: added `Terminals::receive_message(timeout) -> Result<Option<Message>, TerminalError>` (`terminals/runtime/observe.rs`). It applies a holder message exactly like `receive` and returns it, so the serving runtime can feed screens and sequencers.
- `ovis-terminal-server`: new `streams/control.rs` with `TerminalStreams::with_leases`, `TerminalStreams::screen`, `AttachLane::leases` (`attach.leases`) and `AttachLane::acquire_geometry`. Existing streams behaviour is unchanged.
- `conformance` runner:
  - `FixtureKind::Assembled` (`"fixture": "assembled"`) starts `<binary> serve --stdin-lifetime --root <root>/srv` through the same ready/report protocol; `fixture::launch::start` now takes `(program, args)`.
  - Five PTY steps (`pty_open`, `pty_type`, `pty_expect`, `pty_resize`, `pty_exit`) in `scenario/pty.rs` and `engine/pty.rs` and `engine/pty/outer.rs`. Each step records its plan as a descriptor on every platform, and runs only on Unix (module-level cfg).
  - `runner_tests/suite.rs` registers `part-a/task-31.json` in its own `ASSEMBLED` constant, merged sorted into the inventory check. The shared `EXECUTABLE` array is untouched, so a union with integration's own additions compiles.
- CLI exit table: new kinds `not_a_tty` (2), `connection_lost` (4), `lease_held` (11), `attach_refused` (12). `src/lib.rs` gains `pub mod assembly` and `pub mod attach`, which hold only crate-visible items. The shipped manual (`skill.md`) documents the new verbs.
- Manifests: the CLI gains Unix deps on authz, core, store, vt, base64, uuid and rustix (termios/stdio/process; the version the holder resolves) plus a rustix `pty` dev-dep. The conformance runner gains a Unix rustix dep.
- Holder wire generation stays 2.

## Round 4 (gate review at cf5904e, adversarial sweep at 486b130)

Commits, each lane-tested and the batches gated before their push:
- `a7324aa` batch C: mutant-killing tests for `InputRevoked`, `stream_closed` and `PromptService::recover`; the observer test made event-driven; observer/rejection steps in `part-a/task-31` failure; typed `read_os_boot` warning; the clean-stop fix.
- `4b0c971`, `008ac07`, `a4e1313`, `eeb0b12`, `6a8a6df`: authorized merges of `origin/integration` (8ac8489 T37, 26278c1 T59, 291cfee T86, 167c0a7 T58, a896a7d T42). With T42, `conformance/src/fixture/serve.rs` conflicted again: my registration phases now call T42's `agents::register` (which returns its `AgentAssembly`), drop the separate prompt and transcript registrations the assembly now routes, and gate the handler with `assembly.gate(..)`. The schema did not change (OpenRPC write/check/compat exit 0, no diff; SDK `generate:check` fresh); `Cargo.lock` equals the automerge. Only the generated SDK conflicted (plus `conformance/src/fixture/serve.rs` with T58, resolved by keeping both: T58's `boundary::register` and `BoundaryPolicy` inside the registration phases). After each, `openrpc write`, `check` and `compat schema/released.openrpc.json` exit 0 (`compatible_with`), `bun run generate` regenerated the SDK with the merge, and `Cargo.lock` was re-derived (`checkout --theirs`, `generate-lockfile --offline`, `metadata --locked`; identical to the automerge each time).
- `ecf9569` batch A1: `agent.append_records`/`agent.records` (T37), `hello` advertised, `rpc.discover`/`schema.errors`/`schema.events`/`schema.capabilities` from this build's document, `machine.add`, `service.describe`; the hello-vs-schema diff test.
- `486b130` health (T59): `server.status` and `server.metrics` from the boot's health tables; `Measured` around the handler; attach streams report through `StreamSetup::reporting_to`; the runtime publishes its census before it answers a job.
- `0112d21`: the conformance fixture's `serve()` split into registration phases (`fixture/serve/registration.rs`).
- `4c4be85` batch B: the planned restart boundary, `update.*`, `input.barrier`, `attach.takeover`; `recovery/restart` executable.
- `44308bf`: `part-a/task-31` failure without polling (takeover-driven revocation, forced-EOF closing).
- `45386d7` (T58): `BoundaryPolicy` composed, `agent.history` routed, launch bounds on `terminal.run`.
- `1eda49a`, `69ef2dd`: the planned-restart race fix (below).
- T42 assembly commit: `AgentAssembly` and its prompt bridge gate in the daemon; `prompts::a_prompt_opened_*` now asserts the gate's `forbidden` (exit 12) for a forged token, where the prompt lane alone answered `not_owner`.

### Linux CI finding at a035a51: `update.query` stalled past 10 s (six defects, all fixed)

`test (ubuntu-latest)` failed `update::an_activated_update_*`: the first `update.query`, before any staging, got no answer within the CLI's 10 s deadline (`{"kind":"deadline_exceeded","exit_code":13}`). Thread stacks were taken from a server running under gdb, interrupted while a query was over its bound (symbolized binary; harness `docs/evidence/task-31/r4-loaded/gdblat.py`).

1. **Per-call hashing at opt-level 0.** Under load the query thread was still hashing:
   ```
   tokio-rt-worker: sha2::sha256::x86::compress (x86.rs:107)
     <- ovis_terminal_services::update::digest::hash_file (digest.rs:96)
     <- Updates::query (service.rs:115)
   ```
   `Updates::query` re-hashes the active build (124 MB in a debug build) on every call. Fix: the workspace `Cargo.toml` builds `sha2` at `opt-level = 3` in the dev and test profiles (`[profile.dev.package.sha2]`, `[profile.test.package.sha2]`), so they pay what release pays.
2. **Starvation on the control mutex.** On idle cores the query thread was waiting for the lock while the runtime's actor held it during its wait for holder messages:
   ```
   tokio-rt-worker: Mutex::lock_contended (futex.rs:61) <- Control::lock (control.rs:262)
     <- Control::with_terminals (control.rs:175) <- ControlRestart::live_incarnations (control.rs:40)
     <- Updates::query (service.rs:127)
   ovis-terminal-r (actor): Receiver::recv_timeout <- Terminals::receive_message (observe.rs:47)
     <- Control::with_terminals (control.rs:183) <- Runtime::receive (actor.rs:100) <- actor::run (actor.rs:164)
   ```
   The actor blocked for up to 5 ms inside `with_terminals`, released the mutex for microseconds and took it again; `std::sync::Mutex` makes no fairness promise, so any other lifecycle caller (`update.*`, `server.restart`, status reads) could wait seconds. Fix (`assembly/actor.rs`, `assembly/terminal.rs`): the actor drains holder messages under the lock without waiting (`receive_message(Duration::ZERO)`, `Link::Idle` when none) and waits on its own job queue, with the control released.
3. **A restart could discard the reply of the call that asked for it.** In `conn/driver.rs` (seam), a server ending aborted every unjoined request task before sending `$/closing`; a `server.restart`/`update.activate` handler that had already returned lost its reply, so the caller saw `$/closing` with no result (outcome unknown). Fix: `close_for` first writes the reply of every request whose handler already returned (`JoinSet::try_join_next_with_id`), then cancels the rest, then sends `$/closing`. New test `listen::server::restart_reply::the_reply_of_the_call_that_restarted_precedes_closing_and_a_running_call_gets_none` (20 rounds, a never-finishing request in flight): failed on the unfixed driver (round 0 received only `$/closing`), passes fixed.

4. **The endpoint vanished during the hand-over.** A new regression (`update::a_query_racing_a_planned_restart_is_answered_by_one_boot_or_the_next`: `update.query` sent back-to-back while one `update.activate` and 19 `server.restart` calls each restart control) failed in its first round with `server_not_running` (exit 4): the old accept loop removed the socket file before the successor bound a new one. Fix (`listen/server.rs`, seam): `ServerHandle::hand_over` ends serving for a planned restart but keeps the endpoint bound and returns it; the successor generation serves that same socket, so a client connecting meanwhile waits in the listen backlog. `ServerHandle::restart` is `hand_over` plus removal, unchanged for its other callers.
5. **A read met the restart.** With the endpoint kept, the same test next got `restarting` (a read that reached the old boot while control was between boots) and, by construction, could also lose its connection to `$/closing{server_restart}`. Fix (`invocation/transport.rs`, `invocation/execute.rs`): the CLI recognises `$/closing{server_restart}` before end of stream (`CliError::Restarted`, kind `server_restart`, exit 13 for a mutation, whose hint names `op.get`), and a method whose catalogued effect is `Query` that meets a planned restart (that loss, or a `restarting` refusal) is sent again on a new connection until a boot answers, within the same 10 s deadline. Mutations are never re-sent.

6. **The server exited at the hand-over when a read was in flight.** Under load the racing-query test still failed every run with `deadline_exceeded`; the server's stderr in those runs (`r4-loaded/finalbt-*.log`, and earlier `load-2.log`) reads `ovis-terminal: stopping the server failed at: close the dispatch log of the old boot [stop_failed]`. A read runs in `spawn_blocking` holding the dispatch log; cancelling its connection at the restart does not stop the blocking closure, `Dispatcher::shutdown` did not wait for it, so `Arc::try_unwrap` of the log failed, the server exited, and the CLI's read retry met a socket nobody served until its deadline. Fix (`dispatch/execution.rs`, `dispatch/read.rs`, `dispatch/runtime.rs`, seam): every read holds a permit of a read gate in the dispatcher state for as long as its blocking closure holds the log; `Dispatcher::shutdown` acquires every permit after joining operations, then closes the gate (a later read is refused `unavailable`).

`final-8.log` (the 1/25 failure after fixes 1-3) is kept in `r4-loaded/`; the full update flow under gdb (`gflow.py`, 25 servers, query before and after activation) never stalled (`gflow-calls.log`: every query at most 0.55 s, exit 0), and 400 consecutive queries under the same load peaked at 0.515 s. A test that issues the query concurrently with the restarts (finding 4 and 5) is what exposed the remaining failures.

`update.query` latency (2 pinned cores, exit codes checked):

| | before | sha2 override only | all fixes |
|---|---|---|---|
| unloaded | 5.45-6.37 s | 0.5-7.69 s (lock waits) | 0.05-0.06 s |
| 8 busy loops | 3/3 `deadline_exceeded` at 10 s | 0.40-0.77 s | 0.19-0.31 s |

The assembly tests sped up accordingly: the planned-restart test 0.53 s and the update test 0.69 s (previously 10-17 s and 9.4 s), because every lifecycle caller had been waiting for the actor's lock. An earlier note in this PR claimed hashing was not the cause; that measurement did not check exit codes and is withdrawn.

### Linux CI finding at 4e4519f: a planned restart could silently unlink the runtime from the holder

`test (ubuntu-latest)` failed `restart::a_planned_restart_keeps_the_program_and_its_lifetime_and_serves_the_successor_boot`: after the restart, `input.send` was admitted but `terminal.wait_output` never matched (macOS and Windows passed). Root cause, a product defect in core (`crates/ovis-terminal-core/src/restart/control.rs`, seam): `Control::with_terminals` (and `Control::stop`) checked readiness before taking the control lock. The runtime's actor thread could pass that check just as `server.restart` (on a dispatch task) took the lock, wait through the hand-over, and enter after `release()` dropped the old lifecycle and before `finish()` booted the successor. It then got `control_stopped`, which the actor rightly reads as "no holder message will follow" and stopped applying holder output for the rest of the server's life, silently (no warning, matching the empty stderr in the CI log). `Control::restart` itself already re-checked readiness under the lock.

Fix: `with_terminals` and `stop` re-check readiness under the lock, so a caller racing a restart is told `restarting`. The actor maps `restarting` to a pause (it waits up to one poll for a job instead of spinning and logging), and only `control_stopped` or a disconnected holder ends the link (`assembly/actor.rs`, `Link`).

Evidence:
- New core test `restart::qa::readiness::a_caller_racing_a_planned_restart_is_told_restarting_never_control_stopped`: a reader holds the lifecycle while it waits for holder messages (as the actor does) across 20 planned restarts; every refusal must be `restarting`. On the unfixed core it failed 2 of 5 runs (`refusals during planned restarts: {"control_stopped", "restarting"}`); fixed, 5 of 5 pass, and the whole `restart::` QA module passes 20/20.
- Planned-restart test 25× under load (`taskset -c 0-1`, 8 busy loops, heavy gate): **25/25 passed** (max 334 s). Cores 0-1 were also carrying another lane's identical load at the time, so this was heavier than the recipe. Details: `docs/evidence/task-31/r4-loaded/README.md`.
- Update test (`update::an_activated_update_*`): no longer routed. It failed on CI ubuntu at a035a51 (first `update.query` past the CLI's 10 s deadline, `deadline_exceeded`, update.rs:62), and the cause is below. After all fixes, 25 runs under `taskset -c 124-125` (idle cores) with 8 busy loops, through the heavy gate: `restart` 25/25 passed, max 1.56 s; `update` 25/25 passed, max 13.97 s; `update` 25/25 passed, max 1.69 s (`docs/evidence/task-31/r4-loaded/final25/`).

### The planned restart boundary

`server.restart` is routed (and `update.activate`/`update.rollback`, which restart control through `ControlRestart`). After such a call the serve loop compares the control's identity with the one it serves. On a successor boot it:
1. closes every attach stream `server_restart` and revokes input readiness (`TerminalStreams::restarting`);
2. ends the listeners (`ServerHandle::restart`: every connection gets `$/closing{reason:"server_restart", incarnations_preserved:true}`), shuts the WebSocket listener down and joins the old dispatcher, closing its log;
3. reopens the lease journal, the geometry log and the health tables under the successor identity;
4. serves a new generation (dispatcher, handler, listeners) on the same endpoint and WebSocket port, resumes attaches (`recovered`), and announces `{"restarted":{"server_boot_id":...,"runtime_lifetime":...}}` on stdout.

Output waits are armed under the control's `ArmedWaits`, so a restart ends them `cancelled` with their resume cursor.

### Methods hello does not advertise (16 after round 5), each `method_not_found`

`tests/assembly.rs` `hello_advertises_every_schema_method_but_the_unimplemented_which_are_method_not_found` lists exactly these 16 as the allowed gap and fails on any other difference.

| Methods | Why no route (plan todo 31: assemble "all implemented domain handlers"; "unsupported future methods are not falsely advertised") |
|---|---|
| `agent.list`, `agent.note`, `agent.release`, `agent.wait_children`, `doc.list`, `events.ack`, `machine.exec`, `machine.file`, `machine.login`, `machine.rm`, `server.start`, `service.install`, `service.remove` | No landed crate implements them: their params and result types appear only in the protocol crate. `service.install`/`remove` are todo 51's local `ovis-terminal service` verbs; its RPC lane is describe-only by design. `events.ack` has no counterpart in the todo-81 hub (`Subscriptions` has no acknowledgement). |
| `terminal.history`, `terminal.search` | Wire-vs-core cursor mismatch: `Screen::history`/`search` return snapshot-relative line indices with no evicted-line count, while the wire contract numbers lines absolutely (`first_line`, `floor_line`; an absent `from_line` means the newest). Serving them would publish line numbers that shift as history is evicted. |
| `terminal.first_message` | T35's handler is landed but answers only a message a template launch queued, and the daemon does not launch templates: `terminal.run` launches argv programs only (a `template` or `first_message` is `unsupported_field`). The missing route is `terminal.run {template}` through T35's `RunTerminal`, which needs a runtime `Launcher` (reserve, start, boundary, write). `template.list`/`template.get` are served. |

### Declared scenarios (32 remain after round 5; `part-a/task-22`, `-26`, `-27` and `services/worktrees` were flipped)

| Scenario | Missing route or runner capability |
|---|---|
| part-a/task-1 to task-20, task-24, task-25, task-32 | Free-form descriptors written before the runner (no `format` marker, no steps). Each needs its owner's rewrite as `ovis-conformance/1`; for task-24 (leases, takeover) and task-25 (input, barrier) the routes are now served, so the rewrite is the only missing piece. |
| part-a/task-23 | Routes served (`attach.open`/`credit`/`close`/`repaint`/`takeover`); the runner has no `holder_output` injection, no `operator`/`phone`/`bridge` principals and no never-acknowledging viewer step. |
| part-a/task-28 | WebSocket is served; the runner has no ws transport, `ws_ping` or expired-credential step. |
| performance/fairness | The todo-56 scheduler is not in `serve`'s output path. |
| platform/conpty, platform/windows-service | Windows-only; `serve` is Unix-only. |
| recovery/crash | Recovery runs at start-up (served); the runner has no step that SIGKILLs the assembled server and boots it again on the same root (`announced` follows only a planned restart). |
| remote/identity | `machine.add` is served (now T52's `MachineAddAttached`); the scenario joins two runtimes over an argv transport and the runner serves one fixture per case. |
| services/docs-retention | `doc.get`/`doc.set` are served; the retention sweep, its virtual clock and `retention.floor_advanced` are not hosted. |
| services/plugins | `plugin.*` is served; the runner has no plugin fixture files or principals. |

### Seams (outside the write scope, disclosed)

- `crates/ovis-terminal-server/src/conn/driver.rs` (`close_for`) and the test `listen/server/restart_reply.rs` (registered in `listen/server.rs`): replies of finished requests are written before `$/closing` (defect 3 above).
- `Cargo.toml`: `sha2` at `opt-level = 3` in the dev and test profiles (defect 1 above).
- `crates/ovis-terminal-server/src/listen/server.rs`: `ServerHandle::hand_over` keeps the endpoint for the successor (defect 4 above).
- `crates/ovis-terminal-server/src/dispatch/{execution,read,runtime}.rs`: the read gate `Dispatcher::shutdown` waits on (defect 6 above).
- `crates/ovis-terminal-core/src/restart/control.rs`: `with_terminals` and `stop` re-check readiness under the control lock (the race above); new test `restart/qa/readiness.rs` registered in `restart/qa.rs`.
- `crates/ovis-terminal-server/src/dispatch/lanes/health.rs`, `lanes/boundary_policy/gate.rs` and `lanes/prompt_bridge.rs`: `Measured`, `BoundaryPolicy` and `Gated` delegate `RequestHandler::transports`. Without it, hello over WebSocket advertised only `unix` (caught by `tests/assembly/websocket.rs`).
- `crates/ovis-terminal-server/src/streams/`: `takeover.rs` (new) serves `attach.takeover` onto the caller's held stream; `handler.rs` routes it with the same op-id replay table as `attach.open`; `open.rs` `displace` returns the displaced lease; `journal.rs` `LeaseJournal::record` returns the committed sequence (`Option<EventSeq>`), which the takeover result reports (implementors in `ws/qa/seams.rs` and `streams/qa/recorder.rs` updated). New `streams/qa/takeover.rs` test registered in `tests/streams.rs`.
- `conformance/src/`: the `announced` step (`scenario/serve.rs`, `engine/announce.rs`, `fixture/launch.rs::announcement`), `recovery/restart.json` in the suite's `ASSEMBLED` list, `fixture/serve/registration.rs`.
- `crates/ovis-terminal-cli/tests/cli/support.rs`: fixture roots honour `OVIS_FIXTURE_ROOT` (host rule), as the assembly and attach supports do.

### Verification (Linux 7.0.0-38-generic x86_64)

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` at the round-4 head (after the T42 merge and every round-4 fix): **exit 0**, `1330 tests run: 1330 passed (2 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1313 file(s) checked`, `check.sh: all steps passed` (`docs/evidence/task-31/check-r4.log`). Earlier batch gates: batch C 1199/1199, batch A1 1232/1232, both exit 0.
- Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0 and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`docs/evidence/task-31/windows-precheck-r4.log`).
- `cargo nextest run -p ovis-terminal-cli`: 55/55. `cargo nextest run -p ovis-terminal-server`: 204/204. `cargo nextest run -p ovis-terminal-server --test streams`: 7/7 (with the new takeover test). `cargo nextest run -p ovis-terminal-conformance` with the real binary: 28/28 (the suite runs every executable scenario, `recovery/restart` included). `cargo clippy -p ovis-terminal-server -p ovis-terminal-cli -p ovis-terminal-conformance --all-targets -- -D warnings`: exit 0.
- SDK after each merge: `bun install --frozen-lockfile && bun run generate && bun run generate:check && bun run typecheck && bun test`: 30 pass, 0 fail (the real-socket case runs against this build's binaries through `OVIS_CONFORMANCE_BIN`/`OVIS_TERMINAL_BIN`).
- What the new tests assert:
  - `hello_advertises_every_schema_method_but_the_unimplemented_which_are_method_not_found`: hello minus the committed OpenRPC methods is empty; the OpenRPC methods minus hello equal the 36 names above; each answers `method_not_found`.
  - `routes::discovery_reads_*`: `rpc.discover` equals `schema/ovis-terminal.openrpc.json`; `schema.errors`/`schema.events` equal its `x-error-kinds`/`x-events`; `schema.capabilities` equals hello's capabilities, limits and generation.
  - `routes::metrics_count_*`: census `0` then `1` live terminal, `holder_count` `1`; `rpc_latency` counts 3 `terminal.list` calls and 1 failed `terminal.get`; `{"reset":true}` is `invalid_params`.
  - `routes::input_barrier_*`: after the echo arrived the barrier is `reached` with 0 pending bytes; a sequence past the last admitted one is `seq_ahead`.
  - `routes::agent_history_*`: one session listed below the store root, `../../secret` is `forbidden`, and a 70 000-byte argv element is refused with its `bound` before any spawn.
  - `restart::a_planned_restart_*`: receipt boots and incarnations, `$/closing` reason and preserved flag on a held connection, the stdout announcement, same lifetime and new boot in hello and status, same pid and start token, and input still reaches the program afterwards.
  - `update::an_activated_update_*`: a release published below the state is staged by digest, verified by its probe and activated; activation restarts control (closing `server_restart`, announcement, new boot, same lifetime and pid) and `update.query` then reports `9.9.9`.
  - `records::*`: a forged token is `not_owner`; appended records read back in order with their turn.
  - `streams::takeover_*` (server): a caller without the takeover grant is `forbidden`, a stale epoch `lease_epoch_mismatch`; the takeover answers acquired/displaced/seq, the caller gets `input_ready`, the holder's stream closes `taken_over` naming the new lease, and a replay answers once.
  - `attach::session::tests::a_closing_record_*`: after `$/closing`, typed bytes are held, never sent as `input.raw`, and the closing is kept for the loss report.
- CI on the pushed heads: batch C (`a7324aa`) and A1+health (`486b130`) green on every job; the A2 head is watched below.

### Risks

- Health is per boot. After a planned restart, streams on terminals that existed before it keep reporting into the previous boot's consumer table, because `TerminalStreams` fixes its `StreamSetup` at creation; streams and census published afterwards go to the successor's table.
- WebSocket clients are closed `server_shutdown` at a planned restart: `WsServerHandle` has no restart ending. They reconnect to the same port.
- The `server.restart` receipt is written by the connection before the hand-over closes it because the hand-over first revokes every stream with durable commits; the connection driver still aborts unjoined tasks on any ending, so this ordering, not the driver, guarantees delivery.
- `update.*` opens its install root at every boot; the first open of a fresh root hashes the running executable.
- T58's precheck answers an observe-only credential's `input.raw` `forbidden` before the route's own `raw_not_authorized`, and the core maps a missing takeover grant to `forbidden`, not the catalogued `takeover_not_authorized`; the scenarios assert what the binary answers.

## Round 5 (gate review at f6cf0b0)

### B1: the 20 landed handlers, routed and advertised

| Methods | How the daemon serves them | Test on the real binary |
|---|---|---|
| `attach.focus`, `attach.release_geometry` | On the actor under the caller's lease: focus through the terminal's `GeometryController`; release through the new `AttachLane::release_geometry` (beside `acquire_geometry`), which releases only the current geometry lease and journals the transition. A resize is applied synchronously inside the actor, so no queued resize exists for `lease_changed` to fence. | `leases::focus_needs_a_held_lease_and_only_the_geometry_lease_is_released` |
| `layout.get`, `layout.edit`, `selection.set` | Core `Layouts` on the runtime's own layout log (`layout.db`, reopened under the successor identity at a planned restart). `assembly/layout/wire.rs` maps core's binary splits to published nodes. A creating edit's program launches through the runtime's terminal lifecycle with `terminal.run`'s bounds, so the pane's terminal is served like any other. `rename` is `unsupported_field` (core layouts carry no names). | `layout::*` (2 tests) |
| `worktree.list`, `create`, `open`, `remove` | T45's lane on the dispatch log (which gains a `Worktree` event variant). Repositories are registered in `<state>/worktrees.json`. The `Workspaces` port is a layout adapter: opening a checkout creates a layout workspace, and removal closes exactly its workspaces, under the control's own session. | `worktrees::a_checkout_opens_a_workspace_and_its_removal_closes_exactly_that_workspace` |
| `events.subscribe`, `events.update`, `events.unsubscribe` | T81's `Subscriptions` hub per boot on the dispatch log (`report_to` the boot's health; `Request::for_client`). Each subscription is pulled by its own blocking task that writes `event`/`heartbeat`/`missed` on the subscribing connection. Only the subscribing principal may update or close it. | `events::a_subscription_receives_declared_status_and_only_its_owner_closes_it` |
| `session.snapshot` | A dispatched read answered off the executor. Agent bindings come from the dispatch log's committed snapshot (new `StatusService::binding`); terminals, leases and layout are read in one actor job. One page; `page.limit` truncates the terminal list and says so. | `session::the_session_snapshot_holds_terminals_agents_leases_and_the_layout` |
| `turn.get`, `turn.wait`, `events.wait`, `input.interrupt` | `turn.get` through the agent status lane (new `StatusService::turn`). The waits arm the terminal's wait hub on the actor under the control's armed waits and are awaited off it. Declared status reaches the hubs: the runtime drains `status.changed` from its boot's dispatch log between jobs. `events.wait` serves states and exit; `turn_after` and `cursor` are `unsupported_field`. | `turns::declared_status_drives_state_and_turn_waits_and_an_interrupt_reports_its_key` |
| `template.list`, `template.get` | T35's lane as unrecorded reads of `<state>/templates`. The lane's launcher refuses every step and no registered method reaches it. `terminal.first_message` stays unadvertised (table above). | `templates::templates_in_the_state_directory_are_listed_and_read_by_name` |
| `machine.list` (T52 landed) | T52's machine actor lane on the dispatch log. `machine.add` is now `MachineAddAttached`, so each proven machine's link is driven by its own actor. Calls naming another machine go through `RemoteDispatch`; the handler stack is `Revalidated(Measured(BoundaryPolicy(Gated(RemoteDispatch(Assembled)))))`. `ServiceLanes` is not used because the daemon already registers the plugin and update lanes it bundles. | `templates::machine_list_reads_the_actor_roster_which_starts_empty` |

Flipped scenarios, each with happy and failure cases on the assembled binary (`serve: assembled`):
- `part-a/task-22`: subscriptions.
- `part-a/task-26`: resize, focus and release, plus the `forbidden` and `no_lease` refusals.
- `part-a/task-27`: interrupt stop evidence, exit wait, and the typed wait refusals.
- `services/worktrees`: a real Git repository registered by a program in the isolated root, a planned restart, then create, open, scope_required, group remove and checkout_dirty.

Declared-status turn completion cannot be scripted, because no reply exposes the program's process token. The assembly test above asserts it instead, reading the token from the program's own screen.

### B2: `attach.control close{server_restart}` before `$/closing`

The restart hand-over closed every stream `server_restart`, but ended the listeners before the lane's delivery tasks had queued those closes. The hand-over now joins the lane's delivery tasks, bounded by the listener's 5 s grace and warning past it, before ending the listeners. `restart::an_attached_stream_is_closed_server_restart_before_the_connection_closes` asserts the order `input_revoked`, then `close{server_restart}`, then `$/closing`. It fails both without the join and with `Runtime::restarting` made a no-op (both mutants run).

### B3: resize waits on the resize itself

The inner shell runs a foreground job that prints the size when SIGWINCH reaches it (`trap … WINCH; wait`). That job is armed and acknowledged before the outer PTY is resized, and the size is then expected once within one bound. The runner's `retry_type` step (retype every second, up to 20 times) is removed, and `tests/attach.rs` uses the same reporter.

### B4: the actor never waits under the control lock

`serve` takes a hidden `--holder-poll-ms` (default 5). `control::a_spawn_and_a_lifecycle_read_are_served_while_the_runtime_waits_for_requests` stretches the idle wait to ten minutes, and a spawn and `update.query` are still answered at once. With the wait moved back under the control (`receive_message(self.poll)`), the spawn misses its deadline on every run.

### Merge of origin/integration edfdb58 (T52)

The fixture keeps the round-4 registration phases and gains T52's peer role, peer composition, `RemoteDispatch` wrap, peer stop and store release. The conformance launcher keeps one `start(program, args, root)` with T52's search path, and now starts each server in its isolated root. The schema was regenerated (write, check and compat all exit 0), the SDK was regenerated with `bun run generate`, and `Cargo.lock` was regenerated `--offline` and checked with `metadata --locked`.

### Seams of round 5 (outside the write scope, disclosed)

- `crates/ovis-terminal-server/src/streams/control.rs`: `AttachLane::release_geometry`. `streams/records.rs` and `streams.rs`: `lease_record` is public.
- `crates/ovis-terminal-server/src/dispatch/lanes/agent_status.rs`: `GetTurn` (`turn.get`).
- `crates/ovis-terminal-agent/src/status/service.rs`: `StatusService::turn` and `StatusService::binding`.
- `crates/ovis-terminal-server/src/dispatch/remote.rs`: `RemoteDispatch` delegates `transports()`; without it the WebSocket transport disappears from hello.
- `conformance/src/engine/pty.rs` and `scenario/pty.rs`: `retry_type` removed. `fixture/launch.rs`: `current_dir(root)`. `runner_tests/suite.rs`: four scenarios in `ASSEMBLED`.

### Loaded-run findings of round 5 (root-caused from live captures, all fixed)

1. **Lost restart wake (product).** The serve loop was woken by the restarting call's *return*. Under load, `server.restart`'s commit waited for the control lock: run 11 recorded `T31RESTART control_restarted 88369` ms. The dispatcher therefore answered its 10 s pending outcome first. The wake fired and `hand_over` found the control identity unchanged, so it did nothing. When control then restarted, no wake was left. The successor was never served or announced, and the test's `announcement()` waited for ever.

   Fix: the serve loop hands over on control's own `Completed` restart boundary (`Control::boundaries`, forwarded by the `ovis-restarts` thread). It first waits until no restarting call is still answering, through a `watch` counter that remembers its value, so every reply still precedes the connection's `$/closing`.

   Test: `pending::a_restart_answered_pending_is_still_handed_over_and_op_get_resolves_it`. With the control held through the QA FIFO, the restart answers pending with its op id. After the release, the successor is announced within a bounded read, and `op.get` resolves the restart op.
2. **IDEAL-010: the op id must reach the caller (product).**
   - `Limits::client_bound_ms` is defined once as the server deadline plus `REPLY_ALLOWANCE`, and the CLI uses it, so the server's deadline answer always arrives inside the client's wait.
   - The actor-routed lifecycle mutations (`terminal.run`, `kill`, `rm`, `keep`, `rename`, `label`, `recover`) now answer `deadline_exceeded` with their op id at the server deadline. Before, they had no server deadline at all.
   - `op.get` falls through to the dispatcher's ledger, so a `server.restart` op id resolves.

   Tests: `pending::a_held_terminal_run_answers_pending_with_its_op_id_and_op_get_settles_it` and `pending::the_client_waits_longer_than_the_server_deadline`.
3. **Harness hangs (tests).** In the racing test, the scoped query thread's `started` sender was owned by the test thread, so a query failing before its first answer left `first.recv()` waiting for ever. A failing restart call also never set the flag that stops the query loop, so the scope join waited for ever. Now the query thread owns the sender and the flag is set whatever the call's outcome. `Served::announcement` reads under a 60 s hang guard that names the server pid.
4. **Ruled out with evidence.**
   - The `tokio-rt-worker` in `anon_pipe_read` is the blocking-pool thread of the `--stdin-lifetime` read. A healthy idle server shows the same thread, and the executor is the current-thread runtime on the main thread.
   - Planned restarts are in-process, so the server spawns no successor process.
   - `stdio::a_launched_program_holds_none_of_the_servers_standard_streams` shows that programs hold none of the server's stdio pipes.
   - No host memory or IO pressure appeared during the loops (PSI unchanged).
   - The first loaded recipe also pinned nextest to the loaded cores, and its 48-260 s simultaneous stalls of pure unit tests were harness starvation. The final recipe pins only the product (`OVIS_TEST_PIN_CORES`, applied by the test support) to cores 124-125 with 8 busy loops, and keeps nextest and the test binaries on cores 100-123.

**QA seams (disclosed).** `serve --qa-actor-hold <fifo>` makes the actor open the FIFO, holding the control, before its next `terminal.run`. `serve --qa-holder-poll-ms` sets the runtime's idle wait and replaces round 5's hidden `--holder-poll-ms`. Both are documented in `serve --help` and refused (`accept the QA seams`) unless `--root` holds a `qa-fixture` file. The test support marks every root it creates.

**Merges.** `origin/integration` edfdb58 (T52) and 557af58 (T61) were merged.
- 557af58: `quality-matrix --mode contract` passes.
- The T61 `quality/gates` evidence was re-recorded on this tree; its `$equals_file` operand is this lane's schema.
- `evidence-audit` passes with 0 findings and `source-check` passes.
- This lane's code was brought under T61's `source-check`: a test support file split, a dropped `Result` reported instead of discarded, and an `as usize` removed.

**CI-only `remote/isolation` failure (6cf4cdb, ubuntu `all/all`, step 6 `reply[child-on-bravo]/error`).** CI kept no artifact. This scenario runs on the runner's protocol fixture, not the daemon composition. Reproduced the CI way (`taskset -c 104-107 cargo nextest run -p ovis-terminal-conformance -j 4`): 5/5 runs at 28/28. Unreproduced; listed as unmet. **Merge 95e45cc (T60):** Cargo.toml dependency-block union, schema and SDK unchanged; the merged gate passes.

**Unreproduced.** After the stale task-31 `layout.get` assertion was fixed, one `cargo nextest run -p ovis-terminal-conformance` failed at `conformance/src/runner_tests/suite.rs:68` (`assert_eq!(ran.output.status.code(), Some(0), "{}", ran.explain())`). Its `explain()` text was lost: the test helper deletes its evidence directory, and that log was grep-filtered. Twenty reruns were all green:
- 4 direct `all/all` runs,
- 8 runs with two `all/all` processes in parallel,
- 8 full nextest suites.

Evidence: `/dev/shm/ovt-fixtures/t31-scratch/r5-all-*.json`, `r5-par-*.json`, `r5-nx-*.log` (copied under `docs/evidence/task-31/r5-loaded/`). It is listed as unmet, not fixed.

### Verification of round 5 (Linux 7.0.0-38-generic x86_64)

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` at 9a5d9c6 (after the edfdb58, 557af58 and 95e45cc merges): **exit 0**, `1386 tests run: 1386 passed (3 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1404 file(s) checked`, source-check `pass`, evidence-audit `pass` (`docs/evidence/task-31/check-r5.log`).
- Windows pre-check (`cargo check` and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked`, `-D warnings`): exit 0 (`windows-precheck-r5.log`).
- `cargo nextest run -p ovis-terminal-cli`: 71/71 unloaded. `cargo nextest run -p ovis-terminal-conformance` with the real binary: 28/28, with `all/all` executing the four flipped scenarios and 32 declared.
- Loaded runs, recipe `docs/evidence/task-31/r5-loaded/r5-25n.sh`: the product (server, holder, CLI and programs) pinned to cores 124-125 through `OVIS_TEST_PIN_CORES`, 8 busy loops there at `nice -n 10`, nextest and the test binaries on cores 100-123. Result: **25/25**, every run 71/71 in 11-13 s, no SLOW (`final25-nice10-summaries.log`).
- Why the busy loops run at nice 10: with 8 loops at nice 0, `r5-loaded/stall-auto-3980869.txt` shows a just-started server runnable (`R`) with `utime=0 stime=0` after 62 s. It was never scheduled, and that alone produced the 54-258 s outliers.
- Mutants:
  - B2 fails without the delivery join and with `restarting` made a no-op.
  - B4 fails with the wait moved under the control.
  - The hand-over lost wake is reproduced deterministically by the held-control restart test.


## Contract checklist

1. Every u64 on the wire is a decimal string (`uptime_ms`, `revision`, `created_seq`, `output_offset`, `incarnation`, `started_ms`, attach credit offsets). Records are decoded through the closed protocol types (`TerminalRecord`, `InputResult`, `AttachResizeResult`, `AttachOpenResult`) before they leave. `pid` is the schema's u32 JSON integer. The `u64::MAX` round trip is tested.
2. Boundary errors are thiserror enums with `kind()` and `hint()`: `AssemblyError`, `AttachError`, and the `CliError` delegation. Server refusals stay the server's typed error (`Failure::Server`). Internal refusals log their cause to the server's stderr only.
3. Identities reuse the landed newtypes (`TerminalId`, `LeaseId`, `TerminalGeneration`, `ProcessGeneration`, `StreamId`, `ClientId`).
4. Tests assert behaviour on the real binary: the program's own output, real termios, and server-side lease state.
5. No `#[allow]`, no `#[expect]`, no placeholder TODOs. The one `unsafe` (`pre_exec` setsid/TIOCSCTTY in a test and the runner) carries a SAFETY comment.
6. The conformance scenario is executable (format marker present), not deferred.

DEFERRED VERIFICATION (runner lands in Todo 21): none. The runner landed, and `part-a/task-31` ran and passed (exit 0, outputs above).

## macOS fix (round 2)

CI on `a978c77` failed `test (macos-latest)` only, with `ENOTTY` in `a_second_interact_attach_is_held_with_the_owner_and_never_touches_its_terminal` and `connection_loss_restores_the_outer_terminal_and_reports_it`.

Cause: the tests already allocate their own PTY. Its slave is the CLI's stdin/stdout and controlling terminal (`setsid` + `TIOCSCTTY`), and no inherited stdio is used. Both failing tests read the terminal settings with `tcgetattr` on the test's slave fd after the attach client (that PTY's session leader) exited. Darwin revokes a controlling terminal's open descriptors when its session leader exits; Linux does not.

Fix: the post-exit settings are read through a freshly opened slave (`O_NOCTTY`) while the master stays open. Termios belongs to the tty, not the descriptor. The test no longer keeps its own slave descriptor at all. The runner's PTY driver (`conformance/src/engine/pty/outer.rs`) had the same pattern and got the same fix. There is no platform skip.

Proof on Linux (`docs/evidence/task-31/darwin-revoke-fix.log`): the attach suite passes 5/5 under `script -q -e -c ... /dev/null` (stdio is a pty) and 5/5 with stdin `</dev/null`. Conformance `part-a/task-31 --case all --require-no-skips` passes with 49 assertions and exit 0. The runner suite passes 25/25. A scratch merge with integration (not merged into the lane) passes workspace clippy and 53/53 CLI and conformance tests. The macOS outcome itself is verified only by this PR's CI run.

## Round 3: merge-ref compile break

CI on `1a86051` failed to compile `ovis-terminal-cli` at `assembly/terminal.rs:33` with E0308, expecting `Option<LaunchProgram>`. That type exists only on integration: T35 made `TerminalRunParams.program` optional beside `template`. The lane itself built at its base, so the failure came from CI's PR merge ref.

Fix (`d57822e`): the launch program is decoded through its serialized form, which yields `Option<LaunchProgram>` from either revision. Template-only launches stay `unsupported_field`, because `serve` does not route templates.

Verified before pushing:
- **Lane:** `cargo build -p ovis-terminal-cli` and `cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings` pass, CLI tests pass 28/28, and `bash scripts/check.sh` exits 0 (931 passed, 0 hygiene violations).
- **Scratch merge with `origin/integration` `804e508`** (built from `git merge-tree`/`commit-tree` in a throwaway worktree; the lane was not merged): workspace clippy `-D warnings` is clean, and CLI plus conformance pass 53/53.
