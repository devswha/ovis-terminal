feat(agent-integration): coordinate durable commits snapshots and agent assembly (plan todo 42)

## Before

RT-HD-08 (herdr 0.9.5). A second subscription silences both event paths. The source is `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h06-events.log:15`, with a durable copy at `docs/evidence/baseline/smoke/herdr-h06-events.log:15`. The plan names `docs/evidence/baseline/herdr-h06-events.log`, but the copy actually lives under `smoke/`. Its sha256 is recorded in `docs/evidence/task-42/before.log`.

```
10: $ report_agent on A (first subscription target) {"pane_id":"w1:p1","agent":"smoke-agent","state":"blocked","source":"agent"}
11: <- {"id":"r20","result":{"type":"ok"}}
13: event frames total: 1 closed=true
15: TRAP CHECK: status event for A (first subscribe) received=false; for B (second subscribe on same conn) received=false
```

Gap baseline: GAP-020, GAP-025, GAP-031, GAP-032, GAP-034, GAP-037, GAP-079, GAP-080, GAP-081, GAP-083, GAP-085, GAP-087, GAP-088, GAP-090, GAP-097, GAP-098, GAP-105, GAP-125, GAP-132, GAP-133 (docs/design/gap-list.md).

## What changed

**Core commit boundary** (`crates/ovis-terminal-core/src/commit/`).
- `Boundary` is the one handle that terminal actors (`CommitSource::Terminal`), the registry and remote mirrors (`CommitSource::Remote { origin }`) all commit through. It wraps the existing todo-10 coordinator and its dedicated bounded SQLite writer.
- The local sequence is allocated only inside the serialized durable commit. Publication follows the writer's COMMIT.
- `submit` runs the blocking commit through `spawn_blocking` behind a bounded slot pool. When every slot is taken it returns `backpressure`. No lock is held across an await.
- A remote source keeps its upstream `RemoteOrigin` beside the local cursor (`Committed::local_seq` versus `Committed::remote`).
- The durable upstream high-water mark (`commit.remote/<machine>/<runtime>`) is read and advanced as a CAS write in the same transaction. A replayed or older upstream position is refused as `remote_replayed` before any local sequence exists.
- An unreadable mark is `invalid_event`, never reset to a default (GAP-125).
- A store that runs out of space returns `storage_full`. It acknowledges nothing, publishes nothing and consumes no sequence (IDEAL-097).

**Snapshot windows** (`Pager`).
- Each window pins one revision.
- Every page carries the window's cursor and `total`, plus a `continuation` offset and a `truncated` flag (set when the 1 MiB server page bound cut a larger ask).
- Expiry is explicit: `expires_ms = open + ttl`. At or after it the page is `page_expired` and the pin slot is released. `reap(now)` releases exactly the expired windows. Time is supplied by the caller, not by timers.
- All u64 fields are decimal strings.

**Agent assembly** (`crates/ovis-terminal-server/src/dispatch/agent.rs`).
- `AgentAssembly` builds one `StatusService` and shares it with prompts, conversation records and session binding.
- It registers the 14 `AGENT_METHODS` on one dispatcher, so everything lands in one sequence domain. Reads are unrecorded.
- `gate()` serves a handler behind the prompt bridge's program-token gate. This was the production assembly left to todo 42 by PRs #37 and #54.
- The conformance fixture now uses this assembly instead of registering the lanes piecemeal.

**Real scenario** `conformance/scenarios/agent/e2e.json` (happy and failure), plus the never-advertised fixture probe `fixture.commit_barrier`.

## After

Host: Linux x86_64. Binary sha256 `14226607dd1bf9ff6c4c2bc0b90e79795f62f6058b0954d82e9bd18e10ec854b` (60250640 bytes, recorded in the evidence JSON). The cargo target lives on RAM (`$CARGO_TARGET_DIR=/dev/shm/ovt-target-lane-t42`, host policy), so the binary path is `$CARGO_TARGET_DIR/debug/ovis-terminal` instead of the plan's literal `target/debug/ovis-terminal`. Builds used `CARGO_INCREMENTAL=0` (host /dev/shm quota; CI does the same).

```
$ cargo build --locked -p ovis-terminal-cli --bin ovis-terminal                      # exit 0
$ cargo test --no-fail-fast -p ovis-terminal-server -p ovis-terminal-agent -p ovis-terminal-agent-adapters -p ovis-terminal-core --all-targets
  exit 0 - 617 passed, 0 failed across 27 test binaries (docs/evidence/task-42/lane-test.log)
$ cargo clippy -p ovis-terminal-server -p ovis-terminal-agent -p ovis-terminal-agent-adapters -p ovis-terminal-core --all-targets -- -D warnings   # exit 0
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary "$CARGO_TARGET_DIR/debug/ovis-terminal" --scenario agent/e2e --isolated --case all --evidence "$ATTEMPT_DIR/task-42-all.json"
  exit 0 - {"result":"passed","totals":{"assertions":"126","cases":"2","failed":"0","passed":"2","skipped":"0"}}
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary "$CARGO_TARGET_DIR/debug/ovis-terminal" --scenario agent/e2e --isolated --case happy --evidence "$ATTEMPT_DIR/task-42-ovis-terminal-v1.json"
  exit 0 - {"case":"happy","result":"passed","totals":{"assertions":"84","failed":"0","passed":"1","skipped":"0"}}
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary "$CARGO_TARGET_DIR/debug/ovis-terminal" --scenario agent/e2e --isolated --case failure --inject '{"method":"prompt.answer","params":{"prompt_id":"p1","version":1,"force":true}}' --evidence "$ATTEMPT_DIR/task-42-ovis-terminal-v1.json" --append
  exit 0 - {"case":"failure","result":"passed","totals":{"assertions":"40","failed":"0","passed":"1","skipped":"0"}}
$ bash ~/.ovis-toolchain/with-slot.sh bash scripts/check.sh     # exit 0 - nextest 1234 run: 1234 passed, 1 skipped; hygiene 0 violations
$ RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked             # exit 0
$ cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings                  # exit 0
```

**Happy observable.** Values are from `docs/evidence/task-42/task-42-ovis-terminal-v1.json`.
- The bridge client (`ovis-terminal connect`) registers the unknown brand `smoke-agent`. The `phone` socket client (writer principal) reads the same record with `agent.get`.
- `status.report` seq `"2"` is replayed to the phone through `status.get`. A seq `"1"` report is `stale_seq` with `current_seq: "2"`.
- The open form prompt `p1` blocks `input.send` as `blocked` with `prompt_ids: ["p1"]`. The phone answers it once with `{"fields":{"branch":"main"}}`, and the result is `answered`.
- History pages r1 and r2, the phone disconnects, then `resumed` continues from the cursor to r3 (parent r2).
- The retried answer returns the retained receipt, and `op.get` reports `committed`.
- Fixture probe `fixture.commit_barrier`: terminal, registry and remote sources commit 40 records each (`total: "120"`) while a 64 MiB snapshot is paged and a subscription installs. The reply reports `missing "0"`, `duplicate "0"`, `reordered "0"` and `upstream_kept_apart: true` (remote upstream 1001..1040 kept apart from local seqs).
  - Snapshot: `at_least_64_mib`, `every_page_pinned`, `digest_matches` and `holds_exactly_up_to_revision` are all true. A read at `expires_ms` gives `page_expired`.
  - Subscription: `partitions_committed: true`. The snapshot holds 1..=S, and the deliveries are exactly S+1..=120 in order.
- The real binary, `ovis-terminal --json call agent.get --params '{"target":{"name":"agent"}}'`, exits 0 with `/agent/label = "smoke-agent"`.

**Failure observable.**
- The injected `prompt.answer {"prompt_id":"p1","version":1,"force":true}` returns `invalid_params` with `field_path: "params.force"`.
- In the planned (non-injected) run, a hostile text answer to the form is `answer_invalid`.
- Afterwards `fixture.pty` is `"0"` bytes, `p1` is still `open` at version `"1"`, and the reported status is still `working`. No actor or PTY mutation happened.

**Unit/property behaviour tests (new).**
- `crates/ovis-terminal-core/tests/commit.rs`, 9 tests: three sources plus a paged snapshot plus a subscription install with no gap, duplicate or reorder; bounded async submissions with no lock across awaits; replayed and reordered upstream refused; u64::MAX upstream round trip, with numeric rejected; corrupt mark refused; pages pinned to one revision with total, continuation and truncation; expiry and reap; storage full; subscription install at a held COMMIT.
- `crates/ovis-terminal-server/tests/dispatch.rs::agent_assembly`, 2 tests: exact catalog inventory with no replaced route; two clients behind the gate (unknown brand, stale_seq, foreign token `forbidden`, `force` invalid_params, receipt retained after a disconnect).

**Mutants** (`docs/evidence/task-42/mutants.log`). All compiled and were killed with exit 101.
- M1: drop the upstream mark write. Killed by three `remote::*` tests.
- M2: inclusive expiry. Killed by `pages::pages_stay_on_one_revision_until_their_expiry`.
- M3: `truncated: false`. Killed by the same pages test.
- M4: drop `status.clear` from the assembly. Killed by `agent_assembly::the_assembly_routes_exactly_its_catalog_methods_once`.
- M5: never classify storage full. Killed by `storage::a_full_store_acknowledges_and_publishes_nothing`.

## Seams (files outside the write scope, and why)

- `crates/ovis-terminal-core/src/lib.rs`: the `pub mod commit;` line.
- `crates/ovis-terminal-core/Cargo.toml`: `tokio` moves from dev-dependency to dependency, for `spawn_blocking` and `Semaphore` in `Boundary::submit`. Cargo.lock is unchanged.
- `crates/ovis-terminal-core/src/events/coordinator.rs`: an additive getter, `Coordinator::limits()`. The pager needs the page bound to align arbitrary offsets.
- `crates/ovis-terminal-core/tests/commit.rs`: the test harness with `#[path]` modules into `src/commit/qa/`, the same pattern as `tests/events.rs`.
- `crates/ovis-terminal-store/src/{options.rs,store.rs,error.rs}`: additive `StoreLimits::max_pages` / `with_max_pages`, which sets SQLite `max_page_count` on the writer connection, and `StoreError::is_storage_full()`. The disk-full test needs a deterministic SQLITE_FULL, and the boundary needs to classify it. The default is `None`, so behaviour is unchanged.
- `crates/ovis-terminal-server/src/dispatch.rs`: the `pub mod agent;` line.
- `crates/ovis-terminal-server/src/dispatch/agent/qa.rs` and `tests/dispatch.rs`: the assembly's socket test module and its `#[path]` line.
- `conformance/scenarios/agent/e2e.json`: the runner maps `agent/e2e` to this file, not to the plan's `agent/e2e/` directory (precedent: todos 33, 36, 40, 45). `docs/design/traceability.json` already lists `agent/e2e` under todo 42, so it is unchanged.
- `conformance/src/fixture/{agents.rs,prompts.rs,transcripts.rs,serve.rs}`: the fixture's status, prompt, records and session lanes now come from `AgentAssembly`, and the handler is served behind `assembly.gate(..)`. The probes keep their names, but their registration moved into the assembly.
- `conformance/src/fixture/commit_barrier.rs` (+ `commit_barrier/{domain,observe}.rs`), `fixture.rs` and `handler.rs`: the never-advertised `fixture.commit_barrier` probe. It is the same kind of fixture method as `fixture.pty` and `fixture.transcript`.
- `conformance/src/runner_tests/suite.rs`: `EXECUTABLE` gains `agent/e2e.json` (sorted). `declared.json` never listed it.
- `docs/evidence/task-42/`: before, QA, clippy, check, Windows and mutant logs, the evidence JSON and this body.
- No schema, OpenRPC or SDK change. No catalog method was added or advertised.
- `conformance/scenarios/events/commit-barrier/` (in scope) is not created. The commit-barrier proof runs inside `agent/e2e` as the plan's happy QA requires, and its unit proofs live in `core/tests/commit.rs`.

## Risks

- **Fixture stand-in.** `fixture.commit_barrier` runs the real `Boundary`, `Pager`, `Subscriptions`, coordinator and SQLite writer inside the fixture process, but on a dedicated store with a 64 MiB history stand-in section. No wire method for `session.snapshot` paging or `events.subscribe` is served by the binary yet. Assembling the full roster producer and the subscription transport over the wire remains for the assembly owners (todos 63/66). The remote source is a `RemoteOrigin`-tagged commit, not a live remote line (todo 52 assembles the mirror).
- **Gaps between this assembly and production.** `agent.tree` and template launch still register through their own fixture modules. OSC ingestion is not yet wired to the live output pump.
- **Debug-build cost.** The 64 MiB probe takes about 8.5 s in a debug build against the runner's 20 s per-reply bound. A much slower CI runner could approach that bound.
- **`agent.history` is not in `AgentAssembly`.** It is served by T58's `boundary_policy` lane, which needs an authorized root. The daemon assembly (todo 31) must route it as an unrecorded Read and compose `Revalidated(Measured(BoundaryPolicy(gated)))`. The conformance fixture already composes it that way after the merge.

DEFERRED VERIFICATION: none. The runner executes `agent/e2e`, and both cases passed above.

## Merge addendum (authorized by the orchestrator)

Two merge commits sit on top of a062208, with no rewrite: `0f00057` merges integration 291cfee (T86 graphics caps) and `edd7332` merges integration 167c0a7 (T58 boundary policy).
- The only conflict was `.github/workflows/probe-stress-macos.yml`. It was resolved as a union: options end `..., boundary-policy, graphics-caps, commit-barrier`, and the `commit-barrier` job is last.
- `conformance/src/fixture/serve.rs` auto-merged into `Revalidated(Measured(BoundaryPolicy(assembly.gate(handler))))`.
- `Cargo.lock` did not conflict. `cargo metadata --locked` exits 0.
- `openrpc write` / `check` / `compat schema/released.openrpc.json` each exit 0 (`compatible_with`). The regenerated document is byte-identical to the merged one.
- In `packages/client-ts`, `bun run generate` reports `changed: 0`. With `OVIS_*_BIN` pointing at the merged RAM-target binaries, `generate:check`, `typecheck` and `bun test` exit 0: 30 pass, 0 fail.

Gates on the merged tree (edd7332):
```
cargo test --no-fail-fast -p ovis-terminal-server -p ovis-terminal-agent -p ovis-terminal-agent-adapters -p ovis-terminal-core --all-targets   # exit 0, 639 passed, 0 failed (merge-lane-test.log)
cargo clippy -p ovis-terminal-server -p ovis-terminal-agent -p ovis-terminal-agent-adapters -p ovis-terminal-core --all-targets -- -D warnings # exit 0
conformance --scenario agent/e2e --isolated --case all                                   # exit 0, 126 assertions, 2/2 passed (merge-qa-all.log)
bash ~/.ovis-toolchain/with-slot.sh bash scripts/check.sh                               # exit 0, nextest 1285 run: 1285 passed, 1 skipped; hygiene 0 violations (merge-check.log)
RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked   # exit 0
cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings          # exit 0
```
The macOS stress run `probe=commit-barrier` was dispatched once on a062208: https://github.com/devswha/ovis-terminal/actions/runs/41288062895
