feat(documents-retention): commit versioned documents and visible retention floors (todo 50)

Closes GAP-128. Delivers IDEAL-098, IDEAL-101. Enforces EXCL-020, EXCL-021 and EXCL-025, and does not reproduce V-005, V-3905 or V-RT-HC-02.

## Before

`absent`: there was no comparable baseline capability. This is not an executed failing test. Gap baseline: GAP-128.

## After

Host: Linux x86_64 (`uname -sm`), toolchain 1.99.1. Branch head contains integration 2e2cc07 through the orchestrator merges listed under Merges; current `origin/integration` is b9be58f, not merged here. Round-3 `target/debug/ovis-terminal` sha256 `df83a7ad13b530e81d6bb82e04903b8ea016d6a29ff0abe04fa41792144588a5` (`docs/evidence/task-50/binary.sha256`; a local build, so other hosts' hashes differ).

| command | exit |
|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 |
| `bash scripts/check.sh` (nightly fmt, workspace clippy `-D warnings`, nextest 692 run / 692 passed / 1 skipped, doctests, hygiene, json) | 0 |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 |
| `cargo nextest run -p ovis-terminal-server --locked --test documents_retention --no-capture` (5 tests) | 0 |
| `cargo nextest run -p ovis-terminal-core --locked -E 'test(/docs::/)'` (20 tests; the retention suites are mounted under `docs::qa`) | 0 |
| `cargo run … --scenario all/all --isolated --case all` | 0 (16 passed, 0 failed, 0 skipped, 24 declared_only) |

The `failure` socket test replays the plan's injected stale write (`doc.set` named `shared`, version `"0"`, value `{"x":2}`, sent after `shared` reached version 1), verbatim from `docs/evidence/task-50/qa-socket.log`:

```
-> {"id":"…1d65…","jsonrpc":"2.0","method":"doc.set","params":{"name":"shared","op_id":"…1d65…","value":{"x":2},"version":"0"}}
<- {"jsonrpc":"2.0","id":"…1d65…","error":{"code":-32000,"message":"operation failed","data":{"kind":"conflict","hint":"read doc.get and retry the write with its version","retryable":false,"op_id":"…1d65…"}}}
-> (the same request and op_id again)
<- (the same retained conflict)
-> {"id":"get","jsonrpc":"2.0","method":"doc.get","params":{"name":"shared"}}
<- {"jsonrpc":"2.0","id":"get","result":{"name":"shared","value":{"x":1},"version":"1"}}
```

The `happy` socket test sends two `doc.set` requests at version `"0"` on two connections before reading either reply. One reply is `{"seq":"6","version":"1"}`, the other is `conflict`, and the subscriber sees exactly one `doc.changed` event, whose seq equals the reply's `seq`.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The runner has landed, but it cannot route this lane yet. Its only fixture (todo 21) serves `Dispatcher<OperationEvent, OperationProducer>` with file-effect `doc.*` stubs that the `protocol/*` scenarios depend on. It has no `--inject`, `--append` or virtual-clock step. `conformance/scenarios/services/docs-retention.json` is therefore a declaration owned by todo 50 and allowlisted in `declared.json`, blocked by todo 52 (assembly). Attempts, verbatim:

- `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario services/docs-retention --isolated --case all --evidence "$ATTEMPT_DIR/task-50-ovis-terminal-v1.json"` → exit 1, `scenario_not_executable` ("services/docs-retention is a declaration, not a registered executable scenario"). `--case happy` gives the same result.
- `… --case failure --inject '{"method":"doc.set","params":{"name":"shared","version":0,"value":{"x":2}}}' … --append` → exit 2, `error: unexpected argument '--inject' found`.

Note that the plan's literal inject carries `"version":0` as a JSON number. Under the decimal-string contract (checklist item 1) that is `invalid_params`, which the adversarial socket test asserts. The decimal form `"0"` returns `conflict`, as shown above.

## QA

- happy (real socket): two writers race version 0. Exactly one commits version 1 and exactly one `doc.changed` event; `doc.get` returns the winner's value at version 1.
- failure (real socket): a stale write at version 0 returns `conflict` with `op_id`, a retry with the same op id returns the same retained conflict, and `doc.get` stays at version 1.
- Large documents round-trip (real socket, `documents_read_back_byte_exact_up_to_the_largest_accepted_write`): a 16384-byte value (the old receipt failure point) and a 262092-byte value (262090 characters; its canonical `doc.set` params are exactly the 262144-byte ledger bound, which the test asserts) are written at version 0 and read back byte-exact at version 1. Neither read publishes an event. One more character is `too_large` before admission (no `op_id`).
- 1 MiB + 1 byte: over the socket, the 256 KiB canonical-params bound of the op ledger refuses it before admission. The test asserts `too_large` and the absence of `op_id`, which proves that bound fired. The inclusive 1 MiB document bound itself (accepted at exactly 1 MiB, `too_large` at 1 MiB + 1) is asserted only by core tests at the `DocWrite` boundary and through the core ledger path. See Risks.
- Virtual expiry (core, `retention/qa/sweeps.rs`): sweeping all five classes eight virtual days later moves every floor (output_history 3, events 3, receipts 4, plugin_runs 4, finished_terminals 4). Each sweep emits one `retention.floor_advanced` event, and the evicted plugin-run record becomes a tombstone (null value, version 2), not an absence. No wall-clock sleeps.

## Adversarial

- Stale state, cursor below the floor: a cursor at seq 1 after the store trim returns `missed: cursor_expired`, `RetentionFloor::lookup` below the floor returns `expired`, and `resume_after` below the floor returns `gap` (proptest over all u64 values).
- Cancel/resume, expired op id: an op id minted 24 h + 1 ms ago returns `op_expired` (core) and 24 h + 60 s ago returns `op_expired` (socket); nothing is committed and no event is published.
- Malformed input: an oversized document gives `too_large`, a numeric version gives `invalid_params`, a name `a/b`, an empty name or a 129-byte name gives `invalid_name` (128 bytes is accepted), and a foreign `machine` gives `not_found`.
- Retention boundary proptest: the floor position itself is `available`, one below is `expired`, resuming one below continues, and resuming two below is a `gap`. It asserts fixed boundary positions rather than recomputing the comparison.
- Compaction: a snapshot taken before a sweep still reports floor 0 and the new view reports 4. A repeated or older sweep returns `Unchanged` with no event, and a stale tombstone version rolls back the whole sweep (`conflict`, floor and record unchanged).
- Flakiness: virtual clock only, and the race uses a barrier, not sleeps. A macOS stress job (`docs-retention`, 50 iterations) is added at the end of `probe-stress-macos.yml`.

## Contract checklist

1. Every counter in the new JSON (`version`, `seq`, `floor`, `capacity`, `evicted`, `max_age_ms`) is a decimal string, and numeric forms are rejected (tested). u64::MAX round-trips exactly (tested).
2. `DocumentError`, `DocError` and `RetentionError` are thiserror enums with `kind()`/`hint()` and `#[source]`/`#[from]`, and Display does not repeat the kind. Domain failures are retained as the operation's typed `ErrorData`.
3. `RetentionPosition` and `RetentionQuantity` are distinct newtypes; `Option` is used only for "no age bound".
4. Tests assert state transitions, events and typed kinds through the real store; proptest covers the version model, eviction planning and floor outcomes.
5. No `#[allow]`, no placeholders, no prose pins.
6. Holder wire generation is still 2. Contract checklist items are unchanged.

## Merges

The orchestrator union-merged integration into this branch three times on top of b0b1da8: f649283, bc28077 and b1ef937 (the last brings integration 2e2cc07: T77 PR #34 and T43 PR #32). Resolutions:
- `dispatch/runtime.rs`: the new `reads: BTreeMap<&'static str, Box<dyn ReadRoute>>` map sits beside T77's shared `tasks: Arc<Mutex<Tasks>>`. Read routes still bypass the task table and ledger; mutations use T77's table.
- `docs/qa/wire.rs` and the retention policy: `u64::from(limits.event_replay_events)` became `limits.event_replay_events.get()` after T22 made that limit a decimal-string `Count`. The value is unchanged; `the_policy_advertises_every_class_with_its_v1_bound` still pins `"10000"`.
- nightly rustfmt.
- f649283 left an empty `impl<D, P> Execution<D, P> {}` block in `dispatch/execution.rs`. Round 3 removes it.

## Seams

- Round 3 (R2-B1): `tests/documents_retention/scope.rs::doc_get_requires_observe_on_that_document`. Over a real socket, a principal with Observe on another document and a principal with only Interact on `shared` both get `forbidden` from doc.get; Observe on `shared` reads `{version:"1", value:{x:1}}`. Mutant check: with `authority.authorize` removed from `dispatch/read.rs` this test fails at scope.rs:92 and the other four pass (`.omo` log `mutant-no-read-authz.log`); reverted.

- Round 2, B1: `doc.get` is served as an unrecorded read. New `ovis_terminal_server::dispatch::Read` trait and `Dispatcher::register_read`: catalog query, same todo-19 authorization at resolve time, an observe-level action enforced, answered off the executor from one committed snapshot. No operation receipt, no op events, no deadline ledger. New `ovis_terminal_core::events::Coordinator::read_view(read)` opens a snapshot under the commit lock and releases the lock before reading. The receipt cap (16 KiB, `ops/context.rs`) is unchanged. `docs::get_unit` was removed.

- `ovis_terminal_server::dispatch::Execution::commit_unit(build)` (new public API) commits the method's CAS writes, domain events and the operation receipt in one transaction through the landed `Operations::commit_unit`. A `Completion::failed` outcome is returned as that typed error. The shared start-and-recheck prelude was moved into a private `permit` helper; `commit` behaves the same. The server crate now depends on `ovis-terminal-store` (previously a dev-dependency only).
- New modules: `protocol::domain` (with `documents_retention` and its child `retention`), `core::docs`, `core::retention`, and `server::dispatch::lanes::documents_retention` (`register`). The protocol crate now provides `DocRecord::new`, `DocEntry::new` and `DocSetResult::new` constructors.
- `retention.floor_advanced` is an internal durable event type, not yet in the wire event catalog (same as `terminal.spawn_pending`).
- Two files sit outside the literal write scope. `crates/ovis-terminal-server/tests/documents_retention.rs` is the seam's test, which the brief requires. `conformance/scenarios/services/docs-retention.json` is a file because the runner's id grammar maps `services/docs-retention` to that path.

## Risks

- A landed todo-16 test outside this lane, `terminals qa::administration::native_process_roles_have_start_tokens_without_shell_or_agent_guessing`, failed once locally with `foreground unsupported` while another lane's verification was loading the host. It passed in isolation and in the final `check.sh` run. Its sampling of process roles is load-sensitive; reported, not changed.

- N1, owned by follow-up todo 79 (landed todo 10/15 behavior): with more than 8 concurrent mutations, the op-ledger accept path pins a coordinator snapshot per request (`pinned_snapshots: 8`) and fails fast with `limit_exceeded`; 16 concurrent writers can starve. There are no lost updates, and reads no longer take a pin through the ledger.

- Open decision: the landed op-ledger bound (`ops/request.rs`, 256 KiB of canonical params, pinned by `ops/qa/bounds.rs`, todo 15) rejects `doc.set` values between 256 KiB and the advertised 1 MiB `doc_max_bytes` with `too_large`. Either raise the ledger bound for this method or advertise 256 KiB. Not changed here.
- `conflict` carries no `current_version` on the wire because `ErrorData` has no details field (todo 31 follow-up). Clients re-read `doc.get`; the core error does carry `current_version`.
- Retention limits are defined in `RetentionPolicy::v1()` but not yet advertised through `hello`, and no `retention.*` RPC exists. Wiring them is assembly work (todo 52).
- The events and receipts floors come from the store trim; the receipts floor is read through a probe lookup with a non-UUID op id.

## CI history

- 16521cc: run 41211321607 failed only `test (windows-latest)`. The docs::qa fixture cleanup hit Os error 32 because the coordinator was not closed before the temp root was removed. Every other check passed.
- 4e609d1 ("fix(docs): make the document fixture platform-neutral"): runs 41212297861 and 41212297841, all 11 checks green including `test (windows-latest)`.
- macOS stress `docs-retention` (50 iterations): https://github.com/devswha/ovis-terminal/actions/runs/41211325520, success.
- b0b1da8 (round 2 code): PR ci not triggered while the PR was CONFLICTING; macOS stress run 41216932611 success.
- b1ef937 (orchestrator merge of 2e2cc07 on b0b1da8): ci 41221362114 and platform-probes 41221362091, both success.
- Round 3 head ("test(docs): prove doc.get refuses principals without Read scope"): see the PR checks.
