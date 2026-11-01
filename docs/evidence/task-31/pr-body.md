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

Gate: `bash scripts/check.sh` exits 0 at the round-4 head (`1315 tests run: 1315 passed (3 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1293 file(s) checked`), and the Windows pre-check (`cargo check` and `cargo clippy` for `x86_64-pc-windows-gnu` with `-D warnings`) exits 0; details under Verification below.

## What ships

- **`ovis-terminal serve [--root DIR] [--stdin-lifetime] [--websocket-port P]`** (`crates/ovis-terminal-cli/src/assembly/`) assembles the landed crates: the Unix endpoint and optional loopback WebSocket listener, authenticated connections and bearers (owner, observe-only, writer), the SQLite stores, the shared holder under the planned-restart `Control`, and per spawned incarnation `Screen`, `TerminalStreams`/`AttachLane`, `TerminalLeases`, the input `Sequencer` and the `GeometryController`. One actor thread applies holder messages and request jobs in one order.
- **Handler stack:** `Revalidated(Measured(BoundaryPolicy(Assembled)))`. Credentials are revalidated per call; every settled call is counted in `server.metrics`; the todo-58 policy audits each call, prechecks its permission and scrubs secrets from errors; then the attach methods, the runtime routes and one dispatcher of hosted lanes answer.
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
- `4b0c971`, `008ac07`, `a4e1313`, `eeb0b12`: authorized merges of `origin/integration` (8ac8489 T37, 26278c1 T59, 291cfee T86, 167c0a7 T58). Only the generated SDK conflicted (plus `conformance/src/fixture/serve.rs` with T58, resolved by keeping both: T58's `boundary::register` and `BoundaryPolicy` inside the registration phases). After each, `openrpc write`, `check` and `compat schema/released.openrpc.json` exit 0 (`compatible_with`), `bun run generate` regenerated the SDK with the merge, and `Cargo.lock` was re-derived (`checkout --theirs`, `generate-lockfile --offline`, `metadata --locked`; identical to the automerge each time).
- `ecf9569` batch A1: `agent.append_records`/`agent.records` (T37), `hello` advertised, `rpc.discover`/`schema.errors`/`schema.events`/`schema.capabilities` from this build's document, `machine.add`, `service.describe`; the hello-vs-schema diff test.
- `486b130` health (T59): `server.status` and `server.metrics` from the boot's health tables; `Measured` around the handler; attach streams report through `StreamSetup::reporting_to`; the runtime publishes its census before it answers a job.
- `0112d21`: the conformance fixture's `serve()` split into registration phases (`fixture/serve/registration.rs`).
- `4c4be85` batch B: the planned restart boundary, `update.*`, `input.barrier`, `attach.takeover`; `recovery/restart` executable.
- `44308bf`: `part-a/task-31` failure without polling (takeover-driven revocation, forced-EOF closing).
- `45386d7` (T58): `BoundaryPolicy` composed, `agent.history` routed, launch bounds on `terminal.run`.

### The planned restart boundary

`server.restart` is routed (and `update.activate`/`update.rollback`, which restart control through `ControlRestart`). After such a call the serve loop compares the control's identity with the one it serves. On a successor boot it:
1. closes every attach stream `server_restart` and revokes input readiness (`TerminalStreams::restarting`);
2. ends the listeners (`ServerHandle::restart`: every connection gets `$/closing{reason:"server_restart", incarnations_preserved:true}`), shuts the WebSocket listener down and joins the old dispatcher, closing its log;
3. reopens the lease journal, the geometry log and the health tables under the successor identity;
4. serves a new generation (dispatcher, handler, listeners) on the same endpoint and WebSocket port, resumes attaches (`recovered`), and announces `{"restarted":{"server_boot_id":...,"runtime_lifetime":...}}` on stdout.

Output waits are armed under the control's `ArmedWaits`, so a restart ends them `cancelled` with their resume cursor.

### Methods hello does not advertise (36), each `method_not_found`

| Methods | Why no route (plan todo 31: assemble "all implemented domain handlers"; "unsupported future methods are not falsely advertised") |
|---|---|
| `events.subscribe`, `events.update`, `events.unsubscribe`, `events.ack`, `events.wait` | The todo-22 hub subscribes to one coordinator, but `serve` keeps separate event logs (terminals inside `Control`, leases, geometry, dispatch), each with its own sequence. One `events.*` domain needs the shared commit coordinator of todo 42, which has not landed. |
| `session.snapshot` | `layout::SessionProducer` reads the layout and terminal sections from one store transaction; the terminals live in `Control`'s own store. Same todo-42 dependency. |
| `layout.get`, `layout.edit`, `selection.set` | No lane exists. `core::layout::Layouts` (todo 18) needs a coordinator whose snapshot carries `Layout`, and a creating edit needs a launcher bound to the runtime's live-terminal bookkeeping (screen, streams, sequencer, process token); launching through `Terminals` directly would create a terminal with no live screen or attach. |
| `worktree.create`, `worktree.list`, `worktree.open`, `worktree.remove` | The todo-45 lane needs a `Workspaces` owner (the layout service above) and registered repositories; `serve` has neither. |
| `template.list`, `template.get`, `terminal.first_message` | The todo-35 lane needs a `Launcher` (reserve, start, boundary, write) over the runtime. `terminal.run` in `serve` refuses templates and first messages (`unsupported_field`), so a served `terminal.first_message` could never find a launch. |
| `machine.list` | The todo-44 roster is filled only by attaching lines to actors, while `machine.add` (served) records its link in the todo-43 identity registry. Joining the two is todo 52; served now, `machine.list` would report an empty roster contradicting `machine.get`. |
| `turn.get`, `turn.wait`, `input.interrupt` | No lane exists. The todo-27 wait hub and interrupt evidence exist per incarnation, but declared status never reaches the runtime's `Waits` (status reports go only to `StatusService`), so a turn wait could never be satisfied and an interrupt could never prove a stop. |
| `terminal.history`, `terminal.search` | No lane exists. `Screen::history`/`search` return snapshot-relative line indices without an evicted-line count, while the wire contract numbers lines absolutely (`first_line`, `floor_line`; absent `from_line` means the newest). |
| `attach.focus`, `attach.release_geometry` | No lane exists. `GeometryController::focus` records focus without the lease event whose `seq` the result reports, and releasing geometry to the next requester must also settle the controller's in-flight resize (`lease_changed`), which no landed lane sequences. |
| `agent.list`, `agent.note`, `agent.release`, `agent.wait_children`, `doc.list`, `machine.exec`, `machine.file`, `machine.login`, `machine.rm`, `server.start`, `service.install`, `service.remove` | No landed crate implements them: their params and result types appear only in the protocol crate. `service.install`/`remove` are todo 51's local `ovis-terminal service` verbs; its RPC lane is describe-only by design. |

### Declared scenarios (36 remain; `recovery/restart` was flipped)

| Scenario | Missing route or runner capability |
|---|---|
| part-a/task-1 to task-20, task-24, task-25, task-32 | Free-form descriptors written before the runner (no `format` marker, no steps). Each needs its owner's rewrite as `ovis-conformance/1`; for task-24 (leases, takeover) and task-25 (input, barrier) the routes are now served, so the rewrite is the only missing piece. |
| part-a/task-22 | `events.subscribe`/`update`/`unsubscribe` (todo 42, above). |
| part-a/task-23 | Routes served (`attach.open`/`credit`/`close`/`repaint`/`takeover`); the runner has no `holder_output` injection, no `operator`/`phone`/`bridge` principals and no never-acknowledging viewer step. |
| part-a/task-26 | `attach.release_geometry` (above); its other geometry-lease steps (hand-on queue, coalescing) have no runner vocabulary. |
| part-a/task-27 | `turn.wait`, `events.wait`, `input.interrupt` (above). |
| part-a/task-28 | WebSocket is served; the runner has no ws transport, `ws_ping` or expired-credential step. |
| performance/fairness | The todo-56 scheduler is not in `serve`'s output path. |
| platform/conpty, platform/windows-service | Windows-only; `serve` is Unix-only. |
| recovery/crash | Recovery runs at start-up (served); the runner has no step that SIGKILLs the assembled server and boots it again on the same root (`announced` follows only a planned restart). |
| remote/identity | `machine.add` is served; the scenario joins two runtimes over an argv transport and the runner serves one fixture per case. |
| services/docs-retention | `doc.get`/`doc.set` are served; the retention sweep, its virtual clock and `retention.floor_advanced` are not hosted. |
| services/plugins | `plugin.*` is served; the runner has no plugin fixture files or principals. |
| services/worktrees | `worktree.*` (above). |

### Seams (outside the write scope, disclosed)

- `crates/ovis-terminal-server/src/dispatch/lanes/health.rs` and `lanes/boundary_policy/gate.rs`: `Measured` and `BoundaryPolicy` delegate `RequestHandler::transports`. Without it, hello over WebSocket advertised only `unix` (caught by `tests/assembly/websocket.rs`).
- `crates/ovis-terminal-server/src/streams/`: `takeover.rs` (new) serves `attach.takeover` onto the caller's held stream; `handler.rs` routes it with the same op-id replay table as `attach.open`; `open.rs` `displace` returns the displaced lease; `journal.rs` `LeaseJournal::record` returns the committed sequence (`Option<EventSeq>`), which the takeover result reports (implementors in `ws/qa/seams.rs` and `streams/qa/recorder.rs` updated). New `streams/qa/takeover.rs` test registered in `tests/streams.rs`.
- `conformance/src/`: the `announced` step (`scenario/serve.rs`, `engine/announce.rs`, `fixture/launch.rs::announcement`), `recovery/restart.json` in the suite's `ASSEMBLED` list, `fixture/serve/registration.rs`.
- `crates/ovis-terminal-cli/tests/cli/support.rs`: fixture roots honour `OVIS_FIXTURE_ROOT` (host rule), as the assembly and attach supports do.

### Verification (Linux 7.0.0-38-generic x86_64)

- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh` at the round-4 head: **exit 0**, `1315 tests run: 1315 passed (3 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1293 file(s) checked`, `json.sh: 163 JSON file(s) checked`, `check.sh: all steps passed` (`docs/evidence/task-31/check-r4.log`). Earlier batch gates: batch C 1199/1199, batch A1 1232/1232, both exit 0.
- Windows pre-check: `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0 and `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`docs/evidence/task-31/windows-precheck-r4.log`).
- `cargo nextest run -p ovis-terminal-cli`: 54/54. `cargo nextest run -p ovis-terminal-server --test streams`: 7/7 (with the new takeover test). `cargo nextest run -p ovis-terminal-conformance` with the real binary: 28/28 (the suite runs every executable scenario, `recovery/restart` included). `cargo clippy -p ovis-terminal-server -p ovis-terminal-cli -p ovis-terminal-conformance --all-targets -- -D warnings`: exit 0.
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
