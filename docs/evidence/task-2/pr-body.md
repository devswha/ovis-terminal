## Before

RT-HD-01 baseline: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h02-connection.log:2` records one frame received on one connection and `closedByServer=true`. Durable copy: `docs/evidence/baseline/smoke/herdr-h02-connection.log:2`. GAP-007 is the missing frame/logical-result limit and continuation contract.

## After

- `cargo nextest run -p ovis-terminal-protocol --no-capture` — exit 0; 7 tests passed, 0 skipped. Log: `docs/evidence/task-2/protocol-checks.log`.
- `cargo clippy -p ovis-terminal-protocol --all-targets -- -D warnings` — exit 0. Log: `docs/evidence/task-2/protocol-checks.log`.
- `bash scripts/check.sh` — exit 0; workspace nextest ran 18 tests, all passed; formatting, workspace Clippy, doctests, LOC/parameter hygiene, manifest hygiene, and JSON hygiene passed. Log: `docs/evidence/task-2/protocol-checks.log`.
- OS/architecture: Linux 7.0.0-38-generic, x86_64 GNU/Linux.
- `cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-2 --isolated --case happy --evidence "$ATTEMPT_DIR/task-2-ovis-terminal-v1.json"` — exit 2; actual output: `conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)`. No ovis-terminal CLI/socket transcript or runner-generated JSON is available until plan todo 21. Attempt output: `docs/evidence/task-2/conformance-attempt.log`; the blocked attempt record is at `.omo/evidence/ovis-terminal-v1/task-2/task-2-ovis-terminal-v1.json`.

## QA

- Happy: `cargo test -p ovis-terminal-protocol --test wire happy -- --exact --nocapture` — exit 0, one assertion-bearing test passed. It exhausts every byte-cut pattern for two coalesced frames with distinct channel IDs and binary payload bytes, checks mixed payload segmentation and byte preservation, checks a 64 MiB logical result in frame-bounded chunks with total/offset/continuation/truncation metadata, and verifies a coalesced hello reply plus framed bytes switch only at the explicit upgrade call. Log: `docs/evidence/task-2/happy.log`.
- Failure: `cargo test -p ovis-terminal-protocol --test wire failure -- --exact --nocapture` — exit 0, one assertion-bearing test passed. The supplied header declares 16,777,217 bytes and is rejected as `FrameTooLarge` while only the nine-byte header is buffered; three-byte header EOF and short payload EOF return typed truncation errors. Log: `docs/evidence/task-2/failure.log`.
- Backpressure: `cargo test -p ovis-terminal-protocol --lib wire::connection::tests::queued_frames_obey_record_and_byte_limits -- --exact --nocapture` — exit 0, one test passed. It reaches both 64 queued records and 32 MiB queued payload, receives typed `NeedDrain`, then drains and resubmits the unconsumed suffix. Log: `docs/evidence/task-2/gate-fixes.log`.
- Generated coverage: `cargo test -p ovis-terminal-protocol --lib wire::properties:: -- --nocapture` — exit 0, four tests passed. The three proptest properties generate valid frame sequences/split masks, arbitrary payload bytes, and arbitrary oversize lengths; a deterministic regression also resumes after each of the nine header-byte positions. Proptest's default `proptest-regressions` persistence is enabled; no case failed. Log: `docs/evidence/task-2/gate-fixes.log`.

## Adversarial

- Malformed input: the exact failure command above asserts `frame_too_large`, `truncated_frame`, stable kinds/hints, and the original UTF-8 source error.
- Cancel/resume: `cargo test -p ovis-terminal-protocol --lib wire::properties::partial_header_resumes_after_each_byte_position -- --exact --nocapture` — exit 0; decoder state resumes after each header byte.
- Flaky tests: three proptest properties use deterministic pure codec behavior with persisted failure cases; fixed exhaustive cut patterns remain as reviewed regressions. No sleeps or retries are used.
- Queue growth: the backpressure test asserts retained ready bytes never exceed 32 MiB and record count never exceeds 64.

## Risks

The task's real-socket conformance evidence remains deferred because its runner is supplied by todo 21, as stated in the plan's foundational component handoff rule. The implementation has only been exercised locally on Linux x86_64 in this lane; native macOS/Windows behavior is not claimed.
