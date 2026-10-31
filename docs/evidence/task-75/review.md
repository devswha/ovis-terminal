# Direct implementation review

No delegation was used.

## Cause and scope

`Connections::serve` constructs the driver. The accept loop owns its task in a
`JoinSet`, records every joined result during normal serving and shutdown, and
joins aborted tasks after the grace period. `ServerHandle::shutdown` awaits that
accept-loop task. The missing entry therefore does not need another join loop.

The shared connection lifetime is the correct seam. A budget refusal can happen
in an escaped notifier while the driver and writer are already tearing down, so a
post-join overflow snapshot has no temporal meaning. The private `conn/cause.rs`
module now owns one set-once first-cause cell and final writer/result resolution.
Overflow publication, driver termination and writer failure compete to set it at
the point each event occurs. The final result selects backpressure only when
overflow won; otherwise the existing connection result remains authoritative.

The new listener field, poll wrapper, and shutdown notification are `cfg(test)`
only. Production builds have neither the gate nor a way to configure it.

## Test quality

The regression uses the real Unix endpoint, OS peer admission, credential
authentication, request dispatch, budget, writer, connection task, accept-loop
joining, and shutdown report. It substitutes no connection outcome.

The test handler pauses future polling of the driver before refusing an oversized
notification. The separately owned writer still sends `$/closing` and shuts down
its write side. The client reads the actual closing record and EOF. Shutdown runs
concurrently, and the test awaits the exact point where it has signalled all
connections. Shutdown remains unfinished until the driver gate is released.
The returned report must then count one accepted connection, one backpressure
failure, no panics, and no grace-period aborts.

This forced schedule failed before the product fix with `connection_io` instead
of backpressure. There are no sleeps, timeout-based success predicates, prose
assertions, retries, skipped tests, or weakened existing expectations. Timeouts
are failure bounds on actual events. The original two bound tests now print the
complete report on mismatch.

AdversarialVerify round 1 supplied the inverse ordering. The permanent regression
uses real authentication, request dispatch, notifier budgeting and writer output
over a duplex transport. It injects a read error, observes the peer-visible
`connection_io` closing, then overflows through the retained notifier while
writer shutdown is gated. It failed on e9edf37 because the final snapshot returned
backpressure; it passes with the set-once cause cell and returns the original I/O
error. Together the two tests distinguish both temporal orders.

## Programming and remove-ai-slops checks

- Responsibility: budget owns byte admission; `cause.rs` owns ordered cause and
  result resolution; driver/writer record terminal events; server owns reporting.
- `OnceLock` is the standard-library set-once primitive. It replaces an incorrect
  snapshot comparison without a speculative abstraction, dependency, public API,
  parser or normalization layer.
- No lock or watch guard crosses an await.
- Existing typed errors and overflow values are reused. No identity or wire type
  is added; no new production unwrap, expect, panic, numeric cast, unsafe block,
  blanket suppression, or placeholder is introduced.
- The regressions test real connection behavior, not locally assigned values or
  source text. Test gates control scheduling and transport completion without
  mocking a failure, budget outcome or report.
- No unrelated cleanup, dead code, speculative fallback, or compatibility shim.
- The driver is in the 200-250 pure-line warning band; no further growth is
  needed. Repository hygiene is the authoritative size and parameter check.
- Rust and shell LSP executables are unavailable on this host. Compilation,
  Clippy, rustfmt, shell syntax checking, and the repository gate provide the
  executable diagnostics; no LSP success is claimed.

## Seams and assumptions

The first terminal cause is authoritative. Overflow remains the report outcome
only when its set wins before driver/writer termination; an earlier I/O failure
cannot be rewritten by a retained notifier during teardown. The existing
five-second shutdown grace and classification of forcibly aborted tasks are
unchanged.

Normal EOF is deliberately excluded from this failure arbitration. It starts
`drain_after_eof`, where outstanding requests may still produce replies,
notifications or overflow. The half-close regression uses a real duplex EOF and
real retained notifier to prove overflow remains visible during that drain. No
sleep, fake error, mocked budget result or source-text assertion is involved.

The task's supplied GAP-026 / IDEAL-053 regression labels are retained for
traceability; this change proves shutdown failure accounting, not the unrelated
output-wait semantics described by the full IDEAL-053 ledger entry.
