## Before

absent (GAP-013, GAP-144, GAP-147). No comparable new-runtime capability is claimed as a legacy execution. RT-HC-03 / EXCL-026 supplies the negative lesson: delivery or status alone cannot prove process termination.

## After

Persisted accepted, running, committed, failed, cancelled and outcome_unknown records through the existing event coordinator. The principal/runtime/op_id key binds method plus canonical parameters. Identical retries return the retained outcome; changed work returns op_reused with the prior receipt; expired UUIDv7 IDs cannot execute as fresh work. The advertised window is 86400000 ms after acceptance. Intent commits before a single-use running permission is granted. Actual holder write and exit evidence are independent facts. SQLite does not roll back a PTY write.

AdversarialVerify round 1 corrections make concurrent admission a join rather than a leaked CAS conflict: eight identical callers receive the same inspectable record and still obtain one execution permission, while changed payloads under the same ID receive op_reused with the winning receipt. A running permit can now be consumed by a proven no-effect failure, preserving its typed error across reboot; cancellation of that Failed record returns cancelled_before_commit. The read-only accessor has a shipped lease-byte-budget regression that fails if SnapshotLease/SnapshotAssembler are replaced by direct store capture.

**DEFERRED VERIFICATION (runner lands in Todo 21).** The actual command below exited 2. No assembled CLI/socket success is claimed. Rerun the checked-in part-a/task-15 scenario after assembly.

Platform: Linux 7.0.0-38-generic, x86_64. Cargo 1.99.1; rustc 1.99.1.

```sh
export ATTEMPT_DIR="/home/devswha/workspace/ovis-terminal-wt/lane-t15/.omo/evidence/ovis-terminal-v1/task-15"
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-15 --isolated --case happy --evidence "$ATTEMPT_DIR/task-15-ovis-terminal-v1.json"
```

```text
   Compiling ovis-terminal-conformance v0.1.0 (/home/devswha/workspace/ovis-terminal-wt/lane-t15/conformance)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 0.24s
     Running `target/debug/conformance --binary target/debug/ovis-terminal --scenario part-a/task-15 --isolated --case happy --evidence /home/devswha/workspace/ovis-terminal-wt/lane-t15/.omo/evidence/ovis-terminal-v1/task-15/task-15-ovis-terminal-v1.json`
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
EXIT_CODE=2
```

Component QA, not a CLI/socket transcript, drove the real SQLite writer and Unix PTY holder:

```sh
cargo test -p ovis-terminal-core --test ops happy -- --exact --nocapture
```

```text
holder_bytes=[65,82,77,69,68,13,10,69,70,70,69,67,84,58,111,110,101,13,10]
receipt={"op_id":"000005f5-e100-7000-8000-000000000001","method":"terminal.run","state":"committed","result":{"delivered_bytes":"4"},"error":null,"updated_seq":"6","evidence":{"terminal_id":"terminal:1","incarnations":{"terminal_incarnation":"1","process_incarnation":"1","status_seq":null},"intent":{"sequence":"1","grant_epoch":"1","fence_epoch":"1","requested_bytes":"4"},"write":{"written_bytes":"4","output_offset":"7","intent":{"sequence":"1","grant_epoch":"1","fence_epoch":"1","requested_bytes":"4"},"ack_epoch":"1"},"exit":{"code":0,"signal":null,"observed":true,"io_failure":false,"final_offset":"19"},"cancellation_requested":false}}
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out
EXIT_CODE=0
```

```sh
cargo test -p ovis-terminal-core --test ops failure -- --exact --nocapture
```

```text
reused=op_reused; expired=op_expired; unknown={"op_id":"000005f5-e100-7000-8000-000000000001","method":"terminal.run","state":"outcome_unknown","result":null,"error":null,"updated_seq":"3"}
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 16 filtered out
EXIT_CODE=0
```

The happy case asserts exactly one EFFECT:one marker, four acknowledged input bytes, an actual holder exit code 0, equality of the complete receipt on identical retry before and after core/store reopen, and refusal to grant a second execution. The failure case asserts changed argv -> op_reused, an old UUIDv7 -> op_expired, and an effect with no completion acknowledgement -> durable outcome_unknown after reopen, with no replay permission.

## QA

- `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings`: exit 0.
- `cargo nextest run -p ovis-terminal-core`: exit 0; 66 passed, 0 skipped, including 24 operation-focused tests.
- Both exact named QA commands above: exit 0; each ran one assertion-bearing case.
- `bash scripts/check.sh`: exit 0; workspace Clippy, formatting, 326 passing tests, doctests, 0 hygiene violations across 332 files, and 31 JSON artifacts validated. The one existing ignored test is ovis-terminal-store::transactions::crash_child, a subprocess fixture invoked by its parent crash tests, not a disabled operation test.
- TDD began with the new ops integration target failing on absent operation types, exit 101; see tdd-red.log. Intermediate strict-gate failures were fixed without suppressing or weakening tests. The final full log is verification.log.
- Regenerated schema/ovis-terminal.openrpc.json; released.openrpc.json is unchanged. Existing compatibility/catalog tests and three new cancellation contract tests pass, including unknown future outcome tokens and numeric-counter rejection.
- Property tests exercise 32 arbitrary decimal revisions and 32 generated real-store reopen/retry cases. Existing wire/holder fragmentation properties also pass in the full gate.
- Concurrent admission regressions deterministically force eight callers to capture the same empty revision: identical payloads return eight equal records and one effect; changed payloads return one record plus seven receipt-bearing op_reused errors.
- Exact shipped bounds cover 1024 accepted operations versus backpressure at 1025, a 16384-byte persisted receipt versus too_large at 16385, and 262144-byte canonical params versus too_large at 262145.
- `inspection::inspection_enforces_the_snapshot_lease_byte_budget` passes on the production accessor. Replacing it with direct store capture makes that test fail; the mutation transcript is `docs/evidence/task-15/mutation-accessor.log`.

## Adversarial

- Cancellation before running persists cancelled_before_commit across reopen. Running cancellation is pending; later completion remains already_committed with its receipt. Failed facts survive reopen and cancellation.
- Eight simultaneous start claims cause exactly one real file append. Repeated interruptions across four later boots never grant uncertain work again.
- Interruptions before completion with no acknowledgement and after a real holder acknowledgement remain outcome_unknown. Already stored delivery and exit facts are preserved.
- An Applied barrier holds a transaction before COMMIT: inspection cannot expose its revision. The eventual snapshot, event envelope and embedded operation sequence equal the committed receipt.
- Principal isolation, canonical object-key reuse, changed arrays/methods, 24-hour expiry, and a persisted acceptance floor across pruning/reopen/clock rollback are asserted.
- Foreign holder sequence/target evidence returns stale_target. Missing delivery proof cannot become committed input success. Actual InputWritten and Exit are independent facts.
- A proven holder/spawn rejection after running intent persists Failed plus its typed error through reboot instead of becoming outcome_unknown. Cancelling that known no-effect failure preserves the Failed record and returns cancelled_before_commit.
- The snapshot accessor honors the pinned representation byte budget; a direct-store implementation bypasses the bound and is killed by the shipped mutation test.

## Contract checklist

1. New wire counters use semantic decimal-string types. u64::MAX round trips exactly; numeric forms return typed invalid_params, including evidence counters.
2. The operation boundary uses thiserror::Error with stable kind()/hint() and retained source errors. Failed transitions carry the operation ID.
3. Principal, runtime, boot, method and operation identities reuse landed types. Holder write sequence, grant/fence/ack epochs, byte counts and offsets remain distinct semantic types.
4. Tests assert effects, transitions, receipts, rejections, commit visibility and actual store reopen behavior. No prose-pinning tests or mocked operation store/holder.
5. No allow attributes, crate-wide expectations, placeholder TODOs or weakened tests. Two function-scoped needless_pass_by_value expectations document intentional consumption of the single-use effect permission for proven-failure and uncertain-outcome transitions.
6. Conformance exit 2 is recorded verbatim and is DEFERRED-BY-PLAN until Todo 21, not reported as a pass. Cancellation exposes only cancelled_before_commit, already_committed or pending, matching IDEAL-011.

## Seams

- crates/ovis-terminal-core/src/events/inspection.rs:20 adds Coordinator::read_snapshot through SnapshotLease, paged reads and SnapshotAssembler, releasing its lease even on reconstruction failure. No raw connection escapes. ops/qa/inspection.rs proves only committed revisions are visible.
- crates/ovis-terminal-core/src/events/record.rs:123 adds EventData::at_sequence; coordinator.rs:178 stamps embedded operation revisions under the existing commit exclusion boundary before durable serialization. The store remains the sequence allocator. The barrier test asserts embedded and envelope revisions agree. Composite assembly event enums must forward this hook.
- crates/ovis-terminal-protocol/src/schema/methods/server/receipt.rs:43 adds optional typed OperationEvidence to OpRecord without changing released field names. Types remain re-exported from methods::server.
- crates/ovis-terminal-protocol/src/schema/methods/server/operation.rs:184 adds OpCancelResult: all released OpRecord fields remain flattened, plus an open typed cancellation outcome. Supporting evidence is typed, not a string blob. The event catalog adds accepted/running/cancelled/outcome_unknown. Generated OpenRPC, the additive fixture and contract tests are included.
- crates/ovis-terminal-core/src/ops/context.rs retries typed compare-and-swap conflicts by rereading the committed ledger. This converges identical submissions on the retained record and classifies changed work from the winner rather than returning a receipt-less store conflict.

## Risks

- Assembled CLI/socket conformance remains deferred until Todo 21. No native macOS or Windows result is inferred from the local Linux run. CI platform results are reported separately.
- Admission is bounded to 1024 indexed operations per runtime, 16 KiB per retained receipt and 256 KiB canonical params. Capacity rejects new work rather than evicting unexpired receipts. Large managed output belongs in the owning bounded log/output store, referenced by the method result.
- Assembly must provide authenticated context and schema-validated canonical method parameters, obtain running permission before effects, advertise the existing 24-hour limit, and forward the sequence hook for composite event types. This foundational PR does not register unassembled handlers.
- External writes and SQLite are separate durability domains. Unknown delivery is never automatically re-executed or represented as rollback. Querying a prior boot's running intent records the conservative unknown recovery fact.
- rust-analyzer was unavailable in this session; the actual Rust compiler and strict Clippy checked every changed Rust target.

Evidence: docs/evidence/task-15/verification.log, tdd-red.log, conformance-attempt.log and task-15-ovis-terminal-v1.json. Test fixtures remove their owned roots and join holder workers. No merge is performed.
