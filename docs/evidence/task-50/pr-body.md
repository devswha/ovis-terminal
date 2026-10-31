feat(documents-retention): commit versioned documents and visible retention floors (todo 50)

Closes GAP-128. Delivers IDEAL-098, IDEAL-101. Enforces EXCL-020, EXCL-021 and EXCL-025, and does not reproduce V-005, V-3905 or V-RT-HC-02.

## Before

`absent`: there was no comparable baseline capability. This is not an executed failing test. Gap baseline: GAP-128.

## After

Host: Linux x86_64 (`uname -sm`), toolchain 1.99.1. Base: `origin/integration` b280ac9, which had not moved at push time. `target/debug/ovis-terminal` sha256 `f72aedd458f63e9b37671393911fe359ab6d83ad3e54adcdf5cfa789d8599ba3` (`docs/evidence/task-50/binary.sha256`).

| command | exit |
|---|---|
| `cargo build --locked -p ovis-terminal-cli --bin ovis-terminal` | 0 |
| `bash scripts/check.sh` (nightly fmt, workspace clippy `-D warnings`, nextest 578 run / 578 passed / 1 skipped, doctests, hygiene, json) | 0 |
| `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` | 0 |
| `cargo nextest run -p ovis-terminal-server --locked --test documents_retention --no-capture` (3 tests) | 0 |
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
- 1 MiB + 1 byte returns `too_large` and the version is unchanged (socket and core tests). The document bound is inclusive at exactly 1 MiB at the `DocWrite` boundary (core test). See Risks.
- Virtual expiry (core, `retention/qa/sweeps.rs`): sweeping all five classes eight virtual days later moves every floor (output_history 3, events 3, receipts 4, plugin_runs 4, finished_terminals 4). Each sweep emits one `retention.floor_advanced` event, and the evicted plugin-run record becomes a tombstone (null value, version 2), not an absence. No wall-clock sleeps.

## Adversarial

- Stale state, cursor below the floor: a cursor at seq 1 after the store trim returns `missed: cursor_expired`, `RetentionFloor::lookup` below the floor returns `expired`, and `resume_after` below the floor returns `gap` (proptest over all u64 values).
- Cancel/resume, expired op id: an op id minted 24 h + 1 ms ago returns `op_expired` (core) and 24 h + 60 s ago returns `op_expired` (socket); nothing is committed and no event is published.
- Malformed input: an oversized document gives `too_large`, a numeric version gives `invalid_params`, a name `a/b`, an empty name or a 129-byte name gives `invalid_name` (128 bytes is accepted), and a foreign `machine` gives `not_found`.
- Compaction: a snapshot taken before a sweep still reports floor 0 and the new view reports 4. A repeated or older sweep returns `Unchanged` with no event, and a stale tombstone version rolls back the whole sweep (`conflict`, floor and record unchanged).
- Flakiness: virtual clock only, and the race uses a barrier, not sleeps. A macOS stress job (`docs-retention`, 50 iterations) is added at the end of `probe-stress-macos.yml`.

## Contract checklist

1. Every counter in the new JSON (`version`, `seq`, `floor`, `capacity`, `evicted`, `max_age_ms`) is a decimal string, and numeric forms are rejected (tested). u64::MAX round-trips exactly (tested).
2. `DocumentError`, `DocError` and `RetentionError` are thiserror enums with `kind()`/`hint()` and `#[source]`/`#[from]`, and Display does not repeat the kind. Domain failures are retained as the operation's typed `ErrorData`.
3. `RetentionPosition` and `RetentionQuantity` are distinct newtypes; `Option` is used only for "no age bound".
4. Tests assert state transitions, events and typed kinds through the real store; proptest covers the version model, eviction planning and floor outcomes.
5. No `#[allow]`, no placeholders, no prose pins.
6. Holder wire generation is still 2. Contract checklist items are unchanged.

## Seams

- `ovis_terminal_server::dispatch::Execution::commit_unit(build)` (new public API) commits the method's CAS writes, domain events and the operation receipt in one transaction through the landed `Operations::commit_unit`. A `Completion::failed` outcome is returned as that typed error. The shared start-and-recheck prelude was moved into a private `permit` helper; `commit` behaves the same. The server crate now depends on `ovis-terminal-store` (previously a dev-dependency only).
- New modules: `protocol::domain` (with `documents_retention` and its child `retention`), `core::docs`, `core::retention`, and `server::dispatch::lanes::documents_retention` (`register`). The protocol crate now provides `DocRecord::new`, `DocEntry::new` and `DocSetResult::new` constructors.
- `retention.floor_advanced` is an internal durable event type, not yet in the wire event catalog (same as `terminal.spawn_pending`).
- Two files sit outside the literal write scope. `crates/ovis-terminal-server/tests/documents_retention.rs` is the seam's test, which the brief requires. `conformance/scenarios/services/docs-retention.json` is a file because the runner's id grammar maps `services/docs-retention` to that path.

## Risks

- Open decision: the landed op-ledger bound (`ops/request.rs`, 256 KiB of canonical params, pinned by `ops/qa/bounds.rs`, todo 15) rejects `doc.set` values between 256 KiB and the advertised 1 MiB `doc_max_bytes` with `too_large`. Either raise the ledger bound for this method or advertise 256 KiB. Not changed here.
- `conflict` carries no `current_version` on the wire because `ErrorData` has no details field (todo 31 follow-up). Clients re-read `doc.get`; the core error does carry `current_version`.
- Retention limits are defined in `RetentionPolicy::v1()` but not yet advertised through `hello`, and no `retention.*` RPC exists. Wiring them is assembly work (todo 52).
- The events and receipts floors come from the store trim; the receipts floor is read through a probe lookup with a non-UUID op id.
