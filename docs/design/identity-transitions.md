# Identity resolution and lifecycle transitions

This contract closes GAP-009 and GAP-010 for IDEAL-009, IDEAL-045 and
IDEAL-080. Resource identity is not a display name, caller focus, workspace
selection or layout path. A client binding names a machine, runtime lifetime
and opaque resource ID; a resource move or rename cannot redirect that binding.

## Typed identities

`ovis-terminal-core::registry` defines different Rust types for machine,
runtime lifetime, server boot, host boot, workspace, tab, pane, split node,
terminal, process, stream and lease IDs. Their family prefixes are part of
their canonical external string (`terminal:17`, for example); the stable
machine ID carries a UUIDv7 so independently installed hosts cannot share a
machine identity. Their payloads cannot be mixed at compile time. A wrong
family is rejected during parsing rather than interpreted as another ID.
Machine/runtime scope and every identity-bearing generation are also typed.

| Resource | ID type | Generation type | Identity changes when |
| --- | --- | --- | --- |
| Machine | `MachineId` (UUIDv7) | `MachineGeneration` | A machine installation is replaced |
| Runtime lifetime | `RuntimeId` | `RuntimeGeneration` | A new runtime lifetime starts |
| Host OS boot | `HostBootId` | `HostBootGeneration` | Independent OS boot evidence changes |
| Control server | `ServerBootId` | `ServerBootGeneration` | A core process starts |
| Workspace | `WorkspaceId` | `WorkspaceGeneration` | The workspace resource is recreated |
| Tab | `TabId` | `TabGeneration` | The tab resource is recreated |
| Pane | `PaneId` | `PaneGeneration` | The pane resource is recreated |
| Split node | `SplitId` | `SplitGeneration` | The split node is recreated, never on path change |
| Terminal | `TerminalId` | `TerminalGeneration` | A terminal process incarnation is relaunched |
| Process | `ProcessId` | `ProcessGeneration` | A spawned process incarnation is replaced |
| Stream | `StreamId` | `StreamGeneration` | A new attach stream is opened |
| Lease | `LeaseId` | `LeaseGeneration` | A new lease is issued |

Every `u64`-backed ID, generation, PID and start-related counter is encoded as
a canonical decimal string in JSON. IDs are family-prefixed strings. Numeric
JSON values, zero, leading zeroes and overflow are rejected. `ProcessIdentity`
contains the runtime `ProcessId`, `ProcessGeneration`, OS `ProcessPid` and
opaque `ProcessStartToken`; PID equality alone is never process continuity
evidence.

Display names are separate validated values (1–64 ASCII letters, digits,
periods, underscores or hyphens). No identity is derived from a name or path.
`ResourceTarget` contains stable machine/runtime/terminal IDs plus the mutable
name; a layout move changes neither the target nor the identity. Selection and
focus are absent from the registry lookup API.

## Target resolution

Each target-sensitive operation resolves exactly one `TargetRequest` before
dispatch. The request is one of:

* `Id(ResourceTargetKey)`, where the terminal ID is scoped by machine and
  runtime lifetime;
* `Name { machine: Some(MachineId), name }` for a scoped name;
* `Name { machine: None, name }` only when the caller deliberately permits
  cross-machine lookup;
* `CallerBinding { target, expected_scope }` only when the caller explicitly
  asks to reuse its prior binding; or
* `Missing`, which returns `needs_target`.

A unique name resolves to its one candidate. A repeated name returns
`ambiguous_target` with every candidate's machine, runtime, terminal ID and
name, and dispatch does not run. An unknown ID/name returns `not_found`.
A caller binding from another runtime returns `cross_lifetime`; a binding no
longer present returns `stale_target`. None of these paths consults a focused
pane or silently substitutes a current resource.

Process-sensitive operations resolve the exact current `ProcessIdentity`.
If the supplied PID/start-token pair is old, resolution returns
`stale_target` with no replacement process as the result. Reuse of the same
PID with a different start token is a different process incarnation.

## Durable allocation

`IdentityReservation::load_or_reserve_machine` reads the installed machine
identity or durably reserves one UUIDv7 together with its reservation, receipt
and event before returning it. Reopening the same store returns the same
machine ID. `IdentityReservation::reserve<T>` reads the family allocator counter from a
store snapshot, then atomically commits (1) the incremented counter, (2) a
reservation row keyed by the exact family-prefixed ID, (3) an operation
receipt and (4) an `identity.reserved` event in one store mutation. Its
compare-and-swap version prevents concurrent allocators from publishing the
same value. The API returns the ID only after the writer confirms commit;
therefore no caller can publish an unreserved ID. On restart the next
allocation is derived from the durable counter, not process memory. A CAS
conflict or storage failure returns a typed error and no ID. Counter overflow
is an error and never wraps.

## Lifecycle transition table

The table describes each terminal's identity in the existing runtime. “Same”
means the exact value, including the process start token. A core restart or
upgrade requires holder evidence matching the stored `ProcessIdentity` before
the surviving process is retained. A host reboot requires an independently
observed new host boot identity; missing holder evidence by itself is not
reboot proof.

| Event | `RuntimeId` | `ServerBootId` | Terminal ID/generation | Process ID/generation/PID/token | Terminal state and rule |
| --- | --- | --- | --- | --- | --- |
| First holder-confirmed spawn | Same | Same | Same / same | New process ID at generation 1 with observed PID/start token | `Running` after spawn confirmation |
| Core crash and restart; holder confirms survival | Same | Changes | Same / same | Same | Running only when the holder confirms the exact stored identity |
| Planned core restart | Same | Changes | Same / same | Same | Running after adoption proof; interrupted RPC outcomes are separate |
| Upgrade with old holder draining | Same | Changes | Same / same | Same for surviving processes | Old holder remains owner until process exit; replacement core adopts exact evidence |
| Holder loss | Same | Same unless the core also restarts | Same / same | Prior identity retained only as historical evidence; no current process | `Lost`; no “survived” or automatic replacement claim |
| Confirmed relaunch after holder loss | Same | Current boot | Same / increments once | New process ID and generation; PID may be reused only with a new start token | `Running` only after holder spawn confirmation |
| Independent host reboot | New | New | Same / same while metadata is restored | No current process; old identity is historical only | `RestoredMetadata`; never reports a pre-reboot process as surviving |
| Confirmed relaunch after host reboot | New | New | Same / increments once | New process ID and generation with observed PID/start token | `Running` only after spawn evidence; otherwise metadata remains restored |

Illegal transitions (same boot ID on core restart, unchanged runtime/host boot
on reboot, relaunch while already running, or a replacement generation that
does not advance exactly once) return typed failures. Generation increments
use checked arithmetic, so exhaustion is `counter_overflow`. Restored metadata
does not itself authorize a process relaunch.

## Boundary errors

Registry errors are `thiserror` enums with stable `kind()` and recovery
`hint()` values. Relevant kinds include `invalid_identity`, `invalid_name`,
`needs_target`, `ambiguous_target`, `not_found`, `stale_target`,
`cross_lifetime`, `counter_overflow`, `identity_store_invalid` and
`persistence_failed`. Inner store and serialization failures remain available
as error sources. Display strings carry explanations, not duplicated machine
tokens.

## Conformance coverage

`conformance/scenarios/part-a/task-9.json` runs assertion-bearing happy and
failure registry tests. Those tests cover durable ID reservation across store
reopen, rename/move stability, cross-machine duplicate-name candidates,
explicit missing-target rejection, wrong ID family, malformed/zero identity,
full-width decimal-string round trips, core restart, upgrade, holder loss,
confirmed relaunch, host reboot metadata restoration and stale PID/start-token
rejection.

The real-binary `part-a/task-9` socket scenario is deferred until the runner
and assembled CLI land (plan verification strategy, line 275). The component
scenario does not represent that deferred transcript as executed evidence.
