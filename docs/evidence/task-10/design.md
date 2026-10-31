# Todo 10: commit and snapshot boundary

The coordinator owns the store after identity/bootstrap reservation. All domain
mutations must enter through `Coordinator::commit`; assembly must not keep an
independent store writer or read authoritative snapshot sections from live actors.
The synchronous API runs on bounded blocking workers, not Tokio executor threads.
The existing store writer remains the only sequence allocator.

The coordinator mutex covers store submission, waiting for the durable receipt,
publication, snapshot capture, and subscription installation. An event is stored
as the sequence/type columns plus a typed body in the same SQLite row, in the same
transaction as CAS writes and the operation receipt. Live delivery combines the
committed row sequence with that body; replay uses exactly the same conversion.
There is no preallocated in-memory sequence. Publication has no fallible
serialization after COMMIT: queue admission failure marks delivery `missed`,
without converting a committed mutation into a failed mutation.

Each subscription has an independent identity, filter and bounded queue. Snapshot
startup captures one pinned WAL view at R while excluding commits, then installs
delivery before releasing the boundary. Resume either queues retained events
strictly after the cursor with no duplicate snapshot, or returns a typed
`lifetime_mismatch` / `cursor_expired` and a fresh snapshot. A cursor ahead of the
committed head is rejected. A replay exceeding queue admission bounds is rejected
as `limit_exceeded`; the caller can request a fresh snapshot.

`SnapshotProducer` captures owned typed sections from the supplied WAL view.
`SnapshotSections` composes layout, terminal, agent and service records. Missing
producers are explicitly `unavailable`; this foundation does not register any
empty successful domain producer. The test producer reads actual SQLite state.
Domain producers and wire dispatch are owned by later assembly todos.

Paged startup installs delivery and a snapshot lease under the same mutex.
Immutable serialized bytes and a WAL read transaction remain pinned until release
or core shutdown. Every page carries the same cursor and exact total, plus a typed
byte continuation. Released tokens and tokens from another core return
`stale_snapshot`. A concurrent commit cannot change already-pinned bytes.

Registry machine/runtime/boot/resource identities are reused. Local sequences use
the protocol's decimal-string `EventSeq`; remote origin sequences have a distinct
`RemoteSeq` wrapper and retain their original machine and lifetime. All emitted
64-bit counts and offsets use existing canonical-string scalars.

## Seams

No landed crate public API is changed. The new coordinator is a core component,
not a second transport API: Todo 21 must adapt it to the protocol's RPC envelopes
and install real domain producers. Bootstrap identity allocator events precede
coordinator ownership; retained domain replay must use the producer's complete
typed event enum. Unknown or malformed durable event bodies fail with a sourced
typed error rather than being skipped.

## Deferred acceptance

The real-binary conformance runner is still the Todo 21 scaffold and exits 2.
The component happy/failure commands are independently executable now; they do
not establish CLI/socket behavior, macOS/Windows behavior, or full domain closure.
