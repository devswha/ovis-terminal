# ovis-terminal agent manual

`ovis-terminal` is the agent-facing client of the ovis-terminal runtime. It draws
nothing and never prompts. This manual ships inside the binary and describes exactly
this build; `ovis-terminal --skill --json` returns it with the version and protocol
generation it belongs to, and `ovis-terminal schema` returns the protocol of the same
version.

## Output contract

- Add `--json` (or set `OVIS_JSON=1`) for machine output. The mode never depends on
  whether stdout is a terminal: under a PTY and through a pipe the channels are the same.
- Success: one JSON document on stdout, nothing on stderr, exit 0.
- Failure: nothing on stdout, one `{"error": {...}}` object on stderr, and a non-zero
  exit code from the table below. `error.kind` is the stable token to branch on;
  `error.origin` is `cli` when this binary refused or failed, `server` when the
  runtime returned a typed error (its `data` fields are passed through); `error.hint`
  says what to do next and `error.exit_code` repeats the exit code.
- The command line is parsed and every typed parameter is checked before any socket
  is opened. A usage error exits 2 without contacting the endpoint.
- Text mode is for people. A string value is printed with its control characters
  escaped, so program text (a prompt, a screen line) can never start a line of its own
  or reach your terminal as a control sequence. Never pipe output into a shell.

## Discovery (offline, side-effect free)

- `ovis-terminal --skill` prints this manual.
- `ovis-terminal schema` prints the OpenRPC document this build speaks (same bytes as
  `schema/ovis-terminal.openrpc.json` of its source tree); `info.version` is the version
  `--skill --json` reports.
- `ovis-terminal schema cli` prints every command, flag, environment variable and exit
  code of this binary as JSON.
- `ovis-terminal schema contract` prints the runtime contract reference: every method,
  event, error kind, limit, role and exit code, generated from this build's types.
- `ovis-terminal endpoint` prints the endpoint this invocation resolves to
  (`OVIS_TERMINAL_SOCKET`, `OVIS_TERMINAL_SESSION`, `XDG_RUNTIME_DIR`, `XDG_STATE_HOME`,
  `HOME`, `USERNAME`) without touching it.

None of these, and no inspection command, starts, installs or restarts a server. Only
the explicit `serve` and `hold` verbs below run one, in the foreground.

## Credentials and roles

- The bearer credential is read from `OVIS_TERMINAL_CREDENTIAL` and is never printed.
  `serve` prints an owner bearer and an observe-only bearer when it starts.
- A credential grants permissions; roles bundle them. `observer` may read and subscribe,
  `interactor` may also type, resize, launch and change layout, and `owner` holds every
  permission, including takeover, prompt answers, raw input, reports and administration.
  `ovis-terminal schema contract` lists the permission every action needs.
- A program the runtime spawned finds its process token in `OVIS_TERMINAL_PROCESS_TOKEN`.
  The token proves which program owns a terminal; the verbs that report as that program
  send it automatically when `--params` names none. A token of another terminal is
  refused (`not_owner`, or `forbidden` at the prompt bridge gate; exit 12) and changes
  nothing.

## Commands that reach the endpoint

- `ovis-terminal server status` reports `running`, `stopped`, `unreachable` or
  `stale_endpoint`. A stopped server stays stopped.
- `ovis-terminal server metrics` reads stream, queue, lag and latency metrics.
- `ovis-terminal server stop --op-id ID --params '{"scope": ...}'` and
  `ovis-terminal server restart --op-id ID` are explicit mutations.
- `ovis-terminal terminal list` lists terminals.
- `ovis-terminal terminal get --terminal-id ID | --address ADDR | --name NAME` reads one
  record. Without a target it exits 7 with `needs_target` and `candidates`; it never
  picks a terminal for you. `candidates_listed` is false when no server answered.
- `ovis-terminal op get --op-id ID` and `ovis-terminal op cancel --op-id ID` inspect or
  cancel an operation.
- `ovis-terminal call METHOD --params JSON` sends any published method except `hello`.
- `ovis-terminal connect` relays stdin and stdout to the endpoint; stdout carries
  protocol bytes only.

## Typed agent verbs

Each verb sends one published method. Its flags are exactly what that method's closed
params type takes: `--op-id ID` for a mutation, one of `--terminal-id`, `--address` or
`--name` for a terminal-scoped method, and `--params JSON` for every other field. A
field given both as a flag and in `--params` is a usage error. A terminal-scoped verb
whose other fields are valid but that names no target exits 7 with `needs_target` and
`candidates`; a malformed one exits 2 first, before any connection.

- Agents: `ovis-terminal agent register`, `ovis-terminal agent get`,
  `ovis-terminal agent update-session`, `ovis-terminal agent children`,
  `ovis-terminal agent conversation`, `ovis-terminal agent append-records`,
  `ovis-terminal agent records`, `ovis-terminal agent history`.
- Status: `ovis-terminal status get`, `ovis-terminal status report`,
  `ovis-terminal status clear`. A report with a `seq` not above the current one is
  refused with the current sequence.
- Prompts: `ovis-terminal prompt list`, `ovis-terminal prompt get`,
  `ovis-terminal prompt open`, `ovis-terminal prompt answer`, `ovis-terminal prompt close`.
- Turns: `ovis-terminal turn get`, `ovis-terminal turn wait`.
- Templates: `ovis-terminal template list`, `ovis-terminal template get`.

Every verb sends a method this build's `serve` routes; `ovis-terminal --json schema cli`
names each verb's `method`, and `hello.methods` lists what a server routes. The catalog
methods `agent.list`, `agent.note`, `agent.release` and `agent.wait_children` are not
served by this build: they have no verb, and `ovis-terminal call` with them answers
`method_not_found` (exit 4).

Examples (the CLI crate's tests execute every line against `serve` of this build):

```sh
ovis-terminal --json agent get --address t1
ovis-terminal --json agent register --address t1 --op-id 01900000-0000-7000-8000-000000000001 --params '{"label":"my-agent","session":null}'
ovis-terminal --json status report --address t1 --op-id 01900000-0000-7000-8000-000000000002 --params '{"seq":"1","record":{"state":"working"}}'
ovis-terminal --json prompt list --address t1
ovis-terminal --json turn wait --address t1 --params '{"turn":"1","timeout_ms":"1000"}'
```

`agent get` answers `{"agent": null, ...}` for a terminal that runs a plain program;
`not_found` means the terminal does not exist.

## Serving (Unix)

- `ovis-terminal serve [--root DIR] [--stdin-lifetime]` runs the control server in the
  foreground: endpoint, authority, stores, a shared holder and every routed method.
  With `--root` the socket is `DIR/run/s.sock` and state lives in `DIR/state`;
  otherwise the resolved endpoint is used. It prints one line
  `{"ready": {"socket", "bearer", "observer"}}` (owner and observe-only bearers) before
  serving, stops on SIGINT, SIGTERM or, with `--stdin-lifetime`, end of stdin, and then
  prints one `{"report": {...}}` line. Programs it launched end with it.
- `hello` lists exactly the methods `serve` routes; every other catalog method answers
  `method_not_found`, never a placeholder result.
- `ovis-terminal hold [--session S]` runs the session holder in the foreground, owned by
  no service manager.

## Attaching (Unix)

- `ovis-terminal terminal attach --target ADDR | --terminal-id ID | --name NAME` attaches
  this terminal: stdout carries the program's screen, the terminal is raw while
  attached, typed bytes go to the program only under the interact lease, window
  resizes are applied under the geometry lease, and Ctrl-\ detaches. Ctrl-C goes to
  the program. It needs a TTY on stdin and stdout (`not_a_tty`, exit 2, otherwise).
- There is no prefix key and no client mode: every byte other than Ctrl-\, unknown
  escape sequences and non-UTF-8 bytes included, reaches the program unchanged through
  `input.raw`, or the server's typed refusal ends the attach.
- Another holder is never displaced: a held lease exits 11 with `lease_conflict`, the
  holder's lease record in `holder` and the lease change to retry after in `retry`.
- The lease belongs to the connection that acquired it and is recorded under
  `--client-id`, which defaults to `cli-<pid>`. A second attach is always another owner,
  even under the same `--client-id`: it exits 11 with `lease_conflict` and cannot type.
- `--takeover EPOCH` displaces the interact holder only at its current epoch and with
  the takeover grant; a stale epoch exits 11 with `lease_epoch_mismatch`, the epoch
  sent in `expected_epoch` and the current lease in `current`.
- `--observe` needs no TTY, never reads stdin, never writes input and never resizes;
  on a terminal, Ctrl-\ (SIGQUIT) detaches it.
- The outcome record goes to stderr: `{"detached": {"reason", "stream_closed",
  "leases_released", "leases_held", "displaced", ...}}` (exit 0) for Ctrl-\, a hung-up
  terminal, SIGINT, SIGTERM, SIGHUP or SIGQUIT, or the error record for a refusal; a
  signal before the stream is attached exits 10 with `interrupted` and its `stage`; a
  lost connection exits 4 with `connection_lost`. The outer terminal's settings, and
  the modes the program switched on (alternate screen, mouse, bracketed paste, cursor
  keys, keyboard protocol, hidden cursor), are restored on every one of these.

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
| 3 | not_found | not_found, template_not_found, parent_not_found, stream_not_found, not_registered |
| 4 | unavailable | unavailable, server_not_running, stale_endpoint, method_not_found, machine_unreachable, unsupported_transport, backpressure, recovering, not_ready, service_manager_unavailable, service_unsupported, connection_lost |
| 7 | needs_target | needs_target, ambiguous_target |
| 9 | blocked | blocked |
| 10 | cancelled | cancelled, interrupted |
| 11 | conflict | conflict and state conflicts such as op_reused, already_running, lease_conflict or lease_epoch_mismatch |
| 12 | forbidden | forbidden, unauthenticated, not_owner, credential kinds and attach_refused |
| 13 | outcome_unknown | outcome_unknown, deadline_exceeded, manager_timeout, holder_not_ready: inspect the operation or the service status |

`ovis-terminal schema cli` lists the exact kinds of each code, and
`ovis-terminal schema contract` lists every server error kind with its recovery.
