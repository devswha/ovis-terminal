# Task 58 baseline (Before)

Before: `absent`. No comparable baseline capability exists and no failing test was executed
to produce one. The gap rows that state the missing capability:

- docs/design/gap-list.md:154 GAP-129 connections have no authenticated principal or resource scope
- docs/design/gap-list.md:155 GAP-130 observe-only is enforced by Ovis, not by the runtime
- docs/design/gap-list.md:156 GAP-131 authority is not rechecked immediately before each mutation
- docs/design/gap-list.md:157 GAP-132 agent report authority is not bound to a registered process
- docs/design/gap-list.md:158 GAP-133 takeover and prompt answers require only terminal possession
- docs/design/gap-list.md:159 GAP-134 remote request validation against typed schemas and caller scope is not guaranteed
- docs/design/gap-list.md:160 GAP-135 diagnostics have no redaction and retention policy
- docs/design/gap-list.md:161 GAP-136 plugins run with full user authority

On integration da5b456, before this lane:

- `agent.history` was published (`schema/ovis-terminal.openrpc.json`) but no route served it,
  and `AgentHistoryParams` had no store field, so `{"store":"../../secret"}` could only be
  rejected as `invalid_params`, never decided as a path outside an authorized root.
- `credential.rotate` committed its result, including the new bearer text, into the SQLite
  operation ledger (`crates/ovis-terminal-server/src/dispatch/lanes/credential_rotation.rs`,
  `completion`), so `op.get` and an op-id retry returned the bearer again.
- `terminal.run` handed caller `env` to the launcher unchecked, so a caller could set
  `OVIS_TERMINAL_PROCESS_TOKEN` (`crates/ovis-terminal-server/src/dispatch/lanes/template_launch/run.rs`, `resolve`).
- `crates/ovis-terminal-core/src/telemetry/serialize.rs` bounded free text to 256 characters
  with no credential scrubbing; no audit trail or retention policy existed.
