# Holder service lifetime and endpoint discovery (Unix)

Plan todo 51. Closes GAP-001, GAP-016, GAP-017 and GAP-121 for Linux and macOS;
delivers IDEAL-001, IDEAL-014, IDEAL-093 and IDEAL-099 there. Windows services are
todo 54.

## What runs where

A session's shared holder (the process that owns every PTY) runs as its own service
process, `ovis-terminal service run --session S --manager M`. It is never a child that
lives or dies with the command, SSH session or client that asked for it:

| `--manager` | Owner | Survives its starter | Survives logout |
|---|---|---|---|
| `systemd-user` | systemd user unit `ovis-terminal-holder-S.service` (`Type=notify`, `Restart=no`) | yes | only with user lingering (`loginctl enable-linger`) |
| `launchd` | launchd agent `dev.ovis-terminal.holder.S` (`RunAtLoad`, `KeepAlive=false`) | yes | while the `gui/<uid>` domain exists |
| `detached` | nobody: its own session (`setsid`) | yes | no: a login manager that ends every user process ends it |

`service status` reports what is missing for logout survival in
`missing_prerequisites` (`service_manager`, `systemd_user_manager`, `user_linger`,
`launchd_user_domain`). A detached holder always lists `service_manager`: a detached
spawn is not logout survival, and the runtime never claims it is.

No holder restarts on its own. Relaunching a program whose survival is unproven is a
separate decision (todo 49).

## Paths

All paths come from explicit variables; there is no `/tmp` fallback.

| File | Location |
|---|---|
| Holder socket | `$XDG_RUNTIME_DIR` (else `$XDG_STATE_HOME`, else `$HOME/.local/state`) `/ovis-terminal/holder-S.sock` |
| Discovery record | same directory, `holder-S.json`, mode 0600, directory mode 0700 |
| Start lock | same directory, `holder-S.lock`, mode 0600; never removed |
| systemd unit | `$XDG_CONFIG_HOME` (else `$HOME/.config`) `/systemd/user/ovis-terminal-holder-S.service` |
| launchd agent | `$HOME/Library/LaunchAgents/dev.ovis-terminal.holder.S.plist` |

`S` is 1 to 32 ASCII letters, digits, `-` or `_` (default `default`). A private
session name gives private labels and paths, which is how tests and fixtures stay apart
from a user's own service.

## The discovery record is not liveness

```json
{"format":"ovis-terminal-holder-endpoint/1","label":"ovis-terminal-holder-S",
 "manager":"systemd_user","transport":"unix","address":"/run/user/1000/ovis-terminal/holder-S.sock",
 "boot_proof":{"pid":4242,"start_token":"linux:<boot id>:<start ticks>"}}
```

The holder writes it atomically after its socket accepts, and withdraws it (with the
socket) on a clean stop, but only while the record still names its own boot proof.
Status turns the record into a state:

| Observation | State |
|---|---|
| no record, nothing at the socket path | `stopped` |
| recorded PID has the recorded start token and the socket accepts | `running` |
| recorded process alive but the socket is missing or refuses; or an unrecorded socket accepts | `unreachable` |
| recorded PID gone (`process_gone`) or reused (`process_replaced`) | `stale_endpoint` |
| unrecorded socket refuses (`socket_refused`), or the path is not a socket (`not_a_socket`) | `stale_endpoint` |
| record of another format or shape | error `descriptor_unsupported` / `descriptor_malformed`, never a default |

Starting is serialized: a holder takes an exclusive `flock` on the session's start lock, then judges the endpoint, clears it only if proven dead, binds and publishes its record before releasing the lock. A concurrent starter waits, then observes that record and exits with `already_running` carrying the live holder's `holder` boot proof (exit 11); a detached launcher relays that typed refusal. The record is withdrawn under the same lock.

Observing sends nothing: one non-blocking connection attempt bounded at 500 ms, then
close. It never creates a directory, binds, removes, adopts or starts anything, and never
registers a service. A stale endpoint is left in place as evidence; only an explicit
`service start` replaces an endpoint whose process is proven gone.

## Verbs

| Command | Effect |
|---|---|
| `service status` | read-only, as above |
| `service install --manager M` | refuses if the manager does not answer (no file written) or a definition at the path is not ours; writes the definition, reloads systemd |
| `service uninstall --manager M` | removes only a definition carrying our `X-OvisTerminalLabel=` marker, after the holder stopped |
| `service start --manager M` | `systemctl --user start` (returns after `READY=1`), `launchctl bootstrap` (then bounded wait for `running`), or a detached launch that waits for the holder's ready line |
| `service stop` | adopts the holder, lists every held terminal and process in the receipt, sends `Goodbye{shutdown}`, waits for the record to be withdrawn, then settles the manager |

Every manager call is bounded (5 s queries, 20 s start). On expiry only the spawned
manager child is killed, through its own handle, and the result is
`manager_timeout` (exit 13): the effect is unknown, so inspect `service status`.

`server status` is the control server's state and is independent: it stays `stopped`
while a holder runs, and inspecting either never starts the other (EXCL-027, the
RT-HC-04 trap).

## Server lane

`service.describe` (dispatch lane `dispatch::lanes::unix_service::ServiceDescribe`) lists
the registrations this runtime wrote for its configured sessions, with the endpoint
address and unmet prerequisites. The endpoint descriptor carries no boot identity: a
registration is not a running holder. The served binary that registers the lane is
assembled by todo 31.
