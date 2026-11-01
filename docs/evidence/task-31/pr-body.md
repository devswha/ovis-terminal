feat(attach): assemble the binary and plain interactive attach client (plan todo 31)

## Before

RT-HD-07, GAP-039, GAP-040, GAP-041, GAP-042, GAP-044, GAP-045, GAP-056, GAP-057, GAP-137, GAP-145:
`docs/evidence/baseline/smoke/herdr-h08-attach.log:4`. A second herdr attach exited 1 and printed a refusal as terminal text:

```
attach#2 (no takeover) exited=true exitCode=1 output="\u001b[?1016$p\u001b[0 qherdr: server shut down: terminal attach failed: terminal term_65f10d20ab0831 already has an attached client; retry with --takeover\r\n"
```

Before this PR, `ovis-terminal` had no `serve`, `hold` or `terminal attach`. Every scenario's server was the runner's fixture process, and no binary fed holder output into attach streams.

## After

Linux 7.0.0-38-generic x86_64. Real `ovis-terminal` CLI/socket transcript (`docs/evidence/task-31/after-transcript.txt`; bearers redacted):

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

Conformance with the real binary as the server (`"fixture": "assembled"` starts `ovis-terminal serve`). The happy and failure commands below are the brief's literal commands; the brief's single evidence path is written by the `--case all` run:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-31 --isolated --case happy --evidence "$ATTEMPT_DIR/task-31-ovis-terminal-v1-happy.json"
{"case":"happy","error":null,...,"result":"passed","scenario":"part-a/task-31","totals":{"assertions":"23","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}   EXIT=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-31 --isolated --case failure --evidence "$ATTEMPT_DIR/task-31-ovis-terminal-v1-failure.json"
{"case":"failure","error":null,...,"result":"passed","scenario":"part-a/task-31","totals":{"assertions":"26","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}   EXIT=0
$ cargo run -q --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-31 --isolated --case all --require-no-skips --evidence "$ATTEMPT_DIR/task-31-ovis-terminal-v1.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"49","cases":"2","declared_only":"0","failed":"0","passed":"2","skipped":"0"}}   EXIT=0
```

The evidence file is committed as `docs/evidence/task-31/task-31-ovis-terminal-v1.json`. It holds every assertion, every connection transcript (the runner redacts credentials), the attach process records with the drawn screen as stdout, and the serve report.

What the scenario asserts:
- **happy:** hello advertises the attach, input and terminal routes, and `layout.get` answers `method_not_found`. `terminal.run` spawns `/bin/sh` as `t1` (`live`). The real outer-PTY client `terminal attach --target t1` makes the shell evaluate `mark-$((30+1))` to `mark-31`. `attach.leases` names `cli` as interact and geometry owner. After the outer PTY is resized to 120x40 the inner `stty size` prints `40 120`. Ctrl-\ exits 0 with `detached.reason=detach_key` and `stream_closed=true`. The outer terminal settings equal the originals (`pty[outer]/terminal_settings_restored`). `attach.leases` is empty afterwards, and the serve report shows `panicked: "0"` and no failures.
- **failure:** A second interact attach on its own PTY exits 11 with `lease_held`, naming `holder.holder=cli` and role `interact`, and its terminal settings are unchanged. `--target t9` exits 3 with `not_found`, settings unchanged. Interact without a TTY exits 2 with `not_a_tty`. The holder's interact lease is still in place. Stopping the server (forced socket EOF) makes the attached client exit 4 with `connection_lost`, and its outer terminal settings are restored.

Gate (`docs/evidence/task-31/check-summary.log`): `bash scripts/check.sh` exits 0 with `931 tests run: 931 passed (1 slow), 1 skipped` and `workspace-hygiene: 0 violation(s) in 942 file(s) checked`. `cargo nextest run -p ovis-terminal-cli` passes 25/25, and `cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings` exits 0. Windows pre-check (`windows-precheck.log`): `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0, and so does `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings`.

## What ships

- **`ovis-terminal serve [--root DIR] [--stdin-lifetime]`** (`crates/ovis-terminal-cli/src/assembly/`) wires these together:
  - the Unix endpoint listener and authenticated connections (todo 13), the bootstrap owner bearer plus an observe-only bearer (todo 19);
  - SQLite stores, the in-process shared holder (todo 6) and the lifecycle service `Terminals` (todo 16);
  - per spawned incarnation: `Screen` (17), `TerminalStreams` and `AttachLane` (23), `TerminalLeases` (24), the input `Sequencer` (25) and the `GeometryController` (26).

  One actor thread applies holder messages in order: lifecycle commit, then screen and streams, then terminal-query replies into the sequencer's reply lane, then sequencer acknowledgements and pumping to the holder.
- **Routed methods, and hello advertises exactly these:** `server.status`, `terminal.run`, `terminal.list`, `terminal.get`, `op.get`, `input.raw`, `attach.open`, `attach.credit`, `attach.close`, `attach.repaint`, `attach.resize`, `attach.leases`. Every other catalog method answers `method_not_found`; there are no stub results.
  - `input.raw` goes through `GeometryController::input_ready()`, then the interact lease of the same principal and client, then `Sequencer::admit`/`next`.
  - `attach.resize` requires a `GeometryAuthorization`. It commits `geometry.changed`, applies the VT, then the holder resize, then commits the settle events.
  - A ready interact `attach.open` that names a grid takes the geometry lease. It joins the hand-on queue and never displaces anyone.
- **`ovis-terminal hold [--session S]`** runs the session holder in the foreground. It is the existing detached `service run` path.
- **`ovis-terminal terminal attach (--target ADDR | --terminal-id ID | --name NAME) [--observe] [--client-id C]`** (`crates/ovis-terminal-cli/src/attach/`) is a plain client:
  - It uses typed `AttachOpenResult`, `AttachControl` and `AttachData` records.
  - A `RawMode` RAII guard uses TCSADRAIN, so type-ahead survives, and is restored on drop.
  - Typed bytes are held (bounded at 64 KiB) until `input_ready`, then forwarded with `input.raw`.
  - SIGWINCH triggers `attach.resize` under the geometry lease.
  - Ctrl-\, stdin EOF, SIGINT, SIGTERM and SIGHUP detach: the client sends `attach.close` and waits for the CLOSE that follows the committed lease revocation.
  - Ctrl-C goes to the program.
  - Outcome records go to stderr (stdout is the screen).
  - Observe mode needs no TTY and never reads stdin.

## Seams (public API changes outside the write scope, disclosed)

- `ovis-terminal-core`: added `Terminals::receive_message(timeout) -> Result<Option<Message>, TerminalError>` (`terminals/runtime/observe.rs`). It applies a holder message exactly like `receive` and returns it, so the serving runtime can feed screens and sequencers.
- `ovis-terminal-server`: new `streams/control.rs` with `TerminalStreams::with_leases`, `TerminalStreams::screen`, `AttachLane::leases` (`attach.leases`) and `AttachLane::acquire_geometry`. Existing streams behaviour is unchanged.
- `conformance` runner:
  - `FixtureKind::Assembled` (`"fixture": "assembled"`) starts `<binary> serve --stdin-lifetime --root <root>/srv` through the same ready/report protocol; `fixture::launch::start` now takes `(program, args)`.
  - Five PTY steps (`pty_open`, `pty_type`, `pty_expect`, `pty_resize`, `pty_exit`) in `scenario/pty.rs` and `engine/pty.rs` and `engine/pty/outer.rs`. Each step records its plan as a descriptor on every platform, and runs only on Unix (module-level cfg).
  - `runner_tests/suite.rs` registers `part-a/task-31.json` in its own `ASSEMBLED` constant, merged sorted into the inventory check. The shared `EXECUTABLE` array is untouched, so a union with integration's own additions compiles.
- CLI exit table: new kinds `not_a_tty` (2), `connection_lost` (4), `lease_held` (11), `attach_refused` (12). `src/lib.rs` gains `pub mod assembly` and `pub mod attach`, which hold only crate-visible items. The shipped manual (`skill.md`) documents the new verbs.
- Manifests: the CLI gains Unix deps on authz, core, store, vt, base64, uuid and rustix (termios/stdio/process; the version the holder resolves) plus a rustix `pty` dev-dep. The conformance runner gains a Unix rustix dep.
- Holder wire generation stays 2.

## QA

Every item ran for real; the transcripts are in the evidence above.
- `cargo nextest run -p ovis-terminal-cli`: 25/25. `--test attach --test assembly` repeated 20 times: 20/20 runs with 9/9 each.
- **tests/assembly.rs:**
  - hello's method set equals the routed set exactly; `layout.get`, `terminal.kill`, `events.subscribe` and `input.send` are refused.
  - `schema cli` and `--skill` list `serve`, `hold` and `terminal attach`.
  - `terminal.run` spawns a real child (pid > 1) that `get --address`, `get --name`, `list` and `server status` report, with `uptime_ms` a canonical decimal string.
  - `t9` exits 3 `not_found`; `first_message` gives `unsupported_field`; a numeric `op_id` gives `invalid_params`.
- **tests/attach.rs** (real outer PTY, controlling terminal via setsid + TIOCSCTTY):
  - marker, resize (`40 120`), Ctrl-\ exit 0, termios restored, leases empty;
  - Ctrl-C reaches the program and SIGINT to the client detaches with `interrupted` and restored settings;
  - held attach exits 11 naming `cli`, and stale `t9` exits 3, both with settings unchanged;
  - observe with piped stdin never makes the program run the piped command, prints the screen, and exits 0 on SIGTERM; interact without a TTY gives `not_a_tty`;
  - server stop gives exit 4 `connection_lost` with settings restored.
- Unit: `assembly::directory` resolves every explicit form and nothing else, including a `u64::MAX` identity round trip through `t18446744073709551615` and a proptest that an address resolves only to its own identity.

## Adversarial

- **cancel/resume, transport loss:** the server is stopped while attached. The client exits 4 `connection_lost`, its terminal is restored, and the serve report shows `panicked: "0"`.
- **repeated interruptions:** Ctrl-C is forwarded as a byte and kills `sleep`, and the client stays. SIGINT, SIGTERM, SIGHUP and stdin EOF each detach through `attach.close` plus CLOSE. SIGINT and SIGTERM are exercised in tests; SIGHUP shares the same code path but is not exercised by a test. Every signal was sent only to pids this test spawned, asserted > 1.
- **malformed input / stale target:** `--target t9` exits 3 before any raw mode. A numeric `op_id` gives `invalid_params`. An unrouted method gives `method_not_found`.
- **misleading success:** termios is compared as the full `Termios` before and after on the real slave. Lease release is read back from the server (`attach.leases`). The inner PTY size is proven by the program's own `stty size`.
- Found and fixed while probing:
  - type-ahead was discarded by TCSAFLUSH, so raw mode now uses TCSADRAIN;
  - keystrokes typed before `input_ready` were dropped, so they are now held and bounded;
  - the geometry settle commit reused the change's op id and the store rejected it as `DuplicateOperation`, so settle now uses its own op id.

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

## Round 3 (resumed after the host crash): restart control, crash recovery, agent tree

Lane head before this round: `188ee72` (credential rotation, 47 methods). Commits this round:
- `670b165` **feat(assembly): server.stop, durable boot identity, crash recovery at start-up.**
  - Boot opens the planned-restart `Control` (todo 48). Every lifecycle call goes through `Control::with_terminals`.
  - `server.stop` is routed. A committed stop ends serving: the report line is written and the exit code is 0.
  - Each start takes the next server boot id from `<state>/identity.json`, keeping the machine id. Counters are decimal strings: a numeric counter is refused, `u64::MAX` round-trips, and a spent counter is refused. Before this, every boot reused server boot id 1, so the lifecycle's `CoreRestart` transition could never fire.
  - Before adopting the holder, boot reads the terminals the previous boot recorded as `running`. An unplanned boot that inherited any runs core recovery (todo 49) and reports it on the ready line as `ready.recovery` (`cause`, `terminals`, `incidents`). Otherwise the OS boot is stamped.
  - Boot and recovery run off the async runtime, because the stores block (`Ticket::wait`).
- `ecbede7` Merge `origin/integration` `da5b456` (authorized). The TypeScript SDK was regenerated: `bun run generate:check` reports fresh, and `typecheck` exits 0.
- `c54a2c3` **feat(assembly): agent.children, plus hosted-lane registration split into `assembly/lanes/{machine,agents}.rs`** (one call per phase, under the 100-line cap).

hello now advertises **49** methods. That is the round-2 set plus `server.stop` and `agent.children`.

### Verification (round 3, Linux 7.0.0-38-generic x86_64)

- `cargo nextest run -p ovis-terminal-cli`: **39 tests run: 39 passed**. `cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings`: exit 0.
- `bash /home/devswha/.ovis-toolchain/with-slot.sh bash scripts/check.sh`: **EXIT=0**, `1193 tests run: 1193 passed (1 slow), 1 skipped`, `workspace-hygiene: 0 violation(s) in 1203 file(s) checked` (`check-r3.log`).
- Windows pre-check: see `windows-precheck-r3.log`.
- Conformance with the real binary (`qa-r3.log`):
  - `--scenario part-a/task-31 --isolated --case happy`: EXIT=0, `assertions 23`.
  - `--case failure`: EXIT=0, `assertions 26`.
  - `--case all --require-no-skips`: EXIT=0, `assertions 49, cases 2, skipped 0`.
- `--scenario all/all --isolated --case all --require-no-skips`: **EXIT=1**. All 48 executable cases pass with 2009 assertions and 0 failures. The run exits 1 with `declared_only_present`, because `37 declaration files are not executed`; the list is below.
- New real-socket tests:
  - `restart::server_stop_names_its_scope_ends_the_programs_and_the_server_stops_by_itself`: an unscoped stop gives `invalid_params`, one ended incarnation, a self-stop with exit 0 and `panicked: "0"`.
  - `restart::a_boot_after_a_crash_reports_only_the_programs_it_inherited`: a crash with no programs gives no report. A crash with one shell gives `cause: core_crash`, one terminal and at least one incident.
  - `lanes::hosted_lanes_...`: `agent.children` of `t1` is `[]`, and `t9` exits 3 with `not_found`.
  - Unit tests: `identity::tests` (3).
- Mutants, each killed by a shipped test (`mutants-r3.log`):
  1. Boot counter not incremented: `each_start_takes_the_next_boot_and_keeps_the_machine` fails.
  2. The inherited filter inverted: `a_boot_after_a_crash_reports_only_the_programs_it_inherited` fails.
  3. `agent.children` registration removed: the hello set test and the hosted-lanes smoke test fail.
  4. `serve` keeps a second `Arc<Control>`: `server_stop_...` fails with `stop_failed` (the control cannot be released).

### Declared scenarios still not executable (exact missing route or runner capability)

None of the 37 is in `ovis-conformance/1` format; each is a free-form descriptor. Rewriting one needs both of the following:
- every route it calls, served by the binary;
- the runner steps it describes.

| Scenario | What is missing |
|---|---|
| part-a/task-1..20 (except 21), task-3..9, task-32 | Free-form descriptors owned by those todos, with no executable steps. They need descriptor rewrites by their owners. |
| part-a/task-22 | `events.subscribe`/`update`/`unsubscribe` are unrouted. The core hub exists, but `serve` keeps four separate event logs (terminals inside `Control`, leases, geometry, dispatch), each with its own sequence. One `events.*` domain needs the coordinator unification of todo 42. |
| part-a/task-23 | Routes are served (attach.open/credit/close/repaint). The runner has no `holder_output` injection, principal fixtures (`operator`/`phone`/`bridge`) or never-acknowledging viewer step. |
| part-a/task-24, task-25 | `input.interrupt` and `input.barrier` are unrouted (no dispatch lane exists). |
| part-a/task-26 | `attach.resize` and the geometry events are served. The descriptor's geometry-lease steps have no runner step vocabulary. |
| part-a/task-27 | `turn.wait`, `events.wait` and `input.interrupt` are unrouted. |
| part-a/task-28 | The WebSocket transport is served (`serve --websocket-port`). The runner has no ws transport, `ws_ping` or expired-credential principal steps. |
| performance/fairness | The todo-56 scheduler actor is not in the output path of `serve`. |
| platform/conpty, platform/windows-service | Windows-only. `serve` is Unix-only in this PR. |
| recovery/crash | Recovery now runs at start-up (above). The runner has no step that SIGKILLs the assembled server and restarts it on the same root. |
| recovery/restart | `server.restart` is unrouted. An in-process restart moves the control to a successor boot identity, but the listener, lease log and dispatch log keep the old one. `ServerHandle::restart` plus `TerminalStreams::restarting` must be sequenced with a listener rebind first. |
| remote/identity | `machine.add` is unrouted. |
| services/docs-retention | `doc.get`/`doc.set` are served. The retention sweep, its virtual clock and `retention.floor_advanced` are not served. |
| services/plugins | `plugin.*` is served. The runner has no plugin fixture files or principal fixtures. |
| services/worktrees | `worktree.*` is unrouted: no layout `Workspaces` owner or repository configuration exists in `serve`. |

### Not delivered in round 3 (justified)

- **`update.*` (47):** `UpdateLane::register` is all-or-nothing, and `update.activate`/`rollback` restart the control. It has the same listener-rebind blocker as `server.restart`.
- **`service.describe` (51):** the lane implements `Method<OperationEvent, OperationProducer>` only, not the hosted `ServedEvent` dispatcher's event type.
- **Templates through the Sequencer with the fence handshake (35):**
  - The run path still decodes `program` only; template launches are refused `unsupported_field`.
  - **Agent tree events:** the agent tree read is served, but without template/parent launches no `agent.*` tree events are produced yet.
- **Scheduler actor in the output path (56), machine_actor `machine.list` (44), process-info through the todo-55 lane:** not wired. `terminal.process_info` is still served by the lifecycle's own process sample.
- **Not wired, because no handler lane exists to register:** `events.*` (22), `layout.*` (18), `terminal.history`/`search`, `input.interrupt`/`barrier` (24/25).

## Round 3: merge-ref compile break

CI on `1a86051` failed to compile `ovis-terminal-cli` at `assembly/terminal.rs:33` with E0308, expecting `Option<LaunchProgram>`. That type exists only on integration: T35 made `TerminalRunParams.program` optional beside `template`. The lane itself built at its base, so the failure came from CI's PR merge ref.

Fix (`d57822e`): the launch program is decoded through its serialized form, which yields `Option<LaunchProgram>` from either revision. Template-only launches stay `unsupported_field`, because `serve` does not route templates.

Verified before pushing:
- **Lane:** `cargo build -p ovis-terminal-cli` and `cargo clippy -p ovis-terminal-cli --all-targets -- -D warnings` pass, CLI tests pass 28/28, and `bash scripts/check.sh` exits 0 (931 passed, 0 hygiene violations).
- **Scratch merge with `origin/integration` `804e508`** (built from `git merge-tree`/`commit-tree` in a throwaway worktree; the lane was not merged): workspace clippy `-D warnings` is clean, and CLI plus conformance pass 53/53.

## Round 2: served lanes

Pushed on top of `f95159c` without merging integration. Six commits: `a6a8d67`, `18b446c`, `2488b58`, `6cc1fef`, `61d9341`, `b276c50`, then `5e149d7`. Each batch was gated by `bash scripts/check.sh` exiting 0 (last: 936 passed, 0 hygiene violations) plus Windows-target clippy `-D warnings` exiting 0.

hello now advertises 45 methods, each routed to a landed implementation:
- **Lifecycle (core `Terminals`):** `terminal.kill`, `rm`, `keep`, `rename`, `label`, `recover`, `process_info`. Kill waits on the lifecycle's own receive path, so afterwards the live screen and streams are reconciled from the committed record: output is replayed from the holder's tail, then the exit.
- **Reads:** `terminal.screen`, `terminal.output`, `terminal.wait_output`. Waits run on a per-incarnation core `Waits` hub fed in holder order. They are armed on the actor and awaited off it. A cursor behind retention at arming gives `cursor_expired`; a gap after arming gives outcome `gap`; cancellation gives `cancelled`.
- **Input:** `input.send`, `input.keys`, `input.paste` through the todo-25 sequencer, with geometry `input_ready()` checked first and the lease or the unleased write grant enforced.
- **Hosted dispatcher** (one `ServedEvent` coordinator):
  - `doc.get`/`doc.set` (T50);
  - `plugin.list`/`invoke`/`run_get` (T46);
  - `machine.get`/`rename` (T43);
  - `agent.register`/`get` and `status.report`/`get`/`clear` (T33);
  - `prompt.open`/`answer`/`close`/`get`/`list` (T34), with `PromptService::recover` run at startup;
  - `agent.update_session`/`conversation` (T36);
  - `op.cancel`.
- **WebSocket (T28):** `serve --websocket-port P` serves the same handler on loopback, and hello advertises `transports: ["unix","websocket"]`. Server seam: defaulted `RequestHandler::transports()` plus `ServerIdentity::with_transports`; existing handlers are unchanged.

Tests: one real-socket smoke test per wired lane (`tests/assembly/lanes.rs`, `tests/assembly/websocket.rs`). The CLI suite passes 33/33.

**Not delivered in round 2:**
- **Blocked on code absent from this lane's base (`15432bb`), which only exists on integration:** T35 template launch, T40 prompt bridge, T41 agent tree, T48 planned restart, T56 scheduler, T4 amendments, and T57 and T55 (not present in this base either). Wiring them needs integration merged into the lane, which the brief forbids. I asked how to proceed and got no answer.
- **No handler exists in the base for these; each needs new code:** `events.*` (core `Subscriptions` exists without routes), layout (`layout.*`, `selection.set`), `terminal.history`/`search`, `input.interrupt`/`barrier`, `turn.wait`/state waits.
- **Geometry catalog:** `geometry.changed`/`checkpoint`/`input_ready` events and `stale_geometry` are not yet added to the protocol catalogs.
- **Worktrees:** the lane needs a layout `Workspaces` owner and registered repositories, and `serve` has no configuration surface for either.
- **Prompt answers:** they are not typed into the sequencer, because `AnswerKeys` carries no authenticated `Session` and `Sequencer::admit` requires one. That is a seam gap between T34 and T25.
- **OSC status ingestion:** not wired; no process-token provisioning contract exists in the base.
- **Session binding:** routed with no authorized provider stores, so every session source is refused with `invalid_session`.
- **Scenario flips:** declared scenarios were not flipped. Their routes (`events.*` for task-22; restart, fairness, services and platform for the others) are among the items above, and task-23/26/27/28 still need their free-form descriptors rewritten as executable steps.

## Risks and not delivered

- `serve` owns an in-process holder, so programs end with the server. `hold` runs a standalone holder that `serve` does not adopt; out-of-process adoption is todo 48.
- Terminals, leases and geometry use three SQLite files under `<state>/`; todo 42 unifies the coordinators. The machine id is minted per `serve` lifetime.
- `TerminalRecord.finished.finished_ms` is `"0"` (holder `Exit` carries no time). `created_seq` is the first revision this server saw. `status` and `status_at_send` are the never-reported snapshot, because status routes are not served.
- Not wired in this PR, so not advertised: `terminal.kill` and other mutations, `events.*`, waits, prompts (`PromptService::recover` is not run), layout, worktrees, plugins, documents, the WebSocket listener, and geometry.* events or `stale_geometry` in the protocol catalogs. Declared scenarios (task-22/23/26/27/28, services/*, platform/*) stay declared.
- `--observe` never forwarding input is asserted in `tests/attach.rs`, not in the conformance scenario: the runner's `run` step cannot stop a long-running observer.
- No macOS stress job was added to `.github/workflows/probe-stress-macos.yml`. That file is a shared union-merge target, and this PR does not touch it.
- `origin/integration` moved (to `7521817` at round 2) during the lane. Per the brief it was not merged; the orchestrator union-merges.
