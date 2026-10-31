# ovis-terminal agent manual

`ovis-terminal` is the agent-facing client of the ovis-terminal runtime. It draws
nothing and never prompts. This manual ships inside the binary and describes exactly
this build; `ovis-terminal --skill --json` returns it with the version and protocol
generation it belongs to.

## Output contract

- Add `--json` (or set `OVIS_JSON=1`) for machine output. The mode never depends on
  whether stdout is a terminal.
- Success: one JSON document on stdout, nothing on stderr, exit 0.
- Failure: nothing on stdout, one `{"error": {...}}` object on stderr, and a non-zero
  exit code from the table below. `error.kind` is the stable token to branch on;
  `error.origin` is `cli` when this binary refused or failed, `server` when the
  runtime returned a typed error (its `data` fields are passed through).
- The command line is parsed and every typed parameter is checked before any socket
  is opened. A usage error exits 2 without contacting the endpoint.

## Discovery (offline, side-effect free)

- `ovis-terminal --skill` prints this manual.
- `ovis-terminal schema` prints the OpenRPC document this build speaks (same bytes as
  `schema/ovis-terminal.openrpc.json` of its source tree).
- `ovis-terminal schema cli` prints every command, flag, environment variable and exit
  code of this binary as JSON.
- `ovis-terminal endpoint` prints the endpoint this invocation resolves to
  (`OVIS_TERMINAL_SOCKET`, `OVIS_TERMINAL_SESSION`, `XDG_RUNTIME_DIR`, `XDG_STATE_HOME`,
  `HOME`, `USERNAME`) without touching it.

None of these, and no other command, starts, installs or restarts a server.

## Commands that reach the endpoint

- `ovis-terminal server status` reports `running`, `stopped`, `unreachable` or
  `stale_endpoint`. A stopped server stays stopped.
- `ovis-terminal terminal list` lists terminals.
- `ovis-terminal terminal get --terminal-id ID | --address ADDR | --name NAME` reads one
  record. Without a target it exits 7 with `needs_target` and `candidates`; it never
  picks a terminal for you. `candidates_listed` is false when no server answered.
- `ovis-terminal op get --op-id ID` and `ovis-terminal op cancel --op-id ID` inspect or
  cancel an operation.
- `ovis-terminal call METHOD --params JSON` sends any published method except `hello`.
- `ovis-terminal connect` relays stdin and stdout to the endpoint; stdout carries
  protocol bytes only.

The bearer credential is read from `OVIS_TERMINAL_CREDENTIAL` and is never printed.

## Exit codes

| code | name | kinds |
|---|---|---|
| 0 | success | |
| 1 | failure | any kind not listed below |
| 2 | usage | usage, invalid_params, unknown_method, invalid_json_switch, endpoint configuration kinds |
| 3 | not_found | not_found, template_not_found, parent_not_found, stream_not_found |
| 4 | unavailable | unavailable, server_not_running, stale_endpoint, method_not_found, machine_unreachable, unsupported_transport, backpressure, recovering, not_ready |
| 7 | needs_target | needs_target, ambiguous_target |
| 9 | blocked | blocked |
| 10 | cancelled | cancelled |
| 11 | conflict | conflict and state conflicts such as op_reused or already_running |
| 12 | forbidden | forbidden, unauthenticated and credential kinds |
| 13 | outcome_unknown | outcome_unknown, deadline_exceeded: inspect the operation |

`ovis-terminal schema cli` lists the exact kinds of each code.
