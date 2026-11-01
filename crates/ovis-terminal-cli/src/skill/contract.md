## How to read this reference

The tables from `## Methods` on are rendered from the Rust types of this exact build: the
OpenRPC document, the authorization roles and the command-line grammar. The sections
before them state behaviour that types cannot express. They come from the runtime API
advisory with the binding decisions of the work plan applied; `## Advisory overrides
applied` lists every place where the plan replaced the advisory.

## Transport and framing

- A connection is persistent and multiplexed: requests are matched by string `id`, many
  may be in flight, and the server never closes after a reply. A server-initiated close
  sends `$/closing{reason}` first when the socket is writable.
- Plain mode is UTF-8 JSON-RPC 2.0, one message per `\n`-terminated line, no batches.
  Fragmentation never defines a message.
- After a successful `hello` a connection can upgrade to framed mode at an explicit parser
  boundary. Every frame has a nine-byte header `[channel:u32 LE][kind:u8][len:u32 LE]`.
  Channel 0 carries JSON-RPC; DATA on a nonzero channel is downstream terminal data;
  CREDIT carries an eight-byte little-endian cumulative acknowledged delivery offset of
  that never-reused channel and stream; CLOSE is typed control. Oversize, truncated and
  unknown frames are typed errors, never ignored.
- Input, resize, focus and takeover are RPCs (`input.*`, `attach.resize`, `attach.focus`,
  `attach.takeover`), never anonymous writes on a stream.
- Every u64 counter, sequence, revision, offset and generation is a canonical decimal
  string on the wire. A JSON number in its place is `invalid_params`.
- Raw holder output offsets and filtered stream delivery offsets are separate domains.
  Attach control records bind both; CREDIT acknowledges only delivery offsets.

## Identity and targeting

- Identities are distinct types and never conflated: `machine_id`, `runtime_lifetime`,
  `server_boot_id`, `terminal_id`, terminal and process incarnations (a process
  incarnation carries `pid` and a platform start token), `stream_id`, `lease_id` and
  `lease_epoch`, `op_id`, `prompt_id`, `agent_id`.
- A `target` is exactly one of `{terminal_id}`, `{address}` (`t7`, `machine/t7`) or
  `{name}`. Params are closed objects: an unknown or misspelled field is `invalid_params`
  with its `field_path`, and nothing runs. There is no focus and no focus fallback.
- A name that resolves to several terminals is `ambiguous_target` with candidates; a
  command-line read without a target is `needs_target` with candidates, never a guess.

## Operations, idempotency and outcomes

- Every mutation takes a client-minted `op_id`. Resending the same `op_id` with the same
  params returns the retained result; different params are `op_reused`.
- State rows, the operation receipt and its events commit in one transaction, and the
  result is published only after the commit.
- `op.get` distinguishes accepted, running, committed, failed, cancelled and
  outcome-unknown, with supporting receipts or process evidence. A mutation whose
  connection was lost is never resent automatically; ask `op.get` first.
- Admission and later effects are separate facts: `input.send` returns an admission
  receipt with an output cursor and never waits for a status transition; `turn.wait`,
  `terminal.wait_output` and `events.wait` are separate, bounded calls. A cursor older
  than the retained floor answers `gap`, not `timeout`.
- `input.interrupt` reports delivery and observed process or agent state as separate
  fields; stopped is true only with exit evidence or a declared report that left
  `working`.

## Terminal lifecycle states

| from | trigger | to |
|---|---|---|
| none | `terminal.run` committed | `live` with spawn pending |
| `live` (pending) | holder reports pid and start token | `live` |
| `live` (pending) | spawn failure, not kept | `finished` (`spawn_failed`) |
| `live` | program exit or `terminal.kill` observed | `finished` (`exited`, `killed` or `signaled`) |
| `live` | loss while kept | `lost` |
| `lost` | grace elapsed or `terminal.recover` | `restarting` |
| `restarting` | program spawned | `live` with new incarnations |
| `lost` or `restarting` | `terminal.keep` false | `finished` |
| any live state | host reboot detected | `finished` (`host_shutdown`) as restored metadata |
| `finished` | `terminal.rm` or retention | removed, with a tombstone |

Waits end on `lost` as they do on `finished`. A planned control restart changes
`server_boot_id` but not `runtime_lifetime` while holders survive; a reboot starts a new
lifetime and reports restored metadata, never live processes.

## Leases, attach and input ordering

| state | event | next |
|---|---|---|
| no interact lease | `attach.open` interact | held by the opener, who owns geometry |
| any | `attach.open` observe | unchanged; the observer never writes |
| held by A | interact open by B without takeover | held by A; B is told who holds it |
| held by A | `attach.takeover` by B with the takeover grant and A's current epoch | held by B; A's stream closes as taken over |
| held by A | takeover at a stale epoch | held by A; `lease_epoch_mismatch` names the current lease |
| held by A | A closes or disconnects | no lease; geometry owner cleared |
| any | terminal `lost` or `finished` | no lease |

- The epoch advances on every interact transition. Only an explicit, authorized takeover
  displaces a holder.
- No human-controller bypass is implicit: whenever any lease exists, writes require the
  current interact lease, and lease-free agent writes require an explicitly granted
  input scope. DATA from an observer is rejected immediately.
- One terminal actor serializes admission. Input authority, target, process and lease
  identity and the mode revision are checked at admission and again at the actual write
  boundary; takeover or revocation fences entries not yet written and never claims to
  retract bytes already written.
- Terminal-query replies use a separate bounded internal lane and can never become user
  submissions.

## Agent status, prompts and turns

| current root | input | next root | turn |
|---|---|---|---|
| unreported | report with a higher `seq` | the reported state | unchanged |
| `working` | report `idle`, `done`, `error` or `blocked` | that state | ends, +1 |
| not `working` | report `working` | `working` | unchanged |
| any | report with `seq` not above the current one | unchanged; the report is refused with the current sequence | unchanged |
| any | `status.clear` by the owner | cleared | unchanged |
| `working` | process exit | cleared | ends as `exited` |

- Status has exactly two writers: OSC 7501 bytes from the owning program's PTY and
  `status.report` presented with that program's process token. An accepted report stands
  until replaced, cleared or invalidated by its declared lifetime; screen text, process
  names and brands are never status.
- A process token belongs to one process incarnation of one terminal. A report, prompt,
  record or registration that presents another terminal's token is refused and changes
  nothing.
- `agent.register` accepts any label and session reference. `agent.get` on a plain
  program returns `agent: null`; `not_found` is reserved for a missing terminal.
- Prompts are versioned records. `prompt.answer` names the prompt and its version and
  needs the prompt-answer grant; a second answer to the same version is a conflict.
- Provider-store adapters produce conversation records only and never lifecycle status.
  A first message is released only on declared readiness, never on output silence.

## Events, snapshots and replay

- Every event has one envelope with sequence, lifetime, boot, machine, type, target and
  typed data. Snapshots and events share one sequence domain per runtime lifetime.
- A subscription starts with a snapshot at a revision followed only by matching changes
  committed after it. Several subscriptions on one connection are independent; filter
  changes are acknowledged with an effective sequence.
- Reconnecting with a retained cursor replays later matching events; otherwise the
  answer is `missed` with a fresh snapshot. A cursor from another lifetime is never
  replayed.
- Snapshot pagination pins one revision across pages.

## Persistence and recovery

- One SQLite database in WAL mode holds terminals, receipts, events, status, prompts,
  agents and documents; each mutation is one transaction.
- Holders own PTYs and raw retained output; the control core owns VT checkpoints,
  layout, leases, operation state and domain events. One shared holder serves a session
  and build; a planned control restart keeps every program incarnation.
- At start the core recovers holders and checkpoints and reports recovery boundaries.
  Recovery evidence persists across reads: neither `server.status` nor `server.metrics`
  clears it. An unsupported or damaged database is preserved, not overwritten.

## Concurrency and bounds

- Each terminal has one serialized actor and bounded message channels; blocking PTY and
  SQLite work runs off the async executor; every task is cancellable and joined.
- Every mailbox, queue, retention window and allocation is bounded and advertised in
  `hello` (see `## Limits`). Limits are admission budgets, not preallocations; a full
  queue answers `backpressure`, never a silent drop.
- A slow consumer of one stream gets a gap and a fresh checkpoint instead of an unbounded
  backlog; other streams and channel 0 are unaffected.

## Edge cases

- Input to a `lost` or `restarting` terminal is refused as `recovering`; nothing is
  queued and nothing is replayed later.
- `attach.open` on a finished terminal delivers the final checkpoint and then closes.
- A frame larger than the frame limit closes the connection after `$/closing`; its
  streams close and its leases are revoked as on disconnect.
- PID reuse is detected by the process start token; a mismatched incarnation is refused.
- A UTF-8 code point split across PTY reads is decoded correctly on the screen and passed
  unchanged on streams.
- A malformed or oversized OSC 7501 sequence changes no status.
- Duplicate live names are refused; a name that looks like an address is invalid.
- A control-server crash during a send leaves the operation `outcome_unknown` until
  `op.get` shows its evidence; the client must not resend blindly.
- A program that never declares readiness keeps its first message held until the
  deadline, then the message expires; the screen is untouched.
- A document over the size limit is refused with its version unchanged.

## Excluded behaviours

- EXCL-001: the runtime draws no sidebar, onboarding or modal chrome; nothing presentational gates a headless operation.
- EXCL-002: no prefix key and no client mode; no byte is swallowed by an invisible mode.
- EXCL-003: every operation has a typed, discoverable equivalent; nothing depends on a pointer target.
- EXCL-004: an RPC never promotes a client to the foreground or takes geometry authority.
- EXCL-005 and EXCL-006: explicit targets only; a wrong or unknown field is an error, never an answer about another terminal.
- EXCL-007: only an explicit, authorized takeover at the current epoch displaces a controller.
- EXCL-008 and EXCL-009: no screen-tail status, no brand detection, no downloaded detection manifests.
- EXCL-010: a declared agent of any label is queryable; a plain program has `agent: null`.
- EXCL-011, EXCL-012 and EXCL-013: no toasts, sounds or announcement modals; clients decide how to notify from durable events.
- EXCL-014 and EXCL-015: persistent multiplexed connections; independent, acknowledged subscriptions.
- EXCL-016: attach refusals and handoffs are typed outcomes, never terminal text.
- EXCL-017: unknown input sequences travel as raw bytes or fail with a typed error; nothing is discarded silently.
- EXCL-018: destructive scope is visible in help, schema, validation and receipts.
- EXCL-019: reads never inject input or scroll the program they inspect.
- EXCL-020 and EXCL-021: every managed invocation has an operation identity, bounded output and an exit outcome; failures stay queryable.
- EXCL-022: split and pane identities are stable node ids, never tree paths.
- EXCL-023: no force-send; a blocked program is reached through a versioned prompt answer or explicit raw interaction.
- EXCL-024, EXCL-025 and EXCL-026: admission, delivery, output waits and interrupt evidence are separate facts.
- EXCL-027: read-only status, schema, manual and help never start, install or restart a server.

## Advisory overrides applied

- Stack: the advisory's Bun runtime with one holder process per terminal is replaced by a Rust runtime with one shared holder per session and build (plan, Must have).
- Resource model: the advisory's terminals-only model is overruled; a thin deterministic workspace, tab, pane and split layout service exists in the core (plan, Execution strategy; IDEAL-084 to IDEAL-088).
- Frame kinds: the advisory's RESIZE, FOCUS and FACTS frames are replaced by RPCs; framed connections carry DATA, CREDIT and CLOSE only (plan, Settled contracts).
- Credit: the advisory's 32-bit consumed-byte credit is replaced by an eight-byte cumulative acknowledged delivery offset bound to one channel and stream (plan, Settled contracts).
- Observer writes: the advisory's warn-then-close rule is replaced by immediate rejection of observer DATA (plan, Steering and complete-spec reconciliation).
- Input authority: the advisory's optional lease requirement is replaced by a mandatory one whenever any lease exists, with lease-free writes needing an explicit input scope; there is no raw upstream input and no implicit human-lease bypass (plan, Consistency).
- Mode changes: the advisory's single admission-time mode read is replaced by revalidation at the actual write boundary (plan, Consistency).
- Numbers: every u64 value travels as a canonical decimal string (plan, Settled contracts).
- Discovery: `ovis-terminal endpoint` describes the resolved endpoint without probing it; liveness is `server status`, which never starts anything (plan, IDEAL-014 and IDEAL-111).
