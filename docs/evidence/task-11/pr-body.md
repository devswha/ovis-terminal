## Before

`absent` — there is no comparable new-runtime capability claimed as a legacy execution. This lane closes GAP-135 and GAP-142.

## After

`cargo test -p ovis-terminal-core --test telemetry happy -- --exact --nocapture` — exit 0, 1 assertion-bearing test passed. The structured sink record asserts request, operation, principal, machine, runtime, resource, sequence, and outcome fields; hostile text remains escaped data.

`cargo test -p ovis-terminal-core --test telemetry failure -- --exact --nocapture` — exit 0, 1 assertion-bearing test passed. `SECRET_TEST_VALUE` and `secret-input-payload` are absent from captured records.

`cargo test -p ovis-terminal-core --test telemetry invalid_sink_returns_typed_error -- --exact --nocapture` — exit 0; sink failure is asserted as `telemetry_io`.

`cargo nextest run -p ovis-terminal-core` — exit 0, 5 passed, 0 skipped.

`cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` — exit 0.

`bash scripts/check.sh` — exit 0, all steps passed; workspace nextest ran 16 tests with 16 passed, 0 skipped. Evidence: `docs/evidence/task-11/check.log` and `docs/evidence/task-11/tests.log`.

`cargo run --locked --quiet -p ovis-terminal-cli -- --version` — stdout `ovis-terminal 0.1.0`, exit 0. No telemetry diagnostics were written to protocol/CLI stdout.

Platform: Linux 7.0.0-38-generic, x86_64.

The requested assembled binary/socket transcript was not produced: `conformance/src/main.rs` registers no scenarios until todo 21. The sink test verifies correlated machine fields, but that later real-socket acceptance remains outstanding. Full test details are in `docs/evidence/task-11/tests.log`; the attempt manifest is `.omo/evidence/ovis-terminal-v1/task-11/task-11-ovis-terminal-v1.json`.

## QA

- Happy: correlated fields were asserted in the sink record and hostile field content remained escaped data; exact named test passed.
- Failure: credential and secret-input sentinels were absent from captured records; exact named test passed.
- Invalid sink: typed `telemetry_io` was asserted without a panic; exact test passed.
- Stdout: CLI version output was exactly the success line; exit 0.
- Real-socket conformance: unavailable until todo 21 registers the runner scenarios.

## Adversarial

- Prompt injection: hostile text serialized as a quoted, escaped field value.
- Misleading success: sink failure is typed and asserted as `telemetry_io`.
- Malformed input: oversized fields are bounded before serialization; assertion-bearing test passed.
- Secrets: credential and secret-input sentinel values do not occur in serialized sink records.

## Risks

The real CLI/socket correlation transcript required by the assembled conformance gate is not available at this wave because the conformance runner is not implemented. The PR must not be treated as satisfying that assembled acceptance until todo 21 provides the runner and this scenario is exercised against the binary.
