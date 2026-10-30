## Before

`absent` — there is no comparable new-runtime capability claimed as a legacy execution. This lane closes GAP-135 and GAP-142.

## After

`cargo test -p ovis-terminal-core --test telemetry` — exit 0, 6 passed. `bash scripts/check.sh` — exit 0; workspace nextest ran 17 tests, 17 passed and 0 skipped; Clippy, rustfmt, doctests and hygiene checks passed. Logs: `docs/evidence/task-11/check.log` and `docs/evidence/task-11/tests.log`.

`cargo test -p ovis-terminal-core --test telemetry happy -- --exact --nocapture` — exit 0; parsed JSON assertions verify request, operation, principal, machine, runtime, resource and outcome. Sequence is a JSON string, and hostile text round-trips as data.

`cargo test -p ovis-terminal-core --test telemetry maximum_sequence_round_trips_as_decimal_string -- --exact --nocapture` — exit 0; parsed sequence `"18446744073709551615"` round-trips through `u64::from_str` to `u64::MAX`.

`cargo test -p ovis-terminal-core --test telemetry failure -- --exact --nocapture` — exit 0; `SECRET_TEST_VALUE` and `secret-input-payload` are absent from captured records.

`cargo test -p ovis-terminal-core --test telemetry invalid_sink_returns_typed_error -- --exact --nocapture` — exit 0; `TelemetryError::TelemetryIo`, kind `telemetry_io`, the recovery hint and the underlying source are asserted.

The telemetry API uses distinct `RequestId`, `OperationId`, `PrincipalId`, `MachineId`, `RuntimeId`, and `ResourceId` types. `TelemetryError` derives `thiserror::Error` and exposes stable `kind()` and `hint()` accessors.

`cargo run --locked --quiet -p ovis-terminal-cli -- --version` — stdout `ovis-terminal 0.1.0`, exit 0. No telemetry diagnostics were written to protocol/CLI stdout.

Platform: Linux 7.0.0-38-generic, x86_64.

The requested assembled binary/socket transcript was not produced: `conformance/src/main.rs` registers no scenarios until todo 21. The sink test verifies correlated machine fields, but that later real-socket acceptance remains outstanding. Full test details are in `docs/evidence/task-11/tests.log`; the attempt manifest is `.omo/evidence/ovis-terminal-v1/task-11/task-11-ovis-terminal-v1.json`.

## QA

- Happy: correlated fields were parsed from valid JSON; hostile field content remained one value; exact named test passed.
- Precision: maximum `u64` sequence was emitted and parsed as the exact decimal string.
- Failure: credential and secret-input sentinels were absent from captured records; exact named test passed.
- Invalid sink: typed `telemetry_io`, stable recovery hint and preserved source were asserted without a panic.
- Stdout: CLI version output was exactly the success line; exit 0.
- Real-socket conformance: unavailable until todo 21 registers the runner scenarios.

## Adversarial

- Prompt injection: hostile text serialized as a quoted, escaped field value.
- Misleading success: sink failure is typed and asserted as `telemetry_io`.
- Malformed input: oversized fields are bounded before serialization; assertion-bearing test passed.
- Secrets: credential and secret-input sentinel values do not occur in serialized sink records.

## Risks

The real CLI/socket correlation transcript required by the assembled conformance gate is not available at this wave because the conformance runner is not implemented. The PR must not be treated as satisfying that assembled acceptance until todo 21 provides the runner and this scenario is exercised against the binary.
