# Task 75 investigation

Base: e14f424, Linux x86_64. Worktree: lane-t75.

## Hypotheses

1. Shutdown snapshots before joining. Distinguishing evidence: a gated accepted
   connection remains unfinished when the shutdown future returns.
2. Shutdown wins over an already-published overflow in the driver's biased select.
   Distinguishing evidence: the writer sends backpressure and EOF, but the joined
   driver reports success or an I/O error when shutdown becomes ready first.
3. The writer independently closes for overflow without preserving that outcome.
   Distinguishing evidence: writer overflow returns success, leaving the driver as
   the only source of failure accounting.

Inspection: the accept loop already drains its JoinSet before constructing the
final report, including joining aborted tasks after the grace period. The driver
prioritizes shutdown over overflow. The writer returns Ok on overflow. A forced
schedule will distinguish missing joins from a joined task with a lost outcome.

## Confirmed cause

The pre-fix forced schedule produced:

    FORCED_RACE_REPORT=ServeReport { accepted: 1, failures: {"connection_io": 1}, panicked: 0, aborted: 0 }
    left: None
    right: Some(1)

The connection was joined, so hypothesis 1 is refuted. Hypotheses 2 and 3 combine:
the writer independently observes overflow, sends backpressure and EOF, and returns
success. The driver is deliberately gated until shutdown is signalled. Its biased
select chooses shutdown over the pending overflow, then attempts to send closing
into the writer's closed queue. That I/O failure replaces backpressure. Without a
closed-queue error the same schedule can instead report success. The report
correctly counts the joined outcome, but that outcome has already lost its cause.

The round-1 product change read retained overflow state after request teardown and
writer join. That repaired overflow-before-shutdown but lacked temporal precedence:
an escaped notifier could publish overflow after an earlier I/O failure and rewrite
the final outcome.

## AdversarialVerify round 1 correction

The inverse-order test first makes the driver observe a real transport read error,
then reads the actual `$/closing{reason:"connection_io"}` while writer shutdown is
gated. Only afterward does the retained notifier overflow. On e9edf37 the final
error incorrectly became backpressure (`round-2-pre-fix.log`, exit 100).

`conn/cause.rs` now owns one `OnceLock` first-cause cell and final writer/result
resolution. Budget refusal records overflow before publishing its watch signal;
the driver records its chosen terminal branch; and writer encode, write or shutdown
failures record where they occur. The first successful set wins. Later teardown
causes cannot rewrite it. The original overflow-first test and the inverse I/O-first
test both pass, including 50 consecutive combined runs under eight CPU workers.
The existing accept-loop join/drain invariant is unchanged. There is no wire or
public API change.

## Verification correction

The first repository gate failed Clippy's `significant_drop_in_scrutinee` and
`option_if_let_else` rules. The outcome now copies the watch value out before
selecting with `map_or_else`; no watch guard survives that expression. No lint was
suppressed. The in-progress stress attempt was stopped and retained as
`stress-superseded.log`; the required hundred-run proof restarts on the final code.

## Artifact ledger

- Required Rust/linker environment is confined to command shells.
- Load workers will be owned by the stress runner and killed/joined in its trap.
- Test-only connection polling gate will exist only in the unit-test build.
- Pre-fix, regression, stress and repository gate logs are retained as evidence.
- No debugger, container, external service, or unrelated file edits are needed.
