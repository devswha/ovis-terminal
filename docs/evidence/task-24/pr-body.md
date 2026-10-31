## Before

RT-HD-07, GAP-042, GAP-044, GAP-045, GAP-056, GAP-057 and GAP-133: `/tmp/ulw-ovis-yCJzPH/evidence/smoke/herdr-h08-attach.log:6`, with durable copy `docs/evidence/baseline/smoke/herdr-h08-attach.log:6`:

```text
attach#1 after takeover: exited=true exitCode=1 tail="... herdr: server shut down: terminal attach taken over\r\n"
```

herdr's `--takeover` needs no separate permission and no epoch. It disconnects the first attach, records no revoked/acquired event and reports the handoff as terminal text. herdr-core sizes the PTY from the most recently active typing viewer (`viewers.rs:254`, `max_by_key(|v| v.active_at)`).

## After

`crates/ovis-terminal-core/src/leases/` adds per-terminal lease ownership: one interact lease, one geometry lease and any number of observe leases. Each lease has its own `LeaseId` and each control slot its own `LeaseGeneration` epoch. Every operation is authorized first through the landed `Authority::authorize` for its own action: `Subscribe` for observe, `Input` for interact, `Resize` for geometry, and `Takeover` plus the slot action for a takeover. Write and resize admissions return an `Authorization` that `recheck` validates again at the write boundary via `Authority::recheck` plus a lease fence. Lease transitions are `LeaseEvent`s (`lease.acquired`, `lease.geometry`, `lease.revoked`) that `Transition::into_commit` turns into one `Commit` for the landed `Coordinator`, so a takeover's revoked and acquired events share one SQLite transaction and are published after COMMIT.

The published rules are in the `leases` module doc:

- Observers never change the input owner, the geometry owner or the grid.
- Acquiring a held slot returns `lease_conflict` naming the holder; nothing is evicted.
- A takeover needs the takeover grant plus the slot's current epoch. It commits `lease.revoked` (naming the new owner) before the grant (naming the old owner).
- Release and disconnect revoke immediately. Geometry then passes to the interact-capable owner whose geometry request is oldest; repeated requests keep the first position, and activity or focus never reorder the queue.
- While any lease exists, writes need the current interact lease held by the same principal and client. A lease id or terminal address alone grants nothing, and lease-free writes still need the `Input`/`RawInput` grant.

DEFERRED VERIFICATION (runner lands in Todo 21)

Executed on Linux x86_64, rustc 1.99.1 (58cf8f9c3 2026-10-13), `ATTEMPT_DIR=/home/devswha/workspace/ovis-terminal-wt/lane-t24/.omo/evidence/ovis-terminal-v1/task-24`:

```text
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-24 --isolated --case happy --evidence "$ATTEMPT_DIR/task-24-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
$ cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario part-a/task-24 --isolated --case failure --evidence "$ATTEMPT_DIR/task-24-ovis-terminal-v1.json"
conformance: no scenarios are registered in this build (protocol crate ovis-terminal-protocol)
CONFORMANCE_EXIT=2
```

No CLI or socket request was dispatched and no runner evidence JSON was produced (`conformance.log`). `conformance/scenarios/part-a/task-24.json` records the happy and failure steps for the runner. The `attach.*`/`input.*` RPC routes that call this module land with todos 23, 25 and 26, so no `ovis-terminal` CLI/socket transcript exists yet.

The component acceptance cases ran against the real authority and the real SQLite commit path:

```text
$ cargo test -p ovis-terminal-core --test leases happy -- --exact --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out
HAPPY_EXIT=0
$ cargo test -p ovis-terminal-core --test leases failure -- --exact --nocapture
test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 13 filtered out
FAILURE_EXIT=0
```

`happy` printed the two events it received from a real `Coordinator` subscription after `commit` (`qa-happy.log`). These are component output, not a CLI transcript:

```json
[
  {
    "schema_version": 1,
    "machine_id": "machine:00000000-0000-0000-0000-000000000018",
    "runtime_id": "runtime:1",
    "seq": "6",
    "boot_id": "boot:1",
    "type": "lease.revoked",
    "affected": [
      {
        "kind": "terminal",
        "id": "terminal:7"
      }
    ],
    "origin": null,
    "data": {
      "revoked": {
        "lease": {
          "lease_id": "lease:1",
          "slot": "interact",
          "epoch": "1",
          "owner": {
            "principal": "operator",
            "client": "operator"
          }
        },
        "reason": "taken_over",
        "displaced_by": {
          "lease_id": "lease:5",
          "slot": "interact",
          "epoch": "2",
          "owner": {
            "principal": "bridge",
            "client": "bridge"
          }
        }
      }
    }
  },
  {
    "schema_version": 1,
    "machine_id": "machine:00000000-0000-0000-0000-000000000018",
    "runtime_id": "runtime:1",
    "seq": "7",
    "boot_id": "boot:1",
    "type": "lease.acquired",
    "affected": [
      {
        "kind": "terminal",
        "id": "terminal:7"
      }
    ],
    "origin": null,
    "data": {
      "acquired": {
        "lease": {
          "lease_id": "lease:5",
          "slot": "interact",
          "epoch": "2",
          "owner": {
            "principal": "bridge",
            "client": "bridge"
          }
        },
        "displaced": {
          "lease_id": "lease:1",
          "slot": "interact",
          "epoch": "1",
          "owner": {
            "principal": "operator",
            "client": "operator"
          }
        }
      }
    }
  }
]
```

## QA

- `happy` (tests/leases.rs). The operator holds interact and geometry under lease ids reserved durably through `IdentityReservation`. After two observe-only principals attach, it asserts that `view().interact` and `view().geometry` equal their values before the attach, that `observers` lists exactly the two observe leases, and that each observe transition is one `Acquired` event for that lease. The bridge, holding the takeover grant, then takes over at the operator's epoch. The test asserts that the subscription receives `lease.revoked` and then `lease.acquired` at consecutive `seq` values, the second equal to the commit receipt. The revoked event names the operator lease and is `displaced_by` the bridge lease; the acquired event names the bridge lease and `displaced` the operator lease. The new epoch equals the old epoch plus one, owned by the bridge. Geometry is unchanged, and the operator's previously admitted write `Authorization` fails `recheck` with `lease_revoked`.
- `failure` (tests/leases.rs). After the bridge's takeover, the test asserts:
  - a takeover at the old epoch by another takeover-capable principal returns `lease_epoch_mismatch` with `expected` = the old epoch and `current` = the bridge lease;
  - a competing interact acquisition returns `lease_conflict` whose `holder` is the bridge lease;
  - observe-only acquisition of interact and of geometry each return `forbidden`;
  - a takeover by an interact principal without the takeover grant returns `forbidden`;
  - lease-free writes and writes presenting the bridge's lease id from another principal return `no_lease`;
  - `view()` is unchanged.
- QA modules under `src/leases/qa/` (`ownership`, `geometry`, `wire`, `model`), 12 more tests: idempotent re-acquire and retried takeover (`self_lease`, view unchanged); disconnect revoking interact, geometry and observe in that order; release by a non-owner (`no_lease`); lease-free writes fenced once any lease exists; `observe_forbidden_op`/`not_geometry_owner`/`no_lease` for resize; geometry hand-on in acquisition order; independent interact and geometry epochs; observers never queued for geometry; wire epochs; and a 64-case state-machine proptest.

Gate: `bash scripts/check.sh` exit 0 (`check.log`: 448 tests run, 448 passed, 1 skipped; hygiene 0 violations; 42 JSON files checked). Acceptance commands: `cargo nextest run -p ovis-terminal-core` and `cargo clippy -p ovis-terminal-core --all-targets -- -D warnings` (`core-nextest.log`: 107 tests run, 107 passed, `NEXTEST_EXIT=0`; `core-clippy.log`: `CLIPPY_EXIT=0`). TDD red run before the module existed: `tdd-red.log` (unresolved `leases::*` imports, exit 101).

macOS: the `probe-stress-macos` workflow gains a `leases` input that repeats `cargo nextest run -p ovis-terminal-core --test leases` 100 times on macos-latest. Run: STRESS_RUN_URL

## Adversarial

- Stale state, old epoch: `failure` asserts `lease_epoch_mismatch` with no state change. The proptest's ledger asserts that epochs strictly increase per control slot and that no lease id is granted twice across random interleavings.
- Cancel/resume, disconnect release: `disconnect_revokes_every_lease_of_that_owner_in_slot_order` asserts the event order and that the freed slot is acquirable. `geometry_passes_in_acquisition_order_never_to_the_most_active_client` asserts that disconnect hands geometry to the next requester under the id that requester claimed.
- Repeated interruptions, idempotent takeover: `repeated_acquire_and_takeover_change_nothing_the_second_time` asserts that a retried identical takeover returns `self_lease` and leaves the view (epoch and owner) unchanged.
- Misleading success, both owners recorded: `happy` asserts that the revoked and acquired events each name both leases, read back from the durable subscription rather than the in-memory transition.
- Address possession: `failure` and `release_needs_the_owning_principal_not_the_lease_address` assert that another principal presenting a valid lease id gets `no_lease`.
- Mutation probes (`mutations.log`): each guard was removed in turn and the named test failed.
  - Epoch check: `failure` failed.
  - Takeover grant: `failure` failed.
  - Queue order (FIFO changed to latest): the geometry order test failed.
  - Write-boundary fence: `happy` failed.
  - Owner check on writes: `failure` and the proptest failed.
  - Conflict on competing acquire: `failure` and the geometry order test failed.

  Sources were restored after each probe and a minimized proptest regression produced by a mutant was discarded.

## Contract checklist

1. Counters on the wire are decimal strings. Epochs serialize as `LeaseGeneration` strings and lease ids as `lease:<n>`. `takeover_epochs_are_canonical_nonzero_decimal_strings` asserts that numeric and zero-padded `lease_epoch` are rejected by the protocol `TakeoverRequest`, `"0"` returns `invalid_params`, and `u64::MAX` reaches `Takeover::expected()` exactly. `lease_events_round_trip_the_maximal_epoch_and_reject_numeric_counters` asserts that an event JSON with epoch `"18446744073709551615"` round-trips byte-equal and that a numeric epoch fails to decode. A 256-case proptest round-trips arbitrary nonzero epochs and ids.
2. `LeaseError` is one `thiserror` enum with `kind()`, which maps to the protocol `DomainErrorKind` tokens and delegates to `AuthzError::kind()` for authority refusals, and `hint()`. `#[source]` wraps `AuthzError` and `IdentityError`, and `Display` never repeats a kind token.
3. Identities reuse the registry `LeaseId`/`LeaseGeneration`/`TerminalId`, authz `PrincipalId` and protocol `ClientId`. `Option` is used only for real absence (no holder, no displaced lease, lease-free write).
4. Tests assert state transitions, durable event order and typed kinds; proptest covers the wire round trip and the state machine.
5. No `#[allow]` or `#[expect]`, no placeholders. Every file is at most 250 pure LOC and every function has at most 3 parameters (hygiene 0 violations).
6. The conformance attempt is recorded verbatim above as deferred to Todo 21. Holder wire generation stays 2; contract checklist items are unchanged.

## Seams

- New public API in `ovis_terminal_core::leases`: `TerminalLeases`, `Caller`, `Acquire`, `Takeover`, `Write`, `WriteKind`, `Authorization`, `Lease`, `LeaseSlot`, `ControlSlot`, `Owner`, `LeaseEvent`, `RevokeReason`, `Transition`, `Granted`, `LeaseView`, `LeaseError`. No landed crate's public API changed.
- The protocol `LeaseRecord`/`LeaseEventData` have no public constructor outside the protocol crate, so core emits its own `Lease`. It has the same `lease_id`/`epoch` decimal-string encoding plus `slot` and `owner{principal, client}`. The attach layer (todo 23) maps it to the RPC records.
- Callers supply each lease identity (`fresh`), reserved durably via `IdentityReservation::reserve::<LeaseId>` as `happy` does, so ids are never reused across restarts.
- `.github/workflows/probe-stress-macos.yml`: new `leases` input and job; the existing `platform` job now runs on `inputs.probe == 'platform'`, the default, so its behavior is unchanged.

## Risks

- Lease state is in memory per terminal, and its events are durable only when the caller commits the `Transition`. A control restart therefore starts with no leases; suspension within a resume window across planned restart is left to todo 48. Disconnect revokes immediately, the "revoke" branch of the todo's "suspends or revokes".
- `TakeoverRequest.reason` is validated but not copied into lease events; events carry the typed `RevokeReason::TakenOver`.
- Wiring into `attach.*`/`input.*` dispatch, actor serialization and the queued-write recheck belong to todos 23, 25 and 26. Until then there is no real-binary evidence for this todo.
