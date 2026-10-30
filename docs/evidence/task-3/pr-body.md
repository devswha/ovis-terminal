Plan todo 3, wave 2. Closes GAP-006, GAP-014, GAP-015; delivers IDEAL-005, IDEAL-008, IDEAL-013.

## Before

RT-HC-06, GAP-006, GAP-014, GAP-015. hc's stable `kind` and `hint` are a keeper extended here.

Baseline `docs/evidence/baseline/smoke/hc-s13-errors.log:5`:

```json
{"error":{"hint":"see `hc ls --all`","kind":"not_found","message":"no terminal \"nope-t99\""}}
```

The hc envelope lacks `retryable`, `field_path`, `target`, and `op_id`; older herdr RPC errors also fail to guarantee request-ID correlation. Source excerpt is copied to `docs/evidence/task-3/baseline-hc-s13-errors.log`.

## After

Linux x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13). Full command receipts live in `docs/evidence/task-3/`.

| Command | Result | Evidence |
|---|---|---|
| `bash scripts/check.sh` after merge with `origin/integration` | Exit 0: fmt, workspace Clippy `-D warnings`, nextest 58 tests/1 skipped, doctests, hygiene 0/83, manifests, JSON | `check-script-post-merge.log` |
| `cargo nextest run -p ovis-terminal-protocol` | Exit 0: 19 tests passed | `acceptance-nextest-gate-fix.log` |
| `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings` | Exit 0 | `acceptance-clippy.log` |
| `cargo test -p ovis-terminal-protocol --test rpc happy -- --exact --nocapture` | Exit 0: 1 test passed, 8 filtered | `qa-happy-gate-fix.log` |
| `cargo test -p ovis-terminal-protocol --test rpc failure -- --exact --nocapture` | Exit 0: 1 test passed, 8 filtered | `qa-failure-gate-fix.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-3 --isolated --case happy --evidence "$ATTEMPT_DIR/task-3-ovis-terminal-v1.json"` | Exit 2: no scenarios registered | `conformance-attempt-postmerge.log` |

Conformance result, verbatim:

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit=2
```

Conformance execution is **DEFERRED-BY-PLAN** (plan line 275): the real-socket scenario runner belongs to todo 21 and the assembled binary to todo 31. After merging the current integration base, the runner still registers no scenarios and the CLI still lacks RPC assembly. This exit-2 attempt is not a conformance pass; no socket transcript or `task-3-ovis-terminal-v1.json` result is claimed.

## QA

Happy command: `cargo test -p ovis-terminal-protocol --test rpc happy -- --exact --nocapture`. It runs exactly one assertion-bearing `happy` test: parse two requests with IDs `request-a` and `request-b`, admit both in flight, settle `request-b` before `request-a`, and assert each terminal reply has its matching ID, corresponding result, and no error branch. Result: `test happy ... ok`; 1 passed, 8 filtered, exit 0 (`qa-happy-gate-fix.log`).

Failure command: `cargo test -p ovis-terminal-protocol --test rpc failure -- --exact --nocapture`. It feeds `{"jsonrpc":"2.0","id":7,"method":"terminal.get","params":{"pane_id":"p1"}}` to `parse_request` and asserts `invalid_request`, JSON-RPC -32600, and `field_path == "id"`. The parser returns `Err`, so no request value is available to dispatch. Result: `test failure ... ok`; 1 passed, 8 filtered, exit 0 (`qa-failure-gate-fix.log`). The unknown parameter field is independently rejected as `invalid_params` (-32602) with `params.pane_id`.

## Adversarial

- Numeric request ID: rejected as `invalid_request`, field path `id`.
- Unknown top-level request field: rejected as `invalid_request`, field path `debug`.
- Unknown top-level or nested target field and non-string target identity: rejected as `invalid_params` with the precise field path before dispatch.
- Error/result exclusivity and wrong JSON-RPC version: `parse_reply` returns `invalid_response`.
- Duplicate in-flight ID: second begin returns `duplicate_request_id`.
- B1 parser source: `parse_request("{")` exposes `Error::source()`, while the serialized error data omits the serde diagnostic (`malformed_json_preserves_source_without_serializing_parser_prose`).
- B2 unknown method: `BoundaryError::method_not_found()` has and serializes `field_path == "method"`.
- N1 exactly-once settlement: two IDs complete out of order; a second completion returns `unknown_or_settled_request_id`; a mismatched response ID leaves the matching request active. Proptest checks begin/complete sequences against a reference `HashSet` (`tests/rpc_settlement.rs`).
- N2 prose overfit removed: no assertion constrains the wording of Display text; stable kind and hint accessors are tested separately.
- Counter encoding: `DecimalU64` rejects JSON numeric values, non-canonical strings and overflow; `u64::MAX` round-trips exactly. Proptest exercises random u64 values and random settlement sequences against a reference set.

## Contract checklist

1. **Canonical decimal counters:** `DecimalU64` serializes as a decimal JSON string and accepts only canonical strings. Tests cover exact `u64::MAX`, numeric and malformed string rejection, and proptest round trips. This todo defines no other numeric counter field; later sequence/revision/offset/generation fields must use this type.
2. **Typed boundary errors:** `BoundaryError` derives `Debug` and `thiserror::Error`; `kind()` and `hint()` return static strings. `ParseError` and `InvalidResponseJson` retain `serde_json::Error` under `#[source]`. Wire data is built separately and does not serialize source prose. Tests assert `source().is_some()` and that wire JSON omits the source text. Display prose is not pinned.
3. **Semantic newtypes:** `RequestId`, `TargetId`, `OpId`, `FieldPath`, and `DecimalU64` distinguish identities and numeric units. Target/op IDs are accepted by typed methods, not interchangeable with `String`.
4. **Behavior and property tests:** parser, response exclusivity, source-preservation, settlement, target-field and error-code tests assert observable results. Proptest checks counter round trips and random settlement sequences against a reference model.
5. **No suppression or placeholder escapes:** no `#[allow]`, `#[expect]`, TODO, FIXME, `todo!`, or `unimplemented!` in the protocol crate. Test JSON access uses `Value::pointer`/`first()` rather than panic indexing.
6. **Evidence matches assertions:** the QA and adversarial descriptions above state the actual test behavior. Real-socket execution is **DEFERRED-BY-PLAN** until todo 21; the observed exit-2 attempt is recorded in `conformance-attempt.log`.

## Risks

- Connection-wide in-flight capacity/backpressure is owned by the connection layer (todos 13/14); `InFlightRequests` only enforces identity and settlement, not a queue bound.
- Full per-method request schemas and dispatch-time rejection arrive with schemars/dispatcher work in todos 4/14.
- The manifest dependency seam added serde, serde_json, thiserror, and dev proptest; Cargo regenerated `Cargo.lock`. `errors/` and the `tests/rpc_*.rs` files split responsibilities to satisfy the 250 pure-LOC gate; the named `--test rpc happy|failure` targets are unchanged.
- No real-socket transcript exists for this foundational lane; see the explicit deferred conformance result above.
