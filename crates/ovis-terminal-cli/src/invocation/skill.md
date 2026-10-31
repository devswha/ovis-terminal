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

None of these, and no inspection command, starts, installs or restarts a server. Only
the explicit `serve` and `hold` verbs below run one, in the foreground.

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

## Serving (Unix)

- `ovis-terminal serve [--root DIR] [--stdin-lifetime]` runs the control server in the
  foreground: endpoint, authority, stores, a shared holder and every routed method.
  With `--root` the socket is `DIR/run/s.sock` and state lives in `DIR/state`;
  otherwise the resolved endpoint is used. It prints one line
  `{"ready": {"socket", "bearer", "observer"}}` (owner and observe-only bearers) before
  serving, stops on SIGINT, SIGTERM or, with `--stdin-lifetime`, end of stdin, and then
  prints one `{"report": {...}}` line. Programs it launched end with it.
- hello advertises exactly the methods `serve` routes: `server.status`, `terminal.run`,
  `terminal.list`, `terminal.get`, `op.get`, `input.raw` and `attach.open`, `credit`,
  `close`, `repaint`, `resize`, `leases`. Every other catalog method answers
  `method_not_found`.
- `ovis-terminal hold [--session S]` runs the session holder in the foreground, owned by
  no service manager.

## Attaching (Unix)

- `ovis-terminal terminal attach --target ADDR | --terminal-id ID | --name NAME` attaches
  this terminal: stdout carries the program's screen, the terminal is raw while
  attached, typed bytes go to the program only under the interact lease, window
  resizes are applied under the geometry lease, and Ctrl-\ detaches. Ctrl-C goes to
  the program. It needs a TTY on stdin and stdout (`not_a_tty`, exit 2, otherwise).
- Another holder is never displaced: a held lease exits 11 with `lease_held` and the
  holder's lease record in `holder`.
- `--observe` needs no TTY, never reads stdin and never writes input.
- The outcome record goes to stderr: `{"detached": {"reason", "stream_closed", ...}}`
  (exit 0) for Ctrl-\, end of stdin, SIGINT, SIGTERM or SIGHUP, or the error record
  for a refusal; a lost connection exits 4 with `connection_lost`. The outer
  terminal's settings are restored on every one of these.

## Holder service (Unix)

The session holder owns every PTY and outlives the command that started it. Only
`status` is read-only; every other verb is an explicit mutation and names its manager.

- `ovis-terminal service status [--session S]` reports `running`, `stopped`,
  `unreachable` or `stale_endpoint` from the discovery record, the recorded process's
  start token and one connection attempt. It never starts, installs or repairs anything,
  and lists `missing_prerequisites` for surviving a logout.
- `ovis-terminal service install --manager systemd-user|launchd` writes and loads the
  service definition; `ovis-terminal service uninstall --manager ...` removes only a
  definition this runtime wrote, after the holder stopped.
- `ovis-terminal service start --manager systemd-user|launchd|detached` starts the
  holder. `detached` puts it in its own session with no manager: it survives the
  starting command and its terminal, but not a logout that ends every user process.
- `ovis-terminal service stop` stops the holder and lists the holder process and every
  terminal and process it ended. A stale endpoint is reported, not cleaned; the next
  `service start` replaces an endpoint whose process is proven gone.
- `ovis-terminal service run` is the holder process itself, executed by the manager or
  by `start`; do not run it by hand.

## Exit codes

| code | name | kinds |
|---|---|---|
| 0 | success | |
| 1 | failure | any kind not listed below |
| 2 | usage | usage, invalid_params, unknown_method, invalid_json_switch, not_a_tty, endpoint and service path configuration kinds |
| 3 | not_found | not_found, template_not_found, parent_not_found, stream_not_found |
| 4 | unavailable | unavailable, server_not_running, stale_endpoint, method_not_found, machine_unreachable, unsupported_transport, backpressure, recovering, not_ready, service_manager_unavailable, service_unsupported, connection_lost |
| 7 | needs_target | needs_target, ambiguous_target |
| 9 | blocked | blocked |
| 10 | cancelled | cancelled |
| 11 | conflict | conflict and state conflicts such as op_reused, already_running or lease_held |
| 12 | forbidden | forbidden, unauthenticated, credential kinds and attach_refused |
| 13 | outcome_unknown | outcome_unknown, deadline_exceeded, manager_timeout, holder_not_ready: inspect the operation or the service status |

`ovis-terminal schema cli` lists the exact kinds of each code.
