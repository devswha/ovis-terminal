# Direct implementation review

No delegation was used.

## Cause and scope

`Connections::serve` constructs the driver. The accept loop owns its task in a
`JoinSet`, records every joined result during normal serving and shutdown, and
joins aborted tasks after the grace period. `ServerHandle::shutdown` awaits that
accept-loop task. The missing entry therefore does not need another join loop.

The shared connection driver is the correct seam: its writer and request tasks
have ended before the final return. The first refused budget reservation is
retained in a watch channel. Reading that value at this boundary preserves the
overflow outcome even when shutdown, peer EOF, or a closed writer queue was
observed first. Non-overflow outcomes still use the original result combination.

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

## Programming and remove-ai-slops checks

- Responsibility: driver owns connection teardown; server owns connection
  joining/reporting; the new test module owns the forced shutdown schedule.
- The production change is a final outcome selection, with no new helper,
  abstraction, dependency, public API, parser, or normalization layer.
- The watch borrow is copied out immediately; no guard crosses an await.
- Existing typed errors and overflow values are reused. No identity or wire type
  is added; no new production unwrap, expect, panic, numeric cast, unsafe block,
  blanket suppression, or placeholder is introduced.
- The regression tests behavior, not a locally assigned value or source text.
  Gate state is only a scheduling seam, not a mocked failure or mocked report.
- No unrelated cleanup, dead code, speculative fallback, or compatibility shim.
- The driver is in the 200-250 pure-line warning band; no further growth is
  needed. Repository hygiene is the authoritative size and parameter check.
- Rust and shell LSP executables are unavailable on this host. Compilation,
  Clippy, rustfmt, shell syntax checking, and the repository gate provide the
  executable diagnostics; no LSP success is claimed.

## Seams and assumptions

The first published overflow remains authoritative once teardown has joined the
request and writer tasks. This follows the existing first-refusal budget contract
and preserves the close reason the peer already observed. The existing five-second
shutdown grace and classification of forcibly aborted tasks are unchanged.

The task's supplied GAP-026 / IDEAL-053 regression labels are retained for
traceability; this change proves shutdown failure accounting, not the unrelated
output-wait semantics described by the full IDEAL-053 ledger entry.
