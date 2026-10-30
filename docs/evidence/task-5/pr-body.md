## Before

Absent: GAP-064 has no comparable new-runtime capability claimed as a legacy
execution. This change defines the holder protocol contract and does not claim
that the legacy runtime implemented adoption identity semantics.

## After

The holder crate checks passed on Linux 7.0.0-38-generic x86_64.

```text
$ cargo test -p ovis-terminal-holder --test protocol happy -- --exact --nocapture
running 1 test
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --test protocol failure -- --exact --nocapture
running 1 test
test failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --lib -- --nocapture
running 5 tests
test protocol::property_tests::frame_errors_expose_stable_kinds_hints_and_sources ... ok
test protocol::property_tests::rejected_header_does_not_leave_a_partial_decoder_frame ... ok
test protocol::property_tests::arbitrary_fragment_widths_round_trip ... ok
test protocol::property_tests::all_message_kinds_round_trip_at_every_two_fragment_boundary ... ok
test protocol::property_tests::identity_fields_round_trip_as_canonical_decimal_strings ... ok
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
exit: 0

$ cargo nextest run -p ovis-terminal-holder
exit: 0

$ cargo clippy -p ovis-terminal-holder --all-targets -- -D warnings
exit: 0

$ bash scripts/check.sh
check.sh: all steps passed
exit: 0
```

The happy case adopts terminal `t-17` incarnation 4 and process incarnation 9,
retains PID 4242 and `start_token=proc-start-abc`, and replays the
unacknowledged exit with final offset 8 until it is acknowledged. A typed `Tail`
message fixture containing `[104, 101, 97, 100]` over offsets 0..4 is encoded
and decoded through the production frame codec. This proves payload
round-tripping only; it does not claim that a running holder retrieved or
returned those bytes. The failure case checks stable
`unsupported_generation` and `stale_target` kinds and rejects malformed
headers and payloads larger than 16 MiB. Boundary errors use thiserror and
expose stable `kind()` plus recovery `hint()` values; malformed JSON preserves
its serde source.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The requested real-binary scenario was attempted. The conformance runner is
still the Todo 21 scaffold and registers no scenarios, so this is a recorded
deferred result, not a passing CLI/socket conformance claim.

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-5 --isolated --case happy --evidence "$ATTEMPT_DIR/task-5-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
exit: 2
```

Raw command output and crate evidence are in
[`conformance.log`](conformance.log), [`protocol-tests.log`](protocol-tests.log),
and [`checks.log`](checks.log). Re-run the real CLI/socket scenario after Todo 21
registers and implements `part-a/task-5`.

## QA

```text
$ cargo test -p ovis-terminal-holder --test protocol happy -- --exact --nocapture
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --test protocol failure -- --exact --nocapture
test failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 2 filtered out
exit: 0

$ cargo test -p ovis-terminal-holder --lib -- --nocapture
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out
exit: 0
```

## Adversarial

- Malformed input: generation 999, short header, nonzero reserved flags,
  oversized payload, and malformed JSON payload were rejected by
  assertion-bearing tests.
- Stale state: reusing terminal ID `t-reused` with incarnation 3 when
  incarnation 2 is retained returns `stale_target`.
- Repeated interruptions: a second adoption replays the retained exit record;
  an acknowledgement before final offset 8 is rejected and does not remove it.
- Fragmentation: every typed message variant round-trips at every two-part
  fragment boundary, with additional Proptest coverage over arbitrary fragment
  widths and identity fields.
- Error boundaries: adoption and frame errors expose stable kinds and recovery
  hints; malformed JSON payloads preserve their serde source error.
- No real-socket adversarial result is claimed; the Todo 21 runner is not
  implemented in this base branch.

## Risks

- Real CLI/socket conformance remains deferred until the Todo 21 runner and
  runtime assembly land. The attempted command and exit 2 are recorded above.
- This PR does not claim platform PTY survival behavior; those probes are owned
  by later holder/runtime tasks.
