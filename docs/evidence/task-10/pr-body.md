## Before

RT-HD-08 / GAP-020, GAP-024, GAP-025: the original transcript at
`/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h06-events.log:15` reports:

```text
TRAP CHECK: status event for A (first subscribe) received=false; for B (second subscribe on same conn) received=false
```

The existing durable copy is `docs/evidence/baseline/smoke/herdr-h06-events.log:15`
(the path without `smoke/` in the brief does not exist). The events module was an
unimplemented scaffold. `docs/evidence/task-10/tdd.log` records the initial missing-API
failure, exit 101, before implementation.

Base: c1dd6cf3389a27a2027f4e6b203f82ee75fce8a4 (origin/integration).

## After

Implemented the single commit/snapshot/subscription boundary using the existing
SQLite writer and its pinned WAL snapshots. Sequence comes from the durably
committed event row, never an in-memory preallocation. Typed event bodies and
state CAS writes share a transaction. Publication follows the commit receipt.
Inline and paged startup install independent delivery under that same boundary.
Remote-origin sequence and identity remain separate from local-mirror ordering.

Platform: Linux x86_64. Toolchain: rustc 1.99.1 (58cf8f9c3 2026-10-13).
All Cargo invocations used the isolated RUSTUP_HOME/CARGO_HOME and local zigcc
linker specified in AGENTS.md and the executor brief; no wrapper was committed.

```sh
cargo test -p ovis-terminal-core --test events happy -- --exact --nocapture
```

Actual component JSON from that command (not a CLI/socket transcript):

```json
{"schema_version":1,"machine_id":"machine:00000000-0000-0000-0000-000000000001","runtime_id":"runtime:1","seq":"1","boot_id":"boot:1","type":"terminal.updated","affected":[{"kind":"terminal","id":"terminal:1"}],"origin":null,"data":{"value":"one"}}
{"cursor":{"machine_id":"machine:00000000-0000-0000-0000-000000000001","runtime_id":"runtime:1","seq":"1"},"sections":{"layout":{"availability":"unavailable"},"terminals":{"availability":"complete","records":["one"]},"agents":{"availability":"unavailable"},"services":{"availability":"unavailable"}}}
```

```text
test happy ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.01s
HAPPY_EXIT=0
```

```sh
cargo test -p ovis-terminal-core --test events failure -- --exact --nocapture
```

```text
missed=lifetime_mismatch; page.kind=stale_snapshot; write.kind=conflict
test failure ... ok
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 12 filtered out; finished in 0.01s
FAILURE_EXIT=0
```

Final commands and actual exits:

| Command | Exit | Evidence |
| --- | --- | --- |
| `cargo nextest run -p ovis-terminal-core` | 0 | 31 passed, 0 skipped; 13 event tests |
| `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` | 0 | No warnings |
| `bash scripts/check.sh` | 0 | fmt, workspace Clippy, 245 tests passed, doctests, source/manifest/JSON hygiene |

The single workspace skip is the existing store `crash_child` process fixture,
explicitly invoked by its parent tests; this PR adds no ignored tests.
Full final output is in `docs/evidence/task-10/verification.log`. Structured evidence is
`docs/evidence/task-10/task-10-ovis-terminal-v1.json`, copied from the required
`.omo/evidence/ovis-terminal-v1/task-10/` attempt directory.

### DEFERRED VERIFICATION (runner lands in Todo 21)

The exact real-binary attempt was executed, not claimed green:

```sh
export ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t10/.omo/evidence/ovis-terminal-v1/task-10
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-10 --isolated --case happy --evidence "$ATTEMPT_DIR/task-10-ovis-terminal-v1.json"
```

```text
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
```

`docs/evidence/task-10/conformance.log` preserves the attempt. The runner did not
produce a socket transcript or evidence JSON. The executor-created JSON is labeled
component evidence, not runner output. `part-a/task-10.json` supplies executable
component happy/failure cases; real CLI/socket validation remains deferred by the
plan until Todo 21 assembly.

## QA

The happy test pauses the real store at Applied, before COMMIT, and verifies that
an already-installed delivery channel is Empty. It starts a racing subscription,
releases COMMIT and reconstructs state from snapshot R plus only later events,
asserting exactly one mutation and an Empty (not missed/disconnected) residual
queue. The preexisting subscriber receives the same committed cursor.

The failure test asserts lifetime_mismatch plus fresh state, stale_snapshot after
page release, and a real CAS conflict without any published event. Both exact
named commands run one assertion-bearing case. No timing sleeps or mocked store
behavior are used.

## Adversarial

- Disconnect/resume: real store close/reopen under a new server boot replays both
  retained commits once, without an extra snapshot; the next sequence advances.
- Stale state: foreign lifetime and expired cursor return typed missed reasons and
  fresh snapshots; released and old-core page tokens reject as stale_snapshot.
- Mirroring: remote u64::MAX survives durable replay unchanged while local sequence
  follows local commit order.
- Pagination: a 17 MiB record reconstructs from at most 64 KiB pages; a separate
  concurrent-mutation case proves constant revision and exactly one later event.
- Backpressure: queue overflow reports missed; dropping that queue releases the
  aggregate byte budget so a replacement subscriber can receive a later commit.
- Boundary parsing: numeric, leading-zero, signed, whitespace and overflowing
  sequence forms reject with invalid_params. u64::MAX and arbitrary u64 cursors
  round-trip exactly. A second property exercises typed records through SQLite
  commit, close, reopen, replay and snapshot reads.
- Producer completeness: unimplemented layout/agent/service sections remain
  unavailable. Only the implemented test terminal producer reports complete.

These cases all passed in the final core and workspace runs. Component probes do
not establish transport performance or cross-platform functional acceptance.

## Contract checklist

- [x] All 64-bit event sequences, page totals and byte offsets use existing protocol
  canonical decimal-string scalars; explicit u64::MAX and numeric-rejection tests.
- [x] One thiserror boundary enum with kind(), hint() and preserved inner sources;
  protocol MissedReason and landed store/identity errors are reused.
- [x] Registry machine/runtime/boot/layout/terminal identities are reused. Tokens,
  subscriptions and remote-origin sequences remain distinct semantic types.
- [x] Tests assert state transitions, real persistence, exact round trips and typed
  errors. Existing proptest dependency is reused; no prose-pinning tests.
- [x] No allow/expect suppressions, production unwrap/expect, placeholder TODOs or
  speculative domain producers. Source audit maximum is 230 pure lines; repository
  checker reports zero violations and enforces the three-parameter limit.
- [x] Deferred conformance is labeled exit 2, not green; no CLI/socket claim.

## Seams

No landed crate public API or tests were changed. The new core API owns the store
after bootstrap identity reservation. All later domain commits must use it, and
snapshot producers must use the supplied pinned WAL view, including the requested
filter, rather than independent live actor reads. Todo 21 adapts these components
to RPC envelopes and installs actual domain producers. Typed section composition
allows later agents/services without advertising empty successful sections now.
No dependency or Cargo.lock change is needed.

## Risks

- Real-binary/socket conformance is DEFERRED-BY-PLAN until Todo 21; no full IDEAL-023
  domain-roster or CLI/socket acceptance is claimed by this component PR.
- Local verification covers Linux x86_64 only. macOS/Windows evidence must come from
  native CI; no local cross-platform functional claim is made.
- Synchronous coordinator APIs must run on bounded blocking workers. Page leases
  pin WAL readers until explicit release or core shutdown; admission is bounded.
- rust-analyzer and Biome are unavailable on this host. Rust compiler/strict Clippy,
  actual test runs, JSON parsing and repository hygiene are the executed checks.
