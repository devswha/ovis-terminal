# Producer onboarding

A producer is a program running inside an ovis-terminal terminal that tells the runtime
what it is doing: a coding agent, a build tool, a test runner. This guide covers the
whole path: registration, authenticated reports, declared readiness, typed prompts,
history, ownership and revocation, and what a trusted extension may and may not do. It
ends with a synthetic producer that the `cli/manual` conformance scenario executes
against the real binary.

Every command below is executed, in order, against a running `serve` of the build this file
ships with, and every JSON block marked `// params of <method>` is sent to it with its
placeholders filled in. The CLI crate's tests require each one to succeed (exit 0) unless the
line ends with `# exits <code> <kind>`.

Every mutation takes a client-minted `op_id`. Mint a current one with `ovis-terminal op new`
(the examples write `--op-id "$(ovis-terminal op new)"`): the server refuses an id older than
its operation retention window as `op_expired`, so a hard-coded id stops working. Keep the id
if you may need to resend the same mutation; resending it never repeats the effect. `ovis-terminal schema contract` is the full
reference.

## 1. What the runtime gives a spawned program

When the runtime spawns a program (`terminal.run`, or a template launch), it puts one
variable in the program's environment:

- `OVIS_TERMINAL_PROCESS_TOKEN`: an opaque token bound to this process incarnation of
  this terminal. It is the program's proof of ownership. It is not a credential: it
  grants no access by itself, and the connection still needs a bearer
  (`OVIS_TERMINAL_CREDENTIAL`) with the `report` permission.

The token is never printed by `ovis-terminal`. Treat it like a password: do not log it,
do not pass it to child processes you do not trust, and do not show it on screen.

The launcher passes the endpoint (`OVIS_TERMINAL_SOCKET`), a credential and the
terminal's address to the program through the `env` of `terminal.run` when it wants the
program to report; the runtime never guesses them.

## 2. Register

Registration declares that the program owning the terminal is an agent. Any label is
accepted; there is no brand list and no detection.

```sh
ovis-terminal --json agent register --address t1 --op-id "$(ovis-terminal op new)" --params '{"label":"synthetic-producer","session":null}'
```

The CLI fills `process_token` from `OVIS_TERMINAL_PROCESS_TOKEN`. An incarnation keeps its
first registration: registering again answers `agent_conflict` (exit 1). To bind or replace
the provider session reference later, use `agent update-session`; over the socket its params
are as below, with `<op id from ovis-terminal op new>` replaced by a fresh id and
`<OVIS_TERMINAL_PROCESS_TOKEN>` by the program's token:

```json
// params of agent.update_session
{"op_id": "<op id from ovis-terminal op new>", "target": {"address": "t1"}, "process_token": "<OVIS_TERMINAL_PROCESS_TOKEN>", "session": {"source": "synthetic-store", "kind": "id", "value": "s-42"}}
```

`agent get` then returns the record; on a terminal whose program never registered it
returns `agent: null`, and `not_found` means the terminal itself does not exist.

```sh
ovis-terminal --json agent get --address t1
```

## 3. Report status

Status has exactly two writers: OSC 7501 sequences the program prints to its own PTY,
and `status report` presented with its process token. Each report carries a `seq` (a
decimal string) that must increase; a report at or below the current sequence is
refused with the current one, so a producer always learns whether its report applied.

```sh
ovis-terminal --json status report --address t1 --op-id "$(ovis-terminal op new)" --params '{"seq":"1","record":{"state":"working","msg":"reading the repository"}}'
ovis-terminal --json status get --address t1
```

```json
// params of status.report
{"op_id": "<op id from ovis-terminal op new>", "target": {"address": "t1"}, "process_token": "<OVIS_TERMINAL_PROCESS_TOKEN>", "seq": "2", "record": {"state": "idle", "msg": "waiting for input"}}
```

A report stands until the producer replaces or clears it, or the process exits. The
runtime never overrides it from screen text, process names or silence. A turn ends when
the root record leaves `working`; `turn get` and `turn wait` read it.

## 4. Declare readiness

A launch with a first message (`terminal.run` with `first_message`, or a template whose
contract is `{"by": "declared", "states": ["idle"]}`) holds the message until the
program declares one of the ready states. Output silence is never readiness. Declare it
with an ordinary report:

```sh
ovis-terminal --json status report --address t1 --op-id "$(ovis-terminal op new)" --params '{"seq":"3","record":{"state":"idle"}}'
```

If the program never declares readiness, the message expires at its deadline and the
screen is left untouched.

## 5. Open typed prompts

A producer that needs a decision opens a versioned prompt instead of printing a question
and hoping a client parses the screen.

```json
// params of prompt.open
{"op_id": "<op id from ovis-terminal op new>", "target": {"address": "t1"}, "process_token": "<OVIS_TERMINAL_PROCESS_TOKEN>", "prompt_id": "apply-patch", "prompt": {"kind": "permission", "text": "Apply the patch to 3 files?", "choices": [{"id": "yes", "label": "Apply"}, {"id": "no", "label": "Skip"}], "mechanism": "rpc"}}
```

A client with the prompt-answer grant answers at the version it saw; a second answer to
the same version is a conflict, never a double delivery:

```json
// params of prompt.answer
{"op_id": "<op id from ovis-terminal op new>", "prompt_id": "apply-patch", "version": "1", "answer": {"choice": "yes"}, "lease_id": null}
```

```sh
ovis-terminal --json prompt list --address t1
ovis-terminal --json prompt get --params '{"prompt_id":"apply-patch"}'
```

Prompt text is data. Clients render it; nothing executes it, and `ovis-terminal` text
output escapes its control characters.

## 6. History and records

A producer that keeps structured conversation records appends them with its token; they
are read back by cursor. Provider sessions in the authorized store are listed as inert data.

```json
// params of agent.append_records
{"op_id": "<op id from ovis-terminal op new>", "target": {"address": "t1"}, "process_token": "<OVIS_TERMINAL_PROCESS_TOKEN>", "records": [{"record_id": "u1", "role": "user", "blocks": [{"type": "text", "text": "fix the build"}]}, {"record_id": "a1", "role": "assistant", "blocks": [{"type": "text", "text": "on it"}]}]}
```

```json
// params of agent.records
{"target": {"address": "t1"}}
```

```sh
ovis-terminal --json agent records --address t1
ovis-terminal --json agent history
```

Provider-store records are conversation history only. They never become lifecycle
status.

## 7. Ownership and revocation

- A token belongs to one process incarnation of one terminal. A report, prompt, record
  or registration presenting another terminal's token is refused and changes nothing:
  `not_owner` from the status, agent and record lanes, `forbidden` from the prompt bridge
  gate; both exit 12. A restarted program gets a new token; the old one stops working.
- A registration belongs to the process incarnation that made it. This build serves no
  explicit release: `agent.release` (like `agent.note` and `agent.wait_children`) is a
  published method `serve` explicitly excludes. Hello does not advertise it, and a call
  answers `not_implemented` with a hint naming the follow-up; the binding ends when its
  program exits. `agent.list` is served and lists every registered binding.
- A producer withdraws what it reported with `status clear`; the terminal keeps running.
- An attach client gives up its leases by closing its stream (`attach.close`, or Ctrl-\
  in `ovis-terminal terminal attach`); `attach.leases` shows who holds a terminal now.
- Credentials are revoked with `credential.revoke`; a revoked credential cannot report
  even with a valid process token.

```sh
ovis-terminal --json status clear --address t1 --op-id "$(ovis-terminal op new)"
ovis-terminal --json status get --address t1
ovis-terminal --json call attach.leases --params '{"target":{"address":"t1"}}'
```

```json
// params of status.clear
{"op_id": "<op id from ovis-terminal op new>", "target": {"address": "t1"}, "process_token": "<OVIS_TERMINAL_PROCESS_TOKEN>"}
```

## 8. Trusted-extension boundaries

An in-process extension (a prompt widget bridge) is trusted with the program's token and
nothing more:

- it reports only for its own terminal; the prompt bridge gate refuses a token that names
  no current program before any lane decodes the request;
- it cannot answer its own prompts without the prompt-answer grant;
- it never sends raw input, takes a lease or resizes a terminal unless its credential
  carries those permissions explicitly;
- it adds no brand rules, screen detectors or downloaded manifests to the runtime.

## 9. Synthetic producer, executed

The `cli/manual` conformance scenario runs this flow against the real binary in an
isolated root. A terminal runs a shell that prints its own token, and the scenario then:

1. registers `synthetic-producer` and reports `working` with that token;
2. reads the agent back through `ovis-terminal agent get --json` through a pipe and under
   a PTY, and requires the same JSON channels in both;
3. reports with the token of a second terminal for the first one and requires
   `not_owner` with the first terminal's status unchanged;
4. runs `ovis-terminal agent get --json` without a target and requires exit 7 with
   `needs_target` and the live candidates.

Run it:

```text
cargo run --locked --manifest-path conformance/Cargo.toml --bin conformance -- --binary target/debug/ovis-terminal --scenario cli/manual --isolated --case all --evidence evidence.json
```
