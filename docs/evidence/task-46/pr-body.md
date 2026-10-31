# feat(plugins): persist scoped plugin catalogs and bounded run outcomes

Plan todo 46. Closes GAP-117, GAP-118, GAP-136, GAP-144; delivers IDEAL-090, IDEAL-091, IDEAL-109, IDEAL-117 at the RPC route level (see DEFERRED VERIFICATION for the served-process part).

## Before

`absent` (no comparable baseline capability; not an executed failing test). Gap baseline: GAP-117, GAP-118, GAP-136, GAP-144.

## What changed

- `crates/ovis-terminal-services/src/plugins/`: a plugin service rooted at one runtime-owned directory (`<root>/<plugin_id>/plugin.json`, never a user-global path).
  - The manifest format is closed (`deny_unknown_fields`, a 64 KiB bound, and the directory name must equal `plugin_id`); a violating manifest fails discovery with `plugin_invalid`.
  - Each action lists its id, `input_schema`, `contexts` and an explicit `unavailable` reason: `plugin_disabled`, `platform_unsupported`, `executable_missing` or `definition_invalid`.
  - An action is an absolute program plus an argv template. A `{field}` element becomes exactly one argv element; there is no shell and no partial interpolation. Input is validated against a supported JSON-Schema subset (a closed object of string, integer and boolean properties).
  - The program runs with a cleared environment: only `PATH`, `SystemRoot` and `LANG` are copied, plus `OVIS_PLUGIN_RUN_ID`, `OVIS_PLUGIN_ACTION` and `OVIS_PLUGIN_INPUT`. It gets no stdin. stdout and stderr go through one pipe, so their bytes stay in the order the program wrote them.
  - Output keeps a 4 KiB tail with `output_floor` (decimal string) and `truncated`. The run settles from the exit status: 0 is `succeeded`, any other status is `failed` with its `exit_code`, and a killed run is `cancelled`. Cancellation kills only the direct child pid (never a group, and never 0, 1 or -1).
- `crates/ovis-terminal-server/src/dispatch/lanes/plugins.rs`: `plugin.list`, `plugin.invoke` and `plugin.run_get` as `Method` impls, generic over the dispatcher's event and producer types; `routes()` hands them to assembly.
  - `plugin.invoke` is authorized as `plugin_invoke` on `TargetResource::Plugin` before the catalog is read.
  - The program runs inside `Execution::commit`: the ledger stays `running` while it runs, then commits the settled `PluginRunRecord` as the operation receipt.
  - `plugin.run_get` reads the caller's own principal-scoped ledger: a committed receipt, the live record while running, or `outcome_unknown`/`cancelled` via the landed `Settled` kinds.
- `crates/ovis-terminal-protocol/src/domain/plugins.rs`: constructors for the `#[non_exhaustive]` plugin wire records, the `Unavailable` and `PluginContext` tokens, and the `PluginRunId::of_operation` / `operation()` mapping (one invocation is one run).
- `conformance/scenarios/services/plugins.json` (declaration) plus a `declared.json` entry; `.github/workflows/probe-stress-macos.yml` gains a `plugins` option and job (appended last).

## Seams (public API changes outside the write scope; disclosed)

1. `ovis_terminal_server::dispatch::DispatchError::Domain { kind, hint, source }`: a new variant on the `#[non_exhaustive]` enum, so lanes can return their own published kinds (`plugin_disabled`, `action_unavailable`, `spawn_failed`, ...). Before this, the landed enum had no domain variant, and a lane could only surface `internal` or `not_found`. `kind()` and `hint()` delegate to the fields, and `boundary()` is unchanged (it builds the domain error from `kind()` and `hint()`). The lane tests assert the resulting wire kinds.
2. `ovis_terminal_protocol::errors::OpId::as_str()`: a borrowed accessor, same shape as `FieldPath::as_str`.
3. The lane reads `Execution`'s `pub(super)` fields `cancellation`, `state.core` and `context` from inside `dispatch::lanes` (a descendant of `dispatch`). No visibility was widened.
4. `crates/ovis-terminal-services/Cargo.toml` adds `base64` 0.22 (already in Cargo.lock), `serde`, `serde_json`, `thiserror`, `tokio`, and `proptest` as a dev-dependency. `crates/ovis-terminal-protocol/src/domain.rs`, `dispatch/lanes.rs` and `services/src/plugins.rs` are slot module files.
5. The scenario id `services/plugins` resolves to `conformance/scenarios/services/plugins.json`, because the runner's id grammar is exactly `<family>/<name>`. That file sits next to the write-scope directory `conformance/scenarios/services/plugins/`, not inside it.

Holder wire generation stays 2. The settled-contract checklist items are unchanged.

## After

Native platform: `Linux 7.0.0-38-generic x86_64`. Binary under test: `target/debug/ovis-terminal`, sha256 `7d4967003bba55be8d8ba5f81dd3212c506db3c725524e55c266c61d6aa71f36`.

| Command | Exit | Evidence |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `build.log` |
| `cargo test -p ovis-terminal-services -p ovis-terminal-protocol -p ovis-terminal-server --all-targets` | 0 | `acceptance-tests.log` (25 plugin tests: 7 protocol, 15 services, 3 server) |
| `cargo clippy -p ovis-terminal-services -p ovis-terminal-protocol -p ovis-terminal-server --all-targets -- -D warnings` | 0 | `acceptance-clippy.log` |
| `bash scripts/check.sh` | 0 | `check-summary.log` (nextest 580 run: 580 passed, 1 skipped; hygiene 0 violations) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | local cross-check |
| `conformance ... --scenario all/all --isolated --case all` | 0 | `conf-suite.log`: 16 cases passed, 345 assertions, 0 failed, 0 skipped; `services/plugins` counted among 24 declared-only files |

The happy observable over a real Unix socket, with the real `Dispatcher`, `Authority` and SQLite ledger (`cargo test -p ovis-terminal-server --lib lanes::plugins -- --nocapture`; full transcript in `lane-transcript.log`):

```
-> {"method":"plugin.invoke","params":{"action_id":"report","input":{},"op_id":"01a19265-a36d-731f-868f-7da48d1992a8","plugin_id":"fixture","target":null}}
<- {"result":{"created_terminals":[],"exit_code":7,"output_b64":"T0sK","output_floor":"0","run_id":"plugin-run:01a19265-a36d-...","state":"failed","truncated":false}}
-> {"method":"plugin.run_get","params":{"run_id":"plugin-run:01a19265-a36d-731f-868f-7da48d1992a8"}}
<- {"result":{"created_terminals":[],"exit_code":7,"output_b64":"T0sK","output_floor":"0",...,"state":"failed","truncated":false}}
-> {"method":"op.get","params":{"op_id":"01a19265-a36d-731f-868f-7da48d1992a8"}}
<- {"result":{"method":"plugin.invoke","result":{...,"exit_code":7,"state":"failed"},"state":"committed","updated_seq":"6"}}
```

The failure observable from the same harness: an observe-only principal is forbidden before any spawn, and a disabled action reports its reason:

```
-> (observe principal) {"method":"plugin.invoke","params":{"action_id":"admin-action","plugin_id":"fixture",...}}
<- {"error":{"code":-32000,"data":{"kind":"forbidden","hint":"ask an administrator for a grant covering this target","retryable":false}}}
   test asserts plugins/fixture/admin.sh.spawned does not exist (the action script writes it when it runs)
-> (owner) {"method":"plugin.invoke","params":{"action_id":"noop","plugin_id":"off",...}}
<- {"error":{"data":{"kind":"plugin_disabled","hint":"enable the plugin before invoking its actions","op_id":"..."}}}
-> op.get -> {"state":"failed","error":{"kind":"plugin_disabled",...}}
-> (observe principal) plugin.list -> plugins[1].actions[0].unavailable == "plugin_disabled"
```

## DEFERRED VERIFICATION (runner lands in Todo 21)

Todo 21 has landed, but the runner cannot execute this surface yet. No served process registers `plugin.*`: the todo-21 fixture registers only `doc.set`, `doc.get` and `terminal.keep`, and composing service lanes into the real server is todo 52 (`dispatch/services.rs`). The runner also has no `--inject`, `--append` or `principal_fixture`. `services/plugins` therefore ships as a declaration listed in `declared.json` (owner 46). Verbatim attempts:

```
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/plugins --isolated --case all --evidence "$ATTEMPT_DIR/task-46-ovis-terminal-v1.json"
{"case":"all","error":{"detail":"services/plugins is a declaration, not a registered executable scenario","hint":"register the scenario with \"format\": \"ovis-conformance/1\" and typed steps","kind":"scenario_not_executable","step":null},...,"result":"failed","scenario":"services/plugins","totals":{"assertions":"0","cases":"0","declared_only":"0","failed":"0","passed":"0","skipped":"0"}}
EXIT=1
$ cargo run ... --scenario services/plugins --isolated --case happy --evidence "$ATTEMPT_DIR/task-46-ovis-terminal-v1.json"
(same scenario_not_executable) EXIT=1
$ cargo run ... --scenario services/plugins --isolated --case failure --inject '{"method":"plugin.invoke","params":{"action":"admin-action"},"principal_fixture":"observe"}' --evidence "$ATTEMPT_DIR/task-46-ovis-terminal-v1.json" --append
error: unexpected argument '--inject' found
Usage: conformance --binary <BINARY> --scenario <SCENARIO> --isolated --case <CASE>
EXIT=2
```

The plan's real-binary acceptance (failed:0, skipped:0, binary hash, request/result evidence from the runner) is not met by this PR. The same behavior is asserted over a real socket by `dispatch::lanes::plugins::tests`.

## QA

- happy: `printed_ok_then_exit_7_is_retained_as_a_failed_run`. It asserts, over the socket, that `plugin.list` reports `plugin_disabled`; that invoke returns `state:"failed"`, `exit_code:7`, `output_b64:"T0sK"` (`OK\n`), `output_floor:"0"`, `truncated:false` and `created_terminals:[]`; that `plugin.run_get` returns an identical record; and that `op.get` shows `committed` with that record as its result.
- failure: `an_observer_is_forbidden_before_any_program_starts`. It asserts `forbidden` and the absence of the marker file the script writes when it runs; `plugin_disabled` for the disabled action, and its ledger receipt `failed` with error kind `plugin_disabled`; that the observer's `plugin.run_get` of the owner's run is `not_found`; and that the observer's `plugin.list` shows the reason.

## Adversarial

- Misleading success: a script that prints `OK` and exits 7 settles `failed`, `exit_code:7` (services and server tests). Status comes from the exit status only.
- Hung commands: `cancelling_a_hung_run_kills_it_and_commits_cancelled`. The script writes a FIFO once it runs; the test blocks on that read (no sleep), then sends `op.cancel`. The cancel returns `outcome:"pending"` with `cancellation_requested:true`, the invoke reply is `state:"cancelled"` with `exit_code:null`, and `op.get` shows that record as its committed result. The services test `cancellation_kills_a_hung_program_and_records_cancelled` runs `exec sleep 600` under a 30 s bound.
- Prompt or shell injection: `shell_operators_in_input_stay_one_literal_argument`. The input ``x; echo pwned $(id) | tee /tmp/ovt46-pwned && `id` `` arrives as `$#`=1 with the identical text, and `/tmp/ovt46-pwned` is not created. A proptest (`any_text_input_is_exactly_one_argument`) checks that any NUL-free text expands to exactly one argument. Manifests that splice a placeholder into a larger argument (`-c "echo {name}"`) are listed `definition_invalid`.
- Credential and lease inheritance: `the_program_sees_only_the_allowlisted_environment` runs `/usr/bin/env` and asserts that only allowlisted and `OVIS_PLUGIN_*` variables (plus the shell's own `PWD`, `SHLVL`, `_`) reach the program.
- Bounded output: a 10,004-byte run keeps exactly 4,096 bytes ending in the last line, with `output_floor` 5,908 and `truncated:true`. A proptest checks the tail and floor against the concatenated stream under random chunking.

## Contract checklist

1. u64 on the wire: `output_floor` is `ByteOffset` (a decimal string). `a_settled_run_round_trips_with_a_decimal_floor_at_u64_max` covers the u64::MAX round trip, and `a_numeric_output_floor_is_rejected` covers the numeric form. `exit_code` is the process's `i32`, not a counter.
2. Errors: `PluginError` is a `thiserror` enum with `kind()` and `hint()`, and `#[source]` on its I/O and serde causes. `display_never_repeats_the_kind_token` checks Display.
3. Identities: `PluginId`, `PluginActionId`, `PluginRunId`, `TerminalId` and `OpId` stay distinct newtypes. The run-to-op mapping is typed, with a proptest round trip.
4. Behavior tests: state transitions over a real socket and real processes, typed kinds, and proptests for argv expansion, the output tail and id mapping. No test reads back a literal it assigned.
5. No `#[allow]`, no new `#[expect]`, no TODOs. Unix-path tests are gated with a module-level `cfg(all(test, unix))`, and the Windows `-Dwarnings` check passes.
6. Claims above are limited to what the named tests assert.

## Risks

- `op.get` on a failed run shows `state:"committed"`, because the operation's effect, running the program, completed and is proven. The failure lives in `result.state`/`result.exit_code`, which `plugin.invoke` and `plugin.run_get` return directly. A client must read `result`, not only the operation state.
- A spawn failure inside the commit boundary is recorded as `outcome_unknown` (the landed `Execution::commit` contract has no "proven no-effect" path after `start`). The reply kind is still `spawn_failed`. Missing executables are caught earlier, as `executable_missing`.
- Only the `machine` context exists. Invoking with a `target` returns `action_unavailable`, because terminal-context actions need registry resolution from composition.
- `created_terminals` is populated only through `Plugins::link_created` (unit-tested). No served terminal-launch path calls it yet, and plugins receive no runtime credential (none is inherited), so plugin-created resources must be wired by todo 52 through normal authorization.
- `plugin_invalid` is a new open-set kind token that is not yet in `DomainErrorKind`.
- A descendant that keeps the output pipe open after the child exits keeps the run `running` until `op.cancel`. Cancellation kills only the direct child.
- Live run state is in memory. After a restart, a run that was in flight reads as `outcome_unknown` through the ledger, the landed restart semantics.
