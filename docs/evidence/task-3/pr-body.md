Plan todo 3 (wave 2). Closes GAP-006, GAP-014, GAP-015; delivers IDEAL-005, IDEAL-008, IDEAL-013.

Adds `ovis_terminal_protocol::rpc` (string `RequestId`, closed `RpcRequest` parsed only through `parse_request`, `RpcSuccess`/`RpcFailure`/`RpcReply` with result/error exclusivity, `InFlightRequests` duplicate-ID rejection, `DecimalU64` canonical counters) and `ovis_terminal_protocol::errors` (`BoundaryError` thiserror enum, `ErrorKind` with stable `token()`/`hint()`, `JsonRpcCode`, `TargetId`/`OpId`/`FieldPath` newtypes).

## Before

RT-HC-06, GAP-006, GAP-014, GAP-015. Keeper, extended here: hc already emits `kind` + `hint`.

`docs/evidence/baseline/smoke/hc-s13-errors.log:5`

```
{"error":{"hint":"see `hc ls --all`","kind":"not_found","message":"no terminal \"nope-t99\""}}
```

Excerpt with source path: `docs/evidence/task-3/baseline-hc-s13-errors.log`. What the keeper lacks and this lane adds: `retryable`, `field_path`, `target`, `op_id`, a JSON-RPC code derived from the typed kind (never from message text), and request-ID correlation guarantees (GAP-006).

## After

Linux x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13), isolated toolchain. Full logs under `docs/evidence/task-3/`.

| Command | Exit | Evidence |
|---|---|---|
| `bash scripts/check.sh` (fmt, clippy -D warnings, nextest 25/25, doctests, hygiene 0/57, manifests, json) | 0 | `check-script.log` |
| `cargo nextest run -p ovis-terminal-protocol` | 0, 14 tests run, 14 passed | `acceptance-nextest.log` |
| `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings` | 0 | `acceptance-clippy.log` |
| `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-3 --isolated --case happy --evidence "$ATTEMPT_DIR/task-3-ovis-terminal-v1.json"` | **2** | `conformance-attempt.log` |

Conformance output, verbatim:

```
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit=2
```

The runner is implemented by todo 21 and the binary by todo 31; neither is on `origin/integration`. Per plan line 275 the real-socket conformance for part-A foundational todos is deferred until todo 21 lands. **No ovis-terminal socket transcript exists for this lane yet**; the acceptance item "conformance `part-a/task-3` exits 0 after assembly" is unmet by design at this point. `conformance/scenarios/part-a/task-3.json` is checked in for that runner (JSON validated by `scripts/hygiene/json.sh`).

TDD red run (API absent, crate uncompilable) is recorded in `tdd-red.log`.

## QA

Happy: `cargo test -p ovis-terminal-protocol --test rpc happy -- --exact --nocapture` -> `test failure ... ok`, `1 passed; 8 filtered out`, exit 0 (`qa-happy.log`). The test parses two interleaved requests with string IDs `request-a` and `request-b`, admits both into `InFlightRequests`, serializes one `RpcSuccess` per ID and asserts each encoded reply carries exactly its own `id`, a `result`, and no `error` key.

Failure: `cargo test -p ovis-terminal-protocol --test rpc failure -- --exact --nocapture` -> `test failure ... ok`, `1 passed; 8 filtered out`, exit 0 (`qa-failure.log`). The test feeds `{"jsonrpc":"2.0","id":7,"method":"terminal.get","params":{"pane_id":"p1"}}` to `parse_request` and asserts `Err` with `error_kind() == InvalidRequest`, `json_rpc_code() == InvalidRequest` (-32600) and `field_path() == Some("id")`. Because `parse_request` returns `Err`, no `RpcRequest` value exists to dispatch; dispatch count is zero by construction. The unknown `pane_id` field is covered by the separate `rejects_unknown_target_parameter_before_dispatch` test (`invalid_params`, -32602, `field_path == "params.pane_id"`).

## Adversarial

- malformed input, wrong ID type: numeric `id` -> `invalid_request`, `field_path:"id"` (`failure` test).
- malformed input, unknown target field: `params.pane_id` -> `invalid_params` with that path (`rejects_unknown_target_parameter_before_dispatch`); unknown field inside `params.target` -> `invalid_params`, `field_path:"params.target.pane_id"` (`rejects_unknown_nested_target_field_before_dispatch`); non-string `target.terminal_id` -> `invalid_params`, `field_path:"params.target.terminal_id"` (`rejects_non_string_target_identity_before_dispatch`).
- malformed input, unknown envelope field: top-level `debug` -> `invalid_request`, `field_path:"debug"` (`rejects_unknown_request_envelope_fields`).
- misleading success, error/result exclusivity: a reply carrying both `result` and `error` -> `parse_reply` returns `invalid_response` (`rejects_error_and_result_in_one_response`); a reply with `jsonrpc:"1.0"` is likewise rejected (`rejects_response_with_wrong_json_rpc_version`).
- duplicate in-flight ID: second `begin` with the same `RequestId` -> `Err(duplicate_request_id)` (`rejects_duplicate_in_flight_request_ids`).
- precision: `DecimalU64` rejects JSON numbers, `"03"`, `""`, `"+3"`, and `2^64` as a string; `u64::MAX` round-trips exactly; proptest over all `u64` (`tests/rpc_counter.rs`).

## Contract checklist

1. Canonical decimal counters: `rpc::DecimalU64` serializes every u64 as a decimal string and its deserializer rejects JSON numbers (`visit_u64/i64/f64` -> `invalid_type`), leading zeros, sign, empty and overflow. Tests: `decimal_counter_round_trips_u64_max_exactly` (exact `"18446744073709551615"`), `decimal_counter_rejects_json_numbers_and_non_canonical_strings`, proptest `decimal_counter_round_trips_every_value`. This todo defines no other numeric wire field; later todos must use `DecimalU64` for seq/revision/offset/generation.
2. Typed boundary errors: `BoundaryError` is `#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]`; `kind() -> &'static str` returns `ErrorKind::token()` (snake_case machine token, identical to the serde name); `hint() -> &'static str` is per-kind recovery guidance; `#[error("...")]` strings describe the condition and never contain the token (asserted by `!display.contains(kind_token)` in `serializes_structured_boundary_error_fields`). No inner error is wrapped at this boundary, so no `#[source]` field exists; serde/parse failures are mapped to `ParseError`/`InvalidResponse` without carrying serde prose (V-RT-HD-12). No hand-written `Error` impl, no `String` errors.
3. Identity newtypes: `RequestId(String)`, `TargetId(String)`, `OpId(String)`, `FieldPath(String)`, `DecimalU64(u64)`; `with_target`/`with_op_id` take the newtypes, not `&str`. `Option` appears only on `field_path`/`target`/`op_id`, where absence is a real state (IDEAL-013: "when known").
4. Behavior tests: every test drives `parse_request`/`parse_reply`/`InFlightRequests::begin`/serde and asserts the resulting kind, code, path or encoded field; no test reads back a locally assigned literal. `proptest` is a dev-dependency and covers the counter round trip.
5. No `#[allow]`, no `#[expect]`, no TODO/placeholder in the crate (grep: zero matches). Panicking JSON indexing in tests was replaced by `Value::pointer`.
6. Evidence states exactly what each test asserts (see QA/Adversarial). Conformance execution is deferred by plan (line 275) until todo 21; the exit-2 attempt is recorded verbatim above and in `conformance-attempt.log`.

Bounded queues/backpressure (plan:220) are not in this todo's scope: this crate defines envelopes and errors only; `InFlightRequests` is a set keyed by ID whose cap belongs to the connection owner (todo 13), which will reject overflow with a typed kind.

## Risks

- Scope: `crates/ovis-terminal-protocol/Cargo.toml` and `Cargo.lock` were changed (serde, serde_json, thiserror; dev: proptest, serde_json) under the orchestrator ruling that the manifest is a dependency seam; `Cargo.lock` was regenerated by Cargo, not hand-edited (`cargo-scope-decision.md`). `tests/rpc_counter.rs` and `tests/rpc_errors.rs` exist because the 250 pure-LOC gate would not accept one test file; the plan's `--test rpc happy|failure` targets are unchanged.
- `ErrorKind` is `#[non_exhaustive]`; todo 4's OpenRPC generation must enumerate it from `token()` so the schema and the enum cannot drift.
- `RpcRequest::validate_params` checks only the top-level field set and the `target` object shape; full per-method schemas arrive with schemars in todo 4.
- No real-socket proof yet (see After).
