## Before

RT-HD-01 baseline: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h02-connection.log:2` records one frame received on one connection and `closedByServer=true`. The plan names the durable copy as `docs/evidence/baseline/herdr-h02-connection.log:2`; that path is absent from the integration worktree and could not be read. GAP-007 is the missing frame/logical-result limit and continuation contract.

## After

- `cargo nextest run -p ovis-terminal-protocol` — exit 0; 2 tests passed, 0 skipped. Log: `docs/evidence/task-2/protocol-checks.log`.
- `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings` — exit 0. Log: `docs/evidence/task-2/protocol-checks.log`.
- `bash scripts/check.sh` — exit 0; formatting, workspace clippy, workspace nextest, doctests, LOC/parameter hygiene, manifest hygiene, and JSON hygiene passed. Log: `docs/evidence/task-2/protocol-checks.log`.
- OS/architecture: Linux 7.0.0-38-generic, x86_64 GNU/Linux.
- `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-2 --isolated --case happy --evidence "$ATTEMPT_DIR/task-2-ovis-terminal-v1.json"` — exit 2; actual output: `conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)`. No ovis-terminal CLI/socket transcript or runner-generated JSON is available until plan todo 21. Attempt output: `docs/evidence/task-2/conformance-attempt.log`; the blocked attempt record is at `.omo/evidence/ovis-terminal-v1/task-2/task-2-ovis-terminal-v1.json`.

## QA

- Happy: `cargo test -p ovis-terminal-protocol --test wire happy -- --exact --nocapture` — exit 0, one assertion-bearing test passed. It exhausts every byte-cut pattern for two coalesced frames with distinct channel IDs and binary payload bytes, checks mixed payload segmentation and byte preservation, checks a 64 MiB logical result in frame-bounded chunks with total/offset/continuation/truncation metadata, and verifies a coalesced hello reply plus framed bytes switch only at the explicit upgrade call. Log: `docs/evidence/task-2/happy.log`.
- Failure: `cargo test -p ovis-terminal-protocol --test wire failure -- --exact --nocapture` — exit 0, one assertion-bearing test passed. The supplied header declares 16,777,217 bytes and is rejected as `FrameTooLarge` while only the nine-byte header is buffered; three-byte header EOF and short payload EOF return typed truncation errors. Log: `docs/evidence/task-2/failure.log`.

## Adversarial

- Malformed input: oversized length and partial header/payload are asserted in the failure test.
- Cancel/resume: decoder state is retained and resumed after each of the nine header-byte positions in the happy test.
- Flaky tests: segmentation cases are deterministic exhaustive cut patterns and fixed chunk-size runs; no timing or retry behavior is used. Proptest-generated inputs were not added because the task's write scope excludes the crate manifest that would enable its workspace dev dependency.

## Risks

The task's real-socket conformance evidence remains unmet because its runner is supplied by todo 21. The specified durable RT-HD-01 baseline copy is absent from the integration worktree. The implementation has only been exercised on Linux x86_64 in this lane; macOS/Windows behavior is not claimed.
