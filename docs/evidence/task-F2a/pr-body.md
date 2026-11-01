fix(conformance): remote/isolation step-8 flake - stamp the remote child's op id and clock from one reading (plan F2a)

## Root cause (fixture, not product; the scenario's expectation is correct)

`conformance/src/fixture/machines/remote.rs::link` (the stand-in for a remote launch behind `fixture.remote_child`) read the wall clock into the operation `Context.now`, **then** minted the op id with `uuid::Uuid::now_v7()` from the live clock. Admission refuses an id minted after its clock (`crates/ovis-terminal-core/src/ops/admission.rs:93`: `if request.minted_at > self.context.now { return Err(OperationError::InvalidParams) }`). Whenever a millisecond tick fell between the two reads (a window of microseconds), `accept()` failed `InvalidParams`. `internal()` then answered `{"kind":"internal","hint":"inspect the fixture's stderr"}` and **dropped the cause**, so the evidence showed an empty stderr in all three occurrences.

No isolation property was ever violated: the error comes from the fixture's own op admission, before `TreeService::link_remote` runs. The product orders these correctly everywhere: every `Context::new(.., now()?)` site reads the clock after its caller minted the op id (`Call::new(session, OpId::new(now_v7()))` comes first).

## Before

The same reply in all three occurrences:

1. T30 local all/all: `docs/evidence/task-30/flake-remote-isolation.log` (branch lane/t30-prove-attach-input-and-aggre). The surviving all.json shows a 426 ms case and fixture stderr bytes=0.
2. PR #78 CI run 41349272942 attempt 1, job 124976014124 (scripts/check.sh ubuntu), log line 2165.
3. PR #76 CI run 41352257500, job 124984812810 (test ubuntu-latest), log line 1932.

```
"remote/isolation" "happy": {"detail":"expectation reply[child-on-bravo]/error did not hold","hint":"compare expected and actual values in the evidence","kind":"assertion_failed","step":"8"}
  {"actual":{"code":-32603,"data":{"hint":"inspect the fixture's stderr","kind":"internal","retryable":false},"message":"operation failed"},"check":"reply[child-on-bravo]/error","expected":{"$present":false},"passed":false}
```

Full quotes: `docs/evidence/task-F2a/before.log`. Neither CI run uploaded artifacts.

## After

Linux 7.0.0-38-generic x86_64 (`docs/evidence/task-F2a/after.log`):

- **Widened-gap reproduction (the binding proof; diagnostic patch, not committed).** I re-applied the old order (clock read, 2 ms busy wait, live-clock mint). It failed 3/3 at step 8 with the byte-identical reply, and the fixture's stderr showed `InvalidParams`. Re-applied on this branch, the stderr now carries the cause: `{"internal":"InvalidParams"}`.
- **Regression test.** `cargo nextest run --locked --manifest-path conformance/Cargo.toml -E 'test(stamped)'` -> 1 passed, exit 0. Test: `fixture::machines::remote::tests::an_operation_stamped_at_a_reading_is_accepted_at_that_reading`. It stamps three readings in the past (on, just past and just before a millisecond tick) and asserts the real core `Operations::accept` answers `accepted` for each.
- **Mutants.** `docs/evidence/task-F2a/mutants.log`: 4/4 fail the shipped test with `Err("invalid_params")`. M1 is the old code (id minted from the live clock); M2 drops the sub-second part of the clock; M3 sets the clock 1 ms early; M4 mints 1 ms late.
- **Real binary.** `conformance --binary target/debug/ovis-terminal --scenario remote/isolation --isolated --case all` -> result passed, cases 2/2, assertions 111/111, exit 0.
- **Loaded no-regression check: 5/5.** `docs/evidence/task-F2a/loaded/loaded5-summary.log`: each run is the whole conformance crate (31/31 tests, including the all/all suite test) on cores 120-123 with 8 busy loops at nice 10. This is a no-regression check only, not proof the flake is gone: 240 loaded standalone runs of the **old** code never failed, because the window is microseconds wide. So a load loop cannot tell the old and new code apart; the widened-gap reproduction and the deterministic regression test are the evidence.
- **check.sh.** `bash with-slot.sh bash scripts/check.sh` -> exit 0 on the merged tree (with origin/integration 1abcced): fmt, clippy -D warnings, `cargo nextest run --workspace` **1490 tests run: 1490 passed** (2 skipped), doctests, hygiene, source-check verdict pass, evidence-audit (`docs/evidence/task-F2a/check-summary.log`). The first attempt hit the shared `/dev/shm` per-user quota (EDQUOT) while writing all/all evidence after **82/82 cases passed**. That was host contention: I wiped my own RAM target and reran.
- **Windows pre-check.** `RUSTFLAGS=-Dwarnings cargo check --workspace --all-targets --target x86_64-pc-windows-gnu --locked` exit 0; `cargo clippy --workspace --all-targets --target x86_64-pc-windows-gnu --locked -- -D warnings` exit 0 (`docs/evidence/task-F2a/windows-precheck.log`).

## Seams

- `conformance/src/fixture/machines/remote.rs`: the fix (`stamped()`), plus `internal()` now writes its cause to the fixture's stderr as one JSON line. Its hint promised stderr but delivered nothing, which hid this flake for three rounds. Same pattern as `conformance/src/lib.rs:64`.
- `conformance/src/fixture/machines/remote/tests.rs`: new child test module (keeps remote.rs under 250 pure LOC).
- `docs/evidence/task-F2a/`: evidence.
- Merge of origin/integration 1abcced (T70 soak), as instructed; `Cargo.lock` and the schema are unchanged by this lane.

## Risks

- `Uuid::new_v7(Timestamp::from_unix(NoContext, ..))` drops the per-process counter that `now_v7()` adds within one millisecond. Ids stay unique through 74 random bits, but two remote_child ids minted in the same millisecond are not ordered among themselves; nothing orders by them.
- `internal()` now requires `E: Debug`; all 16 call sites already satisfied that.

## DEFERRED VERIFICATION

None: the runner executes remote/isolation (exit 0 above).

## DoneClaim

DONECLAIM
