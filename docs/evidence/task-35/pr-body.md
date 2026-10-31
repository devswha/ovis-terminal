feat(template-launch): launch versioned templates with readiness bound messages

Plan todo 35. Closes GAP-083 and GAP-116. Delivers IDEAL-060 and IDEAL-089. Enforces EXCL-023, EXCL-024 and EXCL-025, and does not reproduce V-4429, V-4431 or V-RT-HC-01.

## Before

`absent`. There was no comparable capability: `template.list` and `template.get` were catalog entries with no route, `terminal.run` was not served by any fixture or runtime, and there was no first-message state. This is not an executed failing test. Gap baseline: GAP-083, GAP-116.

## What changed

- **protocol `domain::template_launch`**
  - `ReadinessContract` is a closed tagged enum, `{by: "declared", states}`. It allows only `idle`/`done`; unknown states and `quiet` are refused (fixes V-4429, the bool `ready_when_quiet`).
  - `FirstMessageState` is `accepted|sent|cancelled|expired`. `FirstMessageReason` and `FirstMessageRecord` carry `queued_seq` and `deadline_ms` as decimal strings.
  - `LaunchSource::of` requires exactly one of `program`/`template`.
  - `LaunchedTerminal` assembles `TerminalRunResult`.
  - `TemplateLaunchError` has `kind()`/`hint()`.
- **services `templates`**
  - `TemplateCatalog` reads closed JSON definitions fresh on every request: `list`, and `get(name, version?)`. With no version it selects the latest dotted-numeric version.
  - `Template::plan` and `LaunchPlan::direct` produce the same `LaunchPlan` (literal argv, env, readiness states, first message). Task text goes only into the first message, never into argv.
  - `TemplateError` (`template_not_found` / `template_invalid`) and `DefinitionError` are thiserror enums, never strings (fixes V-4431).
- **core `launch`**
  - `FirstMessages` is keyed by launch op id. It is owned by the requesting principal (another principal gets `forbidden`, an unknown launch `not_found`), and accepting the same launch twice returns the retained record.
  - `arm` puts a todo-27 `StateWait` on the reserved terminal's `Waits` hub *before* the program starts. `await_ready` blocks on the condvar with no sleeps.
  - `WriteBoundary::admit` rechecks the requesting principal's authority decision (`authority_lapsed`), the interact lease (`lease_held`) and the process incarnation (`process_replaced`). It then encodes the literal text plus Enter through the todo-32 encoder (`encoding_refused`).
  - `release` writes at most once. `settle` ends a message `expired` (deadline) or `cancelled` (exit, lost link, runtime stopped) with zero writes.
- **server `lanes::template_launch`**
  - `terminal.run` serves argv and template programs on one path through a `Launcher` seam (reserve → arm → queue → start → worker). A missing template is refused with `template_not_found` before `reserve`.
  - `terminal.first_message` takes an optional `settled_within_ms` bounded wait.
  - `template.list` and `template.get` are unrecorded reads.
  - Dropping the lane cancels every pending wait and joins its workers.
- **conformance**
  - Executable scenario `agent/launch` (happy + failure), plus the fixture `template_lane.rs` and `launches.rs`.
  - The fixture's `RecordingLauncher` stands in for the holder until todos 31/42 assemble `terminal.run` on the runtime. It starts **no process**: it records argv/env/parent in `effects/spawn-*.json` and submissions in `effects/input-*.json`.
  - Readiness reaches the wait hub from the committed `status.changed` events of the real todo-33 `status.report` lane, through a subscription installed at fixture start.

Agent-declared readiness gates only this first message. Ordinary input and stream geometry are untouched (Todo 26).

## After

Native OS/arch: `Linux 7.0.0-38-generic x86_64`. Binary: `target/debug/ovis-terminal`, sha256 `985313bea3b9c937d60d59a2c30f4938439c02d1488b25335b07f516584aad7b` (from the evidence `binary` field).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                    # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/launch --isolated --case happy --evidence "$ATTEMPT_DIR/task-35-ovis-terminal-v1.json"
{"case":"happy",...,"result":"passed","scenario":"agent/launch","totals":{"assertions":"60","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/launch --isolated --case failure --inject '{"method":"terminal.run","params":{"template":{"name":"build","version":"missing"},"first_message":{"text":"x"}}}' --evidence "$ATTEMPT_DIR/task-35-ovis-terminal-v1.json" --append
{"case":"failure",...,"result":"passed","scenario":"agent/launch","totals":{"assertions":"40","cases":"1","declared_only":"0","failed":"0","passed":"1","skipped":"0"}}
exit=0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario agent/launch --isolated --case all --evidence "$ATTEMPT_DIR/task-35-all.json"
{"case":"all",...,"result":"passed","totals":{"assertions":"103","cases":"2","failed":"0","passed":"2","skipped":"0"}}
exit=0
```

The materialized socket traffic below goes through the `ovis-terminal` bridge (`connect`). It is taken from `docs/evidence/task-35/task-35-wire.log`; ids and op ids are as recorded, long records are elided with `...`.

Template not found, before spawn. The effects directory stays `{}` afterwards, so no spawn file exists:
```
>> {"id":"inject-0","method":"terminal.run","params":{"cwd":"/","first_message":{"text":"x"},"geometry":{"cols":80,"rows":24},"op_id":"01a19385-ab92-...","template":{"name":"build","version":"missing"}}}
<< {"id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"template_not_found","hint":"list templates and pin an existing name and version","retryable":false,"op_id":"01a19385-ab92-..."}}}
```

Happy path: pinned build@1. The message is `accepted` with zero input, still `accepted` after a `working` declaration, and `sent` with exactly one submission after `idle`:
```
<< {"id":"launch","result":{"first_message":{"state":"accepted","queued_seq":null,"terminal_id":"terminal:100",...},"spawn":"spawned","terminal":{"process":{"argv":["/usr/bin/env","build-agent","--target","debug"],...},...}}}
effects: {"spawn-terminal:100.json": {...}}                                      # no input file
<< {"id":"still","result":{"state":"accepted","queued_seq":null,"reason":null,...}}   # after status.report working
<< {"id":"sent","result":{"state":"sent","queued_seq":"1","reason":null,...}}          # after status.report idle
effects: {"spawn-terminal:100.json": {...}, "input-terminal:100.json": [{"queued_seq":"1","text":"run the build\r"}]}
```

Never-ready launch (silent@1, template deadline 400 ms):
```
<< {"id":"expired","result":{"state":"expired","reason":"deadline_passed","queued_seq":null,...}}
effects: {"spawn-terminal:100.json": {...}}                                      # zero input
```

## QA

- happy: the command above exits 0. It covers `template.list` (build@1, build@2, silent@1 with `readiness` and `ready_deadline_ms`) and `template.get` latest (2, release argv) versus pinned (1, debug argv). The launch then goes accepted → (working) accepted → (idle) sent `queued_seq "1"`, and exactly one recorded submission `run the build\r`.
- failure (QA literal inject): the command above exits 0 only because the injected request answers `template_not_found` and the effects directory is empty afterwards. The rest of the case asserts:
  - program and template together → `invalid_params`; an unknown launch → `not_found`;
  - silent@1 → `expired`/`deadline_passed` with zero input;
  - the task-injection launch (next section).
- Without `--inject`, the failure case also runs the planned `first_message.ready_by:"quiet"` request → `invalid_params` at `params.first_message.ready_by`.
- An unplanned `--inject` (`build@9`) exits 1 (`docs/evidence/task-35/qa-transcript.log`).

## Adversarial

- **Hung commands (ready deadline):** silent@1 never declares readiness, so the message expires at its deadline with zero writes (conformance and `core::launch::tests::flow`). Expiry comes from the wait's own deadline, not from polling.
- **Repeated interruptions (one retained launch receipt):** resending `terminal.run` with the same op_id answers the retained receipt for `terminal:100`. There is no second `spawn-*` file and no second submission, and a later `idle` (seq 3) does not write again. The retained receipt is the commit-time record (`accepted`); the current state comes from `terminal.first_message`.
- **Prompt injection:** task `$(touch PWNED)` leaves argv as `["/usr/bin/env","build-agent","--target","debug"]`. It is written once, literally, as `$(touch PWNED)\r` (`input-terminal:101.json`), and no `PWNED` effect appears. In `services::templates::tests`, task text with `` `id` `` and `${HOME}` stays in the first message, and no `PWNED` file is created.
- **Fast readiness:** `server lanes::template_launch::qa::readiness_declared_while_starting_is_not_lost_and_releases_once` declares `idle` inside `start()`, before the worker waits. The message is still `sent` once (`go\r`). The core test covers a declaration made before `arm`.
- **Write-boundary rechecks:** revoking the requester between launch and readiness gives `cancelled`/`authority_lapsed`, read back over the wire with a fresh credential for the same principal. An interact lease taken before readiness gives `cancelled`/`lease_held`. Both cases make zero writes. Core boundary tests also cover `process_replaced` and `encoding_refused` (multiline without bracketed paste).
- **Foreign principal:** `FirstMessages` answers `forbidden` for another principal's launch (core test).

## Contract checklist

1. Every u64 is a decimal string: `queued_seq`, `deadline_ms`, `ready_deadline_ms`, `settled_within_ms`. A u64::MAX exact round trip is tested, a numeric `queued_seq` is rejected, and proptest round-trips `FirstMessageRecord`.
2. Boundary errors are thiserror enums with `kind()`/`hint()`, with `#[source]` where wrapping: `TemplateLaunchError`, `TemplateError` (+ `DefinitionError` source), `LaunchError` (+ `WaitError` source), `LaunchRefusal`. Display never repeats the kind.
3. Identities are newtypes: launch `OpId`, registry `TerminalId`/`ProcessGeneration`, wire `TerminalId`/`Incarnation`/`EventSeq`, `TemplateName`, `TemplateVersion`, `PrincipalId`.
4. Tests assert behavior (state transitions, typed kinds, recorded writes). proptest covers version ordering and record round trips.
5. No `#[allow]`/`#[expect]`, no TODOs. The `Launcher` trait is the seam the unassembled holder path (todos 31/42) plugs into.
6. Holder wire generation stays 2. The contract checklist items are unchanged.

Commands, all exit 0 unless noted:
- `bash scripts/check.sh` → exit 0: nextest 840 tests, workspace hygiene 0 violations, manifests, json.
- `cargo clippy -p ovis-terminal-core -p ovis-terminal-services -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings` → exit 0.
- `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` → exit 0.
- `cargo test -p ovis-terminal-core -p ovis-terminal-services -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` → exit 101. This is **outside this lane**: `core --test subscriptions durability::close_forced_returns_only_after_its_marker_is_durable` (todo 22) failed with `Error: Disconnected` under `cargo test`'s in-binary parallelism. It passes 10/10 alone and passes in check.sh's nextest run. With `--no-fail-fast`, 521 passed and that 1 failed (`docs/evidence/task-35/acceptance-summary.log`). Lane-relevant tests:
  - protocol `template_launch`: 8;
  - services `templates`: 7;
  - core `launch`: 10;
  - server `template_launch` QA: 2.

## Seams

- `TerminalRunParams.program` is now `Option<LaunchProgram>`, and there is a new optional `template: TemplateSelector` (the spec's top-level `template`, needed by the plan's QA literal). Exactly one of the two is required at the domain level (`invalid_params`). `FirstMessage.deadline_ms` is now optional (default 30 s).
- `TerminalRunResult` gains `first_message: Option<FirstMessageRecord>`.
- `TemplateRecord` gains `env`, `readiness`, `ready_deadline_ms` and `body`.
- New method `terminal.first_message` (Query, Untargeted).
- `schema/ovis-terminal.openrpc.json` is regenerated. `openrpc compat schema/released.openrpc.json` → compatible: inputs only widen, outputs only add.
- `ConptyLaunch::admit` matches `Some(LaunchProgram::Argv)`; a top-level template is `TemplateUnresolved` as before.
- Conformance:
  - `fixture/agents.rs` `Directory` grows with launched names.
  - `serve.rs` registers the lane.
  - `conformance/Cargo.toml` adds the holder/services deps, and Cargo.lock gains 2 lines.
  - `protocol/discovery.json` excludes `terminal.kill` instead of `terminal.run`, because the fixture now serves `terminal.run`.
  - `suite.rs` EXECUTABLE adds `agent/launch.json` (N = 14).
- `server/tests/dispatch.rs` gains the `template_launch_memory` and `template_launch` path modules.
- The `probe-stress-macos.yml` `template-launch` option and job are appended at the end.

## Risks

- `terminal.run` is not yet assembled on the real runtime. The conformance fixture uses a recording launcher that starts no process; the runtime `Launcher` over core `Terminals` + holder is todo 31/42 work.
- The first-message book is in memory and does not survive a restart. The write path goes through the encoder plus the launcher, not the todo-25 `Sequencer` (which needs a live `Session` at admission), so `queued_seq` is the launcher's sequence.
- The input decision for the first message is taken in `resolve` and kept, bounded at 256, until `run`.
- `origin/integration` moved to `436506e` (T78, T79) after the lane base `81cb6bf`. Per the lane rules this branch is **not** merged. Expected union points are `suite.rs` EXECUTABLE and the macOS workflow options/jobs.
