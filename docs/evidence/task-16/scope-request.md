# Authorized atomic lifecycle integration scope

The orchestrator granted the additive dependency seam requested below during this
execution. The grant requires one durable SQLite transaction for the operation
outcome, terminal lifecycle state, identity reservations, and lifecycle events.
It explicitly preserves the commit-before-publish barrier, revision domain,
SnapshotLease/Assembler path, and OpRecord contract, and forbids exposing the raw
store, opening a second writer path, or weakening existing tests.

The integration needs two narrow capabilities:

1. Commit terminal state, the operation outcome, and lifecycle events under one
   coordinator transaction. `src/ops/transitions.rs:51` exposes only
   `Operations::commit(permit, result)`. Its private `update` at
   `src/ops/context.rs:197` builds a write for the operation row alone. A caller
   can separately commit terminal writes through `Coordinator::commit`, but that
   creates a crash-visible split between the terminal state and the operation
   outcome. It does not satisfy the plan's atomicity contract.
2. Reserve identities through the same writer and commit boundary.
   `src/registry/reservation.rs:159` exposes `IdentityReservation::reserve` over a
   borrowed `Store`. `src/events/coordinator.rs:75` takes ownership of that store,
   and its state is private to the events module. Reimplementing the allocator in
   terminals, opening a second writer, or preallocating arbitrary identity pools
   would evade rather than reuse the landed seam.

The granted seam is implemented in these modules and their tests:

- `crates/ovis-terminal-core/src/ops/`
- `crates/ovis-terminal-core/src/events/`
- `crates/ovis-terminal-core/src/registry/reservation.rs`
- `crates/ovis-terminal-core/src/registry/reservation/`

The atomic tests use the real SQLite writer. One child process aborts after
COMMIT but before the writer acknowledges it to the coordinator; reopening
recovers the OpRecord, terminal state, reserved identity, and ordered events.
A failing terminal CAS rolls back the entire unit, including the identity
counter and both events. Existing todo-10/15 tests retain their original meaning.

The holder decision is not blocked: the existing generation-two envelope permits
an optional field while its payload structs reject extra fields. The implemented
Hangup extension uses that envelope field, keeps the Output payload and tag
unchanged, adds zero terminal bytes, and leaves Exit as the final lifecycle
message. The real waiter uses `waitid(WNOWAIT)` so the child is still unreaped
when the pump publishes Hangup. A real-child test proves the EOF/reap boundary.
