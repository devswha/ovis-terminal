# Todo 52: exercise remote services and recovery over real endpoints

Wave 15. Closes in this PR (ids declared by the executable scenarios and listed in `docs/design/traceability.json`): GAP-106, GAP-108, GAP-109, GAP-118, plus IDEAL-080, IDEAL-082, IDEAL-083, IDEAL-091 and IDEAL-117. Exclusions enforced: EXCL-014, EXCL-016, EXCL-020 and EXCL-021. The other ids in the todo block are listed under **Unmet** below and are not claimed.

## Before

`absent`. There was no comparable capability; this is not an executed failing test. Gap baseline: GAP-106, GAP-108, GAP-109, GAP-114, GAP-118, GAP-119, GAP-121, GAP-122, GAP-123, GAP-124, GAP-127, GAP-128. Base-tree citations on `origin/integration` 167c0a7 are in `docs/evidence/task-52/before.md`:

> `conformance/scenarios/declared.json:172` "... todo 52 composes remote dispatch across two private runtimes over real endpoints."
> `conformance/scenarios/declared.json:182` "plugin.* routes exist ... but no served process registers them: todo 52 composes service lanes into the real server"
> `conformance/src/fixture/machines.rs:8` "`fixture.remote` stands in for remote dispatch (todo 52)"

The real endpoints also exposed a cross-lane defect, now fixed. `crates/ovis-terminal-remote/src/actors/actor/expiry.rs:42` sends an id-less `$/cancelRequest` notification after a deadline, and `crates/ovis-terminal-server/src/conn/core.rs:130` closes any connection that sends an uncorrelatable record. The result: one stalled call to a real runtime cut that machine's line. The remote/e2e happy case reproduced this before the fix (`peer-after-stall` → `machine_unreachable`).

## What changed

- **Remote dispatch** (`crates/ovis-terminal-server/src/dispatch/remote.rs`, plus `remote/{route,refusal,attach}.rs`):
  - `RemoteDispatch<L, H>` wraps the served handler. A request for another machine goes only to that machine's actor:
    - when `params.target.address` is qualified `<machine>/<rest>`, the qualifier is removed before forwarding;
    - when `params.machine` is a `MachineRef` object, it is removed before forwarding.
  - The machine is resolved once, by id, name or alias. An unknown qualifier is `not_found` at the field path; there is never a local fallback.
  - A machine without a line refuses at once as `machine_unreachable`, carrying `target` and the new `last_seen_seq`. Nothing is sent.
  - A mutation (`op_id`) whose line is lost after its call was handed to the line settles as `outcome_unknown` with its `op_id`. A read that is lost the same way is `machine_unreachable`.
  - A deadline keeps the `op_id`. The machine's own typed refusal passes through unchanged.
  - Local authorization: `input.*` needs Input, `terminal.run` needs Launch, other mutations need Administer, reads need Read.
- **Adding and reconnecting machines:** `MachineAddAttached` composes T43 `machine.add` with the T44 actor table, so the authenticated link the add opened becomes the actor's line. `reconnect()` re-proves a lost machine over a fresh challenge and re-attaches it, replaying orphan notices.
- **Server cancellation** (`conn/core.rs`, `conn/driver.rs`, `conn/driver/settle.rs`): on an established connection, `{"method":"$/cancelRequest","params":{"id":...}}` aborts that request's reply task, which is answered `cancelled`. The connection keeps serving. Any other id-less record still closes it, as before.
- **Service composition** (`dispatch/services.rs`): `ServiceLanes` registers the plugin and update lanes on one dispatcher.
- **Error data:** `ErrorData.last_seen_seq` is an optional `EventSeq`: a decimal string, omitted when absent. It comes with `BoundaryError::with_last_seen_seq`.
- **Conformance fixture:** a second private runtime (the runner re-run as `fixture --peer`, with machine id `…c3`, its own authority and its own store). It is reached through the real `machine.add` over an argv transport that runs **the binary under test** as `ovis-terminal connect`. Fault controls are `fixture.peer` (stop/resume via SIGSTOP/SIGCONT, crash, restart) and `fixture.lost_machine`. Fixture plugins are served through `ServiceLanes`.
- **Two new executable scenarios:** `remote/e2e` and `recovery/e2e`.

## After

Native platform: `Linux 7.0.0-38-generic x86_64`. Binary under test: `/dev/shm/ovt-target-lane-t52/debug/ovis-terminal`, sha256 `f0cc6a64a121993314eb5b5e02d545c8e5a6ae050f512a4fdf4979fd6f2cf903`. Host rule: the cargo target lives on RAM, so `target/debug/ovis-terminal` in the plan's commands is `$CARGO_TARGET_DIR/debug/ovis-terminal` here. `ATTEMPT_DIR=/home/devswha/.ovis-run/scratch/lane-t52/attempt`; the evidence is copied into `docs/evidence/task-52/`.

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | `build.log` |
| `cargo test -p ovis-terminal-server -p ovis-terminal-services -p ovis-terminal-remote --all-targets` | 0 | 303 passed, 0 failed; lane tests: 8 `dispatch::remote::qa`, 1 `dispatch::services`, 2 `conn::core` cancellation (`acceptance-tests.log`) |
| `cargo clippy -p ovis-terminal-server -p ovis-terminal-services -p ovis-terminal-remote --all-targets -- -D warnings` | 0 | `acceptance-clippy.log` |
| `conformance -- --binary $CARGO_TARGET_DIR/debug/ovis-terminal --scenario recovery/e2e --isolated --case all --evidence "$ATTEMPT_DIR/task-52-ovis-terminal-v1.all.json"` | 0 | failed 0, skipped 0, passed 2, 135 assertions |
| `conformance -- ... --scenario recovery/e2e --isolated --case happy --evidence "$ATTEMPT_DIR/task-52-ovis-terminal-v1.json"` | 0 | passed 1, 48 assertions |
| `conformance -- ... --scenario recovery/e2e --isolated --case failure --inject '{"method":"input.send","params":{"target":{"address":"disconnected/t1"}}}' --evidence "$ATTEMPT_DIR/task-52-ovis-terminal-v1.json" --append` | 0 | passed 1, 84 assertions |
| `conformance -- ... --scenario remote/e2e --isolated --case all --evidence "$ATTEMPT_DIR/task-52-remote-e2e.json"` | 0 | failed 0, skipped 0, passed 2, 89 assertions |
| `cargo test -p ovis-terminal-conformance` (runs `all/all`: inventory, traceability) | 0 | `conformance-crate-tests.log` |
| `bash with-slot.sh bash scripts/check.sh` | 0 | nextest 1285 run: 1285 passed, 1 skipped; hygiene 0 violations (`check-summary.log`) |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 | |
| `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` | 0 | |
| `cd packages/client-ts && bun run generate && bun run generate:check && bun run typecheck && bun test` | 0 | 30 pass, 0 fail (SDK regenerated for `last_seen_seq`) |
| `openrpc check schema/ovis-terminal.openrpc.json` / `openrpc compat schema/released.openrpc.json` | 0 / 0 | additive optional field |

Materialized commands the runner executes for each case:

- `conformance fixture --root /tmp/ovc-<pid>-<n> --binary <bin>`: the runtime under test.
- `<bin> connect`: the bridge the scenario speaks through.
- `conformance fixture --root <root>/peer-0 --peer`: the second private runtime.
- `/usr/bin/env OVIS_TERMINAL_SOCKET=<root>/peer-0/run/s.sock <bin> connect`: the stdio transport `machine.add` spawns. Its `hello` carries the peer's credential and a fresh boot challenge.

Raw replies from `task-52-ovis-terminal-v1.json` (failure case, run with `--inject`):

```text
sent:     {"id":"inject-0","jsonrpc":"2.0","method":"input.send","params":{"target":{"address":"disconnected/t1"}}}
received: {"jsonrpc":"2.0","id":"inject-0","error":{"code":-32000,"message":"operation failed","data":{"kind":"machine_unreachable","hint":"reconnect the machine; nothing ran locally instead","retryable":true,"target":"machine:0191f1a2-0000-7000-8000-0000000000d4","last_seen_seq":"41"}}}
received: {"jsonrpc":"2.0","id":"lost-input","error":{"code":-32000,"message":"operation failed","data":{"kind":"outcome_unknown","hint":"inspect the operation on the machine once it is reachable before retrying","retryable":false,"target":"machine:0191f1a2-0000-7000-8000-0000000000c3","op_id":"01a19659-e0e3-7031-b6e9-80d5880f6a36"}}}
received: {"jsonrpc":"2.0","id":"crash","result":{"crashed":true,"health":{"state":"unreachable","last_failure":{"kind":"transport_failed",...},"line_epoch":"1","in_flight":"0",...}}}
received: {"jsonrpc":"2.0","id":"no-local-fallback","error":{"code":-32000,"message":"operation failed","data":{"kind":"machine_unreachable","hint":"reconnect the machine; nothing ran locally instead","retryable":true,"target":"machine:0191f1a2-0000-7000-8000-0000000000c3"}}}
```

The same failure case then asserts:

- `fixture.program parent` (local) is still `live: true`, `incarnation: "1"`, `kills: "0"`. No local fallback reached it.
- A failing plugin run is `state: failed`, `exit_code: 3`, `output_b64: "Ym9vbQo="`. `plugin.run_get` returns the same run, and the op receipt is `committed`.

The happy case asserts:

- The peer is replaced by a fresh process that proves the same machine id: `line_epoch: "2"`, `replayed.remaining: "0"`.
- Lost incarnations: the peer's parent shows `incarnation: "0"`, `live: false` on the new runtime.
- Preserved incarnations: the local parent shows `incarnation: "1"`, `live: true`, `kills: "0"`.
- A relaunch on the peer starts incarnation `"1"` there.

remote/e2e asserts:

- `auth_failed` for a credential the peer never issued, and `trust_mismatch` for the wrong expected machine id. Neither records a machine.
- `deadline_exceeded` for a call to the SIGSTOPped peer, while `machine.list` and a local call answer.
- After SIGCONT, the same line still serves.
- An observer's `input.send` to `peer/parent` is `forbidden`, and nothing is sent to the peer.

All committed transcripts show `credential` values as `<redacted-credential>`.

## Adversarial pass and mutants

Seven mutants each failed a shipped test (`mutants.log`):

1. Local fallback for an unknown machine: `an_unknown_machine_is_refused_and_nothing_runs_locally` FAILED.
2. A lost mutation reported `machine_unreachable`: `a_mutation_lost_in_flight_is_outcome_unknown_and_a_lost_read_is_unreachable` FAILED.
3. `last_seen_seq` dropped: `an_unreachable_machine_refuses_with_its_last_seen_seq_and_sends_nothing` FAILED.
4. `$/cancelRequest` not recognised: `a_cancel_notification_names_its_request_and_any_other_notification_closes` FAILED.
5. Remote input authorized as a read: `an_observer_may_read_a_machine_but_never_send_it_input` FAILED.
6. The cancelled reply task is not aborted: `a_cancel_request_ends_only_the_named_call_and_the_connection_keeps_serving` FAILED (20 s hang guard).
7. A cancelled task answered `internal`: the same test FAILED.

Adversarial classes covered:

- **Malformed input:** a numeric `last_seen_seq` in a remote refusal is `internal`, never a success; a string `params.machine` stays local; an unknown qualifier is `not_found`.
- **Prompt injection:** `rm -rf /` as `input.send` text from an observer is `forbidden` and is never forwarded.
- **Cancel and resume:** SIGSTOP/SIGCONT stall, then a live line.
- **Stale state:** a lost machine keeps its `last_seen_seq`; a restarted peer is a new incarnation set.
- **Hung commands:** the stalled peer hits its deadline while other work continues.
- **Misleading success:** a crash mid-call is `outcome_unknown`, never success or unreachable.
- **Repeated interruptions:** stall, resume, stop, crash within one case.

The independent background review on terra was not possible: the spawn was refused by the subagent nesting limit (depth 2 > 1). The pass above was done in-lane.

## Seams (files outside the literal write scope, and why)

- `crates/ovis-terminal-server/src/conn/core.rs`, `conn/core/tests.rs`, `conn/driver.rs`, new `conn/driver/settle.rs`: honour `$/cancelRequest`, the defect described under Before. `settle` moved into a child module to keep `driver.rs` at or under 250 pure LOC.
- `crates/ovis-terminal-protocol/src/errors/data.rs`, `errors/boundary.rs`, new `errors/boundary/context.rs`, `schema/error_catalog.rs`: the additive `last_seen_seq` and its schema property. `with_principal_epoch` moved next to it to keep `boundary.rs` at or under 250 pure LOC, with no behaviour change. `schema/ovis-terminal.openrpc.json` and `packages/client-ts/src/generated` were regenerated; the generated headers carry the new schema digest.
- `crates/ovis-terminal-remote/src/identity/registry.rs` (`link_mut`), `dispatch/lanes/machine_identity.rs` (`take_link`, registry holds `Option<Link>`), `machine_identity/add.rs`: these hand the added link to the actor. `MachineAdd::new` callers are unchanged.
- `crates/ovis-terminal-server/src/dispatch.rs`: two `pub mod` lines.
- `conformance/src/{cli.rs,lib.rs,fixture.rs,engine/steps.rs}` and `fixture/{launch.rs,launches.rs,serve.rs,handler.rs,machines.rs,machines/remote.rs,agents/directory.rs}`: fixture role, `--binary`/`--peer` flags, routing through `RemoteDispatch`, the identity of the peer's own launches, and resolution of an unqualified address (the forwarded form) as a name. New files: `fixture/{peer.rs,peer/process.rs,peer/lost.rs,services.rs}`.
- `conformance/src/runner_tests/suite.rs` and `conformance/scenarios/declared.json`: inventory, plus an updated reason for the still-declared `services/plugins.json`.
- `docs/design/traceability.json`: `remote/e2e` is listed under GAP-106, GAP-109, IDEAL-080, IDEAL-083, EXCL-014 and EXCL-016.
- Scenario ids: the runner grammar is `<family>/<name>`, so the files are `conformance/scenarios/{recovery,remote}/e2e.json`, beside the plan's `.../e2e/` directories.
- `.github/workflows/probe-stress-macos.yml`: a `service-integration` option and job, appended last.
- `crates/ovis-terminal-services/src/lib.rs` and `crates/ovis-terminal-remote/src/lib.rs` (in scope): the remote crate gains `EitherLine` (one actor table over spawned links and in-process lines); the services crate is unchanged.

## Risks

- `outcome_unknown` is decided as follows: the machine was `connected` when the call was handed to its actor, and the line was then lost. If the line drops in the tiny window between that check and the actor writing the record, a call that was never written is reported `outcome_unknown`. That is conservative (it may have run), never a false "did not run".
- Local authorization classifies by method family and `op_id`. A mutation sent without `op_id` is authorized as a read locally, and is then rejected by the remote's own schema validation. Classifying from the schema's mutation catalog would be stricter (todo 61 gate).
- Cancellation aborts only the reply task. Effects with their own durable lifetime (dispatcher operations) continue, which is the existing reply-versus-effect contract. A late cancel for a request id the client reused after the reply targets the new request.
- The fixture peer is signalled only by the pid captured at its own spawn (asserted greater than 1), using `/bin/kill -STOP|-CONT` and `Child::start_kill`.

## Unmet (not claimed; still owned or deferred)

- **Authenticated SSH loopback:** not executed. The isolated runner has no sshd or host keys. The SSH argv (`ssh -T -e none -o BatchMode=yes -o StrictHostKeyChecking=yes -- <dest> ovis-terminal connect`) stays unit-tested in `transport/spec.rs`. The stdio transport is exercised for real.
- **Dirty worktree (GAP-114 / IDEAL-087):** the worktree lane needs an event domain carrying `WorktreeEvent`, which the fixture's dispatcher lacks (todo 42 assembly).
- **Checkpoint gap and host-reboot metadata (GAP-121, GAP-123, GAP-124 / IDEAL-093, IDEAL-095, IDEAL-096):** `recovery/crash` stays declared. The fixture cannot kill and restart its own core with holders. Covered meanwhile by `recovery::qa`.
- **Update rollback (GAP-127 / IDEAL-100):** already executable as `services/update` (todo 47). `ServiceLanes::with_update` exists but the fixture keeps the todo-47 registration.
- **GAP-119 / GAP-122 / GAP-128:** orphan replay is asserted only as `remaining: "0"` on reconnect (the full matrix stays in `remote/isolation`). Planned restart of the local core and retention floors are not exercised here.
- **Plugin seams from todo 46:** per-plugin degradation of a malformed `plugin.json` and process-group cancel are not changed here. The fixture covers a failing program and a missing program per action.
- **Windows:** integration is completed by the todo-60 CI smoke lane, as the block states.

DEFERRED VERIFICATION: none for the scenarios in this PR. The runner executes both new scenarios on this host with failed 0 and skipped 0. macOS is covered by the `service-integration` stress job (run URL below) and by CI's macOS lane.

## Round 2 (CI on c23c4a3)

CI failed only on `test (windows-latest)`: `dispatch::services::tests::configured_services_register_exactly_their_methods_and_a_second_time_is_refused` returned `Os { code: 32 }` ("being used by another process") from `remove_dir_all`. The test dropped the dispatcher but never closed the SQLite store, and Windows cannot delete an open database file; Unix can, which is why ubuntu and macOS passed. The defect was in the test's cleanup, not in the product. The test now keeps the coordinator and calls `Coordinator::close()` before removing its directory, on every platform. There is no cfg and no skip. Local: the test passes; server clippy `-D warnings` exits 0; fmt and hygiene (0 violations) pass; `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exits 0; the matching `cargo clippy ... -D warnings` exits 0.

macOS stress probe (dispatched once on c23c4a3): https://github.com/devswha/ovis-terminal/actions/runs/41292367994

## Round 3 (CI on 5c1a55f): recovery/e2e happy step 12, `line_epoch`

CI on ubuntu failed `suite::every_scenario_file_is_executed_or_a_listed_declaration`, because `all/all` failed at recovery/e2e happy step 12: `reply[restart]/result/health/line_epoch` expected `"2"`, and `/state` expected `connected`. Both failed together.

**Verdict: product, not expectation.** After a restart the peer's actor has taken exactly one more line, so `connected` with epoch `"2"` is the correct contract. The defect: `MachineActorLane::attach` only queues the new line on the actor's mailbox and returns. So `reconnect()` (and `machine.add`) could report before the actor had applied the line, and a health read then still showed the old, lost line (`unreachable`, epoch 1). Fix (`dispatch/remote/attach.rs`): `attach_applied` subscribes to the machine's health before attaching and returns only once `line_epoch` has advanced. It is bounded, and a timeout is `unavailable` (actor stopped). Both `reconnect` and `MachineAddAttached` use it. The scenario expectation is unchanged.

The same repeat test exposed a second race, in the fixture's fault injector rather than the product: `fixture.peer stop` replied "stopped" as soon as `kill -STOP` returned. SIGSTOP takes effect asynchronously, so under load the peer still answered `peer-stalls` (remote/e2e, 4 failures in 15 runs). A temporary probe of `ps -o stat=` showed the peer in `T` once the stop had landed. The injector now sends SIGSTOP and SIGCONT with `rustix::process::kill_process` to the pid captured at spawn (above 1, never a group), and replies only after `waitpid(pid, WUNTRACED)` reports the child stopped. This is the kernel's own event: no polling, and `/bin/kill` is no longer used. `conformance/Cargo.toml` gains a unix-only `rustix 1.1.6` dependency with the `process` feature. It is already in Cargo.lock for the holder; one edge was added.

Evidence: `stress-round3.log`. Under `taskset -c 0-1` with 8 busy loops, 15 runs each of recovery/e2e and remote/e2e: PASS=30 FAIL=0. Before the injector fix, the same recipe gave remote/e2e 4 failures out of 15. Lane tests: `dispatch::` and `conn::core::` 59 passed; the conformance crate (`all/all`) passed.

## Merge of origin/integration a896a7d

One merge commit after the round-3 fix.

- **Conflicts:** `conformance/src/fixture/serve.rs` was resolved as a union. Integration's agent `assembly.gate(..)` now wraps `RemoteDispatch`, and the transcripts registration integration moved into the assembly was taken from integration. `.github/workflows/probe-stress-macos.yml` was resolved as a union of the options and jobs, with `commit-barrier` and then `service-integration` last.
- **Inventory:** `suite.rs` auto-merged into a sorted union of 31 entries.
- **Lock, schema, SDK:** Cargo.lock is unchanged by the merge (`cargo metadata --offline`, then `--locked`, OK). The OpenRPC schema was rewritten and is byte-identical (`check` and `compat` exit 0). The SDK `generate:check` is fresh and `typecheck` exits 0.
- **Merged tree:** fmt check OK, hygiene 0 violations, clippy `-D warnings` on server, services, remote, protocol and conformance exit 0. `cargo test -p ovis-terminal-server -p ovis-terminal-services -p ovis-terminal-remote --all-targets`: 306 passed, 0 failed. `cargo test -p ovis-terminal-conformance` (all/all, including integration's agent/e2e): passed. The full `check.sh` and Windows legs are left to CI as the gate for this push.

## Round 2 (gate review on 01ecbfc: B1-B4)

All twelve GAPs of todo 52 now have executable steps on the real binary. No GAP was reassigned, so `docs/design/traceability.json` is unchanged by this round. `recovery/e2e` now declares GAP-106/108/109/114/118/119/121/122/123/124/127/128 and IDEAL-080/082/083/087/091/092/093/094/095/096/100/101/117.

| GAP | Executed in | What runs |
|---|---|---|
| GAP-106 | remote/e2e | `machine.add` over stdio argv **and over SSH**: auth_failed for a foreign credential, invalid_params for `-oProxyCommand=id` before any spawn, then the peer added over SSH |
| GAP-108 | recovery/e2e failure | lost machine: `machine_unreachable` + `last_seen_seq "41"`; crashed peer kept in the roster; no local fallback |
| GAP-109 | remote/e2e happy | stalled (SIGSTOP) peer: `deadline_exceeded` while the roster and local calls answer |
| GAP-114 | recovery/e2e failure | dirty worktree on each runtime: `checkout_dirty` naming the checkout, checkout kept and listed dirty (real Git repository) |
| GAP-118 | recovery/e2e failure | failed plugin run (exit 3, durable `run_get`, committed receipt), missing program `action_unavailable` |
| GAP-119 | recovery/e2e happy | orphan **ack loss** with a queued notice: the parent exits while the peer is stopped; delivery stops at its deadline with the notice owed (`remaining "1"`); restart replays it once (`applied "1"`, `remaining "0"`); re-ack is `duplicate` |
| GAP-121 | recovery/e2e | programs in shared and dedicated holders keep running through core loss and a planned control restart (`running: true`, preserved incarnations); a lost dedicated holder affects only its terminal |
| GAP-122 | recovery/e2e happy | `server.restart` on each runtime over the real restartable control: `preserved_incarnations` equals the live set, `ended` and `interrupted_ops` empty, receipt committed |
| GAP-123 | recovery/e2e failure | core lost after its checkpoint: `survived`, `exact: false`, one gap `1244`→`8312` (4 KiB holder tail), on each runtime |
| GAP-124 | recovery/e2e happy | another boot identity: `host_reboot`, `restored_metadata`, nothing adopted, program still running, on each runtime |
| GAP-127 | recovery/e2e failure | update rollback across both runtimes: 3.0.0 fails readiness on the peer and rolls back there (receipt readable only through the peer); local updates stay empty |
| GAP-128 | recovery/e2e failure | a visible retention floor: a plugin run printing 10 000 bytes keeps its 4 KiB tail with `truncated: true`, `output_floor "5904"` |

**How each is reached.**
- **SSH loopback** (`conformance/src/fixture/peer/ssh.rs`): no sshd is needed. The runner puts `<root>/bin` first on the fixture's search path, and the `ssh` there accepts **only** the exact vector `TransportSpec::Ssh` builds (`-T -e none -o BatchMode=yes -o StrictHostKeyChecking=yes -- peer.loopback ovis-terminal connect`). It then bridges stdio to the peer through the binary under test. The real argument vector, handshake and credential check therefore run end to end. Mutant F2 (drop `BatchMode=yes`) is refused by the shim, so the add fails.
- **Orphan routing:** `$/orphaned` was not routed on a served runtime at all, so a real peer could never receive a notice. It is now answered by the machine-actor lane's `orphaned()` (`fixture/peer/orphans.rs`). `fixture.remote_child` can name the real peer, and `fixture.peer deliver` reports what is still owed.
- **Holders (B2), `fixture.holders`:** real `Holder` + `HolderService` on Unix sockets.
  - `exit` runs one program in a **shared** holder and one in a **dedicated** holder on each runtime. Each asserts its final bytes (`LAST-shared`, `LAST-dedicated`) arrived before the holder's Exit message, with `bytes_before_exit` equal to the holder's `final_offset` (17 and 20).
  - `crash`, `reboot` and `unplug` run the core's own `recover()` (todo 49) over those holder groups.
- **Planned restart:** `server.restart` is served from the update lane's real `Control`. `server.stop` stays unserved because `protocol/discovery` pins it.
- **Worktrees:** `worktree.*` is served by a fixture dispatcher whose event domain is operation + worktree events, over its own store and a real Git repository. No production workspace owner exists yet, so opened workspaces are recorded by a fixture owner (`fixture/worktrees.rs`).
- **Remote services:** requests reach the peer through remote dispatch using `params.machine`, which is removed before forwarding (mutant F3: forwarding it fails the peer request).

**B3 (epoch fix pinned).** `dispatch::remote::qa::attach::an_attach_returns_only_after_the_busy_actor_took_the_new_line`:
- A test line whose `send` waits for a permit keeps the actor busy writing an earlier call.
- One poll of `attach_applied` (with a no-op waker) must be **Pending**. Under the `>`→`>=` mutant it is Ready on that same poll, so the test fails deterministically without load.
- After the permit is released, the epoch is 2 and the state is `connected`.
- `attach_applied` now takes the actor table and a lazy attach future, so it is testable without a full machine-actor lane.

**B4 (alias).** Harness machines carry an alias `<name>-old`. `dispatch::remote::qa::alias::an_alias_reaches_the_machine_it_names_and_only_that_machine` routes `box-old/t1` and `machine: {name: box-old}` to `box` only.

**Mutants** (`mutants-r2.log`, every one run here, each reverted):
- Server crate, each failing a named test: M1 (`>`→`>=`), M2 (alias lookup removed), local fallback, lost mutation reported as unreachable, `last_seen_seq` dropped, cancel not recognised, input authorized as a read, cancelled reply not aborted, cancelled answered `internal`.
- Scenario level, on the real binary:
  - F1: no `$/orphaned` route, so the notice stays owed after restart.
  - F2: the SSH argv changed, so `transport_failed` appears instead of `auth_failed`.
  - F3: `machine` forwarded, so the peer request is refused.
- Gate mutant M3 (`MachineAdd::restricted` ignores its allowlist): killed by todo 58's `machine_add_admits_only_allowlisted_endpoints_before_any_connection`.
- Gate mutant M4 (`pause()` returns right after SIGSTOP): killed under load, 3 of 8 runs failed.

**Flake found and fixed in round 2 (root-caused, not retried).** Under load, `recovery/e2e` failed once in 35 runs at `local-crash` (`exact: true`, no gap). Cause: the drill checkpointed whatever bytes had arrived when `MARK` was seen. Under load that chunk could already reach past the holder's retained start, so there was legitimately no gap. Fix: the writer blocks on a FIFO right after `MARK`, and the drill releases it only after checkpointing. The checkpoint is now exactly `1244` and the gap `1244`→`8312`, both pinned literally. Clean control under `taskset -c 0-1` + 8 busy loops: **50/50** (25 each of recovery/e2e and remote/e2e, `stress-round-r2.log`).

**Verification on the round-2 tree** (Linux 7.0.0-38-generic x86_64):

| Command | Exit | Result |
|---|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 | |
| `cargo test -p ovis-terminal-server -p ovis-terminal-services -p ovis-terminal-remote --all-targets` | 0 | 308 passed, 0 failed; 13 lane tests |
| `cargo clippy ... --all-targets -- -D warnings` (server, services, remote, protocol, conformance) | 0 | |
| fmt `--check`, workspace-hygiene, manifests.sh, json.sh | 0 | 0 violations |
| `cargo test -p ovis-terminal-conformance` (`all/all` inventory and traceability) | 0 | |
| conformance `recovery/e2e --case all` | 0 | failed 0, skipped 0, 369 assertions |
| conformance `recovery/e2e --case happy` | 0 | 146 assertions |
| conformance `recovery/e2e --case failure --inject '{"method":"input.send","params":{"target":{"address":"disconnected/t1"}}}' --append` | 0 | 220 assertions |
| conformance `remote/e2e --case all` | 0 | 101 assertions |

The full `scripts/check.sh` and the Windows cross-check are left to CI for this push. The schema, SDK and Cargo.lock are unchanged in round 2.

**Seams (round 2).**
- `conformance/src/fixture/{holders.rs, holders/drills.rs, worktrees.rs, peer/ssh.rs, peer/orphans.rs}` (new).
- `fixture/{peer.rs, peer/process.rs, launch.rs (search path), handler.rs (routes), machines.rs (remote_child names any served machine), serve.rs, services.rs (flooding plugin), update.rs (server.restart on its Control), fixture.rs (mod lines)}`.
- `conformance/scenarios/declared.json`: reasons updated for recovery/crash, recovery/restart, services/worktrees, services/docs-retention and remote/identity. Those files remain todo-owned free-form descriptors; their behaviour now executes in recovery/e2e and remote/e2e.
- Server crate: `dispatch/remote/attach.rs` (`attach_applied` signature) and the new `dispatch/remote/qa/{attach,alias}.rs`.

**Stated plainly.**
- `RemoteDispatch`, `MachineAddAttached`, `ServiceLanes` and `reconnect` are constructed **only in the conformance fixture** until todo 66 assembles the single served binary. The CLI binary starts no server today. Todo 66 must construct them, including `MachineAdd::restricted`, in the real server.
- `MachineAdd::restricted` (the todo-58 endpoint allowlist) **still has no production caller.** The fixture uses `MachineAdd::new`, because its stdio peer uses an argv transport, which a host allowlist refuses by design.
- Document retention floors (`doc.*`) stay with todo 50's lane tests until assembly. The shared fixture's `doc.*` names belong to the todo-21 stubs that protocol scenarios pin.
- Still not done: a planned restart of the *local core's listener* (only the control boot is restarted). The `x-effect: control` methods are still classed as reads locally by remote dispatch; this is a todo-61 note from the gate.

**Round 2 CI fix (633b829):** `rustdoc -D warnings` failed because the public module doc of `fixture::peer::ssh` linked the private `DESTINATION` const. The doc now names it in plain code formatting, with no intra-doc link and no `#[allow]`. `RUSTDOCFLAGS=-Dwarnings cargo doc --no-deps -p ovis-terminal-conformance` (also server, remote, protocol) exits 0.
