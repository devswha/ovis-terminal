## Before

RT-HD-03 / GAP-008, GAP-011, GAP-012 and GAP-134. The legacy wrong-field query answered for focused pane A instead of the requested pane B. Source: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h05-process-info-trap.log:16`; durable copy: `docs/evidence/baseline/smoke/herdr-h05-process-info-trap.log:16` (the baseline is under `smoke/`).

```text
<- {"id":"r16","result":{"type":"pane_process_info","process_info":{"pane_id":"w1:p1","shell_pid":2764185,"foreground_process_group_id":2764185,"foreground_processes":[{"pid":2764185,"name":"bash","argv":["/bin/bash"],"cmdline":"/bin/bash","cwd":"<run-root>/smoke"}]}}}
```

## After

Implement Todo 14's foundational dispatcher on `conn::RequestHandler`. Registration accepts actual adapters with the catalog's closed parameter schema, resolves typed targets and checks principal scope before invoking them. The connection still owns exactly-once response correlation. Dispatch owns bounded operation tasks beyond RPC deadlines and disconnects. The existing operation coordinator supplies durable admission, results, cancellation and deduplication; the effect boundary rechecks authority after the durable running-intent wait.

The default deadline is the advertised 10000 ms. Expiry returns an error of kind `deadline_exceeded`, with `op_id`, decimal-string `deadline_ms` and `elapsed_ms`, and hint `inspect or cancel the operation by op_id`. It is not a successful pending result and does not abandon the operation.

Platform: Linux 7.0.0-38-generic, x86_64; rustc 1.99.1, Cargo 1.99.1, nextest 0.9.150. Exact command:

```sh
cargo test -p ovis-terminal-server --test dispatch happy -- --exact --nocapture
# exit 0; exactly 1 assertion-bearing case
```

Literal socket transcript (the full hello and harness output are in [happy.log](happy.log)):

```text
-> {"id":"slow","jsonrpc":"2.0","method":"doc.set","params":{"name":"test","op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c","value":"payload","version":"0"}}
-> {"id":"status","jsonrpc":"2.0","method":"op.get","params":{"op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c"}}
<- {"jsonrpc":"2.0","id":"status","result":{"error":null,"method":"doc.set","op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c","result":null,"state":"accepted","updated_seq":"1"}}
<- {"jsonrpc":"2.0","id":"slow","error":{"code":-32000,"message":"operation failed","data":{"kind":"deadline_exceeded","hint":"inspect or cancel the operation by op_id","retryable":false,"op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c","deadline_ms":"10000","elapsed_ms":"10001"}}}
-> {"id":"inspect","jsonrpc":"2.0","method":"op.get","params":{"op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c"}}
<- {"jsonrpc":"2.0","id":"inspect","result":{"error":null,"method":"doc.set","op_id":"01a19041-ba07-75d9-b509-8f1bea4c2c3c","result":{"value":"payload","version":"0"},"state":"committed","updated_seq":"3"}}
```

The unrelated status request is `op.get` on that same connection. The fixture's `doc.set` and `terminal.keep` adapters perform real filesystem effects, but they are not the later document or terminal services and are never installed by the production dispatcher. The implemented built-ins are `op.get` and `op.cancel`; other methods require registration. The named failure case asserts the actual bytes `b"\"payload\""` (hex `227061796c6f616422`) and equality of the committed result and post-commit cancellation receipt. Parsed requests and replies are in [task-14-ovis-terminal-v1.json](task-14-ovis-terminal-v1.json).

### DEFERRED VERIFICATION (runner lands in Todo 21)

This foundational PR does not claim assembled CLI conformance. The required attempt was executed, not skipped:

```sh
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-14 --isolated --case happy --evidence "/home/devswha/workspace/ovis-terminal-wt/lane-t14/.omo/evidence/ovis-terminal-v1/task-14/task-14-ovis-terminal-v1.json"
```

Verbatim attempt output:

```text
Linux 7.0.0-38-generic x86_64
    Blocking waiting for file lock on build directory
   Compiling ovis-terminal-conformance v0.1.0 (/home/devswha/workspace/ovis-terminal-wt/lane-t14/conformance)
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 1.79s
     Running `target/debug/conformance --binary target/debug/ovis-terminal --scenario part-a/task-14 --isolated --case happy --evidence /home/devswha/workspace/ovis-terminal-wt/lane-t14/.omo/evidence/ovis-terminal-v1/task-14/task-14-ovis-terminal-v1.json`
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)

EXIT_CODE=2
```

The runner produced no evidence JSON. The checked-in JSON is explicitly executor-produced component evidence and records exit 2. The scenario is supplied at `conformance/scenarios/part-a/task-14.json` for assembly.

## QA

| Command | Observed assertions / result |
| --- | --- |
| `bash scripts/check.sh` | Exit 0: fmt, workspace Clippy, 381 passing tests, doctest command, source-size/parameter and manifest/JSON gates. One existing ignored store child-process fixture is exercised by its parent tests; no skip was added. |
| `cargo nextest run -p ovis-terminal-server` | Exit 0; 45 tests pass, none skipped. |
| `cargo clippy -p ovis-terminal-server --all-targets -- -D warnings` | Exit 0, no suppressed diagnostics. |
| `cargo test -p ovis-terminal-server --test dispatch happy -- --exact --nocapture` | Exit 0; one case proves status-before-deadline-error ordering and later committed inspection of the same operation. |
| `cargo test -p ovis-terminal-server --test dispatch failure -- --exact --nocapture` | Exit 0; one case proves rejection before handler/admission/effects and receipt-preserving post-commit cancellation. |
| `cargo test -p ovis-terminal-server --test dispatch -- --nocapture` | Exit 0; all 12 socket cases pass. |
| `cargo test -p ovis-terminal-protocol --test dispatch_deadline` | Exit 0; 3 tests cover maximum-value/numeric rejection, property roundtrips and additive compatibility/removal rejection. |
| `cargo test -p ovis-terminal-authz --test authority operations::` | Exit 0; 2 seam tests cover scoped ownership, foreign-principal/domain-action denial and revocation. |
| `openrpc check schema/ovis-terminal.openrpc.json` and `openrpc compat schema/released.openrpc.json` through Cargo | Both exit 0. |
| `cargo metadata --locked --no-deps --format-version 1` | Exit 0. |

Complete logs: [check.log](check.log), [server-acceptance.log](server-acceptance.log), [failure.log](failure.log), [adversarial-qa.log](adversarial-qa.log), [contract-tests.log](contract-tests.log), [schema-check.log](schema-check.log).

TDD began with failing builds for the missing deadline constructor, missing dispatch API, and missing operation-authorization variants. The named happy/failure tests subsequently ran assertions; no zero-test filter was counted as a pass.

## Adversarial

- The exact legacy `pane.split` injection is `method_not_found` at `method`, because that method is not in the landed catalog. It is not advertised. Known registered methods reject unknown top-level and nested targeting fields as `invalid_params` with their field paths.
- Real `TargetIndex` resolution returns fully qualified ambiguity candidates without an effect. Explicit terminal 2 produces a target-specific marker while terminal 1 remains untouched; a missing target never falls back to focus.
- Numeric/noncanonical versions and a non-v7 operation ID are rejected before the handler.
- Cancellation before the effect produces `cancelled_before_commit`; cancellation after running intent produces `pending` with retained cancellation evidence; after commit it returns `already_committed` and the retained result.
- Disconnect preserves the owned operation. Shutdown cancels cooperative preparation, joins effects already begun, and does not hold the admission lock while joining.
- 64 timed-out operations retain the negotiated per-connection budget. A new mutation is refused while reserved `op.get` status capacity remains usable; `op.cancel` uses that same reserved control pool. No second replies appear for settled request IDs.
- A document-scoped credential cannot act on another document. Revocation after admission prevents the effect. Own receipts remain inspectable under narrowed credentials; a different principal receives `not_found`.

## Contract checklist

1. All new protocol durations are existing `DurationMs` decimal-string newtypes; `u64::MAX` roundtrips exactly and numeric input is rejected. Existing wire fragmentation and store property tests run in the workspace gate.
2. `DispatchError` derives `thiserror::Error`, has stable `kind()` and `hint()`, and retains inner sources. Existing boundary, target, authz and operation errors are reused.
3. Registry IDs/generations, `PrincipalId`, `OpId`, duration and timestamp types are reused. No parallel identity family was introduced.
4. Tests assert real transitions, file effects, response correlation, strict field rejection and roundtrips. Barriers use channels/notifications/condition variables, not fixed sleeps; the real ten-second wait is itself the behavior under test.
5. No new `#[allow]`, `#[expect]`, placeholder TODO, prose-pinning test or success-shaped unavailable method. The repository source-size and parameter gates pass.
6. The exit-2 assembled-conformance attempt is recorded above as deferred by plan, not reported as a pass.

## Seams

- **Protocol:** additive `ErrorData.deadline_ms`, `elapsed_ms`, and typed ambiguity `candidates`; constructors, generated OpenRPC and compatibility fixture/tests. `DomainErrorKind::DeadlineExceeded` and its common-method catalog entry already existed and are reused, not duplicated. The released baseline is unchanged.
- **Authz:** additive `Action::InspectOperation`, `Action::CancelOperation` and `TargetResource::Operation { principal, op_id }`. These permit principal-owned ledger access using an existing observe/interact grant on the owning machine, even when that grant is resource-scoped. They cannot authorize a domain action or another principal's operation. Decisions remain epoch-bound and commit-rechecked. The authz crate's tests are updated in this PR.
- **Server dependencies:** existing workspace `schemars` verifies registration schema equality; existing `uuid` mints query operation IDs; the existing store crate is a dev dependency. Cargo.lock records these dependency seams. Core, holder and connection public APIs are unchanged.

## Risks

- Assembled CLI/socket conformance is deferred until Todo 21. Linux component QA is not local macOS or Windows runtime evidence.
- Preparation is cooperative and must remain non-mutating. An external effect already begun can remain pending until its owner completes; shutdown joins it and does not invent rollback.
- Assembly must register its actual domain adapters and await dispatcher shutdown before closing the coordinator. This PR does not install unavailable domain services or advertise them.
- Rust LSP is unavailable in this environment; compiler, Clippy and repository gates provide diagnostics. The work was self-reviewed without delegation as requested; no independent-review approval is claimed.
